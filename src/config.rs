//! Config schema items needed by the data layer (`api`, `watch`,
//! `fleet`). The full `Config` struct + `[ui]` / `[alerts]` /
//! `[fleet]` / `[bee]` / `[notifications]` / `[durability]` /
//! `[pubsub]` / `[metrics]` / `[economics]` section structs plus
//! `KeyBindings` / `Styles` still live in `bee-tui` for now — they
//! move into core in a follow-up phase. See `PLAN.md` for the full
//! plan.

use std::{env, path::PathBuf};

use serde::Deserialize;

/// One configured Bee node. Multiple may coexist; multi-node UX is
/// targeted at v0.4 but the schema supports it from day one.
#[derive(Clone, Debug, Deserialize)]
pub struct NodeConfig {
    /// Friendly label shown in the UI (e.g. `"prod-1"`, `"local"`).
    pub name: String,
    /// Bee API base URL (e.g. `"http://localhost:1633"`).
    pub url: String,
    /// Optional bearer token for restricted-mode nodes. Supports the
    /// `@env:VAR_NAME` indirection — see [`NodeConfig::resolved_token`].
    #[serde(default)]
    pub token: Option<String>,
    /// Optional path to this node's log file. When set and bee-tui is
    /// *not* spawning Bee itself (no `[bee]` block / `--bee-bin`), the
    /// cockpit tails this file to populate the bottom log pane's
    /// Bee-side tabs (Errors / Warn / Info / Debug / Bee HTTP). Tailing
    /// starts at end-of-file — pre-existing history is not replayed.
    /// Ignored when bee-tui owns the supervisor (the supervised child's
    /// captured log is tailed instead).
    #[serde(default)]
    pub log_file: Option<PathBuf>,
    /// Optional shell command whose stdout streams this node's log —
    /// e.g. `journalctl -u bee -f`, `docker logs -f bee 2>&1`,
    /// `ssh host 'tail -f /var/log/bee.log'`. Run via `sh -c`, so
    /// pipes / quoting / redirects work. Lets bee-tui surface logs
    /// for a node whose log *file* it can't read directly (remote
    /// host, container, restricted permissions). Takes precedence
    /// over `log_file` when both are set. Same supervisor caveat as
    /// `log_file` — ignored when bee-tui spawns Bee itself.
    #[serde(default)]
    pub log_command: Option<String>,
    /// Marks the default profile loaded on startup. If no entry has
    /// `default = true`, the first node in the list is used.
    #[serde(default)]
    pub default: bool,
}

impl NodeConfig {
    /// Resolve `token` to its concrete value: `Some(env_var)` if the
    /// configured value starts with `@env:`, otherwise the literal.
    pub fn resolved_token(&self) -> Option<String> {
        let raw = self.token.as_deref()?;
        if let Some(var) = raw.strip_prefix("@env:") {
            env::var(var).ok()
        } else {
            Some(raw.to_string())
        }
    }
}

/// `[bee.logs]` table from `config.toml`. Bounds the size of the
/// supervised Bee process's captured stdout+stderr file so a
/// long-running node doesn't fill `$TMPDIR`.
#[derive(Clone, Debug, Deserialize)]
pub struct BeeLogsConfig {
    /// Active log file rolls over once it reaches this many MiB.
    /// Default 64 MiB — large enough that operator-relevant traces
    /// fit in the live file, small enough that rotation happens
    /// within a day or two on a busy node.
    #[serde(default = "default_rotate_size_mb")]
    pub rotate_size_mb: u64,
    /// How many rotated files (`.1` .. `.N`) to retain. Default 5.
    /// At the 64 MiB default that's ~320 MiB of log history kept on
    /// disk; older content is unlinked.
    #[serde(default = "default_keep_files")]
    pub keep_files: u32,
}

impl Default for BeeLogsConfig {
    fn default() -> Self {
        Self {
            rotate_size_mb: default_rotate_size_mb(),
            keep_files: default_keep_files(),
        }
    }
}

fn default_rotate_size_mb() -> u64 {
    64
}
fn default_keep_files() -> u32 {
    5
}

/// `data_dir` / `config_dir` overrides surfaced from the config crate
/// (set by `Config::load`'s defaults).
#[derive(Clone, Debug, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub config_dir: PathBuf,
}

/// `[bee]` table from `config.toml`. Both fields are required so a
/// malformed `[bee]` block fails parse rather than silently spawning
/// nothing.
#[derive(Clone, Debug, Deserialize)]
pub struct BeeConfig {
    /// Path to the `bee` binary. Resolved relative to the working
    /// directory if not absolute — operators usually run bee-tui from
    /// the same shell they used to test the binary, so this is the
    /// least surprising behavior.
    pub bin: PathBuf,
    /// Path to the Bee YAML config file the binary should be started
    /// with. Same relative-to-cwd resolution as `bin`.
    pub config: PathBuf,
    /// `[bee.logs]` subsection — log rotation knobs. Both fields
    /// optional; an absent `[bee.logs]` keeps defaults of 64 MiB
    /// rotation at 5 retained files (~320 MiB ceiling).
    #[serde(default)]
    pub logs: BeeLogsConfig,
    /// `[bee.supervisor]` subsection — auto-restart policy applied
    /// when bee-tui acts as Bee's parent (`[bee].bin` set). Absent
    /// block keeps the v1.11 behaviour: log the crash, dim the top
    /// bar chip, no restart.
    #[serde(default)]
    pub supervisor: BeeSupervisorConfig,
}

/// `[bee.supervisor]` table. Off by default — pre-v1.12 behaviour
/// was "single-shot, no restart". Setting `auto_restart = true`
/// turns on the watchdog with exponential backoff and a per-hour
/// budget; everything else has a sensible default.
#[derive(Clone, Debug, Deserialize)]
pub struct BeeSupervisorConfig {
    /// When `true`, bee-tui re-spawns Bee after the child exits
    /// (any reason — clean exit, signal, OOM kill). When `false`
    /// (default), the supervisor goes dim and reports the exit;
    /// operators restart bee-tui to try again.
    #[serde(default)]
    pub auto_restart: bool,
    /// Maximum restarts allowed within a rolling one-hour window.
    #[serde(default = "default_max_restarts_per_hour")]
    pub max_restarts_per_hour: u32,
    /// Initial backoff in seconds; doubles after each restart up to
    /// `backoff_max_secs`. Default 1.
    #[serde(default = "default_backoff_initial_secs")]
    pub backoff_initial_secs: u64,
    /// Cap on the exponential backoff. Default 30 s.
    #[serde(default = "default_backoff_max_secs")]
    pub backoff_max_secs: u64,
}

impl Default for BeeSupervisorConfig {
    fn default() -> Self {
        Self {
            auto_restart: false,
            max_restarts_per_hour: default_max_restarts_per_hour(),
            backoff_initial_secs: default_backoff_initial_secs(),
            backoff_max_secs: default_backoff_max_secs(),
        }
    }
}

fn default_max_restarts_per_hour() -> u32 {
    6
}
fn default_backoff_initial_secs() -> u64 {
    1
}
fn default_backoff_max_secs() -> u64 {
    30
}

/// `[metrics]` table from `config.toml`. Off by default — a
/// Prometheus endpoint is a network-facing surface, even if it
/// binds to localhost, so we make it a deliberate opt-in.
#[derive(Clone, Debug, Deserialize)]
pub struct MetricsConfig {
    /// Master switch. `false` skips spawning the HTTP server entirely.
    #[serde(default)]
    pub enabled: bool,
    /// Bind address. Defaults to localhost.
    #[serde(default = "default_metrics_addr")]
    pub addr: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            addr: default_metrics_addr(),
        }
    }
}

fn default_metrics_addr() -> String {
    "127.0.0.1:9101".into()
}

/// `[economics]` table from `config.toml`. Optional cost-context
/// oracles. Both fields have sensible defaults so omitting the
/// table entirely is fine.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct EconomicsConfig {
    /// JSON-RPC endpoint that the `:basefee` verb queries for
    /// Gnosis-chain gas pricing.
    #[serde(default)]
    pub gnosis_rpc_url: Option<String>,
    /// When `true`, S3 SWAP renders an always-on Market tile.
    #[serde(default)]
    pub enable_market_tile: bool,
}

/// `[durability]` table from `config.toml`. Knobs for the chunk-graph
/// walker that powers `:durability-check` + `:watch-ref`.
#[derive(Clone, Debug, Deserialize)]
pub struct DurabilityConfig {
    /// When `true`, every completed durability walk probes
    /// `swarmscan_url` for an independent "does the network see
    /// this ref" answer.
    #[serde(default)]
    pub swarmscan_check: bool,
    /// URL template the swarmscan probe hits; `{ref}` is replaced
    /// with the hex-encoded reference at request time.
    #[serde(default = "default_swarmscan_url")]
    pub swarmscan_url: String,
}

impl Default for DurabilityConfig {
    fn default() -> Self {
        Self {
            swarmscan_check: false,
            swarmscan_url: default_swarmscan_url(),
        }
    }
}

fn default_swarmscan_url() -> String {
    "https://api.swarmscan.io/v1/chunks/{ref}".into()
}

/// `[pubsub]` table from `config.toml`. Off by default — fresh
/// installs don't write any pubsub messages to disk.
#[derive(Clone, Debug, Deserialize)]
pub struct PubsubConfig {
    /// Path to a JSONL file that bee-tui appends to whenever a
    /// pubsub frame arrives.
    #[serde(default)]
    pub history_file: Option<PathBuf>,
    /// Active history file rolls over once it reaches this many MiB.
    /// Default 64 MiB. Zero disables rotation.
    #[serde(default = "default_pubsub_rotate_size_mb")]
    pub rotate_size_mb: u64,
    /// How many rotated history files to retain. Default 5.
    #[serde(default = "default_pubsub_keep_files")]
    pub keep_files: u32,
}

impl Default for PubsubConfig {
    fn default() -> Self {
        Self {
            history_file: None,
            rotate_size_mb: default_pubsub_rotate_size_mb(),
            keep_files: default_pubsub_keep_files(),
        }
    }
}

fn default_pubsub_rotate_size_mb() -> u64 {
    64
}

fn default_pubsub_keep_files() -> u32 {
    5
}

/// `[alerts]` table from `config.toml`. Off by default — without a
/// `webhook_url`, the alerter is a no-op.
#[derive(Clone, Debug, Deserialize)]
pub struct AlertsConfig {
    /// Slack/Discord-compatible incoming-webhook URL.
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Per-gate debounce window in seconds. Default 300 (5 min).
    #[serde(default = "default_alerts_debounce_secs")]
    pub debounce_secs: u64,
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            webhook_url: None,
            debounce_secs: default_alerts_debounce_secs(),
        }
    }
}

fn default_alerts_debounce_secs() -> u64 {
    // 5 minutes. Mirrors bee-tui's alerts::DEFAULT_DEBOUNCE_SECS;
    // duplicated as a literal here so this struct doesn't need to
    // import from the alerts module (which still lives in bee-tui).
    5 * 60
}

/// `[fleet]` table from `config.toml`. Off by default — the S15
/// Fleet screen works regardless of whether this is configured;
/// the only thing this section enables is the aggregate webhook.
#[derive(Clone, Debug, Deserialize)]
pub struct FleetConfig {
    /// Slack / Discord-compatible incoming-webhook URL.
    #[serde(default)]
    pub aggregate_webhook_url: Option<String>,
    /// Coalesce window for fleet-aggregate webhooks, in seconds.
    /// Default 60.
    #[serde(default = "default_fleet_window_secs")]
    pub aggregate_window_secs: u64,
}

impl Default for FleetConfig {
    fn default() -> Self {
        Self {
            aggregate_webhook_url: None,
            aggregate_window_secs: default_fleet_window_secs(),
        }
    }
}

fn default_fleet_window_secs() -> u64 {
    60
}

/// `[notifications]` table — v1.14 in-cockpit notification center.
#[derive(Clone, Debug, Deserialize)]
pub struct NotificationsConfig {
    /// In-cockpit transient toast in the top-right corner. Default
    /// `true`.
    #[serde(default = "default_toast_enabled")]
    pub toast_enabled: bool,
    /// How long (seconds) a toast stays on screen before
    /// auto-dismissing. Default 8.
    #[serde(default = "default_toast_seconds")]
    pub toast_seconds: u64,
    /// Fire a libnotify / OS-level notification for Fail / Warn
    /// events. Default `false`.
    #[serde(default)]
    pub desktop: bool,
    /// Terminal-bell threshold. `"off"` (default), `"fail"`, `"warn"`.
    #[serde(default = "default_bell")]
    pub bell: String,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            toast_enabled: default_toast_enabled(),
            toast_seconds: default_toast_seconds(),
            desktop: false,
            bell: default_bell(),
        }
    }
}

fn default_toast_enabled() -> bool {
    true
}
fn default_toast_seconds() -> u64 {
    8
}
fn default_bell() -> String {
    "off".into()
}

/// `[ui]` table from `config.toml`. Every field has a sensible
/// default so the entire section can be omitted without breaking
/// startup.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct UiConfig {
    /// Theme name. Recognised values: `"default"`, `"mono"`.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// ASCII fallback for terminals without Unicode rendering.
    #[serde(default)]
    pub ascii_fallback: bool,
    /// Polling-cadence preset: `"live"` / `"default"` / `"slow"`.
    #[serde(default = "default_refresh")]
    pub refresh: String,
}

fn default_theme() -> String {
    "default".into()
}

fn default_refresh() -> String {
    "default".into()
}
