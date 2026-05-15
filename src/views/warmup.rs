//! S5 — Warmup view computation. Pure half of bee-tui's
//! `components::warmup`: every step's [`StepState`], the elapsed
//! counter (passed in by the caller — depends on wall-clock), and
//! the percentage math for each step. The renderer turns this into
//! a checklist row; snapshot tests pin every step without dealing
//! with TUI plumbing or clock jitter.

use std::time::Duration;

use crate::watch::{HealthSnapshot, StampsSnapshot, TopologySnapshot};

/// Bee's reserve size at depth (`pkg/storer/storer.go`). The reserve
/// fill step uses this as the denominator for the percentage line.
pub const RESERVE_TARGET_CHUNKS: i64 = 65_536;
/// Heuristic peer-bootstrap target. Bee doesn't publish a single
/// "we're done discovering peers" threshold — different versions
/// converge anywhere from 30 to 100. We use 50 as a representative
/// midpoint so the bar reaches Done on a typical mainnet node.
pub const PEER_BOOTSTRAP_TARGET: u64 = 50;
/// Number of consecutive depth observations that must agree for the
/// "kademlia depth stable" step to flip Done. Five ticks at the 1 s
/// cadence ≈ five seconds, long enough to ride out the depth churn
/// during peer bootstrap without dragging a steady node down.
pub const DEPTH_STABILITY_WINDOW: usize = 5;

/// One step of the bootstrap checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    /// Step hasn't started yet — renderer typically draws `░`.
    Pending,
    /// Step in progress, with an integer percentage in `0..=100`.
    /// Renderer typically draws `▒`.
    InProgress(u32),
    /// Step latched done — renderer typically draws `✓`.
    Done,
    /// Insufficient data to classify (snapshots not loaded yet) —
    /// renderer typically draws `·`.
    Unknown,
}

/// One row of the warmup checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmupStep {
    pub label: &'static str,
    pub state: StepState,
    /// Per-step detail line (e.g. `"487 batches"`, `"depth 8 (5/5
    /// ticks stable)"`). Rendered dimmed under the step glyph.
    pub detail: String,
}

/// Aggregated view fed to renderers and snapshot tests. The elapsed
/// counter is part of the view (as a [`Duration`]) but is tracked by
/// the caller because it depends on the renderer's first observation
/// of `is_warming_up=true`. Tests pass deterministic values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmupView {
    /// `true` if Bee currently reports `is_warming_up=true`. After
    /// transition to `false` the renderer freezes the elapsed
    /// counter but leaves the screen useful as a "definition of
    /// done" view.
    pub is_warming_up: bool,
    /// Wall-clock duration since the renderer first observed
    /// `is_warming_up=true`. `None` when the snapshot hasn't loaded.
    pub elapsed: Option<Duration>,
    pub steps: Vec<WarmupStep>,
}

/// Pure, snapshot-driven view computation.
pub fn view_for(
    health: &HealthSnapshot,
    stamps: &StampsSnapshot,
    topology: &TopologySnapshot,
    elapsed: Option<Duration>,
    depth_stable: bool,
) -> WarmupView {
    let is_warming_up = health
        .status
        .as_ref()
        .map(|s| s.is_warming_up)
        .unwrap_or(false);
    let steps = vec![
        postage_step(stamps),
        peers_step(health),
        depth_step(topology, depth_stable),
        reserve_step(health),
        stabilization_step(health),
    ];
    WarmupView {
        is_warming_up,
        elapsed,
        steps,
    }
}

fn postage_step(stamps: &StampsSnapshot) -> WarmupStep {
    if stamps.last_update.is_none() {
        return WarmupStep {
            label: "Postage snapshot loaded",
            state: StepState::Unknown,
            detail: "(awaiting first /stamps poll)".into(),
        };
    }
    let count = stamps.batches.len();
    if count == 0 {
        return WarmupStep {
            label: "Postage snapshot loaded",
            state: StepState::Pending,
            detail: "no batches yet — node may not have any postage attached".into(),
        };
    }
    WarmupStep {
        label: "Postage snapshot loaded",
        state: StepState::Done,
        detail: format!("{count} batch(es)"),
    }
}

fn peers_step(health: &HealthSnapshot) -> WarmupStep {
    let Some(s) = &health.status else {
        return WarmupStep {
            label: "Peer bootstrap",
            state: StepState::Unknown,
            detail: "(awaiting first /status poll)".into(),
        };
    };
    let connected = s.connected_peers as u64;
    let pct = pct_of(connected, PEER_BOOTSTRAP_TARGET);
    let detail = format!("{connected} connected (target ≥ {PEER_BOOTSTRAP_TARGET})");
    if connected >= PEER_BOOTSTRAP_TARGET {
        WarmupStep {
            label: "Peer bootstrap",
            state: StepState::Done,
            detail,
        }
    } else if connected == 0 {
        WarmupStep {
            label: "Peer bootstrap",
            state: StepState::Pending,
            detail,
        }
    } else {
        WarmupStep {
            label: "Peer bootstrap",
            state: StepState::InProgress(pct),
            detail,
        }
    }
}

fn depth_step(topology: &TopologySnapshot, depth_stable: bool) -> WarmupStep {
    let Some(t) = &topology.topology else {
        return WarmupStep {
            label: "Kademlia depth stable",
            state: StepState::Unknown,
            detail: "(awaiting first /topology poll)".into(),
        };
    };
    let detail = if depth_stable {
        format!("depth {} (stable across the observation window)", t.depth)
    } else {
        format!("depth {} (still settling)", t.depth)
    };
    let state = if depth_stable {
        StepState::Done
    } else {
        StepState::InProgress(50)
    };
    WarmupStep {
        label: "Kademlia depth stable",
        state,
        detail,
    }
}

fn reserve_step(health: &HealthSnapshot) -> WarmupStep {
    let Some(s) = &health.status else {
        return WarmupStep {
            label: "Reserve fill",
            state: StepState::Unknown,
            detail: "(awaiting first /status poll)".into(),
        };
    };
    let in_radius = s.reserve_size_within_radius.max(0);
    let pct = pct_of(in_radius as u64, RESERVE_TARGET_CHUNKS as u64);
    let detail = format!("{in_radius} / {RESERVE_TARGET_CHUNKS} in-radius chunks");
    if in_radius >= RESERVE_TARGET_CHUNKS {
        WarmupStep {
            label: "Reserve fill",
            state: StepState::Done,
            detail,
        }
    } else if in_radius == 0 {
        WarmupStep {
            label: "Reserve fill",
            state: StepState::Pending,
            detail,
        }
    } else {
        WarmupStep {
            label: "Reserve fill",
            state: StepState::InProgress(pct),
            detail,
        }
    }
}

fn stabilization_step(health: &HealthSnapshot) -> WarmupStep {
    let Some(s) = &health.status else {
        return WarmupStep {
            label: "Stabilization",
            state: StepState::Unknown,
            detail: "(awaiting first /status poll)".into(),
        };
    };
    if !s.is_warming_up {
        WarmupStep {
            label: "Stabilization",
            state: StepState::Done,
            detail: "Bee reports warmup complete".into(),
        }
    } else {
        WarmupStep {
            label: "Stabilization",
            state: StepState::InProgress(50),
            detail: "Bee still reports is_warming_up=true".into(),
        }
    }
}

fn pct_of(num: u64, denom: u64) -> u32 {
    if denom == 0 {
        return 0;
    }
    let q = num.saturating_mul(100) / denom;
    q.min(100) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_of_handles_zero_denom() {
        assert_eq!(pct_of(10, 0), 0);
    }

    #[test]
    fn pct_of_clamps_to_100() {
        assert_eq!(pct_of(200, 100), 100);
    }
}
