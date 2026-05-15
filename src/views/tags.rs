//! S9 — Tags / uploads view computation. Pure half of bee-tui's
//! `components::tags`: every counter Bee tracks for an in-flight
//! upload (split / seen / stored / sent / synced) plus the
//! lifecycle classifier and the aggregate totals header. The
//! renderer paints the table, the colour, and the per-status glyph
//! label.

use bee::api::Tag;

use crate::watch::TagsSnapshot;

/// Per-tag lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagStatus {
    /// `total <= 0` — Bee hasn't filled in the chunk count yet (the
    /// upload either hasn't started or used a streaming endpoint
    /// that doesn't pre-declare a total).
    Pending,
    /// Chunking in progress: `split < total`.
    Splitting,
    /// All chunks split; pushing them out: `sent < total`.
    Pushing,
    /// Pushed but waiting on enough receipts: `synced < total`.
    Syncing,
    /// `synced >= total > 0`. Done.
    Synced,
}

/// One row of the tags table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    pub uid: u32,
    pub name: String,
    pub total: i64,
    pub split: i64,
    pub seen: i64,
    pub stored: i64,
    pub sent: i64,
    pub synced: i64,
    pub address_short: String,
    /// Full Swarm reference address (`0x` stripped). Rendered on the
    /// row's continuation line so operators can click-drag to copy
    /// without shrinking the table column.
    pub address_full: String,
    pub status: TagStatus,
    /// `synced / total` percentage in `0..=100`. `0` if total ≤ 0.
    pub completion_pct: u32,
    pub started_at: String,
}

/// Aggregate counters across the whole tag list — used by the
/// summary header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TagsTotals {
    pub tags: usize,
    pub split: i64,
    pub sent: i64,
    pub synced: i64,
    /// Number of tags currently in `Splitting` / `Pushing` /
    /// `Syncing` — anything that's not Pending or Synced.
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagsView {
    pub rows: Vec<TagRow>,
    pub totals: TagsTotals,
}

/// Pure, snapshot-driven view computation.
pub fn view_for(snap: &TagsSnapshot) -> TagsView {
    let mut rows: Vec<TagRow> = snap.tags.iter().map(row_from_tag).collect();
    // Newest tags first — Bee assigns monotonically increasing UIDs.
    rows.sort_by_key(|r| std::cmp::Reverse(r.uid));
    let totals = compute_totals(&rows);
    TagsView { rows, totals }
}

fn row_from_tag(t: &Tag) -> TagRow {
    let status = classify_tag(t);
    let completion_pct = if t.total > 0 {
        let pct = (t.synced.max(0) as i128 * 100 / t.total as i128).min(100);
        pct as u32
    } else {
        0
    };
    let name = if t.name.is_empty() {
        format!("tag-{}", t.uid)
    } else {
        t.name.clone()
    };
    TagRow {
        uid: t.uid,
        name,
        total: t.total,
        split: t.split,
        seen: t.seen,
        stored: t.stored,
        sent: t.sent,
        synced: t.synced,
        address_short: short_ref(&t.address),
        address_full: t.address.trim_start_matches("0x").to_string(),
        status,
        completion_pct,
        started_at: t.started_at.clone(),
    }
}

pub fn classify_tag(t: &Tag) -> TagStatus {
    if t.total <= 0 {
        return TagStatus::Pending;
    }
    if t.synced >= t.total {
        return TagStatus::Synced;
    }
    if t.split < t.total {
        return TagStatus::Splitting;
    }
    if t.sent < t.total {
        return TagStatus::Pushing;
    }
    TagStatus::Syncing
}

fn compute_totals(rows: &[TagRow]) -> TagsTotals {
    let mut totals = TagsTotals {
        tags: rows.len(),
        ..TagsTotals::default()
    };
    for r in rows {
        totals.split += r.split;
        totals.sent += r.sent;
        totals.synced += r.synced;
        if matches!(
            r.status,
            TagStatus::Splitting | TagStatus::Pushing | TagStatus::Syncing
        ) {
            totals.active += 1;
        }
    }
    totals
}

pub fn short_ref(s: &str) -> String {
    let trimmed = s.trim_start_matches("0x");
    if trimmed.is_empty() {
        return "—".into();
    }
    if trimmed.len() > 12 {
        format!("{}…{}", &trimmed[..6], &trimmed[trimmed.len() - 4..])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_with(uid: u32, total: i64, split: i64, sent: i64, synced: i64) -> Tag {
        Tag {
            uid,
            name: format!("tag-{uid}"),
            total,
            split,
            seen: split,
            stored: split,
            sent,
            synced,
            address: "ee".repeat(32),
            started_at: "2024-05-07T12:00:00Z".into(),
        }
    }

    #[test]
    fn classify_pending_when_total_zero() {
        assert_eq!(classify_tag(&tag_with(1, 0, 0, 0, 0)), TagStatus::Pending);
    }

    #[test]
    fn classify_splitting_when_split_below_total() {
        assert_eq!(
            classify_tag(&tag_with(1, 100, 50, 0, 0)),
            TagStatus::Splitting
        );
    }

    #[test]
    fn classify_pushing_when_sent_below_total() {
        assert_eq!(
            classify_tag(&tag_with(1, 100, 100, 60, 0)),
            TagStatus::Pushing
        );
    }

    #[test]
    fn classify_syncing_when_synced_below_total() {
        assert_eq!(
            classify_tag(&tag_with(1, 100, 100, 100, 50)),
            TagStatus::Syncing
        );
    }

    #[test]
    fn classify_synced_when_complete() {
        assert_eq!(
            classify_tag(&tag_with(1, 100, 100, 100, 100)),
            TagStatus::Synced
        );
        assert_eq!(
            classify_tag(&tag_with(1, 100, 100, 100, 105)),
            TagStatus::Synced
        );
    }

    #[test]
    fn short_ref_handles_empty() {
        assert_eq!(short_ref(""), "—");
    }
}
