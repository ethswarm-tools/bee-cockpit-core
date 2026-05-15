//! Stamp-domain constants + pure helpers shared by the cockpit logic.
//!
//! The full Stamps view + classification logic still lives in
//! `bee-tui`'s `components/stamps.rs` for now; the threshold
//! constants + the TTL formatter that other modules (`fleet`, the
//! S1 stamp-TTL gate, `views::health`) depend on have moved here.
//! The full split lands in a follow-up phase.

/// TTL-threshold constants. Below `TOPUP_SOON_SECS` we suggest topup
/// in the row's `why` line; below `TOPUP_URGENT_SECS` we flag the
/// batch as red.
pub const TOPUP_SOON_SECS: i64 = 7 * 24 * 3600;
pub const TOPUP_URGENT_SECS: i64 = 24 * 3600;

/// Humanise a TTL in seconds as `"5d 12h"` / `"23h  5m"` / `"expired"`.
/// Negative or zero collapse to `expired` — Bee surfaces those as
/// already-spent batches.
pub fn format_ttl_seconds(secs: i64) -> String {
    if secs <= 0 {
        return "expired".into();
    }
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    if days >= 1 {
        format!("{days}d {hours:>2}h")
    } else {
        let minutes = (secs % 3_600) / 60;
        format!("{hours}h {minutes:>2}m")
    }
}
