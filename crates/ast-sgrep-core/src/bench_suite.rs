use crate::search::{HitKind, SearchHit};
use std::path::PathBuf;

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
            max_rank: 5,
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
    fn unique_ids(values: &[String]) -> Vec<&str> {
        let mut seen = HashSet::new();
        values
            .iter()
            .map(String::as_str)
            .filter(|id| seen.insert(*id))
            .collect()
    }
    let left = unique_ids(left);
    let right = unique_ids(right);
    let ls: HashSet<&str> = left.iter().copied().collect();
    let rs: HashSet<&str> = right.iter().copied().collect();
    let union = ls.union(&rs).count();
    let jaccard = if union == 0 {
        1.0
    } else {
        ls.intersection(&rs).count() as f64 / union as f64
    };
    let right_rank: HashMap<&str, usize> =
        right.iter().enumerate().map(|(r, id)| (*id, r)).collect();
    let shared: Vec<(usize, usize)> = left
        .iter()
        .enumerate()
        .filter_map(|(rank, id)| right_rank.get(id).map(|&o| (rank, o)))
        .collect();
    let rank_correlation = if shared.len() < 2 {
        if shared.len() == 1 {
            1.0
        } else {
            0.0
        }
    } else {
        // e2hc.19d: Re-rank shared items within the shared subset before
        // computing Spearman's correlation. The old code used full-list
        // positions, which penalizes items far apart in the full lists but
        // adjacent in the shared subset, producing meaningless negative
        // correlations (e.g. -127 for lists with different lengths).
        let n = shared.len();
        let mut by_left: Vec<(usize, usize)> = shared.clone();
        by_left.sort_by_key(|(l, _)| *l);
        let mut by_right: Vec<(usize, usize)> = shared.clone();
        by_right.sort_by_key(|(_, r)| *r);
        let left_rank: HashMap<usize, usize> = by_left
            .iter()
            .enumerate()
            .map(|(rank, (l, _))| (*l, rank))
            .collect();
        let right_rank: HashMap<usize, usize> = by_right
            .iter()
            .enumerate()
            .map(|(rank, (_, r))| (*r, rank))
            .collect();
        let nf = n as f64;
        let sq: f64 = shared
            .iter()
            .map(|(l, r)| {
                let d = left_rank[l] as f64 - right_rank[r] as f64;
                d * d
            })
            .sum();
        1.0 - (6.0 * sq) / (nf * (nf * nf - 1.0))
    };
    RankingStability {
        jaccard,
        rank_correlation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_benchmark_case_has_a_specific_identity_oracle() {
        for case in DEFAULT_SUITE.iter().chain(SELF_SUITE) {
            let expected = benchmark_expectation(case)
                .unwrap_or_else(|| panic!("{} has no identity oracle", case.name));
            assert!(
                expected.is_specific(),
                "{} has no identity oracle",
                case.name
            );
            assert!(expected.max_rank > 0, "{} has a vacuous rank", case.name);
        }
    }

    /// e2hc.19d: Shared items in the same relative order must produce
    /// rank_correlation = 1.0, even when their full-list positions differ
    /// (e.g. left has extra non-shared items shifting positions). Pre-fix,
    /// full-list positions were used, yielding meaningless negative values.
    #[test]
    fn rank_correlation_reranks_shared_subset() {
        let left = vec!["x".into(), "a".into(), "b".into(), "c".into()];
        let right = vec!["a".into(), "b".into(), "c".into()];
        let stability = ranking_stability(&left, &right);
        // Shared items a,b,c are in the same relative order in both lists.
        assert_eq!(
            stability.rank_correlation, 1.0,
            "same relative order in shared subset must give perfect correlation"
        );
    }

    /// e2hc.19d: Lists of different lengths with large position offsets must
    /// not produce absurdly negative correlations. Pre-fix, full-list positions
    /// could return values like -127.
    #[test]
    fn rank_correlation_does_not_explode_on_position_offset() {
        let left: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
        // Right list: same items but reversed, with 50 non-shared items prepended
        let mut right: Vec<String> = (0..50).map(|i| format!("noise{i}")).collect();
        right.extend(left.iter().rev().cloned());
        let stability = ranking_stability(&left, &right);
        // Spearman's is bounded in [-1, 1]. Reversed order → -1.0.
        assert!(
            stability.rank_correlation >= -1.0 && stability.rank_correlation <= 1.0,
            "rank_correlation must stay in [-1, 1], got {}",
            stability.rank_correlation
        );
        assert!(
            (stability.rank_correlation - (-1.0)).abs() < 1e-9,
            "reversed shared subset must give -1.0, got {}",
            stability.rank_correlation
        );
    }
}
