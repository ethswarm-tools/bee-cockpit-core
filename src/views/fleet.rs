//! S15 — Fleet view. Pure half of bee-tui's `components::fleet`: the
//! per-node row formatting (status / peers / worst-TTL / ping
//! labels) and the aggregate header counts. The renderer paints the
//! table; the fleet poller (`crate::fleet`) supplies the snapshot.

use crate::fleet::{FleetRow, FleetSnapshot, FleetStatus};

/// Pure, render-ready view of the fleet.
#[derive(Debug, Clone, PartialEq)]
pub struct FleetView {
    pub header: FleetHeader,
    pub rows: Vec<FleetRowView>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FleetHeader {
    pub total: usize,
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FleetRowView {
    pub name: String,
    pub url: String,
    pub default: bool,
    pub active: bool,
    pub status: FleetStatus,
    pub status_label: String,
    pub peers_label: String,
    pub ttl_label: String,
    pub ping_label: String,
    pub why: Option<String>,
}

/// Pure, snapshot-driven view computation.
pub fn view_for(snap: &FleetSnapshot, active_name: &str, selected: usize) -> FleetView {
    let (pass, warn, fail, unknown) = snap.counts();
    let rows = snap
        .rows
        .iter()
        .map(|r| row_view(r, active_name))
        .collect::<Vec<_>>();
    FleetView {
        header: FleetHeader {
            total: snap.rows.len(),
            pass,
            warn,
            fail,
            unknown,
        },
        rows,
        selected,
    }
}

fn row_view(r: &FleetRow, active_name: &str) -> FleetRowView {
    let status_label = match r.status {
        FleetStatus::Pass => "pass".into(),
        FleetStatus::Warn => "warn".into(),
        FleetStatus::Fail => "fail".into(),
        FleetStatus::Unknown => "…loading".into(),
    };
    let peers_label = r.peers.map(|p| p.to_string()).unwrap_or_else(|| "—".into());
    let ttl_label = r
        .worst_ttl_secs
        .map(format_ttl)
        .unwrap_or_else(|| "—".into());
    let ping_label = r
        .ping_ms
        .map(|p| format!("{p}ms"))
        .unwrap_or_else(|| "—".into());
    FleetRowView {
        name: r.name.clone(),
        url: r.url.clone(),
        default: r.default,
        active: r.name == active_name,
        status: r.status,
        status_label,
        peers_label,
        ttl_label,
        ping_label,
        why: r.why.clone(),
    }
}

/// Human-readable TTL — same convention as the Stamps screen, just
/// compacter (one unit max).
pub fn format_ttl(secs: u64) -> String {
    if secs >= 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs >= 3_600 {
        format!("{}h", secs / 3_600)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn row(name: &str, status: FleetStatus, peers: Option<u64>, ttl: Option<u64>) -> FleetRow {
        FleetRow {
            name: name.into(),
            url: format!("http://{name}.example:1633"),
            default: false,
            status,
            peers,
            worst_ttl_secs: ttl,
            ping_ms: Some(12),
            warming_up: false,
            last_probe: Some(Instant::now()),
            why: match status {
                FleetStatus::Fail => Some("0 peers — isolated".into()),
                FleetStatus::Warn => Some("only 2 peers (< 4)".into()),
                _ => None,
            },
        }
    }

    #[test]
    fn view_header_counts_partition() {
        let snap = FleetSnapshot {
            rows: vec![
                row("a", FleetStatus::Pass, Some(87), Some(86_400 * 30)),
                row("b", FleetStatus::Warn, Some(2), Some(86_400 * 30)),
                row("c", FleetStatus::Fail, Some(0), Some(86_400 * 30)),
            ],
            last_update: Some(Instant::now()),
        };
        let view = view_for(&snap, "a", 0);
        assert_eq!(view.header.total, 3);
        assert_eq!(view.header.pass, 1);
        assert_eq!(view.header.warn, 1);
        assert_eq!(view.header.fail, 1);
        assert_eq!(view.header.unknown, 0);
    }

    #[test]
    fn view_active_row_is_marked() {
        let snap = FleetSnapshot {
            rows: vec![row("a", FleetStatus::Pass, Some(87), Some(86_400 * 30))],
            last_update: Some(Instant::now()),
        };
        let view = view_for(&snap, "a", 0);
        assert!(view.rows[0].active);
    }

    #[test]
    fn view_inactive_row_is_not_marked() {
        let snap = FleetSnapshot {
            rows: vec![row("a", FleetStatus::Pass, Some(87), Some(86_400 * 30))],
            last_update: Some(Instant::now()),
        };
        let view = view_for(&snap, "different-context", 0);
        assert!(!view.rows[0].active);
    }

    #[test]
    fn view_ttl_formatting_picks_largest_unit() {
        let snap = FleetSnapshot {
            rows: vec![
                row("days", FleetStatus::Pass, Some(87), Some(86_400 * 30)),
                row("hours", FleetStatus::Warn, Some(87), Some(3_600 * 14)),
                row("mins", FleetStatus::Fail, Some(0), Some(60 * 14)),
            ],
            last_update: Some(Instant::now()),
        };
        let view = view_for(&snap, "", 0);
        assert_eq!(view.rows[0].ttl_label, "30d");
        assert_eq!(view.rows[1].ttl_label, "14h");
        assert_eq!(view.rows[2].ttl_label, "14m");
    }

    #[test]
    fn view_empty_peers_show_dash() {
        let snap = FleetSnapshot {
            rows: vec![row("down", FleetStatus::Fail, None, None)],
            last_update: Some(Instant::now()),
        };
        let view = view_for(&snap, "", 0);
        assert_eq!(view.rows[0].peers_label, "—");
        assert_eq!(view.rows[0].ttl_label, "—");
    }

    #[test]
    fn format_ttl_handles_all_buckets() {
        assert_eq!(format_ttl(86_400 * 5), "5d");
        assert_eq!(format_ttl(3_600 * 3), "3h");
        assert_eq!(format_ttl(60 * 45), "45m");
        assert_eq!(format_ttl(42), "42s");
    }
}
