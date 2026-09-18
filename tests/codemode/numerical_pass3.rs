//! Pass 3 (numerical N3): metamorphic relations for codemode numerics.
//!
//! Owns ONLY relations between runs — no absolute oracles (pass 1), no
//! degenerate-input totality (pass 2): budget monotonicity, min_score
//! monotonicity + nesting, limit prefix nesting, batch order independence,
//! and rerun determinism. Assertions compare values/counts across runs via
//! `matches!`/counts — never message text.
//!
//! Already covered elsewhere (do NOT re-assert): absolute counts, clamps,
//! defaults, thresholds (numerical_pass1/2); BudgetExhausted stickiness
//! (error_api_pass2/3); batch/plan call_count absolutes (batch,
//! durable_recovery); find ordering absolutes (session_plan).
//!
//! Pure transforms and tempdir disk reads only — no index I/O except where
//! find-math relations require an indexed root (lexical only, deterministic).

use ast_sgrep_codemode::{
    BatchCall, BatchRequest, CallError, CodeModeSession, SessionConfig, run_batch,
};
use ast_sgrep_plugins::OutputFormat;
use serde_json::{Value, json};

fn session_at(root: &std::path::Path) -> CodeModeSession {
    CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: None,
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    })
}

fn config_at(root: &std::path::Path) -> SessionConfig {
    SessionConfig {
        root: root.to_path_buf(),
        index_path: None,
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    }
}

fn indexed_session_at(root: &std::path::Path) -> (tempfile::TempDir, CodeModeSession) {
    let index_dir = tempfile::tempdir().expect("index dir");
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(index_dir.path().join("index.db")),
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    });
    let indexed = session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");
    assert_eq!(indexed["ok"], json!(true));
    (index_dir, session)
}

/// Five hits with strictly descending scores; input order is score order.
fn scored_hits() -> Value {
    json!([
        {"kind": "def", "file": "src/s5.rs", "score": 5.0},
        {"kind": "def", "file": "src/s4.rs", "score": 4.0},
        {"kind": "def", "file": "src/s3.rs", "score": 3.0},
        {"kind": "def", "file": "src/s2.rs", "score": 2.0},
        {"kind": "def", "file": "src/s1.rs", "score": 1.0},
    ])
}

fn files_of(out: &Value) -> Vec<String> {
    out["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["file"].as_str().expect("file").to_string())
        .collect()
}

fn is_subsequence(needle: &[String], haystack: &[String]) -> bool {
    let mut rest = haystack.iter();
    needle.iter().all(|n| rest.any(|h| h == n))
}

/// Serve identical pure calls until the budget trips; returns calls served.
/// The trip discriminant is always BudgetExhausted — any other error fails.
fn serve_until_exhausted(session: &mut CodeModeSession, attempts: usize) -> usize {
    let mut served = 0;
    for _ in 0..attempts {
        match session.call("filter_hits", json!({"hits": scored_hits()})) {
            Ok(_) => served += 1,
            Err(e) => {
                assert!(
                    matches!(e, CallError::BudgetExhausted(_)),
                    "trip must be budget, got {e:?}"
                );
                break;
            }
        }
    }
    served
}

#[test]
fn budget_larger_never_serves_fewer_calls() {
    // Monotonicity: served(b) is non-decreasing in b. Budgets span small
    // ints; attempts (10) exceed every budget so each session trips.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut prev_served = 0;
    for budget in [1usize, 2, 3, 5, 8] {
        let mut session = session_at(temp.path());
        session.max_calls = budget;
        let served = serve_until_exhausted(&mut session, 10);
        assert!(
            served >= prev_served,
            "budget {budget} served {served} < {prev_served}"
        );
        assert_eq!(session.call_count(), served);
        prev_served = served;
    }
}

#[test]
fn budget_served_never_exceeds_budget() {
    // Served(b) <= b for every b: the cap binds even under saturation.
    // Also pins equality served == b (each pure call consumes exactly one).
    let temp = tempfile::tempdir().expect("tempdir");
    for budget in [1usize, 2, 4, 7] {
        let mut session = session_at(temp.path());
        session.max_calls = budget;
        let served = serve_until_exhausted(&mut session, 20);
        assert!(served <= budget, "served {served} exceeds budget {budget}");
        assert_eq!(served, budget);
        assert!(session.exhausted());
    }
}

#[test]
fn min_score_lower_threshold_never_yields_fewer_hits() {
    // Counts are non-decreasing as the threshold descends through, above,
    // and below every fixture score. One session; budget (64) is ample.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let mut prev_count = usize::MAX;
    let mut first = true;
    for min in [6.0, 5.0, 4.5, 4.0, 3.0, 2.0, 1.0, 0.0, -1.0] {
        let out = session
            .call(
                "filter_hits",
                json!({"hits": scored_hits(), "min_score": min}),
            )
            .expect("filter runs");
        let count = out["hit_count"].as_u64().expect("count") as usize;
        assert_eq!(count, files_of(&out).len());
        if !first {
            assert!(count >= prev_count, "min {min}: {count} < {prev_count}");
        }
        first = false;
        prev_count = count;
    }
}

#[test]
fn min_score_higher_result_nests_inside_lower() {
    // Filter preserves input order, so the higher-threshold file list is a
    // subsequence of the lower-threshold list for every adjacent pair.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let mut prev_files: Vec<String> = Vec::new();
    let mut first = true;
    for min in [5.0, 4.0, 3.0, 2.0, 1.0, 0.0] {
        let out = session
            .call(
                "filter_hits",
                json!({"hits": scored_hits(), "min_score": min}),
            )
            .expect("filter runs");
        let files = files_of(&out);
        if !first {
            assert!(
                is_subsequence(&prev_files, &files),
                "min-descending break: {prev_files:?} not subsequence of {files:?}"
            );
        }
        first = false;
        prev_files = files;
    }
}

#[test]
fn filter_limit_topk_is_prefix_of_topm() {
    // Truncation keeps input order, so limit k<m yields a strict prefix:
    // counts non-decreasing AND out_k == out_m[..k] for every pair.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let mut prev_files: Vec<String> = Vec::new();
    for limit in [1u64, 2, 3, 5, 100] {
        let out = session
            .call(
                "filter_hits",
                json!({"hits": scored_hits(), "limit": limit}),
            )
            .expect("filter runs");
        let files = files_of(&out);
        assert!(files.len() >= prev_files.len());
        assert_eq!(&files[..prev_files.len()], &prev_files[..]);
        prev_files = files;
    }
}

#[test]
fn select_limit_topk_is_prefix_of_unlimited() {
    // select truncates without reordering: limit k rows equal the first k
    // rows of the unlimited projection for k in 1..3.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let value = json!([{"a": 1}, {"a": 2}, {"a": 3}, {"a": 4}]);
    let full = session
        .call("select", json!({"value": value, "fields": ["a"]}))
        .expect("unlimited select runs");
    let full_rows = full.as_array().expect("array").clone();
    assert_eq!(full_rows.len(), 4);
    for k in [1u64, 2, 3] {
        let out = session
            .call(
                "select",
                json!({"value": value, "fields": ["a"], "limit": k}),
            )
            .expect("limited select runs");
        let rows = out.as_array().expect("array").clone();
        assert_eq!(rows.len(), k as usize);
        assert_eq!(rows, full_rows[..k as usize]);
    }
}

#[test]
fn find_limit_topk_is_prefix_of_topm() {
    // Ranking precedes truncation: limit-1 hits equal the first hit of the
    // limit-5 run, and counts are non-decreasing in the limit. Lexical find
    // over 3 files sharing one marker word; deterministic order.
    let temp = tempfile::tempdir().expect("tempdir");
    for (name, func) in [("a.rs", "a_one"), ("b.rs", "b_two"), ("c.rs", "c_three")] {
        std::fs::write(
            temp.path().join(name),
            format!("// n3nest marker\npub fn {func}() {{}}\n"),
        )
        .expect("write");
    }
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let narrow = session
        .call("find", json!({"query": "n3nest", "limit": 1}))
        .expect("limit 1 runs");
    let wide = session
        .call("find", json!({"query": "n3nest", "limit": 5}))
        .expect("limit 5 runs");
    let narrow_hits = narrow["hits"].as_array().expect("hits").clone();
    let wide_hits = wide["hits"].as_array().expect("hits").clone();
    assert!(narrow_hits.len() <= wide_hits.len());
    assert_eq!(narrow_hits, wide_hits[..narrow_hits.len()]);
}

fn batch_request(calls: Vec<BatchCall>) -> BatchRequest {
    BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit: None,
        parallel: None,
        parallel_mode: None,
        calls,
    }
}

fn catalog_call(id: &str, query: &str) -> BatchCall {
    BatchCall {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": query}),
    }
}

#[test]
fn batch_call_order_does_not_change_counts_or_per_id_results() {
    // Same multiset of calls in forward vs reversed order: call_count,
    // all_ok, mode equal; per-id (ok, value) maps equal. Result POSITION
    // may follow input order, so compare by id, never by index.
    let temp = tempfile::tempdir().expect("tempdir");
    let forward = vec![
        catalog_call("a", "search"),
        catalog_call("b", "filter"),
        catalog_call("c", "batch"),
    ];
    let mut reversed = forward.clone();
    reversed.reverse();
    let r1 = run_batch(config_at(temp.path()), &batch_request(forward)).expect("batch runs");
    let r2 =
        run_batch(config_at(temp.path()), &batch_request(reversed)).expect("reversed runs");
    assert_eq!(r1.call_count, r2.call_count);
    assert_eq!(r1.call_count, 3);
    assert_eq!(r1.all_ok, r2.all_ok);
    assert_eq!(r1.mode, r2.mode);
    let by_id = |r: &ast_sgrep_codemode::BatchResponse| {
        r.results
            .iter()
            .map(|res| (res.id.clone(), (res.ok, res.value.clone())))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(by_id(&r1), by_id(&r2));
}

#[test]
fn filter_rerun_is_deterministic() {
    // Same pure call twice on one session: byte-identical JSON values.
    // Scores straddle the threshold to catch nondeterministic comparison.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let args = json!({"hits": scored_hits(), "min_score": 3.0, "limit": 10});
    let first = session.call("filter_hits", args.clone()).expect("first runs");
    let second = session.call("filter_hits", args).expect("second runs");
    assert_eq!(first, second);
}

#[test]
fn find_rerun_is_deterministic() {
    // Same lexical query twice on one indexed session: identical hit arrays.
    let temp = tempfile::tempdir().expect("tempdir");
    for (name, func) in [("a.rs", "a_one"), ("b.rs", "b_two")] {
        std::fs::write(
            temp.path().join(name),
            format!("// n3det marker\npub fn {func}() {{}}\n"),
        )
        .expect("write");
    }
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    let first = session
        .call("find", json!({"query": "n3det", "limit": 5}))
        .expect("first runs");
    let second = session
        .call("find", json!({"query": "n3det", "limit": 5}))
        .expect("second runs");
    assert_eq!(first["hits"], second["hits"]);
    assert_eq!(first, second);
}

#[test]
fn batch_rerun_is_deterministic() {
    // Same batch twice: mode, counts, flags, and per-call payloads equal.
    // wall_ms is excluded — it is timing, not semantics.
    let temp = tempfile::tempdir().expect("tempdir");
    let req = batch_request(vec![catalog_call("a", "search"), catalog_call("b", "filter")]);
    let r1 = run_batch(config_at(temp.path()), &req).expect("first runs");
    let r2 = run_batch(config_at(temp.path()), &req).expect("second runs");
    assert_eq!(r1.mode, r2.mode);
    assert_eq!(r1.call_count, r2.call_count);
    assert_eq!(r1.all_ok, r2.all_ok);
    assert_eq!(r1.results.len(), r2.results.len());
    for (a, b) in r1.results.iter().zip(r2.results.iter()) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.ok, b.ok);
        assert_eq!(a.value, b.value);
        assert_eq!(a.error, b.error);
    }
}
