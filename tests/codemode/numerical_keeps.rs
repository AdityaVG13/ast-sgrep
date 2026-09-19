//! numerical KEEPs: the 4 standalone drills with no merge home.
//!
//! Each test below is its own surface contract per
//! `tests/catalog/numerical-codemode.md`: the sole suggestion-distance
//! owner, the sole full-plan execution drill, the sole filter→select chain
//! drill, and the sole full find→filter→read→select pipeline. Bodies are
//! verbatim ports of the pass1/pass4 originals.

use ast_sgrep_codemode::{CallError, parse_plan, run_plan};
use ast_sgrep_testkit as testkit;
use ast_sgrep_testkit::{indexed_session_at, scored_hits6};
use serde_json::json;

/// INTENT=typo within len/3 Levenshtein budget suggests (`serach`), far name (`xyzq`) does not.
/// KILLS=distance-threshold (budget-formula).
/// ABSORBS=none (KEEP standalone)
#[test]
fn unknown_tool_suggestion_threshold() {
    // Threshold is (len/3).max(1). "serach" (len 6 -> dist budget 2,
    // Levenshtein 2 from "search" via the a/r swap) suggests; "xyzq"
    // (len 4 -> budget 1, distance >1 from every tool) does not.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = testkit::session_at(temp.path());
    let err = session
        .call("serach", json!({}))
        .expect_err("typo is unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert!(err.to_string().contains("Did you mean search"), "{err}");
    let err = session.call("xyzq", json!({})).expect_err("far is unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert!(!err.to_string().contains("Did you mean"), "{err}");
}

/// INTENT=7-step plan trips on step 5 under budget 4, succeeds with exact outputs under budget 7.
/// KILLS=plan-budget-accounting (step-count/trip-point).
/// ABSORBS=none (KEEP standalone)
#[test]
fn plan_budget_exact_trip_and_success_counts() {
    // Seven-step pure plan, all literal args. Under budget 4 the 5th step
    // trips (payload 4, counter pinned at 4). Under budget 7 the same plan
    // succeeds with call_count 7, seven step outputs, and an exact return.
    let plan_value = json!({
        "steps": [
            {"id": "f1", "tool": "filter_hits", "args": {"hits": scored_hits6()}},
            {"id": "f2", "tool": "filter_hits", "args": {"hits": scored_hits6(), "min_score": 3.0}},
            {"id": "s1", "tool": "select", "args": {"value": [{"a": 1}, {"a": 2}], "fields": ["a"]}},
            {"id": "c1", "tool": "catalog_search", "args": {"query": "filter"}},
            {"id": "f3", "tool": "filter_hits", "args": {"hits": scored_hits6(), "limit": 2}},
            {"id": "s2", "tool": "select", "args": {"value": [{"a": 1}, {"a": 2}], "fields": ["a"], "limit": 1}},
            {"id": "s3", "tool": "select", "args": {"value": [{"a": 7}, {"a": 8}], "fields": ["a"]}},
        ],
        "return": "$s3",
    });
    let plan = parse_plan(&plan_value).expect("plan parses");
    assert_eq!(plan.steps.len(), 7);
    let temp = tempfile::tempdir().expect("tempdir");
    let mut tight = testkit::session_at(temp.path());
    tight.max_calls = 4;
    let err = run_plan(&mut tight, &plan).expect_err("budget 4 must trip on step 5");
    assert!(matches!(err, CallError::BudgetExhausted(4)), "got {err:?}");
    assert_eq!(tight.call_count(), 4);
    assert!(tight.exhausted());

    let mut roomy = testkit::session_at(temp.path());
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

/// INTENT=filter min2.0/limit4 → [f6..f3], select top-2/3 exact rows, 3 calls total.
/// KILLS=chain-ordering (cross-tool-order-break).
/// ABSORBS=none (KEEP standalone)
#[test]
fn filter_then_select_topk_exact_orderings() {
    // Chained top-k: filter min 2.0 + limit 4 yields exactly
    // [f6,f5,f4,f3]; projecting [file,score] with limit 2 yields exactly
    // the first two rows; limit 3 yields exactly those two plus f4.
    // Three calls total on the session.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = testkit::session_at(temp.path());
    let filtered = session
        .call(
            "filter_hits",
            json!({"hits": scored_hits6(), "min_score": 2.0, "limit": 4}),
        )
        .expect("filter runs");
    assert_eq!(filtered["hit_count"], json!(4));
    assert_eq!(
        testkit::hit_files(&filtered),
        vec!["f6.rs", "f5.rs", "f4.rs", "f3.rs"]
    );
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

/// INTENT=index→find(4)→path-filter(1)→read(exact text)→select(3 rows) with per-step counts.
/// KILLS=BEHAVIOR-ONLY.
/// ABSORBS=none (KEEP standalone)
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
