//! S6 — Peers + bin saturation view. Pure half of bee-tui's
//! `components::peers`: the Kademlia bin classification
//! (`BinSaturation`, the bee-go saturation/over-saturation
//! thresholds + the far-bin relaxation), the flattened peer-table
//! row builder, the saturation summary rollup, and the per-peer
//! drill view computation (PLUR formatting, reserve-state
//! formatting, the >5% batch-commitment outlier rule that mirrors
//! bee-scripts/bad-status.sh). The renderer owns the drill-pane
//! fetch channels and the ratatui draw path.

use bee::debug::{Balance, BinInfo, PeerCheques, PeerInfo, PeerStatus, Settlement, Status, Topology};
use num_bigint::BigInt;

use crate::watch::TopologySnapshot;

/// Kademlia bins per Bee build.
pub const BIN_COUNT: usize = 32;
/// `pkg/topology/kademlia/kademlia.go:54` — saturation threshold.
pub const SATURATION_PEERS: u64 = 8;
/// `pkg/topology/kademlia/kademlia.go:55` — over-saturation threshold.
pub const OVER_SATURATION_PEERS: u64 = 18;
/// Bins more than this many positions past depth aren't expected to
/// be saturated; they don't flag as Starving even if connected < 8.
pub const FAR_BIN_RELAXATION: u8 = 4;

/// Tri-state bin saturation classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinSaturation {
    Empty,
    Starving,
    Healthy,
    Over,
}

impl BinSaturation {
    /// Plain-text label without theme glyphs (renderer prepends the
    /// active glyph). `Healthy` is intentionally `""` so the renderer
    /// renders just the pass glyph.
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "—",
            Self::Starving => "STARVING",
            Self::Healthy => "",
            Self::Over => "over",
        }
    }
}

/// One row of the bin saturation strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinStripRow {
    pub bin: u8,
    pub population: u64,
    pub connected: u64,
    pub status: BinSaturation,
    pub is_relevant: bool,
}

/// One row of the peer table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRow {
    pub bin: u8,
    pub peer_short: String,
    pub peer_full: String,
    pub direction: &'static str,
    pub latency: String,
    pub healthy: bool,
    pub reachability: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaturationSummary {
    pub starving: usize,
    pub over: usize,
    pub relevant: usize,
    pub worst_bin: Option<u8>,
    pub worst_connected: u64,
}

impl SaturationSummary {
    pub fn is_alert(&self) -> bool {
        self.starving > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeersView {
    pub bins: Vec<BinStripRow>,
    pub peers: Vec<PeerRow>,
    pub depth: u8,
    pub population: i64,
    pub connected: i64,
    pub reachability: String,
    pub network_availability: String,
    pub light_connected: u64,
    pub saturation: SaturationSummary,
}

/// Per-field outcome of a peer drill fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrillField<T: Clone + PartialEq + Eq> {
    Ok(T),
    Err(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDrillView {
    pub peer_overlay: String,
    pub bin: Option<u8>,
    pub balance: DrillField<String>,
    pub ping: DrillField<String>,
    pub settlement_received: DrillField<String>,
    pub settlement_sent: DrillField<String>,
    pub last_received_cheque: DrillField<Option<String>>,
    pub last_sent_cheque: DrillField<Option<String>>,
    pub storage_radius: DrillField<String>,
    pub reserve_size: DrillField<String>,
    pub pullsync_rate: DrillField<String>,
    pub batch_commitment: DrillField<BatchCommitmentCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchCommitmentCell {
    /// Pre-formatted with thousands grouping (`"99 715 645 440"`).
    pub formatted: String,
    /// True when |peer - local| / local > 5% (bee-scripts parity).
    pub outlier: bool,
}

/// Bundle of the six endpoint results that feeds
/// [`compute_peer_drill_view`].
#[derive(Debug, Clone)]
pub struct PeerDrillFetch {
    pub balance: std::result::Result<Balance, String>,
    pub cheques: std::result::Result<PeerCheques, String>,
    pub settlement: std::result::Result<Settlement, String>,
    pub ping: std::result::Result<String, String>,
    pub peer_status: std::result::Result<Option<PeerStatus>, String>,
    pub local_status: std::result::Result<Status, String>,
}

/// Pure, snapshot-driven view computation.
pub fn view_for(snap: &TopologySnapshot) -> Option<PeersView> {
    let t = snap.topology.as_ref()?;
    let bins = bin_strip_rows(t);
    let peers = peer_rows(t);
    let saturation = compute_saturation_summary(&bins);
    Some(PeersView {
        bins,
        peers,
        depth: t.depth,
        population: t.population,
        connected: t.connected,
        reachability: t.reachability.clone(),
        network_availability: t.network_availability.clone(),
        light_connected: t.light_nodes.connected,
        saturation,
    })
}

/// Pure compute path for the per-peer drill pane.
pub fn compute_peer_drill_view(
    peer: &str,
    bin: Option<u8>,
    fetch: &PeerDrillFetch,
) -> PeerDrillView {
    let balance = match &fetch.balance {
        Ok(b) => DrillField::Ok(format_plur_signed(&b.balance)),
        Err(e) => DrillField::Err(e.clone()),
    };
    let ping = match &fetch.ping {
        Ok(s) => DrillField::Ok(s.clone()),
        Err(e) => DrillField::Err(e.clone()),
    };
    let (settlement_received, settlement_sent) = match &fetch.settlement {
        Ok(s) => (
            DrillField::Ok(format_opt_plur(s.received.as_ref())),
            DrillField::Ok(format_opt_plur(s.sent.as_ref())),
        ),
        Err(e) => (DrillField::Err(e.clone()), DrillField::Err(e.clone())),
    };
    let (last_received_cheque, last_sent_cheque) = match &fetch.cheques {
        Ok(c) => (
            DrillField::Ok(
                c.last_received
                    .as_ref()
                    .map(|q| format_opt_plur(q.payout.as_ref())),
            ),
            DrillField::Ok(
                c.last_sent
                    .as_ref()
                    .map(|q| format_opt_plur(q.payout.as_ref())),
            ),
        ),
        Err(e) => (DrillField::Err(e.clone()), DrillField::Err(e.clone())),
    };
    let (storage_radius, reserve_size, pullsync_rate, batch_commitment) =
        compute_reserve_state_fields(&fetch.peer_status, &fetch.local_status);

    PeerDrillView {
        peer_overlay: peer.to_string(),
        bin,
        balance,
        ping,
        settlement_received,
        settlement_sent,
        last_received_cheque,
        last_sent_cheque,
        storage_radius,
        reserve_size,
        pullsync_rate,
        batch_commitment,
    }
}

fn bin_strip_rows(t: &Topology) -> Vec<BinStripRow> {
    t.bins
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let bin = i as u8;
            let is_relevant = bin <= t.depth.saturating_add(FAR_BIN_RELAXATION);
            BinStripRow {
                bin,
                population: b.population,
                connected: b.connected,
                status: classify_bin(b, bin, t.depth),
                is_relevant,
            }
        })
        .collect()
}

fn compute_saturation_summary(bins: &[BinStripRow]) -> SaturationSummary {
    let mut summary = SaturationSummary::default();
    let mut worst: Option<&BinStripRow> = None;
    for row in bins {
        if row.is_relevant {
            summary.relevant += 1;
        }
        match row.status {
            BinSaturation::Starving => {
                summary.starving += 1;
                let pick_this = match worst {
                    None => true,
                    Some(prev) => {
                        row.connected < prev.connected
                            || (row.connected == prev.connected && row.bin < prev.bin)
                    }
                };
                if pick_this {
                    worst = Some(row);
                }
            }
            BinSaturation::Over => summary.over += 1,
            BinSaturation::Empty | BinSaturation::Healthy => {}
        }
    }
    if let Some(w) = worst {
        summary.worst_bin = Some(w.bin);
        summary.worst_connected = w.connected;
    }
    summary
}

fn classify_bin(b: &BinInfo, bin: u8, depth: u8) -> BinSaturation {
    if b.connected > OVER_SATURATION_PEERS {
        return BinSaturation::Over;
    }
    if b.connected >= SATURATION_PEERS {
        return BinSaturation::Healthy;
    }
    if bin <= depth.saturating_add(FAR_BIN_RELAXATION) {
        BinSaturation::Starving
    } else {
        BinSaturation::Empty
    }
}

fn peer_rows(t: &Topology) -> Vec<PeerRow> {
    let mut out: Vec<PeerRow> = Vec::new();
    for (i, b) in t.bins.iter().enumerate() {
        let bin = i as u8;
        for p in &b.connected_peers {
            out.push(make_peer_row(bin, p));
        }
    }
    out.sort_by(|a, b| {
        a.bin
            .cmp(&b.bin)
            .then_with(|| a.peer_short.cmp(&b.peer_short))
    });
    out
}

fn make_peer_row(bin: u8, p: &PeerInfo) -> PeerRow {
    let peer_short = short_overlay(&p.address);
    let peer_full = p.address.trim_start_matches("0x").to_string();
    let (direction, latency, healthy, reachability) = match &p.metrics {
        Some(m) => {
            let direction = match m.session_connection_direction.as_str() {
                "inbound" => "in",
                "outbound" => "out",
                _ => "?",
            };
            let latency_ms = m.latency_ewma.max(0) as f64 / 1_000_000.0;
            let latency = if m.latency_ewma > 0 {
                format!("{latency_ms:.0}ms")
            } else {
                "—".into()
            };
            (direction, latency, m.healthy, m.reachability.clone())
        }
        None => ("?", "—".into(), false, String::new()),
    };
    PeerRow {
        bin,
        peer_short,
        peer_full,
        direction,
        latency,
        healthy,
        reachability,
    }
}

pub fn short_overlay(s: &str) -> String {
    let trimmed = s.trim_start_matches("0x");
    if trimmed.len() > 10 {
        format!("{}…{}", &trimmed[..6], &trimmed[trimmed.len() - 4..])
    } else {
        trimmed.to_string()
    }
}

fn format_plur_signed(plur: &BigInt) -> String {
    let zero = BigInt::from(0);
    let neg = plur < &zero;
    let abs = if neg { -plur.clone() } else { plur.clone() };
    let scale = BigInt::from(10u64).pow(16);
    let whole = &abs / &scale;
    let frac = &abs % &scale;
    let frac_4 = &frac / BigInt::from(10u64).pow(12);
    let sign = if neg { "-" } else { "+" };
    format!("{sign}BZZ {whole}.{frac_4:0>4}")
}

fn format_opt_plur(plur: Option<&BigInt>) -> String {
    match plur {
        Some(p) => format_plur_signed(p).trim_start_matches('+').to_string(),
        None => "—".to_string(),
    }
}

/// Compute the four reserve-state cells. Outlier rule for
/// `batch_commitment`: |peer - local| / local > 5%.
fn compute_reserve_state_fields(
    peer: &std::result::Result<Option<PeerStatus>, String>,
    local: &std::result::Result<Status, String>,
) -> (
    DrillField<String>,
    DrillField<String>,
    DrillField<String>,
    DrillField<BatchCommitmentCell>,
) {
    let peer_status = match peer {
        Ok(Some(p)) => &p.status,
        Ok(None) => {
            let msg = "(no /status/peers row for this overlay)".to_string();
            return (
                DrillField::Err(msg.clone()),
                DrillField::Err(msg.clone()),
                DrillField::Err(msg.clone()),
                DrillField::Err(msg),
            );
        }
        Err(e) => {
            return (
                DrillField::Err(e.clone()),
                DrillField::Err(e.clone()),
                DrillField::Err(e.clone()),
                DrillField::Err(e.clone()),
            );
        }
    };
    let storage_radius = DrillField::Ok(peer_status.storage_radius.to_string());
    let reserve_size = DrillField::Ok(format_thousands(peer_status.reserve_size));
    let pullsync_rate = DrillField::Ok(format!("{:.2} chunks/s", peer_status.pullsync_rate));
    let outlier = match local {
        Ok(l) if l.batch_commitment > 0 => {
            let delta = (peer_status.batch_commitment - l.batch_commitment).abs() as f64;
            (delta / l.batch_commitment as f64) > 0.05
        }
        _ => false,
    };
    let batch_commitment = DrillField::Ok(BatchCommitmentCell {
        formatted: format_thousands(peer_status.batch_commitment),
        outlier,
    });
    (
        storage_radius,
        reserve_size,
        pullsync_rate,
        batch_commitment,
    )
}

pub fn format_thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    if n < 0 {
        out.push('-');
    }
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}
