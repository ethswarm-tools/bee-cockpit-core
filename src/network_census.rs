//! Network-wide neighborhood census (feature `swarmscan`).
//!
//! Optional module — the cockpit is otherwise pure Bee-node-API. This
//! talks to the Swarmscan indexer (via [`swarmscan`]) to answer the
//! operator's question that the local node alone cannot: *how
//! populated is my neighborhood?* It buckets every indexed overlay by
//! its first `depth` bits and reports the population distribution, plus
//! the operator's own neighborhood. The Swarm analog of bee-scripts'
//! `neighborhood` Go program, surfaced inside the cockpit.
//!
//! Pure computation ([`compute_census`]) is unit-tested; [`collect_census`]
//! is the thin async fetch-then-compute wrapper.

use std::collections::HashMap;

use serde::Serialize;
use swarmscan::Client;

/// Default census bit-depth. Matches the cockpit's coarse neighborhood
/// granularity; an operator can pass their node's storage radius for a
/// view that matches what their node actually reserves.
pub const DEFAULT_DEPTH: u8 = 8;
/// Clamp range for a requested depth (`2^depth` stays within `u64`).
pub const MIN_DEPTH: u8 = 1;
pub const MAX_DEPTH: u8 = 32;
/// How many of the most-populous neighborhoods to surface.
const TOP_N: usize = 12;

/// One neighborhood bucket in the census.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CensusBucket {
    /// Binary prefix of the first `depth` overlay bits.
    pub prefix: String,
    pub population: usize,
    pub full_nodes: usize,
}

/// Network census result. (No `Eq`: `avg_population` is an `f64`.)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NetworkCensus {
    pub depth: u8,
    pub total_nodes: usize,
    /// `2^depth` — number of possible neighborhoods at this depth.
    pub possible: u64,
    /// Neighborhoods with at least one node.
    pub occupied: usize,
    pub max_population: usize,
    /// Mean population across OCCUPIED neighborhoods.
    pub avg_population: f64,
    /// The operator's own neighborhood prefix, if an overlay was given.
    pub my_prefix: Option<String>,
    /// Population of the operator's own neighborhood (0 if it decoded
    /// but no peers share the prefix; `None` if no overlay was given or
    /// it failed to decode).
    pub my_population: Option<usize>,
    /// Full-node count in the operator's own neighborhood.
    pub my_full_nodes: Option<usize>,
    /// The most-populous neighborhoods (up to [`TOP_N`]).
    pub top: Vec<CensusBucket>,
}

fn clamp_depth(depth: u8) -> u8 {
    depth.clamp(MIN_DEPTH, MAX_DEPTH)
}

/// Binary string of the first `depth` bits of a hex overlay, MSB
/// first. `None` if the overlay is too short to supply that many bits.
fn bit_prefix(overlay_hex: &str, depth: u8) -> Option<String> {
    let s = overlay_hex.strip_prefix("0x").unwrap_or(overlay_hex);
    let need_nibbles = (depth as usize).div_ceil(4);
    if s.len() < need_nibbles {
        return None;
    }
    let mut bits = String::with_capacity(depth as usize);
    for i in 0..depth as usize {
        let nibble = u8::from_str_radix(&s[i / 4..i / 4 + 1], 16).ok()?;
        let bit = (nibble >> (3 - (i % 4))) & 1;
        bits.push(if bit == 1 { '1' } else { '0' });
    }
    Some(bits)
}

/// Bucket `(overlay, full_node)` nodes by their first `depth` bits and
/// compute the census, including the operator's own neighborhood if
/// `my_overlay` is given. Overlays that don't decode are skipped.
pub fn compute_census<'a>(
    nodes: impl Iterator<Item = (&'a str, bool)>,
    depth: u8,
    my_overlay: Option<&str>,
) -> NetworkCensus {
    let depth = clamp_depth(depth);
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut total = 0usize;
    for (overlay, full) in nodes {
        let Some(prefix) = bit_prefix(overlay, depth) else {
            continue;
        };
        let entry = map.entry(prefix).or_insert((0, 0));
        entry.0 += 1;
        if full {
            entry.1 += 1;
        }
        total += 1;
    }

    let my_prefix = my_overlay.and_then(|o| bit_prefix(o, depth));
    let (my_population, my_full_nodes) = match &my_prefix {
        Some(p) => {
            let (pop, full) = map.get(p).copied().unwrap_or((0, 0));
            (Some(pop), Some(full))
        }
        None => (None, None),
    };

    let occupied = map.len();
    let max_population = map.values().map(|(p, _)| *p).max().unwrap_or(0);
    let avg_population = if occupied > 0 {
        total as f64 / occupied as f64
    } else {
        0.0
    };

    let mut buckets: Vec<CensusBucket> = map
        .into_iter()
        .map(|(prefix, (population, full_nodes))| CensusBucket {
            prefix,
            population,
            full_nodes,
        })
        .collect();
    buckets.sort_by(|a, b| {
        b.population
            .cmp(&a.population)
            .then_with(|| a.prefix.cmp(&b.prefix))
    });
    buckets.truncate(TOP_N);

    NetworkCensus {
        depth,
        total_nodes: total,
        possible: 1u64 << depth,
        occupied,
        max_population,
        avg_population,
        my_prefix,
        my_population,
        my_full_nodes,
        top: buckets,
    }
}

/// Fetch the Swarmscan network dump and compute the census.
pub async fn collect_census(
    client: &Client,
    my_overlay: Option<&str>,
    depth: u8,
) -> Result<NetworkCensus, String> {
    let dump = client
        .network_dump()
        .await
        .map_err(|e| format!("swarmscan network dump: {e}"))?;
    Ok(compute_census(
        dump.nodes.iter().map(|n| (n.overlay.as_str(), n.full_node)),
        depth,
        my_overlay,
    ))
}

/// Convenience: build a mainnet Swarmscan client and collect a census.
/// Lets a renderer run the census without depending on `swarmscan-rs`
/// directly — it only needs `bee-cockpit-core` with the `swarmscan`
/// feature.
pub async fn collect_census_mainnet(
    my_overlay: Option<&str>,
    depth: u8,
) -> Result<NetworkCensus, String> {
    collect_census(&Client::new(), my_overlay, depth).await
}

/// Look up a single node's geo location + reachability from Swarmscan —
/// the per-peer enrichment bee-scripts' `bad-status.sh` performs. Returns
/// a one-line summary for an operator status row.
pub async fn lookup_geo_mainnet(overlay: &str) -> Result<String, String> {
    let node = Client::new()
        .node(overlay)
        .await
        .map_err(|e| format!("swarmscan node lookup: {e}"))?;
    let country = node
        .location
        .as_ref()
        .and_then(|l| l.country.as_deref())
        .unwrap_or("?");
    let city = node
        .location
        .as_ref()
        .and_then(|l| l.city.as_deref())
        .unwrap_or("");
    let loc = if city.is_empty() {
        country.to_string()
    } else {
        format!("{city}, {country}")
    };
    let reach = if node.unreachable {
        "unreachable"
    } else {
        "reachable"
    };
    let trimmed = overlay.trim_start_matches("0x");
    let short = if trimmed.len() > 10 {
        format!("{}…{}", &trimmed[..6], &trimmed[trimmed.len() - 4..])
    } else {
        trimmed.to_string()
    };
    Ok(format!("{short} → {loc} · {reach}"))
}

/// One-line operator summary of a census, suitable for a status row.
pub fn census_summary(c: &NetworkCensus) -> String {
    let mine = match (c.my_prefix.as_deref(), c.my_population) {
        (Some(p), Some(n)) => format!("your nbhd {p} has {n} node(s); "),
        _ => String::new(),
    };
    format!(
        "census @depth {}: {}{} nodes across {} of {} neighborhoods (max {}, avg {:.1})",
        c.depth, mine, c.total_nodes, c.occupied, c.possible, c.max_population, c.avg_population,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(first_byte: u8) -> String {
        let mut s = format!("{first_byte:02x}");
        s.push_str(&"00".repeat(31));
        s
    }

    #[test]
    fn bit_prefix_reads_msb_first() {
        // 0xb0 = 1011 0000
        assert_eq!(bit_prefix("b0ff", 4).as_deref(), Some("1011"));
        assert_eq!(bit_prefix("b0ff", 8).as_deref(), Some("10110000"));
        assert_eq!(bit_prefix("0x1f", 4).as_deref(), Some("0001"));
        assert_eq!(bit_prefix("a", 8), None); // too short for 8 bits
    }

    #[test]
    fn census_buckets_and_my_neighborhood() {
        // 0x10 (0001…) & 0x1f (0001…) share top 4 bits; 0xf0 (1111…) alone.
        let a = overlay(0x10);
        let b = overlay(0x1f);
        let c = overlay(0xf0);
        let nodes = vec![(a.as_str(), true), (b.as_str(), false), (c.as_str(), true)];
        let census = compute_census(nodes.into_iter(), 4, Some(&a));
        assert_eq!(census.depth, 4);
        assert_eq!(census.total_nodes, 3);
        assert_eq!(census.occupied, 2);
        assert_eq!(census.possible, 16);
        assert_eq!(census.max_population, 2);
        // operator a is in the "0001" neighborhood with b → population 2
        assert_eq!(census.my_prefix.as_deref(), Some("0001"));
        assert_eq!(census.my_population, Some(2));
        assert_eq!(census.my_full_nodes, Some(1));
        // most-populous bucket first
        assert_eq!(census.top[0].prefix, "0001");
        assert_eq!(census.top[0].population, 2);
    }

    #[test]
    fn census_without_overlay_has_no_my_fields() {
        let a = overlay(0x10);
        let census = compute_census(std::iter::once((a.as_str(), true)), 8, None);
        assert_eq!(census.my_prefix, None);
        assert_eq!(census.my_population, None);
        assert_eq!(census.my_full_nodes, None);
        assert_eq!(census.total_nodes, 1);
    }

    #[test]
    fn invalid_overlays_skipped() {
        let census = compute_census(
            vec![("nothex", true), ("", false)].into_iter(),
            4,
            None,
        );
        assert_eq!(census.total_nodes, 0);
        assert_eq!(census.occupied, 0);
    }
}
