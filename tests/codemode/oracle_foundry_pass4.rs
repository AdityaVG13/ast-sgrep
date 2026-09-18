//! Pass 4 (oracle-foundry, Mission 4): L4 end-to-end oracles for codemode.
//!
//! Pass 1/2 own single-surface contracts (formats, budgets, detail levels,
//! parse/validate, batch ceilings, catalog search, miss reasons, scrubbers,
//! pinning, aliases, budget boundary, dup/dangling, single_result). Pass 3
//! owns cross-call relations (rerun Values, order-independence, serial/parallel
//! agreement, monotonicity, idempotence, ref matrix, return default, shaping
//! exclusivity, taxonomy split, unicode, precedence, transform agreement).
//!
//! This pass owns only NEW full-flow compositions through public APIs:
//! indexed session -> plan -> execute -> batch -> budget shaping -> golden
//! text; rerun determinism at the golden-text level; mid-plan budget
//! exhaustion with batch isolation; missing/escaping-root fail-closed;
//! oversized-response fail-closed; bump-before-dispatch taxonomy. Pure
//! catalog/transform legs stay index-free; indexed legs build their own
//! fixed-content temp repos (no sample-fixture dependence).
//!
//! All expectations are hand-computed. Failures assert enum discriminants via
//! `matches!`, never Display text. Golden legs assert byte-exact canonical
//! text against inline literals.

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, BatchCall, BatchRequest, CallError, CodeModeSession,
    ParallelMode, SessionConfig, MAX_CALL_RESPONSE_BYTES,
};
use ast_sgrep_core::search::HitSignal;
use ast_sgrep_core::{HitKind, SearchHit};
use ast_sgrep_plugins::budget::{plan_cost, render, select};
use ast_sgrep_plugins::{DetailLevel, OutputBudget, OutputFormat};
use ast_sgrep_testkit::canonicalize_text;
use serde_json::{json, Value};

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

/// Fixed-content temp repo indexed through the public session API. Returns the
/// tempdir (kept alive by the caller) plus a config for fresh sessions, so
/// plan `call_count` assertions start from zero.
fn indexed_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, SessionConfig) {
    let temp = tempfile::tempdir().expect("tempdir");
    for (name, body) in files {
        let path = temp.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&path, body).expect("write");
    }
    let config = SessionConfig {
        root: temp.path().to_path_buf(),
        index_path: Some(temp.path().join("index.db")),
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    };
    let mut indexer = CodeModeSession::new(config.clone());
    indexer
        .call("index_repo", json!({"force": false}))
        .expect("index");
    (temp, config)
}

fn sample_hit(excerpt: &str) -> SearchHit {
    SearchHit {
        kind: HitKind::Def,
        file: "lib.rs".to_string(),
        line_start: 1,
        line_end: 3,
        symbol: Some("alpha".to_string()),
        caller: None,
        callee: None,
        language: Some("rust".to_string()),
        score: 3.0,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Def],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: excerpt.to_string(),
        byte_span: None,
    }
}

#[test]
fn indexed_plan_execute_shape_golden_flow() {
    // Full L4 flow: index -> search -> filter -> select -> golden text. The
    // unique token appears in exactly one file, so the shaped hit_count is
    // hand-computable end to end.
    let (_temp, config) = indexed_repo(&[
        ("src/a.rs", "pub fn needle_unique_xyz() {}\n"),
        ("src/b.rs", "pub fn other_fn() {}\n"),
    ]);
    let plan = parse_plan(&json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": "needle_unique_xyz", "format": "capsule", "limit": 5}},
        {"id": "narrow", "tool": "filter_hits",
         "args": {"hits": "$seed", "path_contains": "src/a.rs", "limit": 5}},
        {"id": "out", "tool": "select",
         "args": {"value": "$narrow", "fields": ["hit_count"]}},
    ], "return": "$out"}))
    .expect("plan parses");
    let mut session = CodeModeSession::new(config);
    let result = run_plan(&mut session, &plan).expect("plan runs");
    assert!(result.ok);
    assert_eq!(result.call_count, 3);
    assert_eq!(result.return_value, json!({"hit_count": 1}));
    let golden = canonicalize_text(&format!("{}\n", serde_json::to_string_pretty(&result.return_value).expect("pretty")));
    assert_eq!(golden, "{\n  \"hit_count\": 1\n}\n");
}

#[test]
fn batch_output_feeds_plan_end_to_end() {
    // Cross-surface flow: batch catalog results become plan select input.
    // Hand-computed: "search" matches the 8 Search-kind tools (kind is in the
    // haystack) plus filter_hits and catalog_search = 10, ordered search,find.
    let temp = tempfile::tempdir().expect("tempdir");
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![catalog_call("w1", "search"), catalog_call("w2", "search")]),
    )
    .expect("batch runs");
    assert!(batch.all_ok);
    let tools = batch.results[0].value.as_ref().expect("value")["tools"]
        .as_array()
        .expect("tools array")
        .clone();
    assert_eq!(tools.len(), 10);
    assert_eq!(tools[0]["name"], json!("search"));
    assert_eq!(tools[1]["name"], json!("find"));
    let plan = parse_plan(&json!({"steps": [
        {"id": "names", "tool": "select",
         "args": {"value": tools, "fields": ["name"], "limit": 2}},
    ]}))
    .expect("plan parses");
    let mut session = session_at(temp.path());
    let result = run_plan(&mut session, &plan).expect("plan runs");
    assert_eq!(result.return_value, json!([{"name": "search"}, {"name": "find"}]));
}

#[test]
fn rerun_determinism_freezes_golden_text() {
    // Same plan on two fresh sessions plus the direct-call path yield
    // byte-identical canonical golden text; the batch leg agrees on the step
    // payload. "chain" matches only the chain tool (name hit, no kind hit).
    let raw = json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "chain"}},
        {"id": "b", "tool": "select",
         "args": {"value": "$a.tools.0",
                  "fields": ["name", "kind", "read_only", "capsule_default"]}},
    ], "return": "$b"});
    let plan = parse_plan(&raw).expect("plan parses");
    let temp = tempfile::tempdir().expect("tempdir");
    let mut first = session_at(temp.path());
    let mut second = session_at(temp.path());
    let r1 = run_plan(&mut first, &plan).expect("first runs");
    let r2 = run_plan(&mut second, &plan).expect("second runs");
    let mut direct = session_at(temp.path());
    let payload = direct
        .call("catalog_search", json!({"query": "chain"}))
        .expect("direct catalog");
    let shaped = direct
        .call(
            "select",
            json!({"value": payload["tools"][0],
                   "fields": ["name", "kind", "read_only", "capsule_default"]}),
        )
        .expect("direct select");
    assert_eq!(r1.return_value, r2.return_value);
    assert_eq!(r1.return_value, shaped);
    let golden = |v: &Value| {
        canonicalize_text(&format!("{}\n", serde_json::to_string_pretty(v).expect("pretty")))
    };
    // serde_json Map without preserve_order prints keys alphabetically.
    let hand = "{\n  \"capsule_default\": false,\n  \"kind\": \"search\",\n  \"name\": \"chain\",\n  \"read_only\": true\n}\n";
    assert_eq!(golden(&r1.return_value), hand);
    assert_eq!(golden(&r2.return_value), hand);
    assert_eq!(golden(&shaped), hand);
    let batch = run_batch(config_at(temp.path()), &batch_request(vec![catalog_call("c", "chain")]))
        .expect("batch runs");
    assert!(batch.all_ok);
    assert_eq!(
        batch.results[0].value.as_ref().expect("value"),
        &r1.steps["a"]
    );
}

#[test]
fn budget_exhaustion_midplan_with_batch_isolation() {
    // A 3-step plan with max_calls=2 dies at the $ref-consuming last step
    // (ref resolution itself burns no budget: call_count==2, not 3); the
    // spent session stays sticky while batch (own session) is unaffected.
    let raw = json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "chain"}},
        {"id": "c", "tool": "select", "args": {"value": "$a", "fields": ["tools"]}},
    ], "return": "$c"});
    let plan = parse_plan(&raw).expect("plan parses");
    let temp = tempfile::tempdir().expect("tempdir");
    let mut tight = session_at(temp.path());
    tight.max_calls = 2;
    let err = run_plan(&mut tight, &plan).expect_err("tight budget fails");
    assert!(matches!(err, CallError::BudgetExhausted(2)), "got {err:?}");
    assert_eq!(tight.call_count(), 2);
    assert!(tight.exhausted());
    let again = tight
        .call("catalog_search", json!({"query": "search"}))
        .expect_err("stays exhausted");
    assert!(matches!(again, CallError::BudgetExhausted(2)), "got {again:?}");
    let mut roomy = session_at(temp.path());
    roomy.max_calls = 3;
    let ok = run_plan(&mut roomy, &plan).expect("roomy runs");
    assert!(ok.ok);
    assert_eq!(ok.call_count, 3);
    assert!(ok.return_value.get("tools").is_some());
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![
            catalog_call("a", "search"),
            catalog_call("b", "chain"),
            catalog_call("c", "select"),
        ]),
    )
    .expect("batch runs");
    assert!(batch.all_ok);
    assert_eq!(batch.call_count, 3);
}

#[test]
fn missing_root_fail_closed_pure_vs_bound() {
    // Nonexistent root: pure catalog calls stay Ok; root-bound tools fail
    // closed with Other; plans fail at the bound step; batch keeps the
    // failure per-call inside an Ok envelope.
    let temp = tempfile::tempdir().expect("tempdir");
    let missing = temp.path().join("no-such-dir");
    let mut session = session_at(&missing);
    let pure = session
        .call("catalog_search", json!({"query": "search"}))
        .expect("pure tools ignore root");
    assert!(pure.get("tools").is_some());
    let err = session
        .call("index_status", json!({}))
        .expect_err("bound tool fails");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call("read", json!({"path": "x.rs", "start": 1, "end": 1}))
        .expect_err("read fails");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "index_status", "args": {}},
    ]}))
    .expect("plan parses");
    let mut planned = session_at(&missing);
    let err = run_plan(&mut planned, &plan).expect_err("plan fails at bound step");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(planned.call_count(), 2);
    let batch = run_batch(
        config_at(&missing),
        &batch_request(vec![BatchCall {
            id: "s".to_string(),
            tool: "index_status".to_string(),
            args: json!({}),
        }]),
    )
    .expect("batch envelope still Ok");
    assert!(!batch.all_ok);
    assert!(!batch.results[0].ok);
    assert!(batch.results[0].value.is_none());
    assert!(batch.results[0].error.is_some());
}

#[test]
fn root_escape_fail_closed_across_surfaces() {
    // Contained subroot Ok; foreign root refused with Other on direct, plan,
    // and batch paths alike. Discriminants only.
    let root = tempfile::tempdir().expect("root");
    std::fs::create_dir(root.path().join("child")).expect("child");
    let outside = tempfile::tempdir().expect("outside");
    let outside_str = outside.path().to_str().expect("utf8").to_string();
    let mut session = session_at(root.path());
    assert!(session
        .call("index_status", json!({"root": "child"}))
        .is_ok());
    let err = session
        .call("index_status", json!({"root": outside_str}))
        .expect_err("escape fails");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "index_status", "args": {"root": outside_str}},
    ]}))
    .expect("plan parses");
    let mut planned = session_at(root.path());
    let err = run_plan(&mut planned, &plan).expect_err("plan escape fails");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let batch = run_batch(
        config_at(root.path()),
        &batch_request(vec![BatchCall {
            id: "s".to_string(),
            tool: "index_status".to_string(),
            args: json!({"root": outside_str}),
        }]),
    )
    .expect("batch envelope Ok");
    assert!(!batch.all_ok);
    assert!(!batch.results[0].ok);
    assert!(batch.results[0].error.is_some());
}

#[test]
fn oversized_response_fail_closed_across_surfaces() {
    // A select value one byte past the per-call cap fails as Other on the
    // direct and plan paths, and as a per-call failure (value dropped, error
    // set) inside an otherwise-Ok batch envelope.
    assert_eq!(MAX_CALL_RESPONSE_BYTES, 1_048_576);
    let payload = "x".repeat(MAX_CALL_RESPONSE_BYTES + 1);
    let args = json!({"value": {"payload": payload}, "fields": ["payload"]});
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let err = session.call("select", args.clone()).expect_err("oversized");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "select", "args": args},
    ]}))
    .expect("plan parses");
    let mut planned = session_at(temp.path());
    let err = run_plan(&mut planned, &plan).expect_err("plan oversized");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let payload = "x".repeat(MAX_CALL_RESPONSE_BYTES + 1);
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![BatchCall {
            id: "s".to_string(),
            tool: "select".to_string(),
            args: json!({"value": {"payload": payload}, "fields": ["payload"]}),
        }]),
    )
    .expect("batch envelope Ok");
    assert!(!batch.all_ok);
    assert!(!batch.results[0].ok);
    assert!(batch.results[0].value.is_none());
    assert!(batch.results[0].error.is_some());
    let mut small = session_at(temp.path());
    assert_eq!(
        small.call("select", json!({"value": {"a": 1}, "fields": ["a"]}))
            .expect("small ok"),
        json!({"a": 1})
    );
}

#[test]
fn error_taxonomy_bump_order_and_bound_split() {
    // Bump-before-dispatch: a plan dying on an unknown second step consumed
    // 2 calls, not 1. Bound-tool arg failures are Other; pure-tool arg
    // failures are InvalidArgs; batch mirrors the split per-call.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let err = session
        .call("no-such-tool", json!({"query": "x"}))
        .expect_err("unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    let err = session.call("search", json!({})).expect_err("search args");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session.call("read", json!({})).expect_err("read args");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call("catalog_search", json!({}))
        .expect_err("catalog args");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let err = session.call("select", json!({})).expect_err("select args");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let err = session.call("defs", json!({})).expect_err("defs args");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "no-such-tool", "args": {}},
    ]}))
    .expect("plan parses");
    let mut planned = session_at(temp.path());
    let err = run_plan(&mut planned, &plan).expect_err("plan unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert_eq!(planned.call_count(), 2);
    let mut bad = catalog_call("bad", "search");
    bad.tool = "no-such-tool".to_string();
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![catalog_call("good", "search"), bad]),
    )
    .expect("batch runs");
    assert!(!batch.all_ok);
    assert_eq!(batch.call_count, 2);
    assert!(batch.results[0].ok);
    assert!(!batch.results[1].ok);
    assert!(batch.results[0].error.is_none());
    assert!(batch.results[1].error.is_some());
}

#[test]
fn session_read_agrees_with_budget_rendering() {
    // Differential end to end: live session read text is the exact excerpt
    // that budget rendering shapes. 3 lines <= 12-line Block verbatim rule;
    // default budget upgrades the single hit to Block; Signature is
    // hand-computed (first line + first non-trivial body line).
    let text = "pub fn alpha() {\n    1\n}\n";
    let (_temp, config) = indexed_repo(&[("lib.rs", text)]);
    let mut session = CodeModeSession::new(config);
    let window = session
        .call("read", json!({"path": "lib.rs", "start": 1, "end": 3}))
        .expect("read");
    assert_eq!(window["count"], json!(1));
    let body = window["windows"][0]["text"].as_str().expect("text").to_string();
    assert_eq!(body, "pub fn alpha() {\n    1\n}");
    let hit = sample_hit(&body);
    assert_eq!(render(&hit, DetailLevel::Full).body, body);
    assert_eq!(render(&hit, DetailLevel::Metadata).body, "");
    assert_eq!(
        render(&hit, DetailLevel::Signature).body,
        "pub fn alpha() {\n… 1"
    );
    let chosen = select(&[hit], OutputBudget::default());
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].detail, DetailLevel::Block);
    assert_eq!(chosen[0].body, body);
    assert_eq!(plan_cost(&chosen), body.len());
}

#[test]
fn degenerate_inputs_fail_closed_or_documented() {
    // Empty root, missing files, empty step id, impossible threshold, and a
    // single-call forced-parallel batch (len<=1 pins serial).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let err = session
        .call("read", json!({"path": "missing.rs", "start": 1, "end": 2}))
        .expect_err("missing file");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call(
            "edit",
            json!({"path": "missing.rs", "oldText": "a", "newText": "b"}),
        )
        .expect_err("missing edit target");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let empty_id = parse_plan(&json!({"steps": [
        {"id": "", "tool": "catalog_search", "args": {"query": "x"}},
    ]}))
    .expect("empty id parses");
    let mut planned = session_at(temp.path());
    let err = run_plan(&mut planned, &empty_id).expect_err("empty id never runs");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let impossible = session
        .call(
            "filter_hits",
            json!({"hits": [{"kind": "def", "file": "src/a.rs", "score": 9.0}],
                   "min_score": 1e9}),
        )
        .expect("impossible threshold ok");
    assert_eq!(impossible["hit_count"], json!(0));
    assert_eq!(impossible["hits"], json!([]));
    let mut forced = batch_request(vec![catalog_call("solo", "search")]);
    forced.parallel_mode = Some(ParallelMode::Parallel);
    let batch = run_batch(config_at(temp.path()), &forced).expect("solo runs");
    assert!(batch.all_ok);
    assert_eq!(batch.mode, "serial");
    assert_eq!(batch.call_count, 1);
}
