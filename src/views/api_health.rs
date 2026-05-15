//! S8 — RPC / API health view. Pure half of bee-tui's
//! `components::api_health`: call-stats percentile math over a
//! window of `LogEntry`s, the chain-state summary computed from the
//! `/chainstate` snapshot, and the pending-transactions table.
//! The renderer paints the colour gradients on top.

use crate::log_capture::LogEntry;
use crate::watch::{HealthSnapshot, TransactionsSnapshot};

/// Window of recent calls considered for the latency / error-rate
/// summary. Tracks the LogCapture's own ring-buffer capacity (200 in
/// `log_capture::install`) — lifting the cap above that just yields
/// the same numbers since older entries are gone.
pub const STATS_WINDOW: usize = 100;

/// Pending-tx age threshold above which the row colours warn-yellow.
/// 5 minutes — short enough that operators still see colour during a
/// normal Gnosis confirmation cycle (~10s/block, 6+ blocks for
/// finality), long enough that the threshold doesn't fire on every
/// healthy submission.
pub const PENDING_TX_WARN_AGE_SECS: i64 = 300;
/// Above this the row colours fail-red — at this point the operator
/// almost certainly needs to bump gas / cancel.
pub const PENDING_TX_FAIL_AGE_SECS: i64 = 1800;

/// Aggregated call statistics over a window of [`LogEntry`] records.
#[derive(Debug, Clone, PartialEq)]
pub struct CallStats {
    /// Number of entries that contributed (had `elapsed_ms` set).
    pub sample_size: usize,
    /// Median latency in milliseconds. `None` if `sample_size == 0`.
    pub p50_ms: Option<u64>,
    /// 99th-percentile latency in milliseconds. `None` if
    /// `sample_size == 0`.
    pub p99_ms: Option<u64>,
    /// Percentage of entries with `status >= 400`. `0.0` when no
    /// entries have a status code attached.
    pub error_rate_pct: f64,
}

/// Bee's view of the chain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChainStateView {
    pub block: Option<u64>,
    pub chain_tip: Option<u64>,
    /// `chain_tip - block`, surfaced separately so the renderer can
    /// colour-code it without re-doing the subtraction. Negative
    /// values shouldn't happen on a healthy node but are technically
    /// possible during chain reorgs — the field is signed for that.
    pub delta: Option<i64>,
    pub total_amount: Option<String>,
    pub current_price: Option<String>,
}

/// One row of the pending-transactions table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTxRow {
    pub nonce: u64,
    pub hash_short: String,
    pub to_short: String,
    /// Full transaction hash (with the `0x` prefix stripped).
    pub hash_full: String,
    /// Full destination address (`0x` stripped).
    pub to_full: String,
    /// RFC 3339 creation timestamp, rendered verbatim.
    pub created: String,
    pub description: String,
    /// Seconds elapsed since `created`. `None` when the timestamp
    /// failed to parse (or was empty).
    pub age_seconds: Option<i64>,
}

/// Aggregated view fed to renderer and snapshot tests.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiHealthView {
    pub bee_endpoint: String,
    pub call_stats: CallStats,
    pub chain: ChainStateView,
    pub pending: Vec<PendingTxRow>,
}

/// Pure view computation. The log entries arrive as a slice rather
/// than a `LogCapture` handle so tests can stub deterministic samples
/// without spinning up the global tracing layer.
pub fn view_for(
    bee_endpoint: &str,
    recent_calls: &[LogEntry],
    health: &HealthSnapshot,
    transactions: &TransactionsSnapshot,
) -> ApiHealthView {
    ApiHealthView {
        bee_endpoint: bee_endpoint.to_string(),
        call_stats: call_stats_for(recent_calls),
        chain: chain_state_view(health),
        pending: pending_rows(transactions),
    }
}

/// Compute call stats over the last [`STATS_WINDOW`] entries that
/// have `elapsed_ms` populated. Latency percentiles are computed via
/// nearest-rank on the sorted sample.
pub fn call_stats_for(entries: &[LogEntry]) -> CallStats {
    let recent: Vec<&LogEntry> = entries.iter().rev().take(STATS_WINDOW).collect();
    let total = recent.len();
    if total == 0 {
        return CallStats {
            sample_size: 0,
            p50_ms: None,
            p99_ms: None,
            error_rate_pct: 0.0,
        };
    }
    let mut latencies: Vec<u64> = recent.iter().filter_map(|e| e.elapsed_ms).collect();
    latencies.sort_unstable();
    let with_latency = latencies.len();
    let p50_ms = percentile(&latencies, 50);
    let p99_ms = percentile(&latencies, 99);
    let with_status: Vec<u16> = recent.iter().filter_map(|e| e.status).collect();
    let errors = with_status.iter().filter(|s| **s >= 400).count();
    let error_rate_pct = if with_status.is_empty() {
        0.0
    } else {
        (errors as f64) * 100.0 / (with_status.len() as f64)
    };
    CallStats {
        sample_size: with_latency,
        p50_ms,
        p99_ms,
        error_rate_pct,
    }
}

/// Nearest-rank percentile on a pre-sorted slice. Returns `None` for
/// the empty slice. `pct` is in `0..=100`.
fn percentile(sorted: &[u64], pct: u32) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    let rank = (pct as usize * n).div_ceil(100);
    let idx = rank.saturating_sub(1).min(n - 1);
    Some(sorted[idx])
}

fn chain_state_view(health: &HealthSnapshot) -> ChainStateView {
    let Some(cs) = &health.chain_state else {
        return ChainStateView::default();
    };
    let delta = (cs.chain_tip as i64) - (cs.block as i64);
    ChainStateView {
        block: Some(cs.block),
        chain_tip: Some(cs.chain_tip),
        delta: Some(delta),
        total_amount: Some(cs.total_amount.to_string()),
        current_price: Some(cs.current_price.to_string()),
    }
}

fn pending_rows(transactions: &TransactionsSnapshot) -> Vec<PendingTxRow> {
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    transactions
        .pending
        .iter()
        .map(|t| {
            let age_seconds = parse_rfc3339_to_unix(&t.created).map(|ts| now_unix - ts);
            PendingTxRow {
                nonce: t.nonce,
                hash_short: short_hex(&t.transaction_hash),
                to_short: short_hex(&t.to),
                hash_full: t.transaction_hash.trim_start_matches("0x").to_string(),
                to_full: t.to.trim_start_matches("0x").to_string(),
                created: t.created.clone(),
                description: t.description.clone(),
                age_seconds,
            }
        })
        .collect()
}

/// Parse Bee's RFC 3339 timestamp (`"2026-05-07T08:12:03Z"` or
/// `"2026-05-07T08:12:03+00:00"`) into seconds-since-Unix-epoch.
/// Returns `None` for malformed / empty input.
pub fn parse_rfc3339_to_unix(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|odt| odt.unix_timestamp())
}

/// Humanise `age_seconds` into `5s` / `2m 30s` / `8h 15m`. Negative
/// values (clock skew on the host) collapse to `now`. Returns `—`
/// for `None`.
pub fn format_age_humanised(age_seconds: Option<i64>) -> String {
    match age_seconds {
        None => "—".into(),
        Some(s) if s < 0 => "now".into(),
        Some(s) if s < 60 => format!("{s}s"),
        Some(s) if s < 3_600 => {
            let m = s / 60;
            let r = s % 60;
            format!("{m}m {r:>2}s")
        }
        Some(s) => {
            let h = s / 3_600;
            let m = (s % 3_600) / 60;
            format!("{h}h {m:>2}m")
        }
    }
}

fn short_hex(s: &str) -> String {
    let trimmed = s.trim_start_matches("0x");
    if trimmed.len() > 12 {
        format!("{}…{}", &trimmed[..6], &trimmed[trimmed.len() - 4..])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(method: &str, status: Option<u16>, elapsed_ms: Option<u64>) -> LogEntry {
        LogEntry {
            ts: String::new(),
            method: method.into(),
            url: "http://localhost:1633/".into(),
            status,
            elapsed_ms,
            message: String::new(),
        }
    }

    #[test]
    fn parse_rfc3339_z_form() {
        let ts = parse_rfc3339_to_unix("2026-05-07T08:12:03Z").expect("must parse");
        assert!(ts > 1_700_000_000);
    }

    #[test]
    fn parse_rfc3339_offset_form() {
        let ts = parse_rfc3339_to_unix("2026-05-07T08:12:03+00:00").expect("must parse");
        assert!(ts > 1_700_000_000);
    }

    #[test]
    fn parse_rfc3339_returns_none_on_garbage() {
        assert_eq!(parse_rfc3339_to_unix(""), None);
        assert_eq!(parse_rfc3339_to_unix("not a date"), None);
        assert_eq!(parse_rfc3339_to_unix("2026"), None);
    }

    #[test]
    fn format_age_humanised_seconds() {
        assert_eq!(format_age_humanised(Some(0)), "0s");
        assert_eq!(format_age_humanised(Some(45)), "45s");
        assert_eq!(format_age_humanised(Some(59)), "59s");
    }

    #[test]
    fn format_age_humanised_minutes() {
        assert_eq!(format_age_humanised(Some(60)), "1m  0s");
        assert_eq!(format_age_humanised(Some(125)), "2m  5s");
        assert_eq!(format_age_humanised(Some(3_599)), "59m 59s");
    }

    #[test]
    fn format_age_humanised_hours() {
        assert_eq!(format_age_humanised(Some(3_600)), "1h  0m");
        assert_eq!(format_age_humanised(Some(8 * 3_600 + 15 * 60)), "8h 15m");
    }

    #[test]
    fn format_age_humanised_special_cases() {
        assert_eq!(format_age_humanised(None), "—");
        assert_eq!(format_age_humanised(Some(-3)), "now");
    }

    #[test]
    fn call_stats_empty_sample() {
        let stats = call_stats_for(&[]);
        assert_eq!(stats.sample_size, 0);
        assert_eq!(stats.p50_ms, None);
        assert_eq!(stats.p99_ms, None);
        assert_eq!(stats.error_rate_pct, 0.0);
    }

    #[test]
    fn call_stats_all_successful() {
        let entries: Vec<LogEntry> = (1..=100)
            .map(|i| entry("GET", Some(200), Some(i)))
            .collect();
        let stats = call_stats_for(&entries);
        assert_eq!(stats.sample_size, 100);
        assert_eq!(stats.p50_ms, Some(50));
        assert_eq!(stats.p99_ms, Some(99));
        assert_eq!(stats.error_rate_pct, 0.0);
    }

    #[test]
    fn call_stats_mixed_errors() {
        let mut entries: Vec<LogEntry> = (1..=10)
            .map(|i| entry("GET", Some(200), Some(i * 10)))
            .collect();
        entries.push(entry("POST", Some(500), Some(50)));
        entries.push(entry("POST", Some(404), Some(15)));
        let stats = call_stats_for(&entries);
        assert!((stats.error_rate_pct - 16.666_666_666_666_668).abs() < 1e-9);
    }

    #[test]
    fn percentile_single_element() {
        assert_eq!(percentile(&[42], 50), Some(42));
        assert_eq!(percentile(&[42], 99), Some(42));
    }

    #[test]
    fn percentile_empty_returns_none() {
        assert_eq!(percentile(&[], 50), None);
    }

    #[test]
    fn short_hex_truncates_long_address() {
        let s = short_hex("0xabcdef0123456789abcdef0123456789");
        assert!(s.contains('…'));
        assert!(s.starts_with("abcdef"));
    }
}
