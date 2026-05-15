# Extraction plan: `bee-tui` → `bee-cockpit-core`

This is the contract for splitting `bee-tui` into a renderer-agnostic
core (`bee-cockpit-core`) and a TUI renderer that depends on it. A
sibling renderer (`beegui`, egui-based) will depend on the same core.

After the extraction:
- One source of truth for cockpit logic (gates, fleet roll-up, alert
  pipeline, stamp economics, …).
- TUI and GUI ports stay synchronized at the data + logic layer.
- bee-tui's renderer surface shrinks to roughly `components/`, `app`,
  the log pane, the theme, key routing, and the help/command-bar UI.

## The boundary

`bee-cockpit-core` owns **data + pure logic + I/O against Bee**:
HTTP client wrappers, the polling hub, snapshot types, the pure
`view_for` / `compute_*` view functions and their `View` structs,
the alert diff pipeline, the notification center, the fleet
aggregator, the config schema, the `--once` verb implementations,
the Bee supervisor + log tailer.

The renderers (`bee-tui`, `beegui`) own **everything UI**:
`Component` traits / egui widget impls, the app loop, key routing,
themes/glyphs, the log pane viewport, the help overlay, the command
bar, the CLI binary's `clap` struct.

In one line: `bee-cockpit-core` produces structured views; renderers
turn those views into pixels.

## What moves into `bee-cockpit-core`

### Top-level modules (move whole)

- `alerts` — alert diff pipeline, transition logic, `is_worth_alerting`
- `bee_log` — Bee log line parser
- `bee_log_discover` — `/proc`-based log auto-discovery
- `bee_log_tailer` — async tail of file or command stdout
- `bee_log_writer` — supervised-Bee log rotation
- `bee_supervisor` — Bee child-process supervisor + watchdog
- `config_doctor` — Bee yaml audit
- `durability` — chunk-graph durability check
- `economics_oracle` — xBZZ price + Gnosis basefee
- `feed_probe` — feed-manifest lookups
- `feed_timeline` — feed timeline backend
- `fleet` — fleet poller + `FleetSnapshot` + fleet aggregator
- `log_capture` — HTTP-request log stats (pure)
- `manifest_walker` — Mantaray traversal
- `metrics` — Prometheus metric definitions
- `metrics_server` — `/metrics` HTTP listener
- `notifications` — notification center (history + ingestion)
- `once` — `--once` verb dispatch + implementations
- `pprof_bundle` — pprof CPU bundle
- `pubsub` — pubsub PSS/GSOC watch backend, history writer
- `stamp_preview` — stamp economics math (topup/dilute/extend previews)
- `state` — persistent state file (last seen, etc.)
- `uploads` — `upload-file` / `upload-collection` logic
- `utility_verbs` — pure-local verbs (`:hash`, `:cid`, `:pss-target`, …)
- `version_check` — `:check-version` verb
- `api/` — bee-rs client wrapper (`ApiClient`)
- `watch/` — `BeeWatch` polling hub + every `Snapshot` type

### `config` (split)

The schema halves (`Config`, `NodeConfig`, `[bee]`, `[fleet]`,
`[alerts]`, `[notifications]`, `[durability]`, `[pubsub]`,
`[metrics]`, `[economics]` + the file-discovery + `--config`
override + `nodes_from_urls`) move to **core**. The renderer-only
halves (`KeyBindings`, `Styles`, `[ui]` semantics that drive theme
glyphs) stay in their respective renderers and merge over the core
config at app startup.

### Per-screen split (one per `bee-tui/src/components/<name>.rs`)

Each current component file holds BOTH pure logic (`view_for`,
`compute_*`, the `View` struct, gate/row helpers) AND
ratatui-specific render code. Each splits in half:

| File | Pure parts → core | Renderer parts → bee-tui |
|---|---|---|
| `health.rs` | `Gate`, `GateStatus`, `HealthView`, `gates_for`, `compute_*` gates | `Health` Component, `draw`, key handling, scroll |
| `stamps.rs` | `BatchRow`, `BucketRow`, `StampsView`, `BucketDrillView`, classification | `Stamps` Component, drill pane, scroll |
| `swap.rs` | `ChequebookCard`, `CheckRow`, `SettlementRow`, `SwapView`, `MarketTile`, `view_for` | `Swap` Component, `←→` focus, scroll |
| `lottery.rs` | `LotteryView`, `AnchorRow`, `StakeCard`, segments + progress math | `Lottery` Component, rchash bench, scroll |
| `warmup.rs` | `WarmupView`, step compute, freeze-on-warmup-end | `Warmup` Component |
| `peers.rs` | `PeerRow`, `PeersView`, `BinSummary`, drill-result View | `Peers` Component, drill pane, scroll |
| `network.rs` | `UnderlayView`, `NetworkView`, reachability stability | `Network` Component, scroll |
| `api_health.rs` | `ApiStatsView`, `PendingTxView` | `ApiHealth` Component, scroll |
| `tags.rs` | `TagRow`, `TagStatusBadge`, `TagsView` | `Tags` Component, scroll |
| `pins.rs` | `PinRow`, `PinsView`, drill View | `Pins` Component, drill pane, scroll |
| `manifest.rs` | `ManifestNodeRow`, `ManifestView`, tree walk | `Manifest` Component, expand/collapse |
| `watchlist.rs` | `WatchEntryRow`, `WatchlistView` | `Watchlist` Component |
| `feed_timeline.rs` | `FeedEntryRow`, `FeedTimelineView` | `FeedTimeline` Component, scroll |
| `pubsub.rs` | `PubsubRow`, `PubsubView`, filter logic | `Pubsub` Component, scroll, clear |
| `fleet.rs` | `FleetView`, `FleetRowView`, `FleetHeader`, `row_view` | `Fleet` Component, `Enter`-to-switch, scroll |
| `log_pane.rs` | (mostly renderer) | `LogPane` Component, fullscreen toggle, `/` filter |
| `scroll.rs` | — | shared scroll helpers (`clamp_scroll`, `clamp_offset`, `scroll_key`, `render_scrollbar`) |

## What stays in `bee-tui`

- `action.rs` — App action enum (TUI key-driven)
- `app.rs` — TUI app loop, overrides, supervisor wiring, event routing
- `cli.rs` — bee-tui's `clap` CLI struct (renderer-specific flags)
- `components.rs` + `components/*.rs` — ratatui Components (post-split)
- `errors.rs` — `color_eyre` install
- `logging.rs` — `tracing` subscriber init
- `main.rs` — binary entry point
- `theme.rs` — ratatui-specific glyphs + colours
- `tui.rs` — terminal setup, alt-screen handling

## Phased execution (next session)

1. **Move pure leaves first** (no bee-tui-internal deps): `bee_log`,
   `bee_log_discover`, `log_capture`, `stamp_preview`, `manifest_walker`,
   `utility_verbs`, `version_check`, `economics_oracle`. Each move:
   `git mv` → adjust imports → `cargo build` green → snapshot.
2. **Move data backbone**: `api/`, snapshot types, `watch/`,
   `config` schema half, `state`, `metrics`.
3. **Move cross-cutting pipelines**: `alerts`, `notifications`,
   `fleet` (poller + aggregator), `pubsub` backend, `feed_probe`,
   `feed_timeline`, `durability`, `uploads`, `config_doctor`.
4. **Move `bee_supervisor` + `bee_log_writer` + `bee_log_tailer`**
   (the Bee process / log machinery).
5. **Move `once`** — `--once` verb dispatch + implementations.
6. **Split components**: for each `components/<name>.rs`, extract the
   pure view + types to `bee-cockpit-core::views::<name>`, leave the
   `Component` impl in `bee-tui::components::<name>` importing from
   core. Snapshot tests in `tests/sN_*_view.rs` re-target the core
   path; renderer tests stay in `bee-tui`.
7. **Verify**: `cargo build --release` clean on both crates; 509+
   lib tests + all integration test binaries pass; clippy + fmt
   clean; mdBook builds. Live smoke against a running Bee.
8. **Publish**: `bee-cockpit-core` 0.1.0 to crates.io; cut bee-tui
   v1.17.0 with the new dependency.

## Versioning

`bee-cockpit-core` starts at **0.1.0**, pre-1.0 by intent: the
extraction will exercise the API shape, and a 0.x window lets us
adjust the surface as the second renderer (`beegui`) consumes it.
Once the GUI port stabilises against the same core, bee-cockpit-core
goes 1.0.0 with the same semver-stable surface promise bee-tui
already makes.

Renderer crates pin to a compatible core minor (e.g. `0.1`), so
both update in lockstep when the core moves.

## Tests

- `bee-cockpit-core` owns the per-view snapshot tests
  (`tests/sN_*_view.rs` in bee-tui today) and the alert / fleet /
  notification pipeline tests.
- Each renderer owns its own widget / interaction tests.
- A single integration test in `bee-tui` proves the renderer
  successfully consumes a core `View` end-to-end (smoke-level —
  the bulk of behaviour is asserted at the core layer).
