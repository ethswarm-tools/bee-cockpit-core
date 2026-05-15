//! S13 — Durability Watchlist view. Pure half of bee-tui's
//! `components::watchlist`: per-row formatting (status label,
//! detail string, age in seconds) + summary counts. The renderer
//! owns the `RingBuffer<DurabilityResult>`, the cursor, and the
//! draw / key path; this module turns the ring's snapshot at a
//! given wall-clock into a [`WatchlistView`].

use std::collections::VecDeque;
use std::time::SystemTime;

use crate::durability::DurabilityResult;

/// Maximum number of rows the renderer keeps in its ring before
/// evicting the oldest. Exposed so the renderer + tests share the
/// same bound.
pub const MAX_ROWS: usize = 50;

/// One row in the watchlist. Cloneable so the view can be assembled
/// without borrowing the component's storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchlistRow {
    pub reference_hex: String,
    pub status_label: String,
    /// `true` when this row's check completed cleanly; drives green
    /// vs red paint in the renderer.
    pub healthy: bool,
    /// Pre-formatted breakdown: "12 total · 0 lost · 0 errors · 412ms".
    pub detail: String,
    /// Wall-clock seconds since `started_at` at view-build time.
    pub age_seconds: u64,
    pub root_is_manifest: bool,
}

/// View fed to the renderer + snapshot tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchlistView {
    pub rows: Vec<WatchlistRow>,
    pub healthy_count: usize,
    pub unhealthy_count: usize,
}

/// Pure view builder for snapshot tests + the renderer.
pub fn view_for(rows: &VecDeque<DurabilityResult>, now: SystemTime) -> WatchlistView {
    let mut healthy = 0;
    let mut unhealthy = 0;
    let view_rows: Vec<WatchlistRow> = rows
        .iter()
        .map(|r| {
            let h = r.is_healthy();
            if h {
                healthy += 1;
            } else {
                unhealthy += 1;
            }
            let age = now
                .duration_since(r.started_at)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let corrupt_segment = if r.chunks_corrupt > 0 || r.bmt_verified {
                format!(" · {} corrupt", r.chunks_corrupt)
            } else {
                String::new()
            };
            let swarmscan_segment = match r.swarmscan_seen {
                Some(true) => " · scan: seen",
                Some(false) => " · scan: NOT seen",
                None => "",
            };
            let detail = format!(
                "{} total · {} lost · {} errors{} · {}ms{}{}{}",
                r.chunks_total,
                r.chunks_lost,
                r.chunks_errors,
                corrupt_segment,
                r.duration_ms,
                if r.bmt_verified { " · BMT" } else { "" },
                swarmscan_segment,
                if r.truncated { " · truncated" } else { "" },
            );
            WatchlistRow {
                reference_hex: r.reference.to_hex(),
                status_label: if h {
                    "OK".to_string()
                } else {
                    "UNHEALTHY".to_string()
                },
                healthy: h,
                detail,
                age_seconds: age,
                root_is_manifest: r.root_is_manifest,
            }
        })
        .collect();
    WatchlistView {
        rows: view_rows,
        healthy_count: healthy,
        unhealthy_count: unhealthy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bee::swarm::Reference;
    use std::time::Duration;

    fn make_result(healthy: bool, secs_ago: u64) -> DurabilityResult {
        DurabilityResult {
            reference: Reference::from_hex(&"a".repeat(64)).unwrap(),
            started_at: SystemTime::now() - Duration::from_secs(secs_ago),
            duration_ms: 200,
            chunks_total: 4,
            chunks_lost: if healthy { 0 } else { 1 },
            chunks_errors: 0,
            chunks_corrupt: 0,
            root_is_manifest: true,
            truncated: false,
            bmt_verified: true,
            swarmscan_seen: None,
        }
    }

    #[test]
    fn empty_view_has_zero_rows() {
        let rows = VecDeque::new();
        let v = view_for(&rows, SystemTime::now());
        assert_eq!(v.rows.len(), 0);
        assert_eq!(v.healthy_count, 0);
        assert_eq!(v.unhealthy_count, 0);
    }

    #[test]
    fn view_counts_healthy_and_unhealthy_separately() {
        let mut rows = VecDeque::new();
        rows.push_back(make_result(true, 10));
        rows.push_back(make_result(false, 20));
        rows.push_back(make_result(true, 30));
        let v = view_for(&rows, SystemTime::now());
        assert_eq!(v.healthy_count, 2);
        assert_eq!(v.unhealthy_count, 1);
        assert_eq!(v.rows.len(), 3);
    }

    #[test]
    fn view_age_increases_with_time_since_started() {
        let mut rows = VecDeque::new();
        rows.push_back(make_result(true, 60));
        let v = view_for(&rows, SystemTime::now());
        assert!(v.rows[0].age_seconds >= 60);
    }
}
