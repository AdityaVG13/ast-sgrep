use crate::Cli; use anyhow::{bail, Context}; use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchHit, SearchOptions, Searcher};
use clap::Parser; use serde::Deserialize; use serde_json::{json, Map, Value}; use std::path::{Path, PathBuf};
const RECALL_CUTOFFS: [usize; 3] = [1, 5, 20];
#[derive(Parser)] pub(crate) struct EvalArgs { #[arg(long)] gold: PathBuf, #[arg(default_value = ".")] root: PathBuf, #[arg(long, value_name = "MODE")] ab: Option<String> }
#[derive(Debug, Deserialize)] struct GoldFixture { corpus: String, #[serde(default)] queries: Vec<GoldQuery> }
#[derive(Debug, Deserialize, Clone)] struct GoldQuery { name: String, query: String, k: usize, relevant: Vec<GoldRelevant> }
#[derive(Debug, Deserialize, Clone)] struct GoldRelevant { file: String, #[serde(default)] symbol: Option<String> }
fn load_gold(path: &Path) -> anyhow::Result<GoldFixture> {
    let text = std::fs::read_to_string(path).with_context(|| format!("failed to read gold fixture {}", path.display()))?;
    let fixture: GoldFixture = serde_json::from_str(&text).with_context(|| format!("failed to parse gold fixture {}", path.display()))?;
    if fixture.queries.is_empty() { bail!("gold fixture {} has no queries", path.display()); } Ok(fixture)
}
fn gold_hit_matches(rel: &GoldRelevant, hit: &SearchHit) -> bool {
    hit.file.ends_with(&rel.file) && rel.symbol.as_ref().is_none_or(|s| hit.symbol.as_deref() == Some(s.as_str()))
}
struct Scan { first_rank: Option<usize>, found: usize, dcg: f64 }
fn scan(relevant: &[GoldRelevant], hits: &[SearchHit], cutoff: usize) -> Scan {
    let mut matched = vec![false; relevant.len()]; let mut first_rank = None; let mut found = 0usize; let mut dcg = 0.0f64;
    for (idx, hit) in hits.iter().take(cutoff).enumerate() {
        let rank = idx + 1;
        for (ri, rel) in relevant.iter().enumerate() {
            if matched[ri] || !gold_hit_matches(rel, hit) { continue; }
            matched[ri] = true; found += 1; dcg += 1.0 / ((rank as f64) + 1.0).log2(); first_rank.get_or_insert(rank); break;
        }
    }
    Scan { first_rank, found, dcg }
}
fn idcg(ideal: usize) -> f64 { (1..=ideal).map(|r| 1.0 / ((r as f64) + 1.0).log2()).sum() }
fn recall_of(found: usize, relevant: usize) -> f64 { if relevant == 0 { 0.0 } else { found as f64 / relevant as f64 } }
struct QueryEval { name: String, query: String, first_rank: Option<usize>, rr: f64, found: usize, relevant: usize, ndcg: f64, recall_at: [(usize, f64); RECALL_CUTOFFS.len()] }
fn evaluate_query(query: &GoldQuery, hits: &[SearchHit]) -> QueryEval {
    let primary = scan(&query.relevant, hits, query.k); let idcg_v = idcg(query.relevant.len().min(query.k));
    let ndcg = if idcg_v > 0.0 { primary.dcg / idcg_v } else { 0.0 };
    let mut recall_at = [(0usize, 0.0f64); RECALL_CUTOFFS.len()];
    for (slot, &n) in recall_at.iter_mut().zip(RECALL_CUTOFFS.iter()) { let s = scan(&query.relevant, hits, n.min(query.k)); *slot = (n, recall_of(s.found, query.relevant.len())); }
    QueryEval { name: query.name.clone(), query: query.query.clone(), first_rank: primary.first_rank, rr: primary.first_rank.map_or(0.0, |r| 1.0 / r as f64), found: primary.found, relevant: query.relevant.len(), ndcg, recall_at }
}
struct Aggregate { mrr: f64, ndcg: f64, recall_at_k: f64, recall_at: [(usize, f64); RECALL_CUTOFFS.len()], n_queries: usize }
fn aggregate(evals: &[QueryEval]) -> Aggregate {
    let n = evals.len().max(1) as f64; let mut recall_at = [(0usize, 0.0f64); RECALL_CUTOFFS.len()];
    for (i, slot) in recall_at.iter_mut().enumerate() { *slot = (RECALL_CUTOFFS[i], evals.iter().map(|e| e.recall_at[i].1).sum::<f64>() / n); }
    Aggregate { mrr: evals.iter().map(|e| e.rr).sum::<f64>() / n, ndcg: evals.iter().map(|e| e.ndcg).sum::<f64>() / n, recall_at_k: evals.iter().map(|e| recall_of(e.found, e.relevant)).sum::<f64>() / n, recall_at, n_queries: evals.len() }
}
fn round3(x: f64) -> f64 { (x * 1000.0).round() / 1000.0 }
#[derive(Clone, Copy)] struct EvalConfig { no_embed: bool, semantic_only: bool }
impl EvalConfig {
    const HYBRID: Self = Self { no_embed: false, semantic_only: false };
    fn label(self) -> &'static str { if self.semantic_only { "semantic-only" } else if self.no_embed { "no-embed" } else { "hybrid" } }
    fn json(self, root: &Path, index_path: &Path) -> Value {
        json!({"root": root.display().to_string(), "index_path": index_path.display().to_string(), "no_embed": self.no_embed, "semantic_only": self.semantic_only})
    }
}
fn ab_config(mode: &str) -> anyhow::Result<EvalConfig> {
    match mode {
        "no-embed" => Ok(EvalConfig { no_embed: true, semantic_only: false }), "semantic-only" => Ok(EvalConfig { no_embed: false, semantic_only: true }),
        other => bail!("unknown --ab mode {other:?}; expected \"no-embed\" or \"semantic-only\""),
    }
}
fn run_single(cli: &Cli, root: &Path, index_path: &Path, limit: usize, gold: &GoldFixture, cfg: EvalConfig) -> anyhow::Result<(Vec<QueryEval>, Aggregate)> {
    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(), index_path: Some(index_path.to_path_buf()), limit, lang_filter: cli.lang.clone(), use_embed: !cfg.no_embed,
        use_tantivy: cli.tantivy, use_cloud_embed: cli.cloud_embed, use_ollama_embed: cli.ollama_embed, use_semantic_only: false, ann_threshold: cli.ann_threshold, ..SearchOptions::default()
    }).with_context(|| format!("failed to open searcher for eval ({})", cfg.label()))?;
    let evals: Vec<QueryEval> = gold.queries.iter().map(|q| {
        let response = if cfg.semantic_only { searcher.search_semantic(&q.query) } else { searcher.search(&q.query) }
            .with_context(|| format!("query {:?} ({:?}) failed", q.name, q.query))?;
        Ok::<_, anyhow::Error>(evaluate_query(q, &response.hits))
    }).collect::<anyhow::Result<_>>()?;
    let agg = aggregate(&evals); Ok((evals, agg))
}
pub(crate) fn run_eval(cli: &Cli, args: &EvalArgs) -> anyhow::Result<()> {
    let root = crate::effective_root(cli, &args.root); let gold = load_gold(&args.gold)?;
    let max_k = gold.queries.iter().map(|q| q.k).max().unwrap_or(1); let limit = cli.limit.unwrap_or_else(SearchOptions::default_limit).max(max_k);
    let mut _temp_guard = None;
    let index_path = match &cli.index_path {
        Some(p) => p.clone(),
        None => { let dir = tempfile::TempDir::new().context("failed to create temp index dir for eval")?; let path = dir.path().join("index.db"); _temp_guard = Some(dir); path }
    };
    if !cli.json { eprintln!("[asgrep eval] indexing {} into {} ...", root.display(), index_path.display()); }
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(), index_path: Some(index_path.clone()), lang_filter: cli.lang.clone(), respect_gitignore: true, use_tantivy: cli.tantivy, embed_semantic: true,
        embed_backend: EmbedBackend::from_flags(cli.cloud_embed, cli.ollama_embed, cli.neural_embed, false), force_reindex: false, ann_threshold: cli.ann_threshold,
    }).context("failed to open index for eval")?;
    indexer.index_all().context("indexing failed for eval")?;
    match &args.ab {
        Some(mode) => {
            let cfg_b = ab_config(mode)?;
            let (evals_a, agg_a) = run_single(cli, &root, &index_path, limit, &gold, EvalConfig::HYBRID)?;
            let (evals_b, agg_b) = run_single(cli, &root, &index_path, limit, &gold, cfg_b)?;
            print_ab(cli, &args.gold, &gold, &root, &index_path, EvalConfig::HYBRID, cfg_b, &evals_a, &evals_b, &agg_a, &agg_b)?;
        }
        None => {
            let cfg = EvalConfig { no_embed: cli.no_embed, semantic_only: cli.semantic_only };
            let (evals, agg) = run_single(cli, &root, &index_path, limit, &gold, cfg)?;
            print_single(cli, &args.gold, &gold, &root, &index_path, cfg, &evals, &agg)?;
        }
    }
    Ok(())
}
fn rank_s(r: Option<usize>) -> String { r.map_or_else(|| "miss".into(), |n| n.to_string()) }
fn query_eval_json(e: &QueryEval) -> Value {
    let mut recall_at = Map::new(); for (n, v) in &e.recall_at { recall_at.insert(n.to_string(), json!(round3(*v))); }
    let intent = ast_sgrep_core::intent::classify(&ast_sgrep_core::query::ParsedQuery::parse(&e.query));
    json!({"name": e.name, "query": e.query, "intent": intent.as_str(), "first_rank": e.first_rank, "rr": round3(e.rr), "found": e.found, "relevant": e.relevant, "ndcg": round3(e.ndcg), "recall_at": Value::Object(recall_at)})
}
fn aggregate_json(agg: &Aggregate) -> Value {
    let mut m = Map::new(); m.insert("mrr".into(), json!(round3(agg.mrr))); m.insert("ndcg".into(), json!(round3(agg.ndcg)));
    m.insert("recall_at_k".into(), json!(round3(agg.recall_at_k)));
    for (n, v) in &agg.recall_at { m.insert(format!("recall_at_{n}"), json!(round3(*v))); }
    m.insert("n_queries".into(), json!(agg.n_queries)); Value::Object(m)
}
fn single_json(gold_path: &Path, gold: &GoldFixture, root: &Path, index_path: &Path, cfg: EvalConfig, evals: &[QueryEval], agg: &Aggregate) -> Value {
    json!({"gold": gold_path.display().to_string(), "corpus": gold.corpus, "config": cfg.json(root, index_path), "queries": evals.iter().map(query_eval_json).collect::<Vec<_>>(), "aggregate": aggregate_json(agg)})
}
#[allow(clippy::too_many_arguments)]
fn print_single(cli: &Cli, gold_path: &Path, gold: &GoldFixture, root: &Path, index_path: &Path, cfg: EvalConfig, evals: &[QueryEval], agg: &Aggregate) -> anyhow::Result<()> {
    if cli.json { return crate::print_machine_json("eval", single_json(gold_path, gold, root, index_path, cfg, evals, agg)); }
    println!("corpus: {}  queries: {}  config: {}", gold.corpus, evals.len(), cfg.label()); println!();
    println!("| query | first_rank | rr | found/relevant | ndcg |"); println!("|-------|-----------:|---:|----------------:|-----:|");
    for e in evals { println!("| {} | {} | {:.3} | {}/{} | {:.3} |", e.name, rank_s(e.first_rank), e.rr, e.found, e.relevant, e.ndcg); }
    println!();
    println!("MRR={:.3}  Recall@k={:.3}  nDCG@k={:.3}  Recall@1={:.3}  Recall@5={:.3}  Recall@20={:.3}  n={}",
        agg.mrr, agg.recall_at_k, agg.ndcg, agg.recall_at[0].1, agg.recall_at[1].1, agg.recall_at[2].1, agg.n_queries);
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn print_ab(cli: &Cli, gold_path: &Path, gold: &GoldFixture, root: &Path, index_path: &Path, cfg_a: EvalConfig, cfg_b: EvalConfig, evals_a: &[QueryEval], evals_b: &[QueryEval], agg_a: &Aggregate, agg_b: &Aggregate) -> anyhow::Result<()> {
    if cli.json {
        let a = single_json(gold_path, gold, root, index_path, cfg_a, evals_a, agg_a); let b = single_json(gold_path, gold, root, index_path, cfg_b, evals_b, agg_b);
        let queries: Vec<Value> = evals_a.iter().zip(evals_b.iter()).map(|(qa, qb)| json!({"name": qa.name, "rank_a": qa.first_rank, "rank_b": qb.first_rank, "delta_rr": round3(qb.rr - qa.rr), "delta_ndcg": round3(qb.ndcg - qa.ndcg)})).collect();
        let aggregate = json!({"delta_mrr": round3(agg_b.mrr - agg_a.mrr), "delta_ndcg": round3(agg_b.ndcg - agg_a.ndcg), "delta_recall_at_k": round3(agg_b.recall_at_k - agg_a.recall_at_k),
            "delta_recall_at_1": round3(agg_b.recall_at[0].1 - agg_a.recall_at[0].1), "delta_recall_at_5": round3(agg_b.recall_at[1].1 - agg_a.recall_at[1].1), "delta_recall_at_20": round3(agg_b.recall_at[2].1 - agg_a.recall_at[2].1)});
        return crate::print_machine_json("eval", json!({"a": a, "b": b, "diff": {"queries": queries, "aggregate": aggregate}}));
    }
    println!("corpus: {}  queries: {}  A={}  B={}", gold.corpus, evals_a.len(), cfg_a.label(), cfg_b.label()); println!();
    println!("| query | rank A | rank B | delta rr | delta ndcg |"); println!("|-------|-------:|-------:|---------:|-----------:|");
    for (a, b) in evals_a.iter().zip(evals_b.iter()) { println!("| {} | {} | {} | {:+.3} | {:+.3} |", a.name, rank_s(a.first_rank), rank_s(b.first_rank), b.rr - a.rr, b.ndcg - a.ndcg); }
    println!();
    println!("delta MRR={:+.3}  delta Recall@k={:+.3}  delta nDCG@k={:+.3}  delta Recall@1={:+.3}  delta Recall@5={:+.3}  delta Recall@20={:+.3}",
        agg_b.mrr - agg_a.mrr, agg_b.recall_at_k - agg_a.recall_at_k, agg_b.ndcg - agg_a.ndcg, agg_b.recall_at[0].1 - agg_a.recall_at[0].1, agg_b.recall_at[1].1 - agg_a.recall_at[1].1, agg_b.recall_at[2].1 - agg_a.recall_at[2].1);
    Ok(())
}
