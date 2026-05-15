//! S14 Feed Timeline view. Renderer-agnostic per-row formatting plus
//! the aggregate `FeedTimelineView` that turns a [`Timeline`] (the
//! result of `crate::feed_timeline::walk`) into the table the screen
//! draws. Loading / error state lives in the renderer because it
//! reflects screen-local interaction; the row data is shared.

use crate::feed_timeline::{Timeline, TimelineEntry, format_age_secs};

/// One row of the timeline table — pre-formatted column strings the
/// renderer drops straight into a `Span`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedRowView {
    pub index: u64,
    /// Wall-clock age string (`"3h 12m"`, `"—"` when no timestamp).
    pub age_label: String,
    /// Payload size in bytes, rendered as a string for column
    /// alignment.
    pub size_label: String,
    /// `"miss"` (error), `"ref"` (manifest reference), `"raw"`
    /// (inline payload).
    pub kind: &'static str,
    /// Body cell: shortened reference hex, error text, or `"payload
    /// NB"` for inline payloads.
    pub body: String,
    /// `true` when this row represents a fetch error — the renderer
    /// dims the row.
    pub is_error: bool,
    /// Full reference hex (`None` for raw / error rows). Used by the
    /// selected-row detail line.
    pub reference_hex: Option<String>,
}

/// Aggregate table view fed to the renderer + snapshot tests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FeedTimelineView {
    /// Header summary. `None` when no timeline is loaded.
    pub header: Option<FeedTimelineHeader>,
    pub rows: Vec<FeedRowView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedTimelineHeader {
    pub owner_hex_short: String,
    pub topic_hex_short: String,
    pub latest_index: u64,
    pub entry_count: usize,
}

/// Build the table view from a timeline. Returns an empty view when
/// `timeline` is `None`.
pub fn view_for(timeline: Option<&Timeline>, now_unix: u64) -> FeedTimelineView {
    let Some(tm) = timeline else {
        return FeedTimelineView::default();
    };
    FeedTimelineView {
        header: Some(FeedTimelineHeader {
            owner_hex_short: short_hex(&tm.owner_hex, 12),
            topic_hex_short: short_hex(&tm.topic_hex, 8),
            latest_index: tm.latest_index,
            entry_count: tm.entries.len(),
        }),
        rows: tm.entries.iter().map(|e| row_view(e, now_unix)).collect(),
    }
}

/// Build a single row's view. Exposed separately so a renderer that
/// wants finer control over scrolling can iterate entries without
/// allocating the whole view.
pub fn row_view(e: &TimelineEntry, now_unix: u64) -> FeedRowView {
    let age_label = e
        .timestamp_unix
        .map(|ts| format_age_secs(now_unix.saturating_sub(ts)))
        .unwrap_or_else(|| "—".to_string());
    let kind = if e.error.is_some() {
        "miss"
    } else if e.reference_hex.is_some() {
        "ref"
    } else {
        "raw"
    };
    let body = match (&e.error, &e.reference_hex) {
        (Some(err), _) => format!("[{err}]"),
        (_, Some(r)) => short_hex(r, 12),
        (_, None) => format!("payload {}B", e.payload_bytes.saturating_sub(8)),
    };
    FeedRowView {
        index: e.index,
        age_label,
        size_label: e.payload_bytes.to_string(),
        kind,
        body,
        is_error: e.error.is_some(),
        reference_hex: e.reference_hex.clone(),
    }
}

pub fn short_hex(hex: &str, len: usize) -> String {
    let s = hex.trim_start_matches("0x");
    if s.len() > len {
        format!("{}…", &s[..len])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: u64, ref_hex: Option<&str>, error: Option<&str>) -> TimelineEntry {
        TimelineEntry {
            index,
            timestamp_unix: Some(1_700_000_000),
            payload_bytes: 40,
            reference_hex: ref_hex.map(String::from),
            error: error.map(String::from),
        }
    }

    #[test]
    fn row_view_error_marks_is_error() {
        let r = row_view(&entry(7, None, Some("HTTP 500")), 1_700_000_500);
        assert!(r.is_error);
        assert_eq!(r.kind, "miss");
        assert!(r.body.contains("HTTP 500"));
    }

    #[test]
    fn row_view_reference_shortens_hex() {
        let r = row_view(
            &entry(2, Some(&"a".repeat(64)), None),
            1_700_000_500,
        );
        assert!(!r.is_error);
        assert_eq!(r.kind, "ref");
        assert!(r.body.contains('…'));
    }

    #[test]
    fn row_view_raw_payload_falls_through() {
        let r = row_view(&entry(0, None, None), 1_700_000_500);
        assert_eq!(r.kind, "raw");
        assert!(r.body.starts_with("payload "));
        assert!(r.reference_hex.is_none());
    }

    #[test]
    fn view_for_none_timeline_is_empty() {
        let v = view_for(None, 0);
        assert!(v.header.is_none());
        assert!(v.rows.is_empty());
    }
}
