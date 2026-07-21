use std::path::PathBuf; #[derive(Debug, Clone, Copy)] pub struct BenchCase { pub name: &'static str, pub query: &'static str, pub min_hits: usize, } #[derive(Debug, Clone)] pub struct BenchFixture {
    pub name: &'static str, pub root: PathBuf, pub suite: &'static str, } pub const DEFAULT_SUITE: &[BenchCase] = &[
    BenchCase { name: "literal_symbol", query: "process_request", min_hits: 1 }, BenchCase { name: "defs_prefix", query: "defs:auth_refresh", min_hits: 1 },
    BenchCase { name: "callers_prefix", query: "callers:process_request", min_hits: 1 }, BenchCase { name: "nl_auth_refresh", query: "how does auth refresh work", min_hits: 1 },
    BenchCase { name: "synonym_credential_renewal", query: "credential renewal", min_hits: 1 }, ]; pub const SELF_SUITE: &[BenchCase] = &[
    BenchCase { name: "core_searcher", query: "Searcher", min_hits: 1 }, BenchCase { name: "semantic_ivf", query: "semantic_ivf", min_hits: 1 },
    BenchCase { name: "defs_search_pattern", query: "defs:search_pattern", min_hits: 1 }, BenchCase { name: "nl_hybrid_search", query: "how does hybrid search work", min_hits: 1 },
]; pub fn workspace_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..") } pub fn bench_fixtures() -> &'static [BenchFixture] {
    static FIXTURES: std::sync::OnceLock<Vec<BenchFixture>> = std::sync::OnceLock::new(); FIXTURES.get_or_init(|| {
        vec![
            BenchFixture { name: "sample", root: workspace_root().join("tests/fixtures/sample"), suite: "default" }, BenchFixture { name: "self", root: workspace_root(), suite: "self" },
        ]
    })
} pub fn suite_by_name(name: &str) -> Option<&'static [BenchCase]> {
    match name { "default" => Some(DEFAULT_SUITE), "self" => Some(SELF_SUITE), _ => None, }
} pub fn fixture_by_name(name: &str) -> Option<&'static BenchFixture> { bench_fixtures().iter().find(|f| f.name == name) } pub fn list_suite_names() -> &'static [&'static str] { &["default", "self"] }
pub fn list_fixture_names() -> Vec<&'static str> { bench_fixtures().iter().map(|f| f.name).collect() } #[derive(Debug, Clone, Copy, PartialEq)] pub struct RankingStability {
    pub jaccard: f64, pub rank_correlation: f64, } pub fn ranking_stability(left: &[String], right: &[String]) -> RankingStability {
    use std::collections::{HashMap, HashSet}; let ls: HashSet<&str> = left.iter().map(String::as_str).collect(); let rs: HashSet<&str> = right.iter().map(String::as_str).collect(); let union = ls.union(&rs).count();
    let jaccard = if union == 0 { 1.0 } else { ls.intersection(&rs).count() as f64 / union as f64 };
    let right_rank: HashMap<&str, usize> = right.iter().enumerate().map(|(r, id)| (id.as_str(), r)).collect(); let shared: Vec<(usize, usize)> = left
        .iter() .enumerate() .filter_map(|(rank, id)| right_rank.get(id.as_str()).map(|&o| (rank, o))) .collect();
    let rank_correlation = if shared.len() < 2 { if shared.len() == 1 { 1.0 } else { 0.0 } } else {
        let n = shared.len() as f64; let sq: f64 = shared.iter().map(|(a, b)| { let d = *a as f64 - *b as f64; d * d }).sum(); 1.0 - (6.0 * sq) / (n * (n * n - 1.0))
    }; RankingStability { jaccard, rank_correlation }
}
