//! Webhook health-gate alerts. When a gate transitions to `Fail`
//! (or back to `Pass` after being broken), the cockpit POSTs a small
//! JSON object to the operator-configured webhook URL. Slack and
//! Discord-compatible — both accept the same `{"text": "..."}` shape
//! on their incoming-webhook URLs.
//!
//! ## Why
//!
//! Operators don't want to leave the cockpit open on a second monitor
//! to catch a gate failure overnight. A single Slack ping when
//! something flips red is the lowest-effort handoff to mobile / DMs.
//!
//! ## Design constraints
//!
//! * **Opt-in only.** `[alerts].webhook_url` defaults to absent.
//!   No surprise outbound traffic from a fresh install.
//! * **Debounced per-gate.** Each gate has its own 5-minute cool-down
//!   so a flapping `Reachability` doesn't pin the operator's Slack.
//! * **Read-only on Bee.** This module makes outbound HTTP only.
//!   No chain interaction, no Bee-API write.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use crate::views::health::{Gate, GateStatus};

/// Default debounce window per-gate. After firing an alert for gate
/// `X`, we won't fire another alert for `X` until this elapses,
/// regardless of whether the gate flapped in the meantime.
pub const DEFAULT_DEBOUNCE_SECS: u64 = 5 * 60;

/// One transition the state-comparator detected. Each `Alert` becomes
/// one webhook POST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub gate: String,
    pub from: GateStatus,
    pub to: GateStatus,
    pub value: String,
    pub why: Option<String>,
}

impl Alert {
    /// Slack/Discord-compatible JSON body.
    pub fn json_body(&self) -> serde_json::Value {
        serde_json::json!({
            "text": self.message_line(),
        })
    }

    pub fn message_line(&self) -> String {
        let arrow = match (self.from, self.to) {
            (_, GateStatus::Fail) => "🔴 FAILED",
            (_, GateStatus::Warn) => "🟡 WARN",
            (_, GateStatus::Pass) => "🟢 RECOVERED",
            (_, GateStatus::Unknown) => "⚪ UNKNOWN",
        };
        let mut s = format!(
            "bee-tui: {} {} (was {:?}, now {:?}) — {}",
            arrow, self.gate, self.from, self.to, self.value,
        );
        if let Some(why) = &self.why {
            s.push_str(" · ");
            s.push_str(why);
        }
        s
    }

    /// True for transitions worth pinging a **webhook** on.
    /// Transitions to/from `Unknown` are ignored — that's "data not
    /// loaded yet" and flapping during startup would spam.
    pub fn is_worth_alerting(&self) -> bool {
        if self.from == GateStatus::Unknown || self.to == GateStatus::Unknown {
            return false;
        }
        self.from != self.to
    }

    /// True for transitions the in-cockpit **notification center**
    /// should surface. A superset of [`Self::is_worth_alerting`]: it
    /// additionally includes the *first* time a gate is observed in
    /// an adverse state (`Unknown → Warn/Fail`).
    pub fn is_worth_notifying(&self) -> bool {
        self.is_worth_alerting()
            || (self.from == GateStatus::Unknown
                && matches!(self.to, GateStatus::Warn | GateStatus::Fail))
    }
}

#[derive(Debug, Default)]
pub struct AlertState {
    last_status: HashMap<String, GateStatus>,
    last_fired: HashMap<String, SystemTime>,
    debounce: Duration,
}

impl AlertState {
    pub fn new(debounce_secs: u64) -> Self {
        Self {
            last_status: HashMap::new(),
            last_fired: HashMap::new(),
            debounce: Duration::from_secs(debounce_secs),
        }
    }

    pub fn diff_and_record(&mut self, current: &[Gate]) -> Vec<Alert> {
        self.diff_and_record_at(current, SystemTime::now())
    }

    /// Test seam — same as [`Self::diff_and_record`] but uses the
    /// supplied `now` instead of wall-clock time.
    pub fn diff_and_record_at(&mut self, current: &[Gate], now: SystemTime) -> Vec<Alert> {
        let mut out = Vec::new();
        for gate in current {
            let prev = self
                .last_status
                .get(gate.label)
                .copied()
                .unwrap_or(GateStatus::Unknown);
            self.last_status.insert(gate.label.to_string(), gate.status);
            let alert = Alert {
                gate: gate.label.to_string(),
                from: prev,
                to: gate.status,
                value: gate.value.clone(),
                why: gate.why.clone(),
            };
            if alert.is_worth_alerting() {
                if let Some(last) = self.last_fired.get(gate.label) {
                    if now.duration_since(*last).unwrap_or_default() < self.debounce {
                        continue;
                    }
                }
                self.last_fired.insert(gate.label.to_string(), now);
                out.push(alert);
            } else if alert.is_worth_notifying() {
                // Initial adverse observation — surfaced to the
                // notification center but does NOT consume the
                // debounce budget so a later real escalation fires.
                out.push(alert);
            }
        }
        out
    }
}

/// Fire a single alert at `webhook_url`. Returns `Ok(())` on 2xx;
/// `Err(reason)` on every other outcome.
pub async fn fire(webhook_url: &str, alert: &Alert) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("bee-cockpit-core/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("client build: {e}"))?;
    let resp = client
        .post(webhook_url)
        .json(&alert.json_body())
        .send()
        .await
        .map_err(|e| format!("POST {webhook_url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("webhook returned HTTP {}", resp.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(label: &'static str, status: GateStatus, value: &str) -> Gate {
        Gate {
            label,
            status,
            value: value.to_string(),
            why: None,
        }
    }

    #[test]
    fn first_observation_is_unknown_baseline_and_silent() {
        let mut s = AlertState::new(60);
        let out = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        assert!(out.is_empty(), "fresh start should be silent: {out:?}");
    }

    #[test]
    fn pass_to_fail_fires_alert() {
        let mut s = AlertState::new(60);
        let _ = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        let now = SystemTime::now();
        let out = s.diff_and_record_at(&[gate("Health", GateStatus::Fail, "broken")], now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].from, GateStatus::Pass);
        assert_eq!(out[0].to, GateStatus::Fail);
        assert!(out[0].message_line().contains("FAILED"));
    }

    #[test]
    fn fail_to_pass_fires_recovery() {
        let mut s = AlertState::new(60);
        let _ = s.diff_and_record(&[gate("Health", GateStatus::Fail, "broken")]);
        let out = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        assert_eq!(out.len(), 1);
        assert!(out[0].message_line().contains("RECOVERED"));
    }

    #[test]
    fn unchanged_status_is_silent() {
        let mut s = AlertState::new(60);
        let _ = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        let out = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        assert!(out.is_empty());
    }

    #[test]
    fn unknown_transitions_are_ignored() {
        let mut s = AlertState::new(60);
        let _ = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        let out = s.diff_and_record(&[gate("Health", GateStatus::Unknown, "")]);
        assert!(out.is_empty());
        let mut s2 = AlertState::new(60);
        let _ = s2.diff_and_record(&[gate("Health", GateStatus::Unknown, "")]);
        let out = s2.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        assert!(out.is_empty());
    }

    #[test]
    fn debounce_suppresses_repeat_within_window() {
        let mut s = AlertState::new(60);
        let _ = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        let t0 = SystemTime::now();
        let out = s.diff_and_record_at(&[gate("Health", GateStatus::Fail, "broken")], t0);
        assert_eq!(out.len(), 1);
        let t1 = t0 + Duration::from_secs(30);
        let _ = s.diff_and_record_at(&[gate("Health", GateStatus::Pass, "ok")], t1);
        let t2 = t0 + Duration::from_secs(45);
        let out = s.diff_and_record_at(&[gate("Health", GateStatus::Fail, "broken again")], t2);
        assert!(
            out.is_empty(),
            "second fail within 60s should be debounced: {out:?}"
        );
    }

    #[test]
    fn debounce_releases_after_window() {
        let mut s = AlertState::new(60);
        let _ = s.diff_and_record(&[gate("Health", GateStatus::Pass, "ok")]);
        let t0 = SystemTime::now();
        let _ = s.diff_and_record_at(&[gate("Health", GateStatus::Fail, "x")], t0);
        let _ = s.diff_and_record_at(
            &[gate("Health", GateStatus::Pass, "ok")],
            t0 + Duration::from_secs(61),
        );
        let out = s.diff_and_record_at(
            &[gate("Health", GateStatus::Fail, "y")],
            t0 + Duration::from_secs(122),
        );
        assert_eq!(out.len(), 1);
    }

    fn alert(from: GateStatus, to: GateStatus) -> Alert {
        Alert {
            gate: "Stamp TTL".into(),
            from,
            to,
            value: String::new(),
            why: None,
        }
    }

    #[test]
    fn is_worth_notifying_includes_initial_adverse_states() {
        assert!(!alert(GateStatus::Unknown, GateStatus::Warn).is_worth_alerting());
        assert!(alert(GateStatus::Unknown, GateStatus::Warn).is_worth_notifying());
        assert!(!alert(GateStatus::Unknown, GateStatus::Fail).is_worth_alerting());
        assert!(alert(GateStatus::Unknown, GateStatus::Fail).is_worth_notifying());
        assert!(!alert(GateStatus::Unknown, GateStatus::Pass).is_worth_notifying());
        assert!(!alert(GateStatus::Pass, GateStatus::Unknown).is_worth_notifying());
        assert!(alert(GateStatus::Pass, GateStatus::Warn).is_worth_notifying());
        assert!(!alert(GateStatus::Warn, GateStatus::Warn).is_worth_notifying());
    }

    #[test]
    fn diff_records_initial_adverse_gate_without_consuming_debounce() {
        let mut s = AlertState::new(60);
        let t0 = SystemTime::now();
        let out = s.diff_and_record_at(&[gate("Stamp TTL", GateStatus::Warn, "6d left")], t0);
        assert_eq!(
            out.len(),
            1,
            "initial adverse state should surface: {out:?}"
        );
        assert_eq!(out[0].from, GateStatus::Unknown);
        assert_eq!(out[0].to, GateStatus::Warn);
        let out = s.diff_and_record_at(
            &[gate("Stamp TTL", GateStatus::Fail, "20h left")],
            t0 + Duration::from_secs(5),
        );
        assert_eq!(out.len(), 1, "escalation must not be debounced: {out:?}");
        assert_eq!(out[0].from, GateStatus::Warn);
        assert_eq!(out[0].to, GateStatus::Fail);
    }

    #[test]
    fn json_body_uses_text_field() {
        let alert = Alert {
            gate: "Health".into(),
            from: GateStatus::Pass,
            to: GateStatus::Fail,
            value: "broken".into(),
            why: None,
        };
        let body = alert.json_body();
        assert!(body["text"].is_string(), "json: {body}");
        assert!(body["text"].as_str().unwrap().contains("FAILED"));
    }

    #[test]
    fn message_line_includes_why_when_present() {
        let alert = Alert {
            gate: "StorageRadius".into(),
            from: GateStatus::Pass,
            to: GateStatus::Warn,
            value: "below committed".into(),
            why: Some("decreases ONLY on the 30-min reserve worker tick".into()),
        };
        let s = alert.message_line();
        assert!(s.contains("30-min reserve worker tick"));
    }
}
