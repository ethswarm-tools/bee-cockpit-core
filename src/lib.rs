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

// Modules will land here as they're extracted from bee-tui — see PLAN.md.
