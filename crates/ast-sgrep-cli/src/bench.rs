//! Bench / suite / batch timing commands.

use crate::machine::{print_machine_json, print_machine_json_status};
use crate::search_cmd::do_search;
use crate::{open_indexer, open_searcher, resolve_root_index, Cli};
use anyhow::Context;
use ast_sgrep_core::{try_index_db_path, IndexStats, SearchResponse, Searcher};
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
    let db = try_index_db_path(&resolved, index_path.as_deref())?;
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
fn ast_grep_comparison(
    query: &str,
    root: &Path,
    iterations: u32,
    avg_ms: f64,
) -> serde_json::Value {
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

fn bench_history_enabled() -> bool {
    std::env::var("ASGREP_BENCH_HISTORY")
        .ok()
        .map(|v| v != "0")
        .unwrap_or(true)
}

fn update_bench_history(
    label: &str,
    avg_ms: f64,
    cv: f64,
    geomean_ms: Option<f64>,
) -> anyhow::Result<Option<serde_json::Value>> {
    if !bench_history_enabled() {
        return Ok(None);
    }
    let thresholds = crate::keep_gate::KeepThresholds::from_repo();
    let prior = crate::keep_gate::load_committed_prior(label);
    let sample = crate::keep_gate::KeepSample {
        avg_ms,
        cv_pct: cv,
        geomean_ms,
    };
    let verdict = crate::keep_gate::evaluate_keep(sample, prior, thresholds);
    let (host, git_sha, profile) = crate::keep_gate::attribution();
    let path = std::env::var_os("ASGREP_BENCH_HISTORY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(BENCH_HISTORY_PATH));
    let mut root = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or_else(|_| serde_json::json!({"schema_version": "1", "entries": {}}))
    } else {
        serde_json::json!({"schema_version": "1", "entries": {}})
    };
    let entry = serde_json::json!({
        "avg_search_ms": avg_ms,
        "geomean_search_ms": geomean_ms,
        "cv_pct": cv,
        "host": host,
        "git_sha": git_sha,
        "profile": profile,
        "verdict": verdict.as_str(),
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

    let run_path = crate::keep_gate::run_snapshot_path(label);
    if let Some(parent) = run_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut run_doc = entry.clone();
    run_doc["schema_version"] = serde_json::json!("1");
    run_doc["label"] = serde_json::json!(label);
    run_doc["placeholder"] = serde_json::json!(false);
    run_doc["keep_eligible"] = serde_json::json!(matches!(
        verdict,
        crate::keep_gate::KeepVerdict::Keep { .. }
    ));
    std::fs::write(&run_path, serde_json::to_string_pretty(&run_doc)?)?;

    if crate::keep_gate::history_commit_enabled()
        && matches!(
            verdict,
            crate::keep_gate::KeepVerdict::Keep { .. }
                | crate::keep_gate::KeepVerdict::EstablishBaseline
        )
        && cv <= thresholds.cv_ineligible_pct
    {
        let latest = crate::keep_gate::committed_latest_path(label);
        let mut committed = run_doc.clone();
        committed["placeholder"] = serde_json::json!(false);
        std::fs::write(&latest, serde_json::to_string_pretty(&committed)?)?;
    }

    let mut meta = serde_json::json!({
        "path": path.display().to_string(),
        "committed_prior": crate::keep_gate::committed_latest_path(label).display().to_string(),
        "run_path": run_path.display().to_string(),
        "label": label,
        "avg_search_ms": avg_ms,
        "geomean_search_ms": geomean_ms,
        "cv_pct": cv,
        "host": host,
        "git_sha": git_sha,
        "profile": profile,
        "primary_regression_pct": thresholds.primary_regression_pct,
        "geomean_regression_pct": thresholds.geomean_regression_pct,
        "cv_ineligible_pct": thresholds.cv_ineligible_pct,
        "verdict": verdict.as_str(),
        "hotpath_required_for_win_keep": true,
        "competitor_latency_is_not_keep": true,
        "ratchet_ok": !verdict.is_hard_fail(),
    });
    match &verdict {
        crate::keep_gate::KeepVerdict::Keep { regression_pct } => {
            meta["regression_pct"] = serde_json::json!(regression_pct);
        }
        crate::keep_gate::KeepVerdict::EstablishBaseline => {
            meta["keep_win"] = serde_json::json!(false);
        }
        crate::keep_gate::KeepVerdict::RejectRegression {
            regression_pct,
            threshold,
            kind,
        } => {
            meta["regression_pct"] = serde_json::json!(regression_pct);
            meta["threshold"] = serde_json::json!(threshold);
            meta["kind"] = serde_json::json!(kind);
        }
        crate::keep_gate::KeepVerdict::QuarantineCv { cv_pct } => {
            meta["cv_pct"] = serde_json::json!(cv_pct);
            meta["quarantine"] = serde_json::json!("cv_pct");
        }
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

/// Shared collapse: suite, single-query, and batch enforce the same keep gate.
fn enforce_bench_ratchet(history: &Option<serde_json::Value>, subject: &str) -> anyhow::Result<()> {
    let Some(h) = history.as_ref() else {
        return Ok(());
    };
    if crate::keep_gate::bench_ratchet_enabled() && h["ratchet_ok"] == false {
        anyhow::bail!(
            "keep-gate failed for {subject}: verdict={} regression_pct={:?} cv_pct={:?} (host={:?} sha={:?} profile={:?})",
            h["verdict"],
            h.get("regression_pct"),
            h.get("cv_pct"),
            h.get("host"),
            h.get("git_sha"),
            h.get("profile"),
        );
    }
    Ok(())
}

/// Shared human-format ast-grep comparison (suite indent vs single-query wording).
fn print_ast_grep_human(comparison: &serde_json::Value, suite_style: bool, ag_iters: u32) {
    if comparison["compared"] != true {
        if !suite_style {
            if let Some(reason) = comparison["skipped_reason"].as_str() {
                println!("ast-grep comparison skipped: {reason}");
            }
        }
        return;
    }
    let Some(p) = comparison["ast_grep_pattern"].as_str() else {
        return;
    };
    let Some(ms) = comparison["avg_ast_grep_ms"].as_f64() else {
        return;
    };
    if suite_style {
        println!("    ast-grep ({p}): {ms:.2}ms");
        if let Some(sp) = comparison["speedup_vs_ast_grep"].as_f64() {
            println!("    speedup vs ast-grep: {sp:.1}x");
        }
    } else {
        println!("Avg ast-grep (pattern: {p}): {ms:.2}ms over {ag_iters} iterations");
        if let Some(sp) = comparison["speedup_vs_ast_grep"].as_f64() {
            println!("Speedup vs ast-grep: {sp:.1}x");
        }
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
        let expected =
            ast_sgrep_core::bench_suite::benchmark_expectation(case).ok_or_else(|| {
                anyhow::anyhow!("benchmark case '{}' has no identity contract", case.name)
            })?;
        let semantic_only = expected.kind == Some(ast_sgrep_core::HitKind::Embed);
        let (times, last) = timed_searches(&searcher, case.query, semantic_only, iterations)?;
        let hits = last.as_ref().map_or(0, |r| r.hits.len());
        let identity_ok = last.as_ref().is_some_and(|response| {
            response
                .hits
                .iter()
                .take(expected.max_rank)
                .any(|hit| expected.matches(hit))
        });
        let avg = mean_ms(&times);
        let cv = cv_pct(&times);
        let comparison = ast_grep_comparison(case.query, &bench_root, iterations.min(3), avg);
        let case_ok = hits >= case.min_hits && identity_ok;
        results.push(serde_json::json!({
            "name": case.name,
            "query": case.query,
            "avg_search_ms": avg,
            "cv_pct": cv,
            "hits": hits,
            "min_hits": case.min_hits,
            "identity_ok": identity_ok,
            "identity_max_rank": expected.max_rank,
            "ok": case_ok,
            "ast_grep_comparison": comparison,
        }));
    }
    let suite_ok = results.iter().all(|r| r["ok"] == true);
    let case_avgs: Vec<f64> = results
        .iter()
        .filter_map(|r| r["avg_search_ms"].as_f64())
        .collect();
    let suite_avg = mean_ms(&case_avgs);
    let suite_cv = mean_ms(
        &results
            .iter()
            .filter_map(|r| r["cv_pct"].as_f64())
            .collect::<Vec<_>>(),
    );
    let suite_geomean = crate::keep_gate::geomean_ms(&case_avgs);
    let history = update_bench_history(
        &format!("suite:{fixture_name}:{selected}"),
        suite_avg,
        suite_cv,
        suite_geomean,
    )?;
    enforce_bench_ratchet(&history, &format!("suite {selected}"))?;
    if cli.json {
        let mut obj = serde_json::json!({
            "fixture": fixture_name,
            "suite": selected,
            "iterations": iterations,
            "cases": results,
            "suite_ok": suite_ok,
            "avg_search_ms": suite_avg,
            "geomean_search_ms": suite_geomean,
            "cv_pct": suite_cv,
            "bench_history": history,
        });
        // Collapse skip-path fields into existing helper; keep indexed-path without index_ms.
        if let Some(s) = &stats {
            obj["files_indexed"] = serde_json::json!(s.files_indexed);
        } else {
            add_index_json(&mut obj, None, 0.0);
        }
        print_machine_json_status("bench", &obj, suite_ok, if suite_ok { 0 } else { 2 })?;
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
            print_ast_grep_human(&row["ast_grep_comparison"], true, 0);
        }
        if !suite_ok {
            anyhow::bail!("benchmark suite failed hit-count or result-identity thresholds");
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
    let (times, last) = timed_searches(
        &searcher,
        query,
        cli.active_tuning().semantic_only,
        iterations,
    )?;
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
    let history = update_bench_history(&format!("query:{query}"), avg, cv, None)?;
    enforce_bench_ratchet(&history, &format!("query {query:?}"))?;
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
        print_ast_grep_human(&comparison, false, ag_iters);
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
        let (mut samples, last) = timed_searches(
            &searcher,
            query,
            cli.active_tuning().semantic_only,
            iterations,
        )?;
        let cv = cv_pct(&samples);
        samples.sort_by(f64::total_cmp);
        let avg = mean_ms(&samples);
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
                (
                    r.hits.len(),
                    hs.iter()
                        .map(|h| {
                            serde_json::json!({"file": h.file, "line_start": h.line_start, "symbol": h.symbol})
                        })
                        .collect::<Vec<_>>(),
                )
            }
            None => (0, vec![]),
        };
        results.push(serde_json::json!({
            "query": query,
            "avg_search_ms": avg,
            "p50_search_ms": p50,
            "cv_pct": cv,
            "hits": hits,
            "top_10": top_10
        }));
    }
    let batch_avgs: Vec<f64> = results
        .iter()
        .filter_map(|r| r["avg_search_ms"].as_f64())
        .collect();
    let batch_avg = mean_ms(&batch_avgs);
    let batch_cv = mean_ms(
        &results
            .iter()
            .filter_map(|r| r["cv_pct"].as_f64())
            .collect::<Vec<_>>(),
    );
    let batch_geomean = crate::keep_gate::geomean_ms(&batch_avgs);
    let batch_label = format!(
        "batch:{}",
        queries_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("queries")
    );
    let history = update_bench_history(&batch_label, batch_avg, batch_cv, batch_geomean)?;
    enforce_bench_ratchet(&history, &format!("batch {}", queries_path.display()))?;
    if cli.json {
        let mut obj = serde_json::json!({
            "iterations": iterations,
            "queries": results,
            "avg_search_ms": batch_avg,
            "geomean_search_ms": batch_geomean,
            "cv_pct": batch_cv,
            "bench_history": history,
        });
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
                "  {}: avg={:.2}ms p50={:.2}ms cv={:.1}% hits={}",
                r["query"].as_str().unwrap_or("?"),
                r["avg_search_ms"].as_f64().unwrap_or(0.0),
                r["p50_search_ms"].as_f64().unwrap_or(0.0),
                r["cv_pct"].as_f64().unwrap_or(0.0),
                r["hits"].as_u64().unwrap_or(0)
            );
        }
    }
    Ok(())
}
