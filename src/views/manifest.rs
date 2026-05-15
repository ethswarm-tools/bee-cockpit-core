//! S12 — Manifests view. Pure half of bee-tui's
//! `components::manifest`: per-node load-state classification, the
//! flat `TreeRow` rendering of an expanded Mantaray subtree, the
//! header summary, and the hex / label helpers. The renderer owns
//! the fetch channels, the selection cursor, and the scrolling.

use std::collections::{HashMap, HashSet};

use bee::manifest::{MantarayNode, TYPE_EDGE, TYPE_VALUE};
use bee::swarm::Reference;

/// Per-node load status. Distinguishes "operator hasn't asked"
/// (`Idle`) from "we're fetching" (`Loading`) from "we have it"
/// (`Loaded`) from "fetch or parse failed" (`Error`).
#[derive(Debug, Clone)]
pub enum NodeState {
    Idle,
    Loading,
    Loaded(Box<MantarayNode>),
    Error(String),
}

impl NodeState {
    pub fn loaded(&self) -> Option<&MantarayNode> {
        match self {
            Self::Loaded(n) => Some(n),
            _ => None,
        }
    }
}

/// One row in the flat-rendered tree. Pure data; the renderer just
/// walks `ManifestView::rows`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// Indentation level (0 = root's direct children).
    pub depth: u8,
    /// Path segment for this fork (UTF-8-decoded prefix bytes if
    /// valid, else hex-escaped). `(root)` for depth-0 root summary.
    pub label: String,
    /// Glyph that signals state at a glance:
    /// `▼` expanded, `▶` collapsed-with-children, `·` leaf-only-target,
    /// `⌛` loading, `✗` error.
    pub glyph: char,
    /// True when this fork has child forks (TYPE_EDGE bit set).
    pub has_children: bool,
    /// Self-address of this fork's node, hex-encoded.
    pub self_addr_hex: Option<String>,
    /// File reference if this fork carries a target (TYPE_VALUE).
    pub target_ref_hex: Option<String>,
    /// Content-type from metadata, when present.
    pub content_type: Option<String>,
    /// Auxiliary status hint rendered in muted color
    /// (`loading…`, `error: …`).
    pub state_hint: Option<String>,
}

/// View fed to the renderer + snapshot tests. Pure transform of the
/// component's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestView {
    /// Hex-encoded root reference, if `:manifest` has fired.
    pub root_ref_hex: Option<String>,
    /// One-line header summary: "32-byte chunk · 12 forks · 4 leaf nodes"
    /// or "no manifest loaded — type :manifest <ref>".
    pub header: String,
    /// Flat list of visible rows.
    pub rows: Vec<TreeRow>,
}

/// Build the visible-tree row list from current state.
pub fn view_for(
    root_ref: Option<&Reference>,
    root: &NodeState,
    forks_loaded: &HashMap<[u8; 32], NodeState>,
    expanded: &HashSet<[u8; 32]>,
) -> ManifestView {
    let header = build_header(root_ref, root);
    let mut rows: Vec<TreeRow> = Vec::new();
    if let Some(node) = root.loaded() {
        walk_into_rows(node, 0, forks_loaded, expanded, &mut rows);
    }
    ManifestView {
        root_ref_hex: root_ref.map(|r| r.to_hex()),
        header,
        rows,
    }
}

fn build_header(root_ref: Option<&Reference>, root: &NodeState) -> String {
    match (root_ref, root) {
        (None, _) => "no manifest loaded — type :manifest <ref> or :inspect <ref>".into(),
        (Some(r), NodeState::Idle) => format!("ref {} — pending", short_hex(&r.to_hex(), 8)),
        (Some(r), NodeState::Loading) => {
            format!("ref {} — loading root chunk…", short_hex(&r.to_hex(), 8))
        }
        (Some(r), NodeState::Error(e)) => {
            format!("ref {} — error: {}", short_hex(&r.to_hex(), 8), e)
        }
        (Some(r), NodeState::Loaded(node)) => {
            let fork_count = node.forks.len();
            let leaf_count = leaves_under(node);
            format!(
                "ref {} · {} fork{} · {} leaf node{}",
                short_hex(&r.to_hex(), 8),
                fork_count,
                if fork_count == 1 { "" } else { "s" },
                leaf_count,
                if leaf_count == 1 { "" } else { "s" }
            )
        }
    }
}

fn leaves_under(node: &MantarayNode) -> usize {
    if node.forks.is_empty() {
        return if node.is_null_target() { 0 } else { 1 };
    }
    node.forks
        .values()
        .map(|fork| {
            let t = fork.node.determine_type();
            (t & TYPE_VALUE != 0) as usize
        })
        .sum()
}

fn walk_into_rows(
    node: &MantarayNode,
    depth: u8,
    forks_loaded: &HashMap<[u8; 32], NodeState>,
    expanded: &HashSet<[u8; 32]>,
    rows: &mut Vec<TreeRow>,
) {
    for fork in node.forks.values() {
        let typ = fork.node.determine_type();
        let has_children = (typ & TYPE_EDGE) != 0;
        let has_target = (typ & TYPE_VALUE) != 0;
        let self_addr = fork.node.self_address;
        let target_ref_hex = if has_target && !fork.node.is_null_target() {
            Some(hex_lower(&fork.node.target_address))
        } else {
            None
        };
        let content_type = fork
            .node
            .metadata
            .as_ref()
            .and_then(|m| m.get("Content-Type").or_else(|| m.get("content-type")))
            .cloned();

        let is_expanded = self_addr
            .as_ref()
            .map(|a| expanded.contains(a))
            .unwrap_or(false);

        let load_state = self_addr.as_ref().and_then(|a| forks_loaded.get(a));
        let state_hint = match load_state {
            Some(NodeState::Loading) => Some("loading…".to_string()),
            Some(NodeState::Error(e)) => Some(format!("error: {e}")),
            _ => None,
        };
        let glyph = if state_hint
            .as_deref()
            .map(|s| s.starts_with("loading"))
            .unwrap_or(false)
        {
            '⌛'
        } else if state_hint
            .as_deref()
            .map(|s| s.starts_with("error"))
            .unwrap_or(false)
        {
            '✗'
        } else if has_children && is_expanded {
            '▼'
        } else if has_children {
            '▶'
        } else {
            '·'
        };

        rows.push(TreeRow {
            depth,
            label: prefix_to_label(&fork.prefix),
            glyph,
            has_children,
            self_addr_hex: self_addr.map(|a| hex_lower(&a)),
            target_ref_hex,
            content_type,
            state_hint,
        });

        if is_expanded {
            if let Some(addr) = self_addr {
                if let Some(NodeState::Loaded(child)) = forks_loaded.get(&addr) {
                    walk_into_rows(child, depth.saturating_add(1), forks_loaded, expanded, rows);
                }
            }
        }
    }
}

fn prefix_to_label(prefix: &[u8]) -> String {
    if prefix.is_empty() {
        return "(empty)".into();
    }
    if let Ok(s) = std::str::from_utf8(prefix) {
        if s.chars().all(|c| !c.is_control()) {
            return s.to_string();
        }
    }
    hex_lower(prefix)
}

pub fn hex_lower(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 2);
    for byte in b {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn short_hex(s: &str, n: usize) -> String {
    if s.len() <= n * 2 + 1 {
        s.to_string()
    } else {
        format!("{}…{}", &s[..n], &s[s.len() - n..])
    }
}

pub fn parse_hex_32(s: &str) -> Result<[u8; 32], String> {
    let cleaned = s.trim().trim_start_matches("0x");
    if cleaned.len() != 64 {
        return Err(format!("expected 64 hex chars, got {}", cleaned.len()));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] =
            u8::from_str_radix(&cleaned[2 * i..2 * i + 2], 16).map_err(|e| format!("hex: {e}"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_state() -> (NodeState, HashMap<[u8; 32], NodeState>, HashSet<[u8; 32]>) {
        (NodeState::Idle, HashMap::new(), HashSet::new())
    }

    #[test]
    fn header_explains_no_load_yet() {
        let (root, loaded, expanded) = empty_state();
        let view = view_for(None, &root, &loaded, &expanded);
        assert!(
            view.header.contains("no manifest loaded"),
            "{}",
            view.header
        );
        assert!(view.rows.is_empty());
    }

    #[test]
    fn header_explains_loading_state() {
        let (_, loaded, expanded) = empty_state();
        let root = NodeState::Loading;
        let r = Reference::from_hex(&"0".repeat(64)).unwrap();
        let view = view_for(Some(&r), &root, &loaded, &expanded);
        assert!(view.header.contains("loading"), "{}", view.header);
        assert!(view.rows.is_empty());
    }

    #[test]
    fn header_propagates_load_error() {
        let (_, loaded, expanded) = empty_state();
        let root = NodeState::Error("download_chunk: 404".into());
        let r = Reference::from_hex(&"0".repeat(64)).unwrap();
        let view = view_for(Some(&r), &root, &loaded, &expanded);
        assert!(view.header.contains("error"), "{}", view.header);
        assert!(view.header.contains("404"), "{}", view.header);
    }

    #[test]
    fn prefix_to_label_renders_utf8_when_possible() {
        assert_eq!(prefix_to_label(b"index.html"), "index.html");
        assert_eq!(prefix_to_label(&[]), "(empty)");
        assert_eq!(prefix_to_label(&[0x00, 0x01, 0xff]), "0001ff");
    }

    #[test]
    fn short_hex_keeps_short_strings_intact() {
        assert_eq!(short_hex("abcd", 4), "abcd");
        let long = "a".repeat(64);
        let s = short_hex(&long, 8);
        assert!(s.contains('…'));
        assert_eq!(s.chars().filter(|c| *c == 'a').count(), 16);
    }

    #[test]
    fn parse_hex_32_round_trip() {
        let s = "ab".repeat(32);
        let arr = parse_hex_32(&s).unwrap();
        assert_eq!(arr[0], 0xab);
        assert_eq!(arr[31], 0xab);
        assert!(parse_hex_32(&"a".repeat(63)).is_err());
        assert!(parse_hex_32("0xABABA").is_err());
    }

    #[test]
    fn view_with_no_root_loaded_has_zero_rows() {
        let (root, loaded, expanded) = empty_state();
        let view = view_for(None, &root, &loaded, &expanded);
        assert_eq!(view.rows.len(), 0);
        assert_eq!(view.root_ref_hex, None);
    }
}
