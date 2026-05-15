//! S4 — Lottery / redistribution view. Pure half of bee-tui's
//! `components::lottery`: phase enum (commit/reveal/claim/sample),
//! the round-card with phase segments + progress, anchor rows
//! (last-won/played/selected/frozen), and the stake-card status
//! ladder. The renderer owns the rchash benchmark + scroll cursor.

use bee::debug::RedistributionState;
use num_bigint::BigInt;

use crate::views::swap::format_plur;
use crate::watch::{HealthSnapshot, LotterySnapshot};

/// `pkg/storageincentives/agent.go:36` — round length in blocks.
pub const BLOCKS_PER_ROUND: u64 = 152;
/// One-quarter of the round (152 / 4); commit and reveal each take this many
/// blocks, claim takes the remaining 76.
pub const BLOCKS_PER_PHASE: u64 = 38;

/// Discrete phase enum derived from the API's `phase: String`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Commit,
    Reveal,
    Claim,
    /// In-between rounds — the agent is sampling chunks for the next
    /// commit. Bee surfaces this as `phase: "sample"`.
    Sample,
    Unknown,
}

impl Phase {
    pub fn from_api(s: &str) -> Self {
        match s {
            "commit" => Self::Commit,
            "reveal" => Self::Reveal,
            "claim" => Self::Claim,
            "sample" => Self::Sample,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Reveal => "reveal",
            Self::Claim => "claim",
            Self::Sample => "sample",
            Self::Unknown => "?",
        }
    }
}

/// Tri-state per-phase outcome shown in the timeline ribbon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseState {
    Done,
    Active,
    Pending,
}

/// One segment of the timeline ribbon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseSegment {
    pub phase: Phase,
    pub state: PhaseState,
    pub start_block: u64,
    pub end_block: u64,
}

/// Top pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundCard {
    pub round: u64,
    pub block: u64,
    pub block_of_round: u64,
    pub phase: Phase,
    pub phase_label: &'static str,
    pub segments: Vec<PhaseSegment>,
}

/// One row of the anchor-summary pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRow {
    pub label: &'static str,
    pub round: u64,
    pub delta: Option<u64>,
    pub when: String,
}

/// Tri-state stake card status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StakeStatus {
    Unstaked,
    InsufficientGas,
    Frozen,
    Unhealthy,
    Healthy,
    Unknown,
}

impl StakeStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unstaked => "✗ unstaked",
            Self::InsufficientGas => "⚠ low gas",
            Self::Frozen => "✗ frozen",
            Self::Unhealthy => "⚠ unhealthy",
            Self::Healthy => "✓ healthy",
            Self::Unknown => "? unknown",
        }
    }
}

/// Bottom pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakeCard {
    pub status: StakeStatus,
    pub staked: String,
    pub minimum_gas: String,
    pub reward: String,
    pub fees: String,
    pub last_sample: Option<String>,
    pub why: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LotteryView {
    pub round: Option<RoundCard>,
    pub anchors: Vec<AnchorRow>,
    pub stake: StakeCard,
}

/// Pure, snapshot-driven view computation.
pub fn view_for(health: &HealthSnapshot, lottery: &LotterySnapshot) -> LotteryView {
    let round = health.redistribution.as_ref().map(round_card_for);
    let anchors = health
        .redistribution
        .as_ref()
        .map(anchor_rows_for)
        .unwrap_or_default();
    let stake = stake_card_for(health.redistribution.as_ref(), lottery);
    LotteryView {
        round,
        anchors,
        stake,
    }
}

/// Fallback depth when `/status` hasn't reported a `storage_radius`
/// yet. 8 is a typical mainnet radius, so the sample size is
/// representative.
pub const BENCH_DEFAULT_DEPTH: u8 = 8;

/// Pick a depth for the rchash benchmark that mirrors what the agent
/// would actually sample at. Falls back to [`BENCH_DEFAULT_DEPTH`] if
/// `/status` hasn't loaded yet.
pub fn bench_depth(health: &HealthSnapshot) -> u8 {
    let raw = health
        .status
        .as_ref()
        .map(|s| s.storage_radius)
        .unwrap_or(-1);
    if raw <= 0 {
        BENCH_DEFAULT_DEPTH
    } else {
        raw.min(255) as u8
    }
}

fn round_card_for(r: &RedistributionState) -> RoundCard {
    let block_of_round = r.block % BLOCKS_PER_ROUND;
    let phase = Phase::from_api(&r.phase);
    let segments = build_phase_segments(phase, block_of_round);
    RoundCard {
        round: r.round,
        block: r.block,
        block_of_round,
        phase,
        phase_label: phase.label(),
        segments,
    }
}

pub fn build_phase_segments(current: Phase, block_of_round: u64) -> Vec<PhaseSegment> {
    let phases = [
        (Phase::Commit, 0u64, BLOCKS_PER_PHASE),
        (Phase::Reveal, BLOCKS_PER_PHASE, 2 * BLOCKS_PER_PHASE),
        (Phase::Claim, 2 * BLOCKS_PER_PHASE, BLOCKS_PER_ROUND),
    ];
    phases
        .iter()
        .map(|&(p, start, end)| PhaseSegment {
            phase: p,
            state: phase_state_for(p, current, block_of_round, start, end),
            start_block: start,
            end_block: end,
        })
        .collect()
}

fn phase_state_for(
    seg: Phase,
    current: Phase,
    block_of_round: u64,
    start: u64,
    end: u64,
) -> PhaseState {
    if current == seg {
        return PhaseState::Active;
    }
    if matches!(current, Phase::Sample) {
        return PhaseState::Done;
    }
    if block_of_round >= end {
        PhaseState::Done
    } else if block_of_round < start {
        PhaseState::Pending
    } else {
        PhaseState::Active
    }
}

fn anchor_rows_for(r: &RedistributionState) -> Vec<AnchorRow> {
    let current = r.round;
    let make = |label: &'static str, anchor: u64| AnchorRow {
        label,
        round: anchor,
        delta: if anchor == 0 || anchor > current {
            None
        } else {
            Some(current - anchor)
        },
        when: format_when(current, anchor),
    };
    vec![
        make("last won", r.last_won_round),
        make("last played", r.last_played_round),
        make("last selected", r.last_selected_round),
        make("last frozen", r.last_frozen_round),
    ]
}

pub fn format_when(current: u64, anchor: u64) -> String {
    if anchor == 0 {
        return "never".into();
    }
    if anchor > current {
        return format!("round {anchor} (future)");
    }
    let delta = current - anchor;
    match delta {
        0 => "this round".into(),
        1 => "last round".into(),
        n => format!("{n} rounds ago"),
    }
}

fn stake_card_for(r: Option<&RedistributionState>, lottery: &LotterySnapshot) -> StakeCard {
    let zero = BigInt::from(0);
    let staked_bi = lottery.staked.as_ref();
    let staked_str = staked_bi.map(format_plur).unwrap_or_else(|| "—".into());

    let (minimum_gas, reward, fees, last_sample, status_inputs) = match r {
        Some(r) => (
            r.minimum_gas_funds
                .as_ref()
                .map(format_plur)
                .unwrap_or_else(|| "—".into()),
            r.reward
                .as_ref()
                .map(format_plur)
                .unwrap_or_else(|| "—".into()),
            r.fees
                .as_ref()
                .map(format_plur)
                .unwrap_or_else(|| "—".into()),
            (r.last_sample_duration_seconds > 0.0)
                .then(|| format!("{:.1}s", r.last_sample_duration_seconds)),
            Some((
                r.is_frozen,
                r.is_healthy,
                r.has_sufficient_funds,
                r.is_fully_synced,
                r.last_frozen_round,
            )),
        ),
        None => ("—".into(), "—".into(), "—".into(), None, None),
    };

    let (status, why) = match (lottery.last_error.as_deref(), staked_bi, status_inputs) {
        (Some(e), _, _) => (StakeStatus::Unknown, Some(format!("/stake error: {e}"))),
        (_, None, _) => (StakeStatus::Unknown, Some("/stake not loaded yet".into())),
        (_, Some(s), Some((frozen, healthy, sufficient, synced, last_frozen))) if s == &zero => {
            let _ = (frozen, healthy, sufficient, synced, last_frozen);
            (
                StakeStatus::Unstaked,
                Some("0 BZZ staked — node cannot participate in redistribution.".into()),
            )
        }
        (_, Some(_), Some((true, _, _, _, last_frozen))) => (
            StakeStatus::Frozen,
            Some(format!(
                "frozen out at round {last_frozen}; resumes after the freeze window."
            )),
        ),
        (_, Some(_), Some((_, _, false, _, _))) => (
            StakeStatus::InsufficientGas,
            Some("operator wallet has too little native token to play a round.".into()),
        ),
        (_, Some(_), Some((_, false, _, _, _))) => (
            StakeStatus::Unhealthy,
            Some("redistribution worker reports unhealthy — see Health screen.".into()),
        ),
        (_, Some(_), Some((_, _, _, false, _))) => (
            StakeStatus::Unhealthy,
            Some("node is not fully synced — sampling will skip until it catches up.".into()),
        ),
        (_, Some(_), Some(_)) => (StakeStatus::Healthy, None),
        (_, Some(s), None) if s == &zero => (
            StakeStatus::Unstaked,
            Some("0 BZZ staked — node cannot participate in redistribution.".into()),
        ),
        (_, Some(_), None) => (
            StakeStatus::Unknown,
            Some("redistribution state not loaded yet".into()),
        ),
    };

    StakeCard {
        status,
        staked: staked_str,
        minimum_gas,
        reward,
        fees,
        last_sample,
        why,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_from_api_known() {
        assert_eq!(Phase::from_api("commit"), Phase::Commit);
        assert_eq!(Phase::from_api("reveal"), Phase::Reveal);
        assert_eq!(Phase::from_api("claim"), Phase::Claim);
        assert_eq!(Phase::from_api("sample"), Phase::Sample);
    }

    #[test]
    fn phase_from_api_unknown_falls_back() {
        assert_eq!(Phase::from_api(""), Phase::Unknown);
        assert_eq!(Phase::from_api("garbage"), Phase::Unknown);
    }

    #[test]
    fn phase_segments_during_commit() {
        let segs = build_phase_segments(Phase::Commit, 10);
        assert_eq!(segs[0].state, PhaseState::Active);
        assert_eq!(segs[1].state, PhaseState::Pending);
        assert_eq!(segs[2].state, PhaseState::Pending);
    }

    #[test]
    fn phase_segments_during_reveal() {
        let segs = build_phase_segments(Phase::Reveal, 50);
        assert_eq!(segs[0].state, PhaseState::Done);
        assert_eq!(segs[1].state, PhaseState::Active);
        assert_eq!(segs[2].state, PhaseState::Pending);
    }

    #[test]
    fn phase_segments_during_claim() {
        let segs = build_phase_segments(Phase::Claim, 100);
        assert_eq!(segs[0].state, PhaseState::Done);
        assert_eq!(segs[1].state, PhaseState::Done);
        assert_eq!(segs[2].state, PhaseState::Active);
    }

    #[test]
    fn phase_segments_during_sample() {
        let segs = build_phase_segments(Phase::Sample, 0);
        for s in &segs {
            assert_eq!(s.state, PhaseState::Done);
        }
    }

    #[test]
    fn format_when_handles_zero() {
        assert_eq!(format_when(100, 0), "never");
    }

    #[test]
    fn format_when_handles_current() {
        assert_eq!(format_when(100, 100), "this round");
    }

    #[test]
    fn format_when_handles_n_ago() {
        assert_eq!(format_when(100, 95), "5 rounds ago");
    }

    #[test]
    fn bench_depth_falls_back_when_status_missing() {
        assert_eq!(bench_depth(&HealthSnapshot::default()), BENCH_DEFAULT_DEPTH);
    }

    #[test]
    fn bench_depth_falls_back_on_sentinel() {
        let snap = HealthSnapshot {
            status: Some(bee::debug::Status {
                storage_radius: -1,
                ..bee::debug::Status::default()
            }),
            ..HealthSnapshot::default()
        };
        assert_eq!(bench_depth(&snap), BENCH_DEFAULT_DEPTH);
    }

    #[test]
    fn bench_depth_uses_storage_radius_when_present() {
        let snap = HealthSnapshot {
            status: Some(bee::debug::Status {
                storage_radius: 12,
                ..bee::debug::Status::default()
            }),
            ..HealthSnapshot::default()
        };
        assert_eq!(bench_depth(&snap), 12);
    }
}
