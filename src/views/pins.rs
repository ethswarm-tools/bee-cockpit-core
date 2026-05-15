//! S11 — Pins view. Pure half of bee-tui's `components::pins`: the
//! per-pin integrity-check state machine (`CheckState`), the table
//! row (`PinRow`), the sort modes, and the snapshot-driven
//! `view_for` builder. The renderer owns the fetch channels, the
//! cursor, the sort cycler, and the ratatui draw path.

use std::collections::HashMap;

use bee::swarm::Reference;

use crate::watch::PinsSnapshot;

/// Per-pin integrity-check status. `Idle` = operator hasn't asked
/// for a check yet; `Checking` = a `/pins/check` call is in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    Idle,
    Checking,
    Ok {
        total: u64,
        missing: u64,
        invalid: u64,
    },
    Failed(String),
}

impl CheckState {
    pub fn is_unhealthy(&self) -> bool {
        matches!(self, Self::Ok { missing, invalid, .. } if *missing > 0 || *invalid > 0)
    }
    pub fn is_healthy(&self) -> bool {
        matches!(
            self,
            Self::Ok {
                missing: 0,
                invalid: 0,
                ..
            }
        )
    }
}

/// One row of the pins table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRow {
    pub reference: Reference,
    pub reference_short: String,
    pub check: CheckState,
}

/// How the rows are ordered. Operator cycles via `s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    /// Bee's response order — `curl /pins` parity.
    Reference,
    /// Unhealthy → unchecked → healthy.
    BadFirst,
    /// Largest pins first by `total` chunk count.
    TotalChunks,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            Self::Reference => Self::BadFirst,
            Self::BadFirst => Self::TotalChunks,
            Self::TotalChunks => Self::Reference,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Reference => "ref order",
            Self::BadFirst => "bad first",
            Self::TotalChunks => "by size",
        }
    }
}

/// View fed to the renderer + snapshot tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinsView {
    pub rows: Vec<PinRow>,
    pub sort: SortMode,
    pub total_pins: usize,
    pub healthy: usize,
    pub unhealthy: usize,
    pub unchecked: usize,
}

/// Pure view builder.
pub fn view_for(
    snap: &PinsSnapshot,
    checks: &HashMap<Reference, CheckState>,
    sort: SortMode,
) -> PinsView {
    let mut rows: Vec<PinRow> = snap
        .pins
        .iter()
        .map(|r| {
            let check = checks.get(r).cloned().unwrap_or(CheckState::Idle);
            PinRow {
                reference: r.clone(),
                reference_short: short_ref(&r.to_hex()),
                check,
            }
        })
        .collect();

    match sort {
        SortMode::Reference => {}
        SortMode::BadFirst => {
            rows.sort_by_key(|r| match &r.check {
                CheckState::Ok {
                    missing, invalid, ..
                } if *missing > 0 || *invalid > 0 => 0,
                CheckState::Failed(_) => 1,
                CheckState::Idle => 2,
                CheckState::Checking => 3,
                CheckState::Ok { .. } => 4,
            });
        }
        SortMode::TotalChunks => {
            rows.sort_by_key(|r| match &r.check {
                CheckState::Ok { total, .. } => std::cmp::Reverse(*total),
                _ => std::cmp::Reverse(0),
            });
        }
    }

    let mut healthy = 0;
    let mut unhealthy = 0;
    let mut unchecked = 0;
    for r in &rows {
        if r.check.is_healthy() {
            healthy += 1;
        } else if r.check.is_unhealthy() {
            unhealthy += 1;
        } else if matches!(r.check, CheckState::Idle) {
            unchecked += 1;
        }
    }

    PinsView {
        total_pins: rows.len(),
        rows,
        sort,
        healthy,
        unhealthy,
        unchecked,
    }
}

/// `prefix…suffix` form for the table display.
pub fn short_ref(hex: &str) -> String {
    let trimmed = hex.trim_start_matches("0x");
    if trimmed.len() > 14 {
        format!("{}…{}", &trimmed[..8], &trimmed[trimmed.len() - 4..])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(byte: u8) -> Reference {
        Reference::new(&[byte; 32]).unwrap()
    }

    fn ok(total: u64, missing: u64, invalid: u64) -> CheckState {
        CheckState::Ok {
            total,
            missing,
            invalid,
        }
    }

    #[test]
    fn check_state_health_predicates() {
        assert!(ok(10, 0, 0).is_healthy());
        assert!(!ok(10, 0, 0).is_unhealthy());
        assert!(ok(10, 1, 0).is_unhealthy());
        assert!(ok(10, 0, 1).is_unhealthy());
        assert!(!CheckState::Idle.is_healthy());
        assert!(!CheckState::Idle.is_unhealthy());
        assert!(!CheckState::Checking.is_unhealthy());
    }

    #[test]
    fn view_for_empty_snapshot_renders_zero_counts() {
        let snap = PinsSnapshot::default();
        let view = view_for(&snap, &HashMap::new(), SortMode::Reference);
        assert_eq!(view.total_pins, 0);
        assert_eq!(view.healthy, 0);
        assert_eq!(view.unhealthy, 0);
        assert_eq!(view.unchecked, 0);
    }

    #[test]
    fn view_for_counts_health_buckets() {
        let snap = PinsSnapshot {
            pins: vec![r(1), r(2), r(3), r(4)],
            ..PinsSnapshot::default()
        };
        let mut checks = HashMap::new();
        checks.insert(r(1), ok(100, 0, 0));
        checks.insert(r(2), ok(100, 5, 0));
        checks.insert(r(3), CheckState::Failed("nope".into()));
        let view = view_for(&snap, &checks, SortMode::Reference);
        assert_eq!(view.total_pins, 4);
        assert_eq!(view.healthy, 1);
        assert_eq!(view.unhealthy, 1);
        assert_eq!(view.unchecked, 1);
    }

    #[test]
    fn view_for_default_sort_preserves_response_order() {
        let snap = PinsSnapshot {
            pins: vec![r(3), r(1), r(2)],
            ..PinsSnapshot::default()
        };
        let view = view_for(&snap, &HashMap::new(), SortMode::Reference);
        assert_eq!(view.rows[0].reference, r(3));
        assert_eq!(view.rows[1].reference, r(1));
        assert_eq!(view.rows[2].reference, r(2));
    }

    #[test]
    fn view_for_bad_first_surfaces_unhealthy_then_failed_then_unchecked_then_healthy() {
        let snap = PinsSnapshot {
            pins: vec![r(1), r(2), r(3), r(4), r(5)],
            ..PinsSnapshot::default()
        };
        let mut checks = HashMap::new();
        checks.insert(r(1), ok(10, 0, 0));
        checks.insert(r(2), ok(10, 1, 0));
        checks.insert(r(3), CheckState::Failed("e".into()));
        checks.insert(r(4), CheckState::Checking);
        let view = view_for(&snap, &checks, SortMode::BadFirst);
        let order: Vec<_> = view.rows.iter().map(|r| r.reference.clone()).collect();
        assert_eq!(order, vec![r(2), r(3), r(5), r(4), r(1)]);
    }

    #[test]
    fn view_for_total_chunks_sorts_descending_with_unchecked_last() {
        let snap = PinsSnapshot {
            pins: vec![r(1), r(2), r(3), r(4)],
            ..PinsSnapshot::default()
        };
        let mut checks = HashMap::new();
        checks.insert(r(1), ok(50, 0, 0));
        checks.insert(r(2), ok(500, 0, 0));
        checks.insert(r(3), ok(5, 0, 0));
        let view = view_for(&snap, &checks, SortMode::TotalChunks);
        let order: Vec<_> = view.rows.iter().map(|r| r.reference.clone()).collect();
        assert_eq!(order, vec![r(2), r(1), r(3), r(4)]);
    }

    #[test]
    fn sort_mode_cycles() {
        assert_eq!(SortMode::Reference.next(), SortMode::BadFirst);
        assert_eq!(SortMode::BadFirst.next(), SortMode::TotalChunks);
        assert_eq!(SortMode::TotalChunks.next(), SortMode::Reference);
    }

    #[test]
    fn short_ref_keeps_short_strings_intact() {
        assert_eq!(short_ref("abcd"), "abcd");
        assert_eq!(short_ref("0x1234"), "1234");
        assert_eq!(
            short_ref("aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"),
            "aabbccdd…8899"
        );
    }
}
