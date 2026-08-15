use crate::{index_options, search_options, Cli};
use anyhow::{bail, Context};
use ast_sgrep_core::search::DegradedChannel;
use ast_sgrep_core::{Indexer, SearchHit, SearchOptions, Searcher};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
const RECALL_CUTOFFS: [usize; 3] = [1, 5, 20];
const RESOLUTION_TIERS: [&str; 8] = [
    "compiler_exact",
    "scip_occurrence",
    "import_resolved",
    "file_local_unique",
    "repository_unique",
    "name_only",
    "ambiguous",
    "unresolved",
];
#[derive(Parser)]
pub(crate) struct EvalArgs {
    #[arg(long)]
    gold: PathBuf,
    #[arg(long, value_name = "PATH", help = "Optional SCIP JSON index overlay")]
    scip: Option<PathBuf>,
    #[arg(default_value = ".")]
    root: PathBuf,
    #[arg(long, value_name = "MODE")]
    ab: Option<String>,
}
#[derive(Debug, Deserialize)]
struct GoldFixture {
    corpus: String,
    #[serde(default)]
    queries: Vec<GoldQuery>,
    #[serde(default)]
    graph_edges: Vec<GoldGraphQuery>,
}
#[derive(Debug, Deserialize, Clone)]
struct GoldQuery {
    name: String,
    query: String,
    k: usize,
    relevant: Vec<GoldRelevant>,
}
#[derive(Debug, Deserialize, Clone)]
struct GoldRelevant {
    file: String,
    #[serde(default)]
    symbol: Option<String>,
}
#[derive(Debug, Deserialize, Clone)]
struct GoldGraphQuery {
    name: String,
    query: String,
    k: usize,
    relevant: Vec<GoldGraphEdge>,
}
#[derive(Debug, Deserialize, Clone)]
struct GoldGraphEdge {
    file: String,
    caller: String,
    callee: String,
    #[serde(default)]
    line: Option<u32>,
}
fn load_gold(path: &Path) -> anyhow::Result<GoldFixture> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read gold fixture {}", path.display()))?;
    let fixture: GoldFixture = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse gold fixture {}", path.display()))?;
    if fixture.queries.is_empty() && fixture.graph_edges.is_empty() {
        bail!(
            "gold fixture {} has no retrieval or graph-edge queries",
            path.display()
        );
    }
    Ok(fixture)
}
fn gold_hit_matches(rel: &GoldRelevant, hit: &SearchHit) -> bool {
    hit.file.ends_with(&rel.file)
        && rel
            .symbol
            .as_ref()
            .is_none_or(|s| hit.symbol.as_deref() == Some(s.as_str()))
}
struct Scan {
    first_rank: Option<usize>,
    found: usize,
    dcg: f64,
}
fn scan(relevant: &[GoldRelevant], hits: &[SearchHit], cutoff: usize) -> Scan {
    let mut matched = vec![false; relevant.len()];
    let mut first_rank = None;
    let mut found = 0usize;
    let mut dcg = 0.0f64;
    for (idx, hit) in hits.iter().take(cutoff).enumerate() {
        let rank = idx + 1;
        for (ri, rel) in relevant.iter().enumerate() {
            if matched[ri] || !gold_hit_matches(rel, hit) {
                continue;
            }
            matched[ri] = true;
            found += 1;
            dcg += 1.0 / ((rank as f64) + 1.0).log2();
            first_rank.get_or_insert(rank);
            break;
        }
    }
    Scan {
        first_rank,
        found,
        dcg,
    }
}
fn idcg(ideal: usize) -> f64 {
    (1..=ideal).map(|r| 1.0 / ((r as f64) + 1.0).log2()).sum()
}
fn recall_of(found: usize, relevant: usize) -> f64 {
    if relevant == 0 {
        0.0
    } else {
        found as f64 / relevant as f64
    }
}
struct QueryEval {
    name: String,
    query: String,
    first_rank: Option<usize>,
    rr: f64,
    found: usize,
    relevant: usize,
    ndcg: f64,
    recall_at: [(usize, f64); RECALL_CUTOFFS.len()],
}
fn evaluate_query(query: &GoldQuery, hits: &[SearchHit]) -> QueryEval {
    let primary = scan(&query.relevant, hits, query.k);
    let idcg_v = idcg(query.relevant.len().min(query.k));
    let ndcg = if idcg_v > 0.0 {
        primary.dcg / idcg_v
    } else {
        0.0
    };
    let mut recall_at = [(0usize, 0.0f64); RECALL_CUTOFFS.len()];
    for (slot, &n) in recall_at.iter_mut().zip(RECALL_CUTOFFS.iter()) {
        let s = scan(&query.relevant, hits, n.min(query.k));
        *slot = (n, recall_of(s.found, query.relevant.len()));
    }
    QueryEval {
        name: query.name.clone(),
        query: query.query.clone(),
        first_rank: primary.first_rank,
        rr: primary.first_rank.map_or(0.0, |r| 1.0 / r as f64),
        found: primary.found,
        relevant: query.relevant.len(),
        ndcg,
        recall_at,
    }
}
struct Aggregate {
    mrr: f64,
    ndcg: f64,
    recall_at_k: f64,
    recall_at: [(usize, f64); RECALL_CUTOFFS.len()],
    n_queries: usize,
}
#[derive(Debug, Clone, Default, Serialize)]
struct ResolutionPrecision {
    predicted: usize,
    correct: usize,
    precision: Option<f64>,
}
#[derive(Debug, Clone, Serialize)]
struct GraphPrecisionReport {
    labeled_queries: usize,
    gold_edges: usize,
    scip_requested: bool,
    scip_loaded: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    degraded_channels: Vec<DegradedChannel>,
    by_resolution: BTreeMap<String, ResolutionPrecision>,
}
struct EvalRun {
    queries: Vec<QueryEval>,
    aggregate: Aggregate,
    graph: GraphPrecisionReport,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GraphEdgeKey {
    file: String,
    line: u32,
    caller: String,
    callee: String,
}
#[derive(Debug, Clone)]
struct PredictedGraphEdge {
    key: GraphEdgeKey,
    tier: String,
    tier_rank: u8,
}
#[derive(Clone, Copy)]
struct ScipEvalState<'a> {
    requested: bool,
    degraded_channels: &'a [DegradedChannel],
}
fn aggregate(evals: &[QueryEval]) -> Aggregate {
    let n = evals.len().max(1) as f64;
    let mut recall_at = [(0usize, 0.0f64); RECALL_CUTOFFS.len()];
    for (i, slot) in recall_at.iter_mut().enumerate() {
        *slot = (
            RECALL_CUTOFFS[i],
            evals.iter().map(|e| e.recall_at[i].1).sum::<f64>() / n,
        );
    }
    Aggregate {
        mrr: evals.iter().map(|e| e.rr).sum::<f64>() / n,
        ndcg: evals.iter().map(|e| e.ndcg).sum::<f64>() / n,
        recall_at_k: evals
            .iter()
            .map(|e| recall_of(e.found, e.relevant))
            .sum::<f64>()
            / n,
        recall_at,
        n_queries: evals.len(),
    }
}
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}
fn graph_edge_from_hit(hit: &SearchHit) -> Option<PredictedGraphEdge> {
    let caller = hit.caller.as_ref()?;
    let callee = hit.callee.as_ref()?;
    let (tier, tier_rank) = hit.resolution.as_ref().map_or_else(
        || ("unresolved".to_owned(), u8::MAX),
        |resolution| (resolution.as_str().to_owned(), resolution.rank()),
    );
    Some(PredictedGraphEdge {
        key: GraphEdgeKey {
            file: hit.file.clone(),
            line: hit.line_start,
            caller: caller.clone(),
            callee: callee.clone(),
        },
        tier,
        tier_rank,
    })
}
fn gold_graph_edge_matches(gold: &GoldGraphEdge, predicted: &GraphEdgeKey) -> bool {
    Path::new(&predicted.file).ends_with(Path::new(&gold.file))
        && predicted.caller == gold.caller
        && predicted.callee == gold.callee
        && gold.line.is_none_or(|line| predicted.line == line)
}
fn graph_precision(
    searcher: &Searcher,
    queries: &[GoldGraphQuery],
    scip: ScipEvalState<'_>,
) -> anyhow::Result<GraphPrecisionReport> {
    let mut by_resolution = RESOLUTION_TIERS
        .into_iter()
        .map(|tier| (tier.to_owned(), ResolutionPrecision::default()))
        .collect::<BTreeMap<_, _>>();
    let mut gold_edges = 0usize;
    for query in queries {
        gold_edges += query.relevant.len();
        let mut predicted = BTreeMap::<GraphEdgeKey, PredictedGraphEdge>::new();
        let response = searcher
            .search(&query.query)
            .with_context(|| format!("graph query {:?} ({:?}) failed", query.name, query.query))?;
        for edge in response
            .hits
            .iter()
            .take(query.k)
            .filter_map(graph_edge_from_hit)
        {
            predicted
                .entry(edge.key.clone())
                .and_modify(|current| {
                    if edge.tier_rank < current.tier_rank {
                        *current = edge.clone();
                    }
                })
                .or_insert(edge);
        }
        for edge in predicted.values() {
            let tier = by_resolution.entry(edge.tier.clone()).or_default();
            tier.predicted += 1;
            if query
                .relevant
                .iter()
                .any(|gold| gold_graph_edge_matches(gold, &edge.key))
            {
                tier.correct += 1;
            }
        }
    }
    for tier in by_resolution.values_mut() {
        tier.precision =
            (tier.predicted > 0).then(|| round3(tier.correct as f64 / tier.predicted as f64));
    }
    Ok(GraphPrecisionReport {
        labeled_queries: queries.len(),
        gold_edges,
        scip_requested: scip.requested,
        scip_loaded: scip.requested && scip.degraded_channels.is_empty(),
        degraded_channels: scip.degraded_channels.to_vec(),
        by_resolution,
    })
}
#[derive(Clone, Copy)]
struct EvalConfig {
    no_embed: bool,
    semantic_only: bool,
    /// 7d5x.4: rank embed hits by the concatenated chunk vector only,
    /// skipping the intent-weighted per-field rescore.
    concat_embed: bool,
}
impl EvalConfig {
    const HYBRID: Self = Self {
        no_embed: false,
        semantic_only: false,
        concat_embed: false,
    };
    fn label(self) -> &'static str {
        if self.semantic_only {
            "semantic-only"
        } else if self.no_embed {
            "no-embed"
        } else if self.concat_embed {
            "concat-embed"
        } else {
            "hybrid"
        }
    }
    fn json(self, root: &Path, index_path: &Path) -> Value {
        json!({"root": root.display().to_string(), "index_path": index_path.display().to_string(), "no_embed": self.no_embed, "semantic_only": self.semantic_only, "concat_embed": self.concat_embed})
    }
}
fn ab_config(mode: &str) -> anyhow::Result<EvalConfig> {
    match mode {
        "no-embed" => Ok(EvalConfig {
            no_embed: true,
            ..EvalConfig::HYBRID
        }),
        "semantic-only" => Ok(EvalConfig {
            semantic_only: true,
            ..EvalConfig::HYBRID
        }),
        "concat-embed" => Ok(EvalConfig {
            concat_embed: true,
            ..EvalConfig::HYBRID
        }),
        other => bail!(
            "unknown --ab mode {other:?}; expected \"no-embed\", \"semantic-only\", or \"concat-embed\""
        ),
    }
}
fn run_single(
    cli: &Cli,
    root: &Path,
    index_path: &Path,
    limit: usize,
    gold: &GoldFixture,
    cfg: EvalConfig,
    scip: ScipEvalState<'_>,
) -> anyhow::Result<EvalRun> {
    let mut opts = search_options(root, cli);
    opts.index_path = Some(index_path.to_path_buf());
    opts.limit = limit;
    opts.use_embed = !cfg.no_embed;
    opts.use_semantic_only = false;
    opts.use_field_rescoring = !cfg.concat_embed;
    let searcher = Searcher::new(opts)
        .with_context(|| format!("failed to open searcher for eval ({})", cfg.label()))?;
    let evals: Vec<QueryEval> = gold
        .queries
        .iter()
        .map(|q| {
            let response = if cfg.semantic_only {
                searcher.search_semantic(&q.query)
            } else {
                searcher.search(&q.query)
            }
            .with_context(|| format!("query {:?} ({:?}) failed", q.name, q.query))?;
            Ok::<_, anyhow::Error>(evaluate_query(q, &response.hits))
        })
        .collect::<anyhow::Result<_>>()?;
    let aggregate = aggregate(&evals);
    let graph = graph_precision(&searcher, &gold.graph_edges, scip)?;
    Ok(EvalRun {
        queries: evals,
        aggregate,
        graph,
    })
}
pub(crate) fn run_eval(cli: &Cli, args: &EvalArgs) -> anyhow::Result<()> {
    let root = crate::effective_root(cli, &args.root);
    let gold = load_gold(&args.gold)?;
    let max_k = gold
        .queries
        .iter()
        .map(|query| query.k)
        .chain(gold.graph_edges.iter().map(|query| query.k))
        .max()
        .unwrap_or(1);
    let limit = cli
        .limit
        .unwrap_or_else(SearchOptions::default_limit)
        .max(max_k);
    let mut _temp_guard = None;
    let index_path = match &cli.index_path {
        Some(p) => p.clone(),
        None => {
            let dir =
                tempfile::TempDir::new().context("failed to create temp index dir for eval")?;
            let path = dir.path().join("index.db");
            _temp_guard = Some(dir);
            path
        }
    };
    if !cli.json {
        eprintln!(
            "[asgrep eval] indexing {} into {} ...",
            root.display(),
            index_path.display()
        );
    }
    let mut idx_opts = index_options(&root, cli);
    idx_opts.index_path = Some(index_path.clone());
    idx_opts.embed_semantic = true;
    let tuning = cli.active_tuning();
    idx_opts.embed_backend = ast_sgrep_core::EmbedBackend::from_flags(tuning.neural_embed, false);
    let mut indexer = Indexer::new(idx_opts).context("failed to open index for eval")?;
    indexer.index_all().context("indexing failed for eval")?;
    let degraded_channels = crate::index_cmd::ingest_scip(&indexer, args.scip.as_deref())?;
    drop(indexer);
    let scip = ScipEvalState {
        requested: args.scip.is_some(),
        degraded_channels: &degraded_channels,
    };
    match &args.ab {
        Some(mode) => {
            let cfg_b = ab_config(mode)?;
            let run_a = run_single(
                cli,
                &root,
                &index_path,
                limit,
                &gold,
                EvalConfig::HYBRID,
                scip,
            )?;
            let run_b = run_single(cli, &root, &index_path, limit, &gold, cfg_b, scip)?;
            print_ab(
                cli,
                &args.gold,
                &gold,
                &root,
                &index_path,
                EvalConfig::HYBRID,
                cfg_b,
                &run_a,
                &run_b,
            )
        }
        None => {
            let tuning = cli.active_tuning();
            let cfg = EvalConfig {
                no_embed: tuning.no_embed,
                semantic_only: tuning.semantic_only,
                concat_embed: false,
            };
            let run = run_single(cli, &root, &index_path, limit, &gold, cfg, scip)?;
            print_single(cli, &args.gold, &gold, &root, &index_path, cfg, &run)
        }
    }
}
fn rank_s(r: Option<usize>) -> String {
    r.map_or_else(|| "miss".into(), |n| n.to_string())
}
fn query_eval_json(e: &QueryEval) -> Value {
    let mut recall_at = Map::new();
    for (n, v) in &e.recall_at {
        recall_at.insert(n.to_string(), json!(round3(*v)));
    }
    let intent =
        ast_sgrep_core::intent::classify(&ast_sgrep_core::query::ParsedQuery::parse(&e.query));
    json!({"name": e.name, "query": e.query, "intent": intent.as_str(), "first_rank": e.first_rank, "rr": round3(e.rr), "found": e.found, "relevant": e.relevant, "ndcg": round3(e.ndcg), "recall_at": Value::Object(recall_at)})
}
fn aggregate_json(agg: &Aggregate) -> Value {
    let mut m = Map::new();
    m.insert("mrr".into(), json!(round3(agg.mrr)));
    m.insert("ndcg".into(), json!(round3(agg.ndcg)));
    m.insert("recall_at_k".into(), json!(round3(agg.recall_at_k)));
    for (n, v) in &agg.recall_at {
        m.insert(format!("recall_at_{n}"), json!(round3(*v)));
    }
    m.insert("n_queries".into(), json!(agg.n_queries));
    Value::Object(m)
}
fn single_json(
    gold_path: &Path,
    gold: &GoldFixture,
    root: &Path,
    index_path: &Path,
    cfg: EvalConfig,
    run: &EvalRun,
) -> Value {
    json!({"gold": gold_path.display().to_string(), "corpus": gold.corpus, "config": cfg.json(root, index_path), "queries": run.queries.iter().map(query_eval_json).collect::<Vec<_>>(), "aggregate": aggregate_json(&run.aggregate), "graph_edge_precision": run.graph})
}
#[allow(clippy::too_many_arguments)]
fn print_single(
    cli: &Cli,
    gold_path: &Path,
    gold: &GoldFixture,
    root: &Path,
    index_path: &Path,
    cfg: EvalConfig,
    run: &EvalRun,
) -> anyhow::Result<()> {
    if cli.json {
        return crate::print_machine_json(
            "eval",
            single_json(gold_path, gold, root, index_path, cfg, run),
        );
    }
    println!(
        "corpus: {}  queries: {}  config: {}",
        gold.corpus,
        run.queries.len(),
        cfg.label()
    );
    println!();
    println!("| query | first_rank | rr | found/relevant | ndcg |");
    println!("|-------|-----------:|---:|----------------:|-----:|");
    for e in &run.queries {
        println!(
            "| {} | {} | {:.3} | {}/{} | {:.3} |",
            e.name,
            rank_s(e.first_rank),
            e.rr,
            e.found,
            e.relevant,
            e.ndcg
        );
    }
    println!();
    println!("MRR={:.3}  Recall@k={:.3}  nDCG@k={:.3}  Recall@1={:.3}  Recall@5={:.3}  Recall@20={:.3}  n={}",
        run.aggregate.mrr, run.aggregate.recall_at_k, run.aggregate.ndcg, run.aggregate.recall_at[0].1, run.aggregate.recall_at[1].1, run.aggregate.recall_at[2].1, run.aggregate.n_queries);
    print_graph_precision(&run.graph, None);
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn print_ab(
    cli: &Cli,
    gold_path: &Path,
    gold: &GoldFixture,
    root: &Path,
    index_path: &Path,
    cfg_a: EvalConfig,
    cfg_b: EvalConfig,
    run_a: &EvalRun,
    run_b: &EvalRun,
) -> anyhow::Result<()> {
    if cli.json {
        let a = single_json(gold_path, gold, root, index_path, cfg_a, run_a);
        let b = single_json(gold_path, gold, root, index_path, cfg_b, run_b);
        let queries: Vec<Value> = run_a.queries.iter().zip(run_b.queries.iter()).map(|(qa, qb)| json!({"name": qa.name, "rank_a": qa.first_rank, "rank_b": qb.first_rank, "delta_rr": round3(qb.rr - qa.rr), "delta_ndcg": round3(qb.ndcg - qa.ndcg)})).collect();
        let aggregate = json!({"delta_mrr": round3(run_b.aggregate.mrr - run_a.aggregate.mrr), "delta_ndcg": round3(run_b.aggregate.ndcg - run_a.aggregate.ndcg), "delta_recall_at_k": round3(run_b.aggregate.recall_at_k - run_a.aggregate.recall_at_k),
            "delta_recall_at_1": round3(run_b.aggregate.recall_at[0].1 - run_a.aggregate.recall_at[0].1), "delta_recall_at_5": round3(run_b.aggregate.recall_at[1].1 - run_a.aggregate.recall_at[1].1), "delta_recall_at_20": round3(run_b.aggregate.recall_at[2].1 - run_a.aggregate.recall_at[2].1)});
        return crate::print_machine_json(
            "eval",
            json!({"a": a, "b": b, "diff": {"queries": queries, "aggregate": aggregate}}),
        );
    }
    println!(
        "corpus: {}  queries: {}  A={}  B={}",
        gold.corpus,
        run_a.queries.len(),
        cfg_a.label(),
        cfg_b.label()
    );
    println!();
    println!("| query | rank A | rank B | delta rr | delta ndcg |");
    println!("|-------|-------:|-------:|---------:|-----------:|");
    for (a, b) in run_a.queries.iter().zip(run_b.queries.iter()) {
        println!(
            "| {} | {} | {} | {:+.3} | {:+.3} |",
            a.name,
            rank_s(a.first_rank),
            rank_s(b.first_rank),
            b.rr - a.rr,
            b.ndcg - a.ndcg
        );
    }
    println!();
    println!("delta MRR={:+.3}  delta Recall@k={:+.3}  delta nDCG@k={:+.3}  delta Recall@1={:+.3}  delta Recall@5={:+.3}  delta Recall@20={:+.3}",
        run_b.aggregate.mrr - run_a.aggregate.mrr, run_b.aggregate.recall_at_k - run_a.aggregate.recall_at_k, run_b.aggregate.ndcg - run_a.aggregate.ndcg, run_b.aggregate.recall_at[0].1 - run_a.aggregate.recall_at[0].1, run_b.aggregate.recall_at[1].1 - run_a.aggregate.recall_at[1].1, run_b.aggregate.recall_at[2].1 - run_a.aggregate.recall_at[2].1);
    print_graph_precision(&run_a.graph, Some("A"));
    print_graph_precision(&run_b.graph, Some("B"));
    Ok(())
}

fn print_graph_precision(report: &GraphPrecisionReport, label: Option<&str>) {
    if report.labeled_queries == 0 {
        return;
    }
    println!();
    println!(
        "graph-edge precision{}: {} labeled queries, {} gold edges",
        label.map_or_else(String::new, |label| format!(" ({label})")),
        report.labeled_queries,
        report.gold_edges
    );
    println!("| resolution | correct/predicted | precision |");
    println!("|------------|------------------:|----------:|");
    for tier_name in RESOLUTION_TIERS {
        let tier = &report.by_resolution[tier_name];
        let precision = tier
            .precision
            .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.3}"));
        println!(
            "| {tier_name} | {}/{} | {precision} |",
            tier.correct, tier.predicted
        );
    }
}
