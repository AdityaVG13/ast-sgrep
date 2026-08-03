//! Benchmark command runners.

use crate::search_cmd::do_search;
use crate::{
    open_indexer, open_searcher, print_machine_json, resolve_root_index, Cli, MAX_QUERIES_FILE_LINES,
};
use anyhow::Context;
use ast_sgrep_core::{index_db_path, IndexStats, SearchResponse, Searcher};
use std::path::Path;
use std::time::Instant;

pub(crate) fn run_bench_command(
    root: &Path,
    cli: &Cli,
    query: &str,
    iterations: u32,
    suite: Option<&str>,
    fixture: &str,
    queries_file: Option<&Path>,
    skip_index: bool,
) -> anyhow::Result<()> {
    if let Some(path) = queries_file {
        return run_bench_batch(root, cli, path, iterations, skip_index);
    }
    match suite {
        Some(name) => run_bench_suite(root, cli, name, fixture, iterations, skip_index),
        None => run_bench(root, cli, query, iterations, skip_index),
    }
}
fn maybe_index(root: &Path, cli: &Cli, skip: bool) -> anyhow::Result<(Option<IndexStats>, f64)> {
    if skip {
        return Ok((None, 0.0));
    }
    let mut indexer = open_indexer(root, cli)?;
    let t0 = Instant::now();
    Ok((
        Some(indexer.index_all()?),
        t0.elapsed().as_secs_f64() * 1000.0,
    ))
}
fn bench_searcher(root: &Path, cli: &Cli, skip_index: bool) -> anyhow::Result<Searcher> {
    let (resolved, index_path) = resolve_root_index(cli, root);
    let db = index_db_path(&resolved, index_path.as_deref());
    if skip_index && !db.exists() {
        anyhow::bail!(
            "failed to open existing index at {} (run `asgrep index` first)",
            db.display()
        );
    }
    open_searcher(root, cli)
}
fn timed_searches(
    s: &Searcher,
    q: &str,
    semantic: bool,
    iterations: u32,
) -> anyhow::Result<(Vec<f64>, Option<SearchResponse>)> {
    let mut times = Vec::with_capacity(iterations as usize);
    let mut last = None;
    for _ in 0..iterations {
        let t0 = Instant::now();
        last = Some(do_search(s, q, semantic)?);
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    Ok((times, last))
}
fn add_index_json(obj: &mut serde_json::Value, stats: Option<&IndexStats>, index_ms: f64) {
    if let Some(s) = stats {
        obj["files_indexed"] = serde_json::json!(s.files_indexed);
        obj["index_ms"] = serde_json::json!(index_ms);
    } else {
        obj["index_skipped"] = serde_json::json!(true);
        obj["index_ms"] = serde_json::json!(0.0);
        obj["files_indexed"] = serde_json::Value::Null;
    }
}
fn print_index_skipped(stats: Option<&IndexStats>, index_ms: Option<f64>) {
    match (stats, index_ms) {
        (Some(s), Some(ms)) => println!("Indexed {} files in {ms:.2}ms", s.files_indexed),
        (Some(s), None) => println!("Indexed {} files", s.files_indexed),
        _ => println!("Index skipped (using existing index)"),
    }
}
fn run_bench_suite(
    root: &Path,
    cli: &Cli,
    suite_name: &str,
    fixture_name: &str,
    iterations: u32,
    skip_index: bool,
) -> anyhow::Result<()> {
    use ast_sgrep_core::bench_suite;
    let fix = bench_suite::fixture_by_name(fixture_name).with_context(|| {
        format!(
            "unknown fixture {fixture_name:?}; available: {}",
            bench_suite::list_fixture_names().join(", ")
        )
    })?;
    let selected = if suite_name.is_empty() {
        fix.suite
    } else {
        suite_name
    };
    let cases = bench_suite::suite_by_name(selected).with_context(|| {
        format!(
            "unknown suite {suite_name:?}; available: {}",
            bench_suite::list_suite_names().join(", ")
        )
    })?;
    let bench_root = if root.as_os_str() == "." {
        fix.root.to_path_buf()
    } else {
        root.to_path_buf()
    };
    let (stats, _) = maybe_index(&bench_root, cli, skip_index)?;
    let searcher = bench_searcher(&bench_root, cli, skip_index)?;
    let results: Vec<serde_json::Value> = cases.iter().map(|case| {
        let (times, last) = timed_searches(&searcher, case.query, false, iterations)?;
        let hits = last.as_ref().map_or(0, |r| r.hits.len());
        let avg = times.iter().sum::<f64>() / f64::from(iterations.max(1));
        let ag_pat = ast_sgrep_core::pattern::ast_grep_pattern_for_query(case.query);
        let ag_ms = ag_pat.as_ref().and_then(|p| ast_sgrep_core::pattern::bench_ast_grep(p, &bench_root, iterations.min(3)));
        Ok(serde_json::json!({"name": case.name, "query": case.query, "avg_search_ms": avg, "hits": hits, "min_hits": case.min_hits, "ok": hits >= case.min_hits,
            "ast_grep_pattern": ag_pat, "avg_ast_grep_ms": ag_ms, "speedup_vs_ast_grep": ag_ms.map(|ag| ag / avg)}))
    }).collect::<anyhow::Result<_>>()?;
    if cli.json {
        let mut obj = serde_json::json!({"fixture": fixture_name, "suite": suite_name, "iterations": iterations, "cases": results});
        if let Some(s) = &stats {
            obj["files_indexed"] = serde_json::json!(s.files_indexed);
        } else {
            obj["index_skipped"] = serde_json::json!(true);
            obj["index_ms"] = serde_json::json!(0.0);
            obj["files_indexed"] = serde_json::Value::Null;
        }
        print_machine_json("bench", &obj)?;
    } else {
        println!("Benchmark fixture: {fixture_name}, suite: {suite_name}");
        print_index_skipped(stats.as_ref(), None);
        for row in &results {
            let st = if row["ok"].as_bool().unwrap_or(false) {
                "ok"
            } else {
                "FAIL"
            };
            println!(
                "  {}: {:.2}ms avg, {} hits {st}",
                row["name"].as_str().unwrap_or("?"),
                row["avg_search_ms"].as_f64().unwrap_or(0.0),
                row["hits"].as_u64().unwrap_or(0)
            );
            if let (Some(p), Some(ms)) = (
                row["ast_grep_pattern"].as_str(),
                row["avg_ast_grep_ms"].as_f64(),
            ) {
                println!("    ast-grep ({p}): {ms:.2}ms");
                if let Some(sp) = row["speedup_vs_ast_grep"].as_f64() {
                    println!("    speedup vs ast-grep: {sp:.1}x");
                }
            }
        }
    }
    if results.iter().any(|r| r["ok"] == false) {
        anyhow::bail!("benchmark suite had cases below min_hits threshold");
    }
    Ok(())
}
pub(crate) fn run_bench(
    root: &Path,
    cli: &Cli,
    query: &str,
    iterations: u32,
    skip_index: bool,
) -> anyhow::Result<()> {
    let (stats_opt, index_ms) = maybe_index(root, cli, skip_index)?;
    let searcher = bench_searcher(root, cli, skip_index)?;
    let (times, last) = timed_searches(&searcher, query, cli.semantic_only, iterations)?;
    let hits = last.as_ref().map_or(0, |r| r.hits.len());
    let avg = times.iter().sum::<f64>() / f64::from(iterations.max(1));
    let first = times.first().copied().unwrap_or_default();
    let warm = if times.len() > 1 {
        times[1..].iter().sum::<f64>() / (times.len() - 1) as f64
    } else {
        first
    };
    let ag_iters = iterations.min(3);
    let ag_pat = ast_sgrep_core::pattern::ast_grep_pattern_for_query(query);
    let ag_ms = ag_pat
        .as_ref()
        .and_then(|p| ast_sgrep_core::pattern::bench_ast_grep(p, root, ag_iters));
    let speedup = ag_ms.map(|ag| ag / avg);
    if cli.json {
        let mut obj = serde_json::json!({"query": query, "iterations": iterations, "avg_search_ms": avg, "first_search_ms": first, "warm_search_ms": warm, "cold_overhead_ms": first - warm, "hits": hits, "ast_grep_pattern": ag_pat, "ast_grep_iterations": ag_iters, "avg_ast_grep_ms": ag_ms, "speedup_vs_ast_grep": speedup});
        add_index_json(&mut obj, stats_opt.as_ref(), index_ms);
        print_machine_json("bench", &obj)?;
    } else {
        println!("Benchmark (v1.0 targets: search <20ms, 0% false callers)");
        print_index_skipped(stats_opt.as_ref(), Some(index_ms));
        println!("Query: {query}");
        println!("Avg search: {avg:.2}ms over {iterations} iterations ({hits} hits)");
        if let (Some(p), Some(ms)) = (&ag_pat, ag_ms) {
            println!("Avg ast-grep (pattern: {p}): {ms:.2}ms over {ag_iters} iterations");
            if let Some(sp) = speedup {
                println!("Speedup vs ast-grep: {sp:.1}x");
            }
        }
    }
    Ok(())
}
fn run_bench_batch(
    root: &Path,
    cli: &Cli,
    queries_path: &Path,
    iterations: u32,
    skip_index: bool,
) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(queries_path)
        .with_context(|| format!("failed to read queries file {}", queries_path.display()))?;
    let queries: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if queries.is_empty() {
        anyhow::bail!("queries file is empty or contains only blank lines");
    }
    if queries.len() > MAX_QUERIES_FILE_LINES {
        anyhow::bail!(
            "queries file has {} lines; maximum is {MAX_QUERIES_FILE_LINES}",
            queries.len()
        );
    }
    let (stats_opt, index_ms) = maybe_index(root, cli, skip_index)?;
    let searcher = bench_searcher(root, cli, skip_index)?;
    let mut results = Vec::with_capacity(queries.len());
    for query in &queries {
        let (mut samples, last) = timed_searches(&searcher, query, cli.semantic_only, iterations)?;
        samples.sort_by(f64::total_cmp);
        let avg = samples.iter().sum::<f64>() / f64::from(iterations.max(1));
        let p50 = if samples.is_empty() {
            0.0
        } else {
            samples[(samples.len() - 1) / 2]
        };
        let (hits, top_10) = match &last {
            Some(r) => {
                let mut hs: Vec<_> = r.hits.iter().collect();
                hs.sort_by(|a, b| {
                    b.score
                        .total_cmp(&a.score)
                        .then_with(|| a.file.cmp(&b.file))
                        .then_with(|| a.line_start.cmp(&b.line_start))
                });
                hs.truncate(10);
                (r.hits.len(), hs.iter().map(|h| serde_json::json!({"file": h.file, "line_start": h.line_start, "symbol": h.symbol})).collect::<Vec<_>>())
            }
            None => (0, vec![]),
        };
        results.push(serde_json::json!({"query": query, "avg_search_ms": avg, "p50_search_ms": p50, "hits": hits, "top_10": top_10}));
    }
    if cli.json {
        let mut obj = serde_json::json!({"iterations": iterations, "queries": results});
        add_index_json(&mut obj, stats_opt.as_ref(), index_ms);
        print_machine_json("bench", &obj)?;
    } else {
        println!(
            "Batch benchmark: {} queries over {} iterations each",
            queries.len(),
            iterations
        );
        print_index_skipped(stats_opt.as_ref(), Some(index_ms));
        for r in &results {
            println!(
                "  {}: avg={:.2}ms p50={:.2}ms hits={}",
                r["query"].as_str().unwrap_or("?"),
                r["avg_search_ms"].as_f64().unwrap_or(0.0),
                r["p50_search_ms"].as_f64().unwrap_or(0.0),
                r["hits"].as_u64().unwrap_or(0)
            );
        }
    }
    Ok(())
}
