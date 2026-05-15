//! S3 — SWAP / cheques view. Pure half of bee-tui's
//! `components::swap`: chequebook card classification, per-peer
//! cheque/settlement row formatting + sorting, the PLUR formatter,
//! and the optional Market tile pre-formatted lines. The renderer
//! owns the watch subscriptions, the two-pane focus toggle, and the
//! scroll offsets.

use bee::debug::{ChequebookBalance, LastCheque, Settlement, Settlements};
use num_bigint::BigInt;

use crate::economics_oracle::EconomicsSnapshot;
use crate::watch::SwapSnapshot;

/// Tri-state outcome for the chequebook balance card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapStatus {
    Empty,
    Healthy,
    Tight,
    Unknown,
}

impl SwapStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "○ unfunded",
            Self::Healthy => "✓ healthy",
            Self::Tight => "⚠ tight",
            Self::Unknown => "? unknown",
        }
    }
}

/// Snapshot-friendly view of the chequebook card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChequebookCard {
    pub status: SwapStatus,
    pub total: String,
    pub available: String,
    /// `available / total` as 0..=100. `0` if total is zero.
    pub available_pct: u32,
    pub why: Option<String>,
}

/// One row of the "last received cheques" pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRow {
    pub peer_short: String,
    pub peer_full: String,
    pub payout: String,
    /// `true` if this peer has not sent us any cheque yet.
    pub never: bool,
}

/// One row of the per-peer settlements pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementRow {
    pub peer_short: String,
    pub peer_full: String,
    pub received: String,
    pub sent: String,
    /// Sign-prefixed net (`+x` if we're owed, `-x` if we owe).
    pub net: String,
    /// `true` when |net| > 0.5 BZZ — flagged in red.
    pub net_flagged: bool,
}

/// Snapshot of the Market tile lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketTile {
    pub price_line: String,
    pub gas_line: String,
    pub stale_why: Option<String>,
    /// `true` while no poll has completed — renderer drives the
    /// spinner glyph.
    pub cold_start: bool,
}

/// Aggregated view fed to renderer and snapshot tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapView {
    pub card: ChequebookCard,
    pub chequebook_address: Option<String>,
    pub cheques: Vec<CheckRow>,
    pub settlements: Vec<SettlementRow>,
    pub time_total_received: Option<String>,
    pub time_total_sent: Option<String>,
    pub market: Option<MarketTile>,
}

/// Pure, snapshot-driven view computation. `market = None` when
/// `[economics].enable_market_tile` is off.
pub fn view_for(snap: &SwapSnapshot, market: Option<&EconomicsSnapshot>) -> SwapView {
    let card = card_for(snap.chequebook.as_ref());
    let cheques = cheque_rows_for(&snap.last_received);
    let settlements = settlement_rows_for(snap.settlements.as_ref());
    let time_total_received = snap
        .time_settlements
        .as_ref()
        .and_then(|s| s.total_received.as_ref())
        .map(format_plur);
    let time_total_sent = snap
        .time_settlements
        .as_ref()
        .and_then(|s| s.total_sent.as_ref())
        .map(format_plur);
    let market = market.map(market_tile_for);
    SwapView {
        card,
        chequebook_address: snap.chequebook_address.clone(),
        cheques,
        settlements,
        time_total_received,
        time_total_sent,
        market,
    }
}

/// Convenience wrapper for snapshot tests that don't exercise the
/// optional Market tile.
pub fn view_for_no_market(snap: &SwapSnapshot) -> SwapView {
    view_for(snap, None)
}

fn market_tile_for(m: &EconomicsSnapshot) -> MarketTile {
    let price_line = match &m.price {
        Some(p) => format!("BZZ ≈ ${:.4}", p.usd),
        None => "BZZ ≈ —".to_string(),
    };
    let gas_line = match &m.gas {
        Some(g) => match g.max_priority_fee_gwei {
            Some(tip) => format!(
                "gas: {:.2} base + {:.2} tip = {:.2} gwei",
                g.base_fee_gwei,
                tip,
                g.total_gwei(),
            ),
            None => format!("gas: {:.2} gwei base", g.base_fee_gwei),
        },
        None => "gas: —".to_string(),
    };
    MarketTile {
        price_line,
        gas_line,
        stale_why: m.last_error.clone(),
        cold_start: m.last_polled.is_none(),
    }
}

fn card_for(cb: Option<&ChequebookBalance>) -> ChequebookCard {
    let Some(cb) = cb else {
        return ChequebookCard {
            status: SwapStatus::Unknown,
            total: "—".into(),
            available: "—".into(),
            available_pct: 0,
            why: Some("/chequebook/balance not available yet".into()),
        };
    };
    let zero = BigInt::from(0);
    let total = &cb.total_balance;
    let avail = &cb.available_balance;
    let total_str = format_plur(total);
    let avail_str = format_plur(avail);
    if total == &zero {
        return ChequebookCard {
            status: SwapStatus::Empty,
            total: total_str,
            available: avail_str,
            available_pct: 0,
            why: Some("chequebook holds 0 BZZ — fund it to send cheques.".into()),
        };
    }
    let pct = pct_of(avail, total);
    let (status, why) = if pct < 20 {
        (
            SwapStatus::Tight,
            Some(format!(
                "only {pct}% available — most BZZ is tied up in unsettled debt."
            )),
        )
    } else {
        (SwapStatus::Healthy, None)
    };
    ChequebookCard {
        status,
        total: total_str,
        available: avail_str,
        available_pct: pct,
        why,
    }
}

fn cheque_rows_for(last_received: &[LastCheque]) -> Vec<CheckRow> {
    let mut rows: Vec<CheckRow> = last_received
        .iter()
        .map(|lc| {
            let payout_bi = lc.last_received.as_ref().and_then(|c| c.payout.as_ref());
            let (payout, never) = match payout_bi {
                Some(p) => (format_plur(p), false),
                None => ("—".into(), true),
            };
            CheckRow {
                peer_short: short_peer(&lc.peer),
                peer_full: lc.peer.trim_start_matches("0x").to_string(),
                payout,
                never,
            }
        })
        .collect();
    rows.sort_by(|a, b| match (a.never, b.never) {
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        _ => b.payout.cmp(&a.payout),
    });
    rows
}

fn settlement_rows_for(s: Option<&Settlements>) -> Vec<SettlementRow> {
    let Some(s) = s else { return Vec::new() };
    let mut sorted: Vec<&Settlement> = s.settlements.iter().collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(abs_net(s)));
    sorted.into_iter().map(settlement_row).collect()
}

fn abs_net(s: &Settlement) -> BigInt {
    let zero = BigInt::from(0);
    let recv = s.received.as_ref().unwrap_or(&zero);
    let sent = s.sent.as_ref().unwrap_or(&zero);
    let net = recv - sent;
    if net < zero { -net } else { net }
}

fn settlement_row(s: &Settlement) -> SettlementRow {
    let zero = BigInt::from(0);
    let recv = s.received.as_ref().unwrap_or(&zero);
    let sent = s.sent.as_ref().unwrap_or(&zero);
    let net_bi = recv - sent;
    let net = format_plur_signed(&net_bi);
    // Flag peers >0.5 BZZ out of balance (5 * 10^15 PLUR).
    let half_bzz = BigInt::from(5_000_000_000_000_000u64);
    let abs = if net_bi < BigInt::from(0) {
        -net_bi
    } else {
        net_bi
    };
    let net_flagged = abs > half_bzz;
    SettlementRow {
        peer_short: short_peer(&s.peer),
        peer_full: s.peer.trim_start_matches("0x").to_string(),
        received: format_plur(recv),
        sent: format_plur(sent),
        net,
        net_flagged,
    }
}

/// Format a PLUR amount as `BZZ x.xxxx`. PLUR has 16 decimals; we
/// render 4 fractional digits so 0.0001 BZZ is the smallest visible
/// unit.
pub fn format_plur(plur: &BigInt) -> String {
    format_plur_inner(plur, false)
}

fn format_plur_signed(plur: &BigInt) -> String {
    format_plur_inner(plur, true)
}

fn format_plur_inner(plur: &BigInt, signed: bool) -> String {
    let zero = BigInt::from(0);
    let neg = plur < &zero;
    let abs = if neg { -plur.clone() } else { plur.clone() };
    let scale = BigInt::from(10u64).pow(16);
    let whole = &abs / &scale;
    let frac = &abs % &scale;
    let frac_4 = &frac / BigInt::from(10u64).pow(12);
    let sign = if neg {
        "-"
    } else if signed {
        "+"
    } else {
        ""
    };
    format!("{sign}BZZ {whole}.{frac_4:0>4}")
}

fn pct_of(num: &BigInt, denom: &BigInt) -> u32 {
    let zero = BigInt::from(0);
    if denom == &zero {
        return 0;
    }
    let scaled = num * BigInt::from(100);
    let q = &scaled / denom;
    let q_str = q.to_string();
    let q_u: u128 = q_str.parse().unwrap_or(0);
    q_u.min(100) as u32
}

fn short_peer(p: &str) -> String {
    let trimmed = p.trim_start_matches("0x");
    if trimmed.len() > 10 {
        format!("{}…{}", &trimmed[..6], &trimmed[trimmed.len() - 4..])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_plur_zero() {
        assert_eq!(format_plur(&BigInt::from(0)), "BZZ 0.0000");
    }

    #[test]
    fn format_plur_one_bzz() {
        let one = BigInt::from(10u64).pow(16);
        assert_eq!(format_plur(&one), "BZZ 1.0000");
    }

    #[test]
    fn format_plur_fractional() {
        let half = BigInt::from(5_000_000_000_000_000u64);
        assert_eq!(format_plur(&half), "BZZ 0.5000");
    }

    #[test]
    fn format_plur_signed_negative() {
        let one = BigInt::from(10u64).pow(16);
        assert_eq!(format_plur_signed(&-one), "-BZZ 1.0000");
    }

    #[test]
    fn format_plur_signed_positive() {
        let one = BigInt::from(10u64).pow(16);
        assert_eq!(format_plur_signed(&one), "+BZZ 1.0000");
    }

    #[test]
    fn pct_of_handles_zero_denom() {
        assert_eq!(pct_of(&BigInt::from(10), &BigInt::from(0)), 0);
    }

    #[test]
    fn pct_of_clamps_to_100() {
        assert_eq!(pct_of(&BigInt::from(200), &BigInt::from(100)), 100);
    }

    #[test]
    fn short_peer_truncates_long_overlay() {
        let p = "0xabcdef0123456789abcdef0123456789";
        let s = short_peer(p);
        assert!(s.contains('…'));
        assert!(s.starts_with("abcdef"));
    }

    #[test]
    fn short_peer_passes_short_through() {
        assert_eq!(short_peer("abcd"), "abcd");
    }
}
