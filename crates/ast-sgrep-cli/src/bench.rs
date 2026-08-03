//! Benchmark suite / batch helpers.

use crate::machine::{print_machine_json, print_machine_json_with_ok};
use crate::search_cmd::do_search;
use crate::{open_indexer, open_searcher, resolve_root_index, Cli};
use anyhow::Context;
use ast_sgrep_core::{index_db_path, IndexStats, SearchResponse, Searcher};
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAX_QUERIES_FILE_LINES: usize = 1000;

#[allow(clippy::too_many_arguments)]
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
pub(crate) fn maybe_index(root: &Path, cli: &Cli, skip: bool) -> anyhow::Result<(Option<IndexStats>, f64)> {
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
fn mean_ms(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        0.0
    } else {
        samples.iter().sum::<f64>() / samples.len() as f64
    }
}
/// Sample coefficient of variation as a percent (0 when fewer than 2 samples).
fn cv_pct(samples: &[f64]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mean = mean_ms(samples);
    if mean == 0.0 {
        return 0.0;
    }
    let var = samples
        .iter()
        .map(|x| {
            let d = x - mean;
            d * d
        })
        .sum::<f64>()
        / (samples.len() - 1) as f64;
    (var.sqrt() / mean) * 100.0
}
/// Optional ast-grep timing only for `pattern:` queries when the binary exists.
/// Hybrid/token comparisons are vacuous and must not emit speedup claims.
fn ast_grep_comparison(query: &str, root: &Path, iterations: u32, avg_ms: f64) -> serde_json::Value {
    let Some(pat) = query
        .strip_prefix("pattern:")
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        return serde_json::json!({
            "compared": false,
            "skipped_reason": "ast-grep timing only runs for pattern: queries; hybrid/token speedup_vs_ast_grep claims are vacuous"
        });
    };
    match ast_sgrep_core::pattern::bench_ast_grep(pat, root, iterations.max(1)) {
        Some(ms) if avg_ms > 0.0 => serde_json::json!({
            "compared": true,
            "ast_grep_pattern": pat,
            "avg_ast_grep_ms": ms,
            "speedup_vs_ast_grep": ms / avg_ms
        }),
        Some(ms) => serde_json::json!({
            "compared": true,
            "ast_grep_pattern": pat,
            "avg_ast_grep_ms": ms,
            "speedup_vs_ast_grep": serde_json::Value::Null
        }),
        None => serde_json::json!({
            "compared": false,
            "ast_grep_pattern": pat,
            "skipped_reason": "ast-grep binary not available"
        }),
    }
}
const BENCH_HISTORY_PATH: &str = ".bench-history.json";
/// Default regression ratchet: fail when current mean exceeds prior mean by this percent.
const BENCH_RATCHET_PCT: f64 = 50.0;
fn bench_history_enabled() -> bool {
    std::env::var("ASGREP_BENCH_HISTORY")
        .ok()
        .map(|v| v != "0")
        .unwrap_or(true)
}
fn bench_ratchet_enabled() -> bool {
    std::env::var("ASGREP_BENCH_RATCHET").ok().as_deref() == Some("1")
}
fn update_bench_history(
    label: &str,
    avg_ms: f64,
    cv: f64,
) -> anyhow::Result<Option<serde_json::Value>> {
    if !bench_history_enabled() {
        return Ok(None);
    }
    let path = std::env::var_os("ASGREP_BENCH_HISTORY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(BENCH_HISTORY_PATH));
    let mut root = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or_else(|_| serde_json::json!({"schema_version": "1", "entries": {}}))
    } else {
        serde_json::json!({"schema_version": "1", "entries": {}})
    };
    let prior_avg = root
        .pointer(&format!("/entries/{label}/avg_search_ms"))
        .and_then(|v| v.as_f64());
    let entry = serde_json::json!({
        "avg_search_ms": avg_ms,
        "cv_pct": cv,
        "updated_unix_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    });
    root.as_object_mut()
        .context("bench history root")?
        .entry("entries")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .context("bench history entries")?
        .insert(label.to_string(), entry.clone());
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    std::fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    let mut meta = serde_json::json!({
        "path": path.display().to_string(),
        "label": label,
        "avg_search_ms": avg_ms,
        "cv_pct": cv,
        "prior_avg_search_ms": prior_avg,
        "ratchet_pct": BENCH_RATCHET_PCT,
    });
    if let Some(prior) = prior_avg {
        let regression_pct = if prior > 0.0 {
            ((avg_ms - prior) / prior) * 100.0
        } else {
            0.0
        };
        meta["regression_pct"] = serde_json::json!(regression_pct);
        meta["ratchet_ok"] = serde_json::json!(regression_pct <= BENCH_RATCHET_PCT);
    } else {
        meta["ratchet_ok"] = serde_json::json!(true);
    }
    Ok(Some(meta))
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
    let mut results: Vec<serde_json::Value> = Vec::with_capacity(cases.len());
    for case in cases {
        let (times, last) = timed_searches(&searcher, case.query, false, iterations)?;
        let hits = last.as_ref().map_or(0, |r| r.hits.len());
        let avg = mean_ms(&times);
        let cv = cv_pct(&times);
        let comparison = ast_grep_comparison(case.query, &bench_root, iterations.min(3), avg);
        results.push(serde_json::json!({
            "name": case.name,
            "query": case.query,
            "avg_search_ms": avg,
            "cv_pct": cv,
            "hits": hits,
            "min_hits": case.min_hits,
            "ok": hits >= case.min_hits,
            "ast_grep_comparison": comparison,
        }));
    }
    let suite_ok = results.iter().all(|r| r["ok"] == true);
    let suite_avg = mean_ms(
        &results
            .iter()
            .filter_map(|r| r["avg_search_ms"].as_f64())
            .collect::<Vec<_>>(),
    );
    let suite_cv = mean_ms(
        &results
            .iter()
            .filter_map(|r| r["cv_pct"].as_f64())
            .collect::<Vec<_>>(),
    );
    let history = update_bench_history(
        &format!("suite:{fixture_name}:{selected}"),
        suite_avg,
        suite_cv,
    )?;
    if let Some(ref h) = history {
        if bench_ratchet_enabled() && h["ratchet_ok"] == false {
            anyhow::bail!(
                "bench ratchet failed for suite {selected}: regression_pct={:?} exceeds {}%",
                h.get("regression_pct"),
                BENCH_RATCHET_PCT
            );
        }
    }
    if cli.json {
        let mut obj = serde_json::json!({
            "fixture": fixture_name,
            "suite": selected,
            "iterations": iterations,
            "cases": results,
            "suite_ok": suite_ok,
            "avg_search_ms": suite_avg,
            "cv_pct": suite_cv,
            "bench_history": history,
        });
        if let Some(s) = &stats {
            obj["files_indexed"] = serde_json::json!(s.files_indexed);
        } else {
            obj["index_skipped"] = serde_json::json!(true);
            obj["index_ms"] = serde_json::json!(0.0);
            obj["files_indexed"] = serde_json::Value::Null;
        }
        // Single envelope: ok reflects suite outcome (no success-then-failure dual JSON).
        print_machine_json_with_ok("bench", &obj, suite_ok)?;
        if !suite_ok {
            std::process::exit(2);
        }
    } else {
        println!("Benchmark fixture: {fixture_name}, suite: {selected}");
        print_index_skipped(stats.as_ref(), None);
        for row in &results {
            let st = if row["ok"].as_bool().unwrap_or(false) {
                "ok"
            } else {
                "FAIL"
            };
            println!(
                "  {}: {:.2}ms avg (cv {:.1}%), {} hits {st}",
                row["name"].as_str().unwrap_or("?"),
                row["avg_search_ms"].as_f64().unwrap_or(0.0),
                row["cv_pct"].as_f64().unwrap_or(0.0),
                row["hits"].as_u64().unwrap_or(0)
            );
            if row["ast_grep_comparison"]["compared"] == true {
                if let (Some(p), Some(ms)) = (
                    row["ast_grep_comparison"]["ast_grep_pattern"].as_str(),
                    row["ast_grep_comparison"]["avg_ast_grep_ms"].as_f64(),
                ) {
                    println!("    ast-grep ({p}): {ms:.2}ms");
                    if let Some(sp) = row["ast_grep_comparison"]["speedup_vs_ast_grep"].as_f64() {
                        println!("    speedup vs ast-grep: {sp:.1}x");
                    }
                }
            }
        }
        if !suite_ok {
            anyhow::bail!("benchmark suite had cases below min_hits threshold");
        }
    }
    Ok(())
}
fn run_bench(
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
    let avg = mean_ms(&times);
    let cv = cv_pct(&times);
    let first = times.first().copied().unwrap_or_default();
    let warm = if times.len() > 1 {
        mean_ms(&times[1..])
    } else {
        first
    };
    let ag_iters = iterations.min(3);
    let comparison = ast_grep_comparison(query, root, ag_iters, avg);
    let history = update_bench_history(&format!("query:{query}"), avg, cv)?;
    if let Some(ref h) = history {
        if bench_ratchet_enabled() && h["ratchet_ok"] == false {
            anyhow::bail!(
                "bench ratchet failed for query {query:?}: regression_pct={:?} exceeds {}%",
                h.get("regression_pct"),
                BENCH_RATCHET_PCT
            );
        }
    }
    if cli.json {
        let mut obj = serde_json::json!({
            "query": query,
            "iterations": iterations,
            "avg_search_ms": avg,
            "cv_pct": cv,
            "first_search_ms": first,
            "warm_search_ms": warm,
            "cold_overhead_ms": first - warm,
            "hits": hits,
            "ast_grep_comparison": comparison,
            "bench_history": history,
        });
        add_index_json(&mut obj, stats_opt.as_ref(), index_ms);
        print_machine_json("bench", &obj)?;
    } else {
        println!("Benchmark (v1.0 targets: search <20ms, 0% false callers)");
        print_index_skipped(stats_opt.as_ref(), Some(index_ms));
        println!("Query: {query}");
        println!("Avg search: {avg:.2}ms over {iterations} iterations (cv {cv:.1}%, {hits} hits)");
        if comparison["compared"] == true {
            if let (Some(p), Some(ms)) = (
                comparison["ast_grep_pattern"].as_str(),
                comparison["avg_ast_grep_ms"].as_f64(),
            ) {
                println!("Avg ast-grep (pattern: {p}): {ms:.2}ms over {ag_iters} iterations");
                if let Some(sp) = comparison["speedup_vs_ast_grep"].as_f64() {
                    println!("Speedup vs ast-grep: {sp:.1}x");
                }
            }
        } else if let Some(reason) = comparison["skipped_reason"].as_str() {
            println!("ast-grep comparison skipped: {reason}");
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
