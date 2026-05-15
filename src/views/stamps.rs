//! S2 — Stamps view. Pure half of bee-tui's `components::stamps`:
//! per-batch row classification (StampStatus + the volume / worst-
//! bucket / TTL math), the bucket-histogram drill view, and the
//! predicted-economics formula (`bzz = amount × 2^depth / 1e16`)
//! that swarm-cli / beekeeper-stamper / gateway-proxy all use. The
//! renderer owns the drill mpsc channel and the table draw path.

use bee::postage::{BatchBucket, PostageBatch, PostageBatchBuckets};

use crate::stamps::{TOPUP_SOON_SECS, TOPUP_URGENT_SECS, format_ttl_seconds};
use crate::watch::StampsSnapshot;

/// Tri-state row outcome with `Pending` for chain-confirmation gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampStatus {
    Pending,
    Expired,
    Critical,
    Skewed,
    Healthy,
}

impl StampStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "⏳ pending",
            Self::Expired => "✗ expired",
            Self::Critical => "✗ critical",
            Self::Skewed => "⚠ skewed",
            Self::Healthy => "✓",
        }
    }
}

/// One row of the stamps table.
#[derive(Debug, Clone)]
pub struct StampRow {
    pub label: String,
    pub batch_id_short: String,
    pub volume: String,
    /// Worst-bucket fill percentage in `0..=100`. This *is* what the
    /// API calls `utilization`.
    pub worst_bucket_pct: u32,
    pub worst_bucket_raw: String,
    pub ttl: String,
    pub ttl_seconds: i64,
    pub immutable: bool,
    pub status: StampStatus,
    pub why: Option<String>,
}

/// Bucket fill distribution for [`StampDrillView`]. Six bins keep the
/// display compact while still distinguishing "nearly full" from
/// "actually full". Ordered low → high.
pub const FILL_BIN_LABELS: &[&str] = &[
    "0 %",
    "1 – 19 %",
    "20 – 49 %",
    "50 – 79 %",
    "80 – 99 %",
    "100 %",
];

/// Aggregated drill view for the bucket histogram screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampDrillView {
    pub depth: u8,
    pub bucket_depth: u8,
    pub upper_bound: u32,
    pub total_chunks: u64,
    pub theoretical_capacity: u128,
    pub fill_distribution: [u32; 6],
    /// Up to 10 worst buckets sorted by collisions descending.
    pub worst_buckets: Vec<WorstBucket>,
    pub worst_pct: u32,
    pub economics: Option<StampEconomics>,
}

/// Predicted economics for a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampEconomics {
    pub bzz_paid: String,
    pub volume_humanised: String,
    pub bzz_per_gib: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorstBucket {
    pub bucket_id: u32,
    pub collisions: u32,
    pub pct: u32,
}

/// Pure, snapshot-driven row computation.
pub fn rows_for(snap: &StampsSnapshot) -> Vec<StampRow> {
    snap.batches.iter().map(row_from_batch).collect()
}

/// Pure compute path for the drill pane. Buckets per-bucket
/// collisions into [`FILL_BIN_LABELS`] bins, picks the top-10
/// worst, totals the chunk count. `batch` is optional — when
/// supplied, predicted economics (`bzz_paid` etc.) are derived
/// from `batch.amount` × `2^depth` and rendered in the drill
/// header.
pub fn compute_drill_view(
    buckets: &PostageBatchBuckets,
    batch: Option<&PostageBatch>,
) -> StampDrillView {
    let upper_bound = buckets.bucket_upper_bound.max(1);
    let mut fill_distribution = [0u32; 6];
    let mut total_chunks: u64 = 0;
    for b in &buckets.buckets {
        total_chunks += u64::from(b.collisions);
        let bin = bucket_fill_bin(b.collisions, upper_bound);
        fill_distribution[bin] += 1;
    }
    let mut sorted: Vec<&BatchBucket> = buckets.buckets.iter().collect();
    sorted.sort_by(|a, b| {
        b.collisions
            .cmp(&a.collisions)
            .then_with(|| a.bucket_id.cmp(&b.bucket_id))
    });
    let worst_buckets: Vec<WorstBucket> = sorted
        .iter()
        .take(10)
        .map(|b| WorstBucket {
            bucket_id: b.bucket_id,
            collisions: b.collisions,
            pct: pct_of(b.collisions, upper_bound),
        })
        .collect();
    let worst_pct = worst_buckets.first().map(|w| w.pct).unwrap_or(0);
    let theoretical_capacity = (1u128 << buckets.bucket_depth) * u128::from(upper_bound);
    let economics = batch.and_then(compute_stamp_economics);
    StampDrillView {
        depth: buckets.depth,
        bucket_depth: buckets.bucket_depth,
        upper_bound,
        total_chunks,
        theoretical_capacity,
        fill_distribution,
        worst_buckets,
        worst_pct,
        economics,
    }
}

fn bucket_fill_bin(collisions: u32, upper_bound: u32) -> usize {
    if collisions == 0 {
        return 0;
    }
    if collisions >= upper_bound {
        return 5;
    }
    let pct = pct_of(collisions, upper_bound);
    match pct {
        0 => 0,
        1..=19 => 1,
        20..=49 => 2,
        50..=79 => 3,
        80..=99 => 4,
        _ => 5,
    }
}

pub fn pct_of(collisions: u32, upper_bound: u32) -> u32 {
    if upper_bound == 0 {
        return 0;
    }
    let pct = (u64::from(collisions) * 100) / u64::from(upper_bound);
    pct.min(100) as u32
}

fn row_from_batch(b: &PostageBatch) -> StampRow {
    let label = if b.label.is_empty() {
        "(unlabeled)".to_string()
    } else {
        b.label.clone()
    };
    let batch_hex = b.batch_id.to_hex();
    let batch_id_short = if batch_hex.len() > 8 {
        format!("{}…", &batch_hex[..8])
    } else {
        batch_hex
    };
    let theoretical_bytes: u128 = (1u128 << b.depth) * 4096;
    let volume = format_bytes(theoretical_bytes);
    let worst_bucket_pct = worst_bucket_pct(b);
    let upper_bound = 1u32 << b.depth.saturating_sub(b.bucket_depth);
    let worst_bucket_raw = format!("{}/{}", b.utilization, upper_bound);
    let ttl = format_ttl_seconds(b.batch_ttl);

    let (status, why) = if !b.usable {
        (
            StampStatus::Pending,
            Some("waiting on chain confirmation (~10 blocks).".into()),
        )
    } else if b.batch_ttl <= 0 {
        (
            StampStatus::Expired,
            Some("paid balance exhausted; topup or stop using.".into()),
        )
    } else if worst_bucket_pct >= 95 {
        (
            StampStatus::Critical,
            Some(if b.immutable {
                "immutable batch will REJECT next upload at this bucket.".into()
            } else {
                "mutable batch will silently overwrite oldest chunks.".into()
            }),
        )
    } else if b.batch_ttl <= TOPUP_URGENT_SECS {
        (
            StampStatus::Critical,
            Some(format!(
                "topup URGENT — TTL {} (under {}h threshold).",
                ttl,
                TOPUP_URGENT_SECS / 3600
            )),
        )
    } else if worst_bucket_pct >= 80 {
        (
            StampStatus::Skewed,
            Some(format!(
                "worst bucket {worst_bucket_pct}% > safe headroom — dilute or stop using."
            )),
        )
    } else if b.batch_ttl <= TOPUP_SOON_SECS {
        (
            StampStatus::Skewed,
            Some(format!(
                "topup soon — TTL {} (under {}d planning threshold).",
                ttl,
                TOPUP_SOON_SECS / 86_400
            )),
        )
    } else {
        (StampStatus::Healthy, None)
    };

    StampRow {
        label,
        batch_id_short,
        volume,
        worst_bucket_pct,
        worst_bucket_raw,
        ttl,
        ttl_seconds: b.batch_ttl,
        immutable: b.immutable,
        status,
        why,
    }
}

/// `MaxBucketCount` (Bee's `utilization`) as a 0..=100 percentage of
/// the per-bucket upper bound `2^(depth - bucket_depth)`.
fn worst_bucket_pct(b: &PostageBatch) -> u32 {
    let upper_bound: u32 = 1u32 << b.depth.saturating_sub(b.bucket_depth);
    if upper_bound == 0 {
        0
    } else {
        let pct = (u64::from(b.utilization) * 100) / u64::from(upper_bound);
        pct.min(100) as u32
    }
}

fn compute_stamp_economics(b: &PostageBatch) -> Option<StampEconomics> {
    let amount = b.amount.as_ref()?;
    let two_pow_depth: num_bigint::BigInt = num_bigint::BigInt::from(1u32) << b.depth as usize;
    let total_plur = amount * &two_pow_depth;
    let bzz: f64 = total_plur.to_string().parse::<f64>().ok()? / 1e16;

    let cap_bytes: u128 = (1u128 << b.depth) * 4096;
    let volume_humanised = format_bytes(cap_bytes);

    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let gib = cap_bytes as f64 / GIB;
    let bzz_per_gib = if gib > 0.0 {
        format!("{:.4} BZZ/GiB", bzz / gib)
    } else {
        "n/a".to_string()
    };

    Some(StampEconomics {
        bzz_paid: format!("{bzz:.4} BZZ"),
        volume_humanised,
        bzz_per_gib,
    })
}

/// Bytes → IEC binary (KiB / MiB / GiB / TiB).
pub fn format_bytes(bytes: u128) -> String {
    const K: u128 = 1024;
    const M: u128 = K * 1024;
    const G: u128 = M * 1024;
    const T: u128 = G * 1024;
    if bytes >= T {
        format!("{:.1} TiB", bytes as f64 / T as f64)
    } else if bytes >= G {
        format!("{:.1} GiB", bytes as f64 / G as f64)
    } else if bytes >= M {
        format!("{:.1} MiB", bytes as f64 / M as f64)
    } else if bytes >= K {
        format!("{:.1} KiB", bytes as f64 / K as f64)
    } else {
        format!("{bytes} B")
    }
}
