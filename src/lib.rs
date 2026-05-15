//! Shared cockpit logic for [`bee-tui`] and [`beegui`].
//!
//! This crate is the renderer-agnostic half of the bee-tui project:
//! HTTP client wrappers around [`bee-rs`], the [`BeeWatch`] polling
//! hub, every snapshot type the screens render from (HealthSnapshot,
//! StampsSnapshot, FleetSnapshot, …), and the pure `view_for` /
//! `compute_*` functions that turn those snapshots into render-ready
//! view structs. The TUI and GUI both consume the same surface.
//!
//! See `PLAN.md` in the repo root for the extraction plan from
//! `bee-tui` and the contract between this crate and its renderers.
//!
//! [`bee-tui`]: https://crates.io/crates/bee-tui
//! [`beegui`]: https://github.com/ethswarm-tools/beegui

// Phase 1 — pure leaves extracted from bee-tui (no crate-internal deps).
pub mod bee_log_discover;
pub mod config_doctor;
pub mod economics_oracle;
pub mod log_capture;
pub mod pprof_bundle;
pub mod state;
pub mod utility_verbs;
pub mod version_check;

// Phase 2 — data backbone. `config` here holds just the items the
// data layer needs (NodeConfig + helpers); the rest of bee-tui's
// config schema (KeyBindings, Styles, the section structs) follows
// in a later phase.
pub mod api;
pub mod config;
pub mod durability;
pub mod feed_probe;
pub mod feed_timeline;
pub mod fleet;
pub mod manifest_walker;
pub mod pubsub;
pub mod stamps;
pub mod uploads;
pub mod watch;
