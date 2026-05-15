//! Stamp-domain constants shared by the cockpit logic.
//!
//! The full Stamps view + classification logic still lives in
//! `bee-tui`'s `components/stamps.rs` for now; only the threshold
//! constants other modules (`fleet`, S1 stamp-TTL gate) depend on
//! have moved here. The full split lands in a follow-up phase.

/// TTL-threshold constants. Below `TOPUP_SOON_SECS` we suggest topup
/// in the row's `why` line; below `TOPUP_URGENT_SECS` we flag the
/// batch as red.
pub const TOPUP_SOON_SECS: i64 = 7 * 24 * 3600;
pub const TOPUP_URGENT_SECS: i64 = 24 * 3600;
