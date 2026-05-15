//! S15 Pubsub watch view. Per-row formatting and the case-insensitive
//! substring filter live here; the renderer owns the ring buffer,
//! the cursor, and the active-subscription count.

use std::time::SystemTime;

use crate::pubsub::{PubsubKind, PubsubMessage, smart_preview};

/// One row of the pubsub timeline — pre-formatted columns for the
/// renderer plus the underlying payload byte count for the detail
/// pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubsubRowView {
    pub time_label: String,
    pub kind: PubsubKind,
    pub kind_label: &'static str,
    pub channel: String,
    pub channel_short: String,
    pub payload_bytes: usize,
    pub preview_short: String,
    pub preview_long: String,
}

/// View fed to the renderer + snapshot tests. Rows are emitted in
/// the same order as the source ring (`msgs[0]` is the most-recent
/// message).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PubsubView {
    pub rows: Vec<PubsubRowView>,
}

/// Build the view, optionally filtered. `filter` is matched
/// case-insensitively against the channel hex *and* the short
/// payload preview — operators can grep on either.
pub fn view_for<'a, I>(messages: I, filter: Option<&str>) -> PubsubView
where
    I: IntoIterator<Item = &'a PubsubMessage>,
{
    let needle = filter.map(|s| s.to_ascii_lowercase());
    let rows = messages
        .into_iter()
        .filter(|m| match_filter(m, needle.as_deref()))
        .map(row_view)
        .collect();
    PubsubView { rows }
}

/// True iff `msg` matches a (case-insensitive, already-lowercased)
/// substring filter. `None` matches everything.
pub fn match_filter(msg: &PubsubMessage, needle_lower: Option<&str>) -> bool {
    let Some(needle) = needle_lower else {
        return true;
    };
    if msg.channel.to_ascii_lowercase().contains(needle) {
        return true;
    }
    smart_preview(&msg.payload, 200)
        .to_ascii_lowercase()
        .contains(needle)
}

pub fn row_view(msg: &PubsubMessage) -> PubsubRowView {
    PubsubRowView {
        time_label: format_clock(msg.received_at),
        kind: msg.kind,
        kind_label: match msg.kind {
            PubsubKind::Pss => "PSS ",
            PubsubKind::Gsoc => "GSOC",
        },
        channel: msg.channel.clone(),
        channel_short: short_hex(&msg.channel, 12),
        payload_bytes: msg.payload.len(),
        preview_short: smart_preview(&msg.payload, 50),
        preview_long: smart_preview(&msg.payload, 200),
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

pub fn format_clock(t: SystemTime) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let secs = t
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(kind: PubsubKind, channel: &str, payload: &[u8]) -> PubsubMessage {
        PubsubMessage {
            received_at: SystemTime::now(),
            kind,
            channel: channel.to_string(),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn match_filter_no_filter_passes_everything() {
        let m = msg(PubsubKind::Pss, "abc123", b"hello");
        assert!(match_filter(&m, None));
    }

    #[test]
    fn match_filter_substring_in_channel_case_insensitive() {
        let m = msg(PubsubKind::Pss, "cafebabe1234", b"unrelated");
        assert!(match_filter(&m, Some("cafe")));
    }

    #[test]
    fn match_filter_substring_in_preview() {
        let m = msg(PubsubKind::Pss, "topic", b"{\"event\":\"ping\"}");
        assert!(match_filter(&m, Some("ping")));
    }

    #[test]
    fn match_filter_no_match_drops() {
        let m = msg(PubsubKind::Pss, "topic", b"hello");
        assert!(!match_filter(&m, Some("xyz")));
    }
}
