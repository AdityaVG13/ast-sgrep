//! Pass 4 (numerical N4): end-to-end numeric drills for codemode.
//!
//! Owns ONLY full session flows with exact numeric assertions: mixed-tool
//! budget exhaustion with per-step call counts, mid-plan budget trips with
//! exact trip/success counts, threshold sweeps with exact survivor vectors,
//! batch-size sweeps with exact counts, batch threshold sweeps with exact
//! per-id counts, filter->select top-k chains with exact orderings, and
//! find->filter->read->select pipelines with exact counts. Assertions pin
//! exact values/counts/orderings — never message text.
//!
//! Already covered elsewhere (do NOT re-assert): single-call absolute oracles
//! and clamps (numerical_pass1); degenerate-input totality (numerical_pass2);
//! cross-run relations without absolutes — monotonicity, nesting, order
//! independence, rerun determinism (numerical_pass3); BudgetExhausted
//! stickiness and budget-2 trip shapes (oracle_foundry_pass2/4,
//! error_api_pass1/2/3/4); batch envelope/identity validation and all_ok:false
//! error shapes (error_api_pass2/3/4, oracle_foundry_pass3); find ordering
//! absolutes (session_plan).
//!
//! Pure transforms and tempdir fixtures only — no index I/O except where
//! find/read pipeline drills require an indexed root (lexical only,
//! deterministic).

use ast_sgrep_codemode::{
    BatchCall, BatchRequest, CallError, CodeModeSession, SessionConfig, parse_plan, run_batch,
    run_plan,
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

/// Six hits, strictly descending scores 6..1; input order is score order.
fn scored_hits() -> Value {
    json!([
        {"kind": "def", "file": "f6.rs", "score": 6.0},
        {"kind": "def", "file": "f5.rs", "score": 5.0},
        {"kind": "def", "file": "f4.rs", "score": 4.0},
        {"kind": "def", "file": "f3.rs", "score": 3.0},
        {"kind": "def", "file": "f2.rs", "score": 2.0},
        {"kind": "def", "file": "f1.rs", "score": 1.0},
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

fn catalog_call(id: &str) -> BatchCall {
    BatchCall {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    }
}

#[test]
fn budget_mixed_tool_flow_trips_at_exact_count() {
    // Four DISTINCT pure tools each consume exactly one call: the counter
    // reads 1, 2, 3, 4 after each step. The 5th call trips with payload 4
    // (the budget), the counter stays at 4, and the session is exhausted.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 4;
    session
        .call("catalog_search", json!({"query": "search"}))
        .expect("step 1 runs");
    assert_eq!(session.call_count(), 1);
    assert!(!session.exhausted());
    session
        .call("filter_hits", json!({"hits": scored_hits()}))
        .expect("step 2 runs");
    assert_eq!(session.call_count(), 2);
    session
        .call(
            "select",
            json!({"value": [{"a": 1}], "fields": ["a"]}),
        )
        .expect("step 3 runs");
    assert_eq!(session.call_count(), 3);
    session
        .call("catalog_describe", json!({"name": "search"}))
        .expect("step 4 runs");
    assert_eq!(session.call_count(), 4);
    assert!(session.exhausted());
    let err = session
        .call("filter_hits", json!({"hits": scored_hits()}))
        .expect_err("5th call must trip");
    assert!(matches!(err, CallError::BudgetExhausted(4)), "got {err:?}");
    assert_eq!(session.call_count(), 4);
    assert!(session.exhausted());
}

#[test]
fn plan_budget_exact_trip_and_success_counts() {
    // Seven-step pure plan, all literal args. Under budget 4 the 5th step
    // trips (payload 4, counter pinned at 4). Under budget 7 the same plan
    // succeeds with call_count 7, seven step outputs, and an exact return.
    let plan_value = json!({
        "steps": [
            {"id": "f1", "tool": "filter_hits", "args": {"hits": scored_hits()}},
            {"id": "f2", "tool": "filter_hits", "args": {"hits": scored_hits(), "min_score": 3.0}},
            {"id": "s1", "tool": "select", "args": {"value": [{"a": 1}, {"a": 2}], "fields": ["a"]}},
            {"id": "c1", "tool": "catalog_search", "args": {"query": "filter"}},
            {"id": "f3", "tool": "filter_hits", "args": {"hits": scored_hits(), "limit": 2}},
            {"id": "s2", "tool": "select", "args": {"value": [{"a": 1}, {"a": 2}], "fields": ["a"], "limit": 1}},
            {"id": "s3", "tool": "select", "args": {"value": [{"a": 7}, {"a": 8}], "fields": ["a"]}},
        ],
        "return": "$s3",
    });
    let plan = parse_plan(&plan_value).expect("plan parses");
    assert_eq!(plan.steps.len(), 7);
    let temp = tempfile::tempdir().expect("tempdir");
    let mut tight = session_at(temp.path());
    tight.max_calls = 4;
    let err = run_plan(&mut tight, &plan).expect_err("budget 4 must trip on step 5");
    assert!(matches!(err, CallError::BudgetExhausted(4)), "got {err:?}");
    assert_eq!(tight.call_count(), 4);
    assert!(tight.exhausted());

    let mut roomy = session_at(temp.path());
    roomy.max_calls = 7;
    let result = run_plan(&mut roomy, &plan).expect("budget 7 runs all steps");
    assert!(result.ok);
    assert_eq!(result.call_count, 7);
    assert_eq!(roomy.call_count(), 7);
    assert_eq!(result.steps.len(), 7);
    assert_eq!(result.return_value, json!([{"a": 7}, {"a": 8}]));
    assert_eq!(
        result.steps["f2"]["hit_count"],
        json!(4),
        "min 3.0 keeps scores 6,5,4,3"
    );
    assert_eq!(result.steps["f3"]["hit_count"], json!(2));
    assert_eq!(result.steps["s2"], json!([{"a": 1}]));
}

#[test]
fn threshold_sweep_exact_survivor_counts() {
    // Full sweep over scores 6..1 in ONE session: exact survivor vector
    // [1,2,3,4,5,6,6], exact file order at the mid threshold, and exactly
    // 7 calls consumed. Pass 3 asserted relations; these are absolutes.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let expected = [(6.0, 1), (5.0, 2), (4.0, 3), (3.0, 4), (2.0, 5), (1.0, 6), (0.0, 6)];
    for (min, count) in expected {
        let out = session
            .call(
                "filter_hits",
                json!({"hits": scored_hits(), "min_score": min}),
            )
            .expect("filter runs");
        assert_eq!(out["hit_count"], json!(count), "min {min}");
        assert_eq!(files_of(&out).len(), count);
        assert_eq!(files_of(&out)[0], "f6.rs".to_string(), "min {min}");
    }
    let out = session
        .call("filter_hits", json!({"hits": scored_hits(), "min_score": 3.0}))
        .expect("mid threshold reruns");
    assert_eq!(files_of(&out), vec!["f6.rs", "f5.rs", "f4.rs", "f3.rs"]);
    assert_eq!(session.call_count(), 8);
}

#[test]
fn batch_size_sweep_exact_counts() {
    // Sizes 1..32 (the MAX_BATCH_CALLS ceiling): every wave reports
    // call_count == N, N results, N ok flags, all_ok, serial mode.
    let temp = tempfile::tempdir().expect("tempdir");
    for n in [1usize, 2, 4, 8, 16, 32] {
        let calls: Vec<BatchCall> = (0..n)
            .map(|i| catalog_call(&format!("c{i}")))
            .collect();
        let resp =
            run_batch(config_at(temp.path()), &batch_request(calls)).expect("batch runs");
        assert_eq!(resp.call_count, n, "size {n}");
        assert_eq!(resp.results.len(), n, "size {n}");
        assert!(resp.all_ok, "size {n}");
        assert_eq!(resp.mode, "serial", "size {n}");
        assert_eq!(
            resp.results.iter().filter(|r| r.ok).count(),
            n,
            "size {n}"
        );
    }
}

#[test]
fn batch_threshold_sweep_exact_per_id_counts() {
    // One 5-call batch fans a threshold sweep across per-call thresholds:
    // exact per-id survivor map {t5:2, t4:3, t3:4, t2:5, t1:6} — scores
    // 6..1, inclusive reject rule, input order preserved throughout.
    let temp = tempfile::tempdir().expect("tempdir");
    let thresholds = [("t5", 5.0), ("t4", 4.0), ("t3", 3.0), ("t2", 2.0), ("t1", 1.0)];
    let calls: Vec<BatchCall> = thresholds
        .iter()
        .map(|(id, min)| BatchCall {
            id: id.to_string(),
            tool: "filter_hits".to_string(),
            args: json!({"hits": scored_hits(), "min_score": min}),
        })
        .collect();
    let resp = run_batch(config_at(temp.path()), &batch_request(calls)).expect("batch runs");
    assert!(resp.all_ok);
    assert_eq!(resp.call_count, 5);
    assert_eq!(resp.results.len(), 5);
    assert_eq!(resp.mode, "serial");
    let counts: std::collections::BTreeMap<&str, u64> = resp
        .results
        .iter()
        .map(|r| {
            assert!(r.ok, "row {} must succeed", r.id);
            let n = r.value.as_ref().expect("value")["hit_count"]
                .as_u64()
                .expect("count");
            (r.id.as_str(), n)
        })
        .collect();
    assert_eq!(
        counts,
        std::collections::BTreeMap::from([("t1", 6), ("t2", 5), ("t3", 4), ("t4", 3), ("t5", 2)])
    );
}

#[test]
fn filter_then_select_topk_exact_orderings() {
    // Chained top-k: filter min 2.0 + limit 4 yields exactly
    // [f6,f5,f4,f3]; projecting [file,score] with limit 2 yields exactly
    // the first two rows; limit 3 yields exactly those two plus f4.
    // Three calls total on the session.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let filtered = session
        .call(
            "filter_hits",
            json!({"hits": scored_hits(), "min_score": 2.0, "limit": 4}),
        )
        .expect("filter runs");
    assert_eq!(filtered["hit_count"], json!(4));
    assert_eq!(files_of(&filtered), vec!["f6.rs", "f5.rs", "f4.rs", "f3.rs"]);
    let top2 = session
        .call(
            "select",
            json!({"value": filtered["hits"], "fields": ["file", "score"], "limit": 2}),
        )
        .expect("select 2 runs");
    assert_eq!(
        top2,
        json!([
            {"file": "f6.rs", "score": 6.0},
            {"file": "f5.rs", "score": 5.0},
        ])
    );
    let top3 = session
        .call(
            "select",
            json!({"value": filtered["hits"], "fields": ["file", "score"], "limit": 3}),
        )
        .expect("select 3 runs");
    assert_eq!(
        top3,
        json!([
            {"file": "f6.rs", "score": 6.0},
            {"file": "f5.rs", "score": 5.0},
            {"file": "f4.rs", "score": 4.0},
        ])
    );
    assert_eq!(session.call_count(), 3);
}

#[test]
fn find_filter_read_select_pipeline_exact_counts() {
    // Full indexed pipeline with per-step call counts: index(1) -> find
    // limit 10 over 4 single-match files yields exactly 4 hits(2) ->
    // path_contains narrows to exactly 1 hit(3) -> read of that file's
    // first two lines returns the exact text(4) -> select projects exactly
    // 3 file rows(5).
    let temp = tempfile::tempdir().expect("tempdir");
    for stem in ["a", "b", "c", "d"] {
        std::fs::write(
            temp.path().join(format!("n4pipe_{stem}.rs")),
            format!("// n4pipeline marker\npub fn {stem}_fn() {{}}\n"),
        )
        .expect("write");
    }
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    assert_eq!(session.call_count(), 1);
    let found = session
        .call("find", json!({"query": "n4pipeline", "limit": 10}))
        .expect("find runs");
    assert_eq!(session.call_count(), 2);
    let hits = found["hits"].as_array().expect("hits").clone();
    assert_eq!(hits.len(), 4);
    let narrowed = session
        .call(
            "filter_hits",
            json!({"hits": hits, "path_contains": "n4pipe_c"}),
        )
        .expect("filter runs");
    assert_eq!(session.call_count(), 3);
    assert_eq!(narrowed["hit_count"], json!(1));
    let file = narrowed["hits"][0]["file"].as_str().expect("file");
    assert!(file.ends_with("n4pipe_c.rs"), "got {file}");
    let read = session
        .call("read", json!({"path": "n4pipe_c.rs", "start": 1, "end": 2}))
        .expect("read runs");
    assert_eq!(session.call_count(), 4);
    assert_eq!(read["count"], json!(1));
    assert_eq!(read["windows"][0]["start"], json!(1));
    assert_eq!(read["windows"][0]["end"], json!(2));
    assert_eq!(
        read["windows"][0]["text"],
        json!("// n4pipeline marker\npub fn c_fn() {}")
    );
    let projected = session
        .call(
            "select",
            json!({"value": found["hits"], "fields": ["file"], "limit": 3}),
        )
        .expect("select runs");
    assert_eq!(session.call_count(), 5);
    assert_eq!(projected.as_array().expect("rows").len(), 3);
}

#[test]
fn read_fanout_exact_window_counts() {
    // One read fans out to 3 refs over a 6-line file: count==3, three
    // windows, exact (start,end,text) per window in ref order. Index(1) +
    // read(2) consume exactly 2 calls.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("fanout.txt"), "r1\nr2\nr3\nr4\nr5\nr6").expect("write");
    let (_index_dir, mut session) = indexed_session_at(temp.path());
    assert_eq!(session.call_count(), 1);
    let out = session
        .call(
            "read",
            json!({"refs": [
                {"path": "fanout.txt", "start": 1, "end": 2},
                {"path": "fanout.txt", "start": 5, "end": 6},
                {"path": "fanout.txt", "start": 3, "end": 3},
            ]}),
        )
        .expect("fanout read runs");
    assert_eq!(session.call_count(), 2);
    assert_eq!(out["count"], json!(3));
    let windows = out["windows"].as_array().expect("windows");
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[0]["start"], json!(1));
    assert_eq!(windows[0]["end"], json!(2));
    assert_eq!(windows[0]["text"], json!("r1\nr2"));
    assert_eq!(windows[1]["start"], json!(5));
    assert_eq!(windows[1]["end"], json!(6));
    assert_eq!(windows[1]["text"], json!("r5\nr6"));
    assert_eq!(windows[2]["start"], json!(3));
    assert_eq!(windows[2]["end"], json!(3));
    assert_eq!(windows[2]["text"], json!("r3"));
}
