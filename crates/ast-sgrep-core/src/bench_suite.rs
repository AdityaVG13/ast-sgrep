use crate::search::{HitKind, SearchHit};
use crate::semantic_ivf::load_semantic_ivf;
use crate::{Result, StoreError};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static IVF_BENCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
pub struct BenchExpectation {
    pub kind: Option<HitKind>,
    pub symbol: Option<&'static str>,
    pub callee: Option<&'static str>,
    pub file_suffix: Option<&'static str>,
    pub excerpt_contains: Option<&'static str>,
    pub max_rank: usize,
}

impl BenchExpectation {
    pub fn matches(&self, hit: &SearchHit) -> bool {
        self.kind.is_none_or(|kind| hit.kind == kind)
            && self
                .symbol
                .is_none_or(|symbol| hit.symbol.as_deref() == Some(symbol))
            && self
                .callee
                .is_none_or(|callee| hit.callee.as_deref() == Some(callee))
            && self
                .file_suffix
                .is_none_or(|suffix| hit.file.ends_with(suffix))
            && self
                .excerpt_contains
                .is_none_or(|needle| hit.excerpt.contains(needle))
    }

    pub fn is_specific(&self) -> bool {
        self.kind.is_some()
            || self.symbol.is_some()
            || self.callee.is_some()
            || self.file_suffix.is_some()
            || self.excerpt_contains.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BenchCase {
    pub name: &'static str,
    pub query: &'static str,
    pub min_hits: usize,
}
#[derive(Debug, Clone)]
pub struct BenchFixture {
    pub name: &'static str,
    pub root: PathBuf,
    pub suite: &'static str,
}
pub const DEFAULT_SUITE: &[BenchCase] = &[
    BenchCase {
        name: "literal_symbol",
        query: "process_request",
        min_hits: 1,
    },
    BenchCase {
        name: "defs_prefix",
        query: "defs:auth_refresh",
        min_hits: 1,
    },
    BenchCase {
        name: "callers_prefix",
        query: "callers:process_request",
        min_hits: 1,
    },
    BenchCase {
        name: "nl_auth_refresh",
        query: "how does auth refresh work",
        min_hits: 1,
    },
    BenchCase {
        name: "synonym_credential_renewal",
        query: "credential renewal",
        min_hits: 1,
    },
];
pub const SELF_SUITE: &[BenchCase] = &[
    BenchCase {
        name: "core_searcher",
        query: "Searcher",
        min_hits: 1,
    },
    BenchCase {
        name: "semantic_ivf",
        query: "semantic_ivf",
        min_hits: 1,
    },
    BenchCase {
        name: "defs_search_pattern",
        query: "defs:search_pattern",
        min_hits: 1,
    },
    BenchCase {
        name: "nl_hybrid_search",
        query: "how does hybrid search work",
        min_hits: 1,
    },
];
pub fn benchmark_expectation(case: &BenchCase) -> Option<BenchExpectation> {
    let expected = match case.name {
        "literal_symbol" => BenchExpectation {
            kind: Some(HitKind::Def),
            symbol: Some("process_request"),
            callee: None,
            file_suffix: None,
            excerpt_contains: None,
            max_rank: 5,
        },
        "defs_prefix" => BenchExpectation {
            kind: Some(HitKind::Def),
            symbol: Some("auth_refresh"),
            callee: None,
            file_suffix: None,
            excerpt_contains: None,
            max_rank: 3,
        },
        "callers_prefix" => BenchExpectation {
            kind: Some(HitKind::Caller),
            symbol: None,
            callee: Some("process_request"),
            file_suffix: None,
            excerpt_contains: None,
            max_rank: 3,
        },
        "nl_auth_refresh" => BenchExpectation {
            kind: Some(HitKind::Def),
            symbol: Some("auth_refresh"),
            callee: None,
            file_suffix: None,
            excerpt_contains: None,
            max_rank: 8,
        },
        "synonym_credential_renewal" => BenchExpectation {
            kind: Some(HitKind::Embed),
            symbol: Some("auth_refresh"),
            callee: None,
            file_suffix: None,
            excerpt_contains: None,
            max_rank: 16,
        },
        "core_searcher" => BenchExpectation {
            kind: Some(HitKind::Def),
            symbol: Some("Searcher"),
            callee: None,
            file_suffix: None,
            excerpt_contains: None,
            max_rank: 5,
        },
        "semantic_ivf" => BenchExpectation {
            kind: None,
            symbol: None,
            callee: None,
            file_suffix: Some("semantic_ivf.rs"),
            excerpt_contains: None,
            max_rank: 8,
        },
        "defs_search_pattern" => BenchExpectation {
            kind: Some(HitKind::Def),
            symbol: Some("search_pattern"),
            callee: None,
            file_suffix: None,
            excerpt_contains: None,
            max_rank: 3,
        },
        "nl_hybrid_search" => BenchExpectation {
            kind: None,
            symbol: None,
            callee: None,
            file_suffix: Some("search/mod.rs"),
            excerpt_contains: None,
            max_rank: 8,
        },
        _ => return None,
    };
    Some(expected)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IvfOpenLatency {
    pub samples: usize,
    pub fresh_inode_p99_ns: u64,
    pub warm_p99_ns: u64,
    pub sidecar_bytes: u64,
    pub mapped_vector_bytes: usize,
    pub resident_index_bytes: usize,
}

pub fn measure_semantic_ivf_open_p99(
    path: &Path,
    fingerprint: [u8; 32],
    samples: usize,
) -> Result<IvfOpenLatency> {
    if !(100..=1_000).contains(&samples) {
        return Err(StoreError::Other(
            "semantic IVF p99 samples must be between 100 and 1000".into(),
        ));
    }
    let source = fs::read(path)?;
    let sequence = IVF_BENCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut fresh_inode = Vec::with_capacity(samples);
    for sample in 0..samples {
        let temporary = path.with_file_name(format!(
            ".semantic-ivf-fresh-{}-{sequence}-{sample}.ivf",
            std::process::id()
        ));
        let result = (|| -> Result<u64> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&source)?;
            file.sync_all()?;
            drop(file);
            evict_file_cache(&temporary)?;
            let started = Instant::now();
            let opened = load_semantic_ivf(&temporary, fingerprint)?.ok_or_else(|| {
                StoreError::Other("fresh-inode semantic IVF fixture was rejected".into())
            })?;
            if !opened.is_mapped() {
                return Err(StoreError::Other(
                    "semantic IVF benchmark opened owned vectors".into(),
                ));
            }
            std::hint::black_box(opened.chunk_count());
            Ok(elapsed_ns(started))
        })();
        let _ = fs::remove_file(&temporary);
        fresh_inode.push(result?);
    }

    let warmup = load_semantic_ivf(path, fingerprint)?
        .ok_or_else(|| StoreError::Other("semantic IVF benchmark sidecar was rejected".into()))?;
    if !warmup.is_mapped() {
        return Err(StoreError::Other(
            "semantic IVF benchmark warmup opened owned vectors".into(),
        ));
    }
    std::hint::black_box(warmup.chunk_count());
    let mapped_vector_bytes = warmup.mapped_vector_bytes();
    let resident_index_bytes = warmup.resident_index_bytes();
    drop(warmup);
    let mut warm = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        let opened = load_semantic_ivf(path, fingerprint)?
            .ok_or_else(|| StoreError::Other("warm semantic IVF sidecar was rejected".into()))?;
        if !opened.is_mapped() {
            return Err(StoreError::Other(
                "semantic IVF warm open returned owned vectors".into(),
            ));
        }
        std::hint::black_box(opened.chunk_count());
        warm.push(elapsed_ns(started));
    }
    Ok(IvfOpenLatency {
        samples,
        fresh_inode_p99_ns: percentile_99(fresh_inode),
        warm_p99_ns: percentile_99(warm),
        sidecar_bytes: path.metadata()?.len(),
        mapped_vector_bytes,
        resident_index_bytes,
    })
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// 99th percentile of latency samples (nanoseconds).
///
/// Empty input returns `0` without panicking (d2a1.3). Call sites currently
/// only pass non-empty 100..=1000 sample sets; the empty path is defensive.
fn percentile_99(mut samples: Vec<u64>) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let index = samples
        .len()
        .saturating_mul(99)
        .div_ceil(100)
        .saturating_sub(1);
    samples[index.min(samples.len() - 1)]
}

fn evict_file_cache(path: &Path) -> Result<()> {
    // A unique inode per sample guarantees a cold mapping/page-table path. The helper
    // deliberately avoids privileged global page-cache controls; disk-cache residency
    // remains an OS property and is reported separately from warm repeated opens.
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn bench_fixtures() -> &'static [BenchFixture] {
    static FIXTURES: std::sync::OnceLock<Vec<BenchFixture>> = std::sync::OnceLock::new();
    FIXTURES.get_or_init(|| {
        vec![
            BenchFixture {
                name: "sample",
                root: workspace_root().join("tests/fixtures/sample"),
                suite: "default",
            },
            BenchFixture {
                name: "self",
                root: workspace_root(),
                suite: "self",
            },
        ]
    })
}
pub fn suite_by_name(name: &str) -> Option<&'static [BenchCase]> {
    match name {
        "default" => Some(DEFAULT_SUITE),
        "self" => Some(SELF_SUITE),
        _ => None,
    }
}
pub fn fixture_by_name(name: &str) -> Option<&'static BenchFixture> {
    bench_fixtures().iter().find(|f| f.name == name)
}
pub fn list_suite_names() -> &'static [&'static str] {
    &["default", "self"]
}
pub fn list_fixture_names() -> Vec<&'static str> {
    bench_fixtures().iter().map(|f| f.name).collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingStability {
    pub jaccard: f64,
    pub rank_correlation: f64,
}

pub fn ranking_stability(left: &[String], right: &[String]) -> RankingStability {
    use std::collections::{HashMap, HashSet};
    let ls: HashSet<&str> = left.iter().map(String::as_str).collect();
    let rs: HashSet<&str> = right.iter().map(String::as_str).collect();
    let union = ls.union(&rs).count();
    let jaccard = if union == 0 {
        1.0
    } else {
        ls.intersection(&rs).count() as f64 / union as f64
    };
    let right_rank: HashMap<&str, usize> = right
        .iter()
        .enumerate()
        .map(|(rank, id)| (id.as_str(), rank))
        .collect();
    let shared: Vec<(usize, usize)> = left
        .iter()
        .enumerate()
        .filter_map(|(rank, id)| right_rank.get(id.as_str()).map(|&other| (rank, other)))
        .collect();
    let rank_correlation = if shared.len() < 2 {
        if shared.len() == 1 {
            1.0
        } else {
            0.0
        }
    } else {
        let mut by_left = shared.clone();
        by_left.sort_by_key(|(left, _)| *left);
        let mut by_right = shared.clone();
        by_right.sort_by_key(|(_, right)| *right);
        let left_rank: HashMap<usize, usize> = by_left
            .iter()
            .enumerate()
            .map(|(rank, (left, _))| (*left, rank))
            .collect();
        let right_rank: HashMap<usize, usize> = by_right
            .iter()
            .enumerate()
            .map(|(rank, (_, right))| (*right, rank))
            .collect();
        let n = shared.len() as f64;
        let squared_distance: f64 = shared
            .iter()
            .map(|(left, right)| {
                let distance = left_rank[left] as f64 - right_rank[right] as f64;
                distance * distance
            })
            .sum();
        1.0 - (6.0 * squared_distance) / (n * (n * n - 1.0))
    };
    RankingStability {
        jaccard,
        rank_correlation,
    }
}

