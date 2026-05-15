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
