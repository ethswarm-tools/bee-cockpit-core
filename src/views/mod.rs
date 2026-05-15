//! Renderer-agnostic per-screen view structs and `view_for` /
//! `compute_*` functions. Each module mirrors a screen / pane in
//! bee-tui's `components/<name>.rs`: the pure view-data half lives
//! here, the ratatui-bound rendering half stays in bee-tui (and
//! beegui will grow its own egui-bound half).
//!
//! The contract: a `view_for(snapshots) -> View` function (or a small
//! set of `compute_*` helpers when the screen has multiple panes)
//! that turns one or more [`crate::watch`] snapshots into a
//! deterministic, ratatui-free struct the renderer pages straight
//! onto the screen. No `Component` impls, no `Frame`, no `Style`,
//! no `Color`.

pub mod api_health;
pub mod manifest;
pub mod tags;
pub mod warmup;
pub mod watchlist;
