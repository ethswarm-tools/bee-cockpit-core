//! Full-state support bundle for `:diagnose --bundle`.
//!
//! The Swarm analog of `bee-scripts/snapshot.sh` + `collect-all.sh`:
//! one shareable file capturing the node's operator-relevant state so
//! an operator can attach "here's everything about my node" to a
//! support thread without running a dozen `curl | jq` calls by hand.
//!
//! Unlike the existing `:diagnose` text bundle (a health-gate digest +
//! the recent-call log), this collects the *raw API state* across
//! health, status, topology, stamps, chequebook, stake and peers.
//!
//! We fetch each endpoint as raw JSON rather than going through the
//! typed [`bee`] response structs: the typed structs are
//! `Deserialize`-only (no `Serialize`), and a raw capture is both
//! faithful to what the node actually returned and robust to API
//! version drift. Endpoints that fail are recorded inline as
//! `{"error": "..."}` so a partial node still produces a useful
//! bundle — the same forgiving behaviour as the shell scripts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};

/// API paths captured in the bundle, relative to the node base URL.
/// Mirrors the set `snapshot.sh` collects, plus topology/peers/stamps.
pub const BUNDLE_ENDPOINTS: &[&str] = &[
    "health",
    "status",
    "addresses",
    "chainstate",
    "wallet",
    "redistributionstate",
    "reservestate",
    "topology",
    "stamps",
    "chequebook/balance",
    "chequebook/address",
    "settlements",
    "stake",
    "blocklist",
    "peers",
    "transactions",
];

/// Per-endpoint HTTP timeout. Generous enough for a busy node's
/// `/topology` without hanging the whole bundle on one stuck endpoint.
const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(20);

/// The assembled bundle. Serializes to the on-disk JSON document.
#[derive(Debug, Clone, Serialize)]
pub struct SupportBundle {
    /// Collection time, Unix seconds (UTC). Avoids pulling in a
    /// date-formatting dependency; readers can render it.
    pub collected_at_unix: u64,
    /// Node API base URL the bundle was collected from.
    pub node: String,
    /// Identifier of the tool + version that produced the bundle.
    pub generated_by: String,
    /// endpoint path → parsed JSON body, or `{"error": "..."}`.
    pub endpoints: Map<String, Value>,
}

impl SupportBundle {
    /// Number of endpoints that returned a usable body.
    pub fn ok_count(&self) -> usize {
        self.endpoints.values().filter(|v| !is_error(v)).count()
    }

    /// Number of endpoints that errored (unreachable, non-2xx, …).
    pub fn error_count(&self) -> usize {
        self.endpoints.len() - self.ok_count()
    }

    /// Operator-facing one-liner, matching the `pprof_bundle` style.
    pub fn summary(&self, path: &Path) -> String {
        format!(
            "support bundle: {} of {} endpoints captured ({} failed) → {}",
            self.ok_count(),
            self.endpoints.len(),
            self.error_count(),
            path.display(),
        )
    }
}

/// True when a captured endpoint value is our error sentinel.
fn is_error(v: &Value) -> bool {
    v.get("error").is_some()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Fetch every [`BUNDLE_ENDPOINTS`] path concurrently and assemble a
/// [`SupportBundle`]. Never fails as a whole — a node that is down
/// produces a bundle full of `{"error": ...}` entries, which is itself
/// the useful signal.
pub async fn collect(base_url: &str, auth_token: Option<&str>, generated_by: &str) -> SupportBundle {
    let base = base_url.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .timeout(ENDPOINT_TIMEOUT)
        .build()
        .unwrap_or_default();

    let futures = BUNDLE_ENDPOINTS.iter().map(|path| {
        let client = &client;
        let base = &base;
        async move {
            (
                (*path).to_string(),
                fetch_json(client, base, path, auth_token).await,
            )
        }
    });
    let results = futures::future::join_all(futures).await;

    let mut endpoints = Map::new();
    for (k, v) in results {
        endpoints.insert(k, v);
    }

    SupportBundle {
        collected_at_unix: now_unix(),
        node: base,
        generated_by: generated_by.to_string(),
        endpoints,
    }
}

/// [`collect`] then write the pretty-printed JSON to `path`, creating
/// parent directories as needed. Returns the bundle (for its summary)
/// alongside the path written.
pub async fn collect_and_write(
    base_url: &str,
    auth_token: Option<&str>,
    generated_by: &str,
    path: PathBuf,
) -> Result<(SupportBundle, PathBuf), String> {
    let bundle = collect(base_url, auth_token, generated_by).await;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    let body = serde_json::to_string_pretty(&bundle).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok((bundle, path))
}

async fn fetch_json(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    auth: Option<&str>,
) -> Value {
    let url = format!("{base}/{path}");
    let mut req = client.get(&url);
    if let Some(token) = auth {
        req = req.bearer_auth(token);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return json!({ "error": format!("GET {url}: {e}") }),
    };
    let status = resp.status();
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => return json!({ "error": format!("read body of {url}: {e}") }),
    };
    if !status.is_success() {
        return json!({ "error": format!("HTTP {status}"), "body": body });
    }
    // Most endpoints are JSON; a couple (e.g. /health) may be plain
    // text — keep the raw string rather than dropping it.
    serde_json::from_str::<Value>(&body).unwrap_or_else(|_| json!({ "raw": body }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_with(entries: &[(&str, Value)]) -> SupportBundle {
        let mut endpoints = Map::new();
        for (k, v) in entries {
            endpoints.insert((*k).to_string(), v.clone());
        }
        SupportBundle {
            collected_at_unix: 0,
            node: "http://localhost:1633".into(),
            generated_by: "test".into(),
            endpoints,
        }
    }

    #[test]
    fn endpoint_set_covers_the_essentials() {
        for p in ["health", "status", "topology", "stamps", "chequebook/balance"] {
            assert!(BUNDLE_ENDPOINTS.contains(&p), "missing {p}");
        }
    }

    #[test]
    fn ok_and_error_counts_split_on_error_sentinel() {
        let b = bundle_with(&[
            ("health", json!("ok")),
            ("status", json!({"connectedPeers": 12})),
            ("topology", json!({"error": "HTTP 500 Internal Server Error"})),
        ]);
        assert_eq!(b.ok_count(), 2);
        assert_eq!(b.error_count(), 1);
    }

    #[test]
    fn summary_mentions_counts_and_path() {
        let b = bundle_with(&[("health", json!("ok"))]);
        let s = b.summary(Path::new("/tmp/bee-support-1.json"));
        assert!(s.contains("1 of 1"));
        assert!(s.contains("/tmp/bee-support-1.json"));
    }

    #[test]
    fn serializes_to_json_object() {
        let b = bundle_with(&[("health", json!("ok"))]);
        let v: Value = serde_json::to_value(&b).unwrap();
        assert_eq!(v["node"], "http://localhost:1633");
        assert_eq!(v["endpoints"]["health"], "ok");
    }
}
