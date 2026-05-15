//! Renderer-agnostic config schema, file-discovery helpers, and the
//! generic [`load_raw`] loader. The data-shape half — section structs
//! (`BeeConfig`, `AlertsConfig`, …), the top-level [`Config`] struct,
//! and the loader — lives here. The pieces that depend on TUI-only
//! types (`KeyBindings`, `Styles`, key/colour parsers) stay in
//! `bee-tui`; that crate wraps core's `Config` with `TuiConfig`.

use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, de::DeserializeOwned};

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

/// Renderer-specific path prefixes. Each renderer (bee-tui, beegui)
/// supplies its own so core's path helpers can stay renderer-agnostic
/// while still resolving the right env-var override / platform
/// config-dir / `~/.config/<app>/` triple.
#[derive(Clone, Copy, Debug)]
pub struct ConfigPaths {
    /// Used as the project name in `ProjectDirs::from("com",
    /// "ethswarm-tools", app_name)` and as the leaf folder under
    /// `~/.config/<app_name>/` (the cross-platform fallback path).
    pub app_name: &'static str,
    /// Env var that overrides the config directory when set.
    /// E.g. `"BEE_TUI_CONFIG"` for bee-tui.
    pub config_env: &'static str,
    /// Env var that overrides the data directory when set.
    /// E.g. `"BEE_TUI_DATA"` for bee-tui.
    pub data_env: &'static str,
}

/// Default node list when the user hasn't configured any: a single
/// `local` profile pointing at `http://localhost:1633`.
pub fn default_nodes() -> Vec<NodeConfig> {
    vec![NodeConfig {
        name: "local".to_string(),
        url: "http://localhost:1633".to_string(),
        token: None,
        log_file: None,
        log_command: None,
        default: true,
    }]
}

/// Prepend `http://` to a scheme-less URL so `localhost:1633` works
/// as a positional argument.
pub fn normalize_url(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("http://{url}")
    }
}

/// Extract the host (no scheme, no port, no path) from a URL.
/// Handles `[ipv6]:port` and `host:port` forms.
pub fn host_of(url: &str) -> &str {
    let no_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host_port = no_scheme.split(['/', '?', '#']).next().unwrap_or(no_scheme);
    if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host_port,
        }
    }
}

/// Derive a short node name from a URL. A multi-label domain
/// (`bee-eu.example.com`) collapses to its first label (`bee-eu`);
/// bare hosts (`localhost`) and IP literals are kept whole. Returns
/// an empty string when no host can be parsed.
pub fn node_name_from_url(url: &str) -> String {
    let host = host_of(url);
    if host.is_empty() {
        return String::new();
    }
    let ip_like = host.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !ip_like && host.contains('.') {
        host.split('.').next().unwrap_or(host).to_string()
    } else {
        host.to_string()
    }
}

/// Build an ad-hoc node list from positional URL arguments
/// (`bee-tui url1 url2 …`). The first URL is the default/active node;
/// names are derived from each URL's host with `-2`, `-3`, … suffixes
/// on collision and a `nodeN` fallback when no host parses.
/// Scheme-less URLs are normalised to `http://`.
pub fn nodes_from_urls(urls: &[String]) -> Vec<NodeConfig> {
    let mut used: HashSet<String> = HashSet::new();
    urls.iter()
        .enumerate()
        .map(|(i, raw)| {
            let url = normalize_url(raw);
            let derived = node_name_from_url(&url);
            let base = if derived.is_empty() {
                format!("node{}", i + 1)
            } else {
                derived
            };
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = format!("{base}-{n}");
                n += 1;
            }
            NodeConfig {
                name,
                url,
                token: None,
                log_file: None,
                log_command: None,
                default: i == 0,
            }
        })
        .collect()
}

/// Config file names recognised in a search directory, in precedence
/// order. The first one present wins.
pub const CONFIG_FILE_CANDIDATES: [(&str, config::FileFormat); 5] = [
    ("config.json5", config::FileFormat::Json5),
    ("config.json", config::FileFormat::Json),
    ("config.yaml", config::FileFormat::Yaml),
    ("config.toml", config::FileFormat::Toml),
    ("config.ini", config::FileFormat::Ini),
];

/// Map a config file path to its [`config::FileFormat`] by extension.
/// Returns `None` for an unrecognised or missing extension. Backs the
/// `--config <file>` flag, which (unlike the directory search) has no
/// fixed file name to key the format off.
pub fn format_from_extension(path: &Path) -> Option<config::FileFormat> {
    match path
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "toml" => Some(config::FileFormat::Toml),
        "json5" => Some(config::FileFormat::Json5),
        "json" => Some(config::FileFormat::Json),
        "yaml" | "yml" => Some(config::FileFormat::Yaml),
        "ini" => Some(config::FileFormat::Ini),
        _ => None,
    }
}

pub fn dedup_dirs(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    dirs.into_iter()
        .filter(|d| seen.insert(d.clone()))
        .collect()
}

/// [`ProjectDirs`] for the calling renderer.
pub fn project_directory(paths: &ConfigPaths) -> Option<ProjectDirs> {
    ProjectDirs::from("com", "ethswarm-tools", paths.app_name)
}

/// Platform-native config directory: XDG on Linux, `Application
/// Support` on macOS, Known Folders on Windows. Last-resort entry in
/// [`config_search_dirs`].
pub fn platform_config_dir(paths: &ConfigPaths) -> PathBuf {
    if let Some(proj_dirs) = project_directory(paths) {
        proj_dirs.config_local_dir().to_path_buf()
    } else {
        PathBuf::from(".").join(".config")
    }
}

/// Ordered list of directories searched for a config file. The first
/// directory that holds a recognised `config.*` file wins.
/// `~/.config/<app_name>/` is searched on *every* platform so macOS
/// and Windows devs don't have to hunt down the platform-native path.
pub fn config_search_dirs(paths: &ConfigPaths) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(explicit) = env::var(paths.config_env).ok().map(PathBuf::from) {
        dirs.push(explicit);
    }
    if let Some(base) = directories::BaseDirs::new() {
        dirs.push(base.home_dir().join(".config").join(paths.app_name));
    }
    dirs.push(platform_config_dir(paths));
    dedup_dirs(dirs)
}

/// The directory a config file was actually found in — the first
/// entry of [`config_search_dirs`] that contains a recognised
/// `config.*`. `None` when no config file exists anywhere on the
/// search path.
pub fn resolved_config_dir(paths: &ConfigPaths) -> Option<PathBuf> {
    config_search_dirs(paths).into_iter().find(|dir| {
        CONFIG_FILE_CANDIDATES
            .iter()
            .any(|(file, _)| dir.join(file).exists())
    })
}

/// The config directory the renderer uses: the resolved one if a
/// config file exists, otherwise the env-var override, otherwise the
/// platform-native default.
pub fn get_config_dir(paths: &ConfigPaths) -> PathBuf {
    resolved_config_dir(paths)
        .or_else(|| env::var(paths.config_env).ok().map(PathBuf::from))
        .unwrap_or_else(|| platform_config_dir(paths))
}

/// Data directory: the env-var override if set, otherwise the
/// platform-native data-local dir from [`ProjectDirs`], otherwise
/// `./.data`.
pub fn get_data_dir(paths: &ConfigPaths) -> PathBuf {
    if let Some(s) = env::var(paths.data_env).ok().map(PathBuf::from) {
        s
    } else if let Some(proj_dirs) = project_directory(paths) {
        proj_dirs.data_local_dir().to_path_buf()
    } else {
        PathBuf::from(".").join(".data")
    }
}

/// Top-level config schema, renderer-agnostic. bee-tui wraps this with
/// `TuiConfig` to add `keybindings` + `styles`; beegui will compose
/// it similarly with whatever GUI-only settings it needs.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default, flatten)]
    pub config: AppConfig,
    #[serde(default = "default_nodes")]
    pub nodes: Vec<NodeConfig>,
    /// `[ui]` section — theme + ascii-fallback knobs.
    #[serde(default)]
    pub ui: UiConfig,
    /// `[bee]` section — when present, the renderer spawns the Bee
    /// node itself before opening the cockpit. Absence keeps the
    /// legacy behaviour of connecting to an already-running Bee.
    #[serde(default)]
    pub bee: Option<BeeConfig>,
    /// `[metrics]` section — when present and `enabled = true`, the
    /// renderer exposes a Prometheus `/metrics` endpoint on the
    /// configured address. Default off because exposing an HTTP
    /// listener should be an explicit operator opt-in.
    #[serde(default)]
    pub metrics: MetricsConfig,
    /// `[economics]` section — optional cost-context oracles
    /// (xBZZ → USD price + Gnosis chain gas). The `:price` verb works
    /// without configuration (uses Swarm's public token service);
    /// `:basefee` requires `gnosis_rpc_url` to be set.
    #[serde(default)]
    pub economics: EconomicsConfig,
    /// `[alerts]` section — webhook ping when a health gate flips.
    /// Disabled when `webhook_url` is absent (the default).
    #[serde(default)]
    pub alerts: AlertsConfig,
    /// `[durability]` section — knobs for `:durability-check` and
    /// `:watch-ref`.
    #[serde(default)]
    pub durability: DurabilityConfig,
    /// `[pubsub]` section — optional history-file writer for the
    /// S15 Pubsub watch live tail.
    #[serde(default)]
    pub pubsub: PubsubConfig,
    /// `[fleet]` section — fleet-aggregate webhook.
    #[serde(default)]
    pub fleet: FleetConfig,
    /// `[notifications]` section — in-cockpit notification center.
    #[serde(default)]
    pub notifications: NotificationsConfig,
}

impl Config {
    /// Pick the active node profile: first entry with
    /// `default = true`, otherwise the first entry, otherwise
    /// [`None`].
    pub fn active_node(&self) -> Option<&NodeConfig> {
        self.nodes
            .iter()
            .find(|n| n.default)
            .or_else(|| self.nodes.first())
    }
}

/// Generic config loader. File discovery + env-var overrides +
/// deserialization, parameterised over the renderer's concrete config
/// type `T`. Renderers call this with `T = TuiConfig` /
/// `T = GuiConfig`; core never has to know about renderer-only fields
/// like `keybindings`/`styles`.
///
/// When `explicit_file` is `Some`, that exact file is loaded — it
/// must exist and have a recognised extension, and the directory
/// search is skipped entirely. This backs the `--config` CLI flag.
/// When `None`, the standard search path is used and missing files
/// silently fall through to `T::default()` for unspecified fields.
pub fn load_raw<T>(
    paths: &ConfigPaths,
    explicit_file: Option<&Path>,
) -> Result<T, config::ConfigError>
where
    T: DeserializeOwned + Default,
{
    let data_dir = get_data_dir(paths);
    let config_dir = get_config_dir(paths);
    let mut builder = config::Config::builder()
        .set_default("data_dir", data_dir.to_str().unwrap())?
        .set_default("config_dir", config_dir.to_str().unwrap())?;

    if let Some(file) = explicit_file {
        let format = format_from_extension(file).ok_or_else(|| {
            config::ConfigError::Message(format!(
                "unrecognised config file extension for {} — expected one of: \
                 toml, json5, json, yaml, yml, ini",
                file.display()
            ))
        })?;
        if !file.exists() {
            return Err(config::ConfigError::Message(format!(
                "config file not found: {}",
                file.display()
            )));
        }
        builder = builder.add_source(
            config::File::from(file.to_path_buf())
                .format(format)
                .required(true),
        );
    } else {
        let search_dirs = config_search_dirs(paths);
        let mut found_config = false;
        'search: for dir in &search_dirs {
            for (file, format) in &CONFIG_FILE_CANDIDATES {
                let path = dir.join(file);
                if path.exists() {
                    builder = builder
                        .add_source(config::File::from(path).format(*format).required(false));
                    found_config = true;
                    break 'search;
                }
            }
        }
        if !found_config {
            tracing::error!(
                "No configuration file found. Searched: {}. \
                 Application may not behave as expected",
                search_dirs
                    .iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    builder.build()?.try_deserialize()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    const TEST_PATHS: ConfigPaths = ConfigPaths {
        app_name: "bee-cockpit-core-test",
        config_env: "BEE_COCKPIT_CORE_TEST_CONFIG",
        data_env: "BEE_COCKPIT_CORE_TEST_DATA",
    };

    #[test]
    fn dedup_dirs_keeps_first_occurrence_in_order() {
        let dirs = vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/a"),
            PathBuf::from("/c"),
            PathBuf::from("/b"),
        ];
        assert_eq!(
            dedup_dirs(dirs),
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c"),
            ]
        );
    }

    #[test]
    fn format_from_extension_maps_known_extensions() {
        use config::FileFormat;
        assert_eq!(
            format_from_extension(Path::new("a/b/config.toml")),
            Some(FileFormat::Toml)
        );
        assert_eq!(
            format_from_extension(Path::new("nodes.JSON5")),
            Some(FileFormat::Json5)
        );
        assert_eq!(
            format_from_extension(Path::new("nodes.yml")),
            Some(FileFormat::Yaml)
        );
        assert_eq!(
            format_from_extension(Path::new("nodes.yaml")),
            Some(FileFormat::Yaml)
        );
        assert_eq!(
            format_from_extension(Path::new("nodes.ini")),
            Some(FileFormat::Ini)
        );
        assert_eq!(format_from_extension(Path::new("nodes.conf")), None);
        assert_eq!(format_from_extension(Path::new("nodes")), None);
    }

    #[test]
    fn node_name_from_url_derives_short_names() {
        assert_eq!(node_name_from_url("http://localhost:1633"), "localhost");
        assert_eq!(
            node_name_from_url("https://bee-eu.example.com:1633"),
            "bee-eu"
        );
        assert_eq!(node_name_from_url("http://10.0.1.5:1633"), "10.0.1.5");
        assert_eq!(node_name_from_url("http://[::1]:1633"), "::1");
        assert_eq!(node_name_from_url("bee.example.org/"), "bee");
    }

    #[test]
    fn nodes_from_urls_builds_adhoc_fleet() {
        let nodes = nodes_from_urls(&[
            "http://localhost:1633".to_string(),
            "bee-eu.example.com:1633".to_string(),
        ]);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "localhost");
        assert_eq!(nodes[0].url, "http://localhost:1633");
        assert!(nodes[0].default);
        assert_eq!(nodes[1].name, "bee-eu");
        assert_eq!(nodes[1].url, "http://bee-eu.example.com:1633");
        assert!(!nodes[1].default);
    }

    #[test]
    fn nodes_from_urls_disambiguates_colliding_names() {
        let nodes = nodes_from_urls(&[
            "http://bee.a.com:1633".to_string(),
            "http://bee.b.com:1633".to_string(),
            "http://bee.c.com:1633".to_string(),
        ]);
        assert_eq!(nodes[0].name, "bee");
        assert_eq!(nodes[1].name, "bee-2");
        assert_eq!(nodes[2].name, "bee-3");
    }

    #[test]
    fn config_search_dirs_includes_dot_config_app_dir() {
        let dirs = config_search_dirs(&TEST_PATHS);
        let expected_suffix = format!(".config/{}", TEST_PATHS.app_name);
        assert!(
            dirs.iter().any(|d| d.ends_with(&expected_suffix)),
            "expected ~/{expected_suffix} in search path, got {dirs:?}"
        );
    }
}
