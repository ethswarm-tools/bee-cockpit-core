# bee-cockpit-core

Shared cockpit logic for [`bee-tui`](https://github.com/ethswarm-tools/bee-tui)
and [`beegui`](https://github.com/ethswarm-tools/beegui).

This crate is the renderer-agnostic half of the bee-tui project — the
data layer, the polling hub, and the pure functions that turn raw
[Bee] state into render-ready views. The TUI and the GUI both depend
on it for one source of truth on "what's a Health gate", "when does
the Stamp TTL go warn", "what does the Fleet roll-up say".

## What's in here (target shape, post-extraction)

- **Bee API client wrappers** — thin layer over [`bee-rs`] for the
  endpoints the cockpit polls (`/health`, `/status`, `/stamps`,
  `/chequebook/*`, `/redistribution`, `/peers`, `/topology`, …).
- **`BeeWatch` poller hub** — every screen subscribes to its own
  `tokio::sync::watch::Receiver<Snapshot>`. Cadences are per-resource
  (2 s health, 5 s topology, 30 s swap, …).
- **Snapshot types** — `HealthSnapshot`, `StampsSnapshot`,
  `SwapSnapshot`, `LotterySnapshot`, `WarmupSnapshot`, `PeersSnapshot`,
  `NetworkSnapshot`, `TransactionsSnapshot`, `TagsSnapshot`,
  `FleetSnapshot`, …
- **Pure `view_for` / `compute_*` functions** — snapshot → view
  (`HealthView` with its `Gate`s, `StampsView`, `SwapView`,
  `FleetView`, …). These are the *semver-stable surface* of the
  crate; the renderer just draws what they return.
- **Alert diff pipeline** + **notification center** + **fleet
  aggregator** — the cross-cutting logic that doesn't belong to any
  one screen.
- **Config schema** — `Config`, `NodeConfig`, `[ui]`, `[alerts]`,
  `[bee]`, `[fleet]`, `[notifications]`, …

## What's NOT in here

The renderers own everything UI:

- **`bee-tui`** owns ratatui components, the TUI app loop, key
  routing, the bottom log pane, the command bar, the help overlay,
  the theme system.
- **`beegui`** owns the egui app shell and widget layer.

The boundary is: this crate produces structured views; renderers
turn those views into pixels.

## Status

🚧 **0.1.0 unreleased.** The extraction from `bee-tui` is in
progress — see [`PLAN.md`](./PLAN.md) for the contract and the
phased move-list.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE)
or [MIT license](./LICENSE-MIT) at your option.

[Bee]: https://www.ethswarm.org/
[`bee-rs`]: https://github.com/ethswarm-tools/bee-rs
