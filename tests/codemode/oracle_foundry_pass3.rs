//! Pass 3 (oracle-foundry, Mission 4): L3 metamorphic / differential /
//! adversarial oracles for codemode session flows, plan execution ordering,
//! batch result shaping, guard/apply boundaries, error taxonomy, idempotency.
//!
//! Pass 1/2 own: output formats, compact budgets, detail levels, plan
//! parse/validate, batch validation ceilings, catalog search, output budget,
//! miss reasons, golden trim/unify/terminate, scrubber presets, session root
//! pinning, tool-name aliases, budget `>=` boundary, run_plan dup/dangling,
//! single_result. This pass asserts only NEW relations over those surfaces:
//! rerun determinism, order-independence, budget monotonicity, serial/parallel
//! differential agreement, transform cross-path agreement, ref-resolution
//! adversarial matrix, taxonomy discriminants, unicode/empty edges.
//!
//! All expectations are hand-computed. Failures assert enum discriminants via
//! `matches!`, never Display text. Every call below is a pure transform or
//! catalog lookup — no index I/O, fully deterministic.

use ast_sgrep_codemode::{
    catalog_search, parse_plan, run_batch, run_plan, tool_catalog, BatchCall, BatchRequest,
    CallError, CodeModeSession, ParallelMode, SessionConfig,
};
use ast_sgrep_plugins::OutputFormat;
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

fn catalog_call(id: &str) -> BatchCall {
    BatchCall {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    }
}

fn search_plan() -> Value {
    json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "select", "args": {"value": "$a", "fields": ["tools"]}},
    ], "return": "$b"})
}

#[test]
fn plan_rerun_is_deterministic_across_fresh_sessions() {
    // Metamorphic: same plan + two fresh sessions => identical ok, steps map,
    // return_value, call_count. Kills render-cache / ordering nondeterminism.
    let temp = tempfile::tempdir().expect("tempdir");
    let plan = parse_plan(&search_plan()).expect("plan parses");
    let mut first = session_at(temp.path());
    let mut second = session_at(temp.path());
    let r1 = run_plan(&mut first, &plan).expect("first runs");
    let r2 = run_plan(&mut second, &plan).expect("second runs");
    assert!(r1.ok && r2.ok);
    assert_eq!(r1.call_count, 2);
    assert_eq!(r1.call_count, r2.call_count);
    assert_eq!(r1.steps, r2.steps);
    assert_eq!(r1.return_value, r2.return_value);
    // Hand-computed: catalog order starts with "search" itself.
    assert_eq!(r1.return_value["tools"][0]["name"], json!("search"));
}

#[test]
fn batch_validation_and_outcome_are_order_independent() {
    // Metamorphic: permuting calls permutes results but preserves the per-id
    // ok-map, all_ok, and call_count. Kills order-sensitive validation.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut bad = catalog_call("bad");
    bad.tool = "no-such-tool".to_string();
    let forward = vec![catalog_call("x"), bad.clone(), catalog_call("y")];
    let reversed: Vec<BatchCall> = forward.iter().rev().cloned().collect();
    let r1 = run_batch(config_at(temp.path()), &batch_request(forward)).expect("runs");
    let r2 = run_batch(config_at(temp.path()), &batch_request(reversed)).expect("runs");
    assert!(!r1.all_ok && !r2.all_ok);
    assert_eq!((r1.call_count, r2.call_count), (3, 3));
    let by_id = |r: &ast_sgrep_codemode::BatchResponse| {
        r.results.iter().map(|c| (c.id.clone(), c.ok)).collect::<Vec<_>>()
    };
    let mut m1 = by_id(&r1);
    let mut m2 = by_id(&r2);
    m1.sort();
    m2.sort();
    assert_eq!(m1, m2);
    assert_eq!(m1, vec![("bad".to_string(), false), ("x".to_string(), true), ("y".to_string(), true)]);
    // Results follow input order, not sorted order.
    let ids: Vec<&str> = r1.results.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["x", "bad", "y"]);
}

#[test]
fn batch_serial_parallel_paths_agree_per_call() {
    // Differential: independent run_serial / run_parallel paths agree on every
    // per-id (ok, value) pair for pure calls; only the mode label differs.
    let temp = tempfile::tempdir().expect("tempdir");
    let calls = || vec![catalog_call("p"), catalog_call("q")];
    let mut serial_req = batch_request(calls());
    serial_req.parallel_mode = Some(ParallelMode::Serial);
    let mut parallel_req = batch_request(calls());
    parallel_req.parallel_mode = Some(ParallelMode::Parallel);
    let serial = run_batch(config_at(temp.path()), &serial_req).expect("serial");
    let parallel = run_batch(config_at(temp.path()), &parallel_req).expect("parallel");
    assert_eq!(serial.mode, "serial");
    assert_eq!(parallel.mode, "parallel");
    assert!(serial.all_ok && parallel.all_ok);
    assert_eq!(serial.results.len(), 2);
    for (s, p) in serial.results.iter().zip(parallel.results.iter()) {
        assert_eq!(s.id, p.id);
        assert!(s.ok && p.ok);
        assert_eq!(s.value, p.value);
    }
}

#[test]
fn budget_exhaustion_is_monotone_and_sticky() {
    // Metamorphic: a smaller budget never completes more calls; once exhausted
    // the session stays exhausted with the same budget payload. Kills
    // budget-reset and off-by-one mutants beyond the pass-2 boundary.
    let temp = tempfile::tempdir().expect("tempdir");
    let plan = parse_plan(&search_plan()).expect("plan parses");
    let mut tight = session_at(temp.path());
    tight.max_calls = 1;
    let mut roomy = session_at(temp.path());
    roomy.max_calls = 3;
    let err = run_plan(&mut tight, &plan).expect_err("tight budget fails");
    assert!(matches!(err, CallError::BudgetExhausted(1)), "got {err:?}");
    let ok = run_plan(&mut roomy, &plan).expect("roomy runs");
    assert!(ok.ok);
    assert!(tight.call_count() <= roomy.call_count());
    assert_eq!((tight.call_count(), roomy.call_count()), (1, 2));
    assert!(tight.exhausted());
    // Sticky: the next call fails with the identical discriminant + payload.
    let again = tight
        .call("catalog_search", json!({"query": "search"}))
        .expect_err("stays exhausted");
    assert!(matches!(again, CallError::BudgetExhausted(1)), "got {again:?}");
}

#[test]
fn empty_plan_and_batch_edges_fail_stably() {
    // Edge stability: empty-plan run and empty-batch run fail with the same
    // discriminant on repeat (no state bleed); empty transform inputs stay Ok
    // with hand-computed zero shapes. Kills error-caching and empty/OOM flaps.
    let temp = tempfile::tempdir().expect("tempdir");
    let empty_plan = parse_plan(&json!({"steps": []})).expect("empty parses");
    for _ in 0..2 {
        let mut session = session_at(temp.path());
        let err = run_plan(&mut session, &empty_plan).expect_err("empty never runs");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
        let err = run_batch(config_at(temp.path()), &batch_request(vec![])).expect_err("empty");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    }
    let mut session = session_at(temp.path());
    let filtered = session
        .call("filter_hits", json!({"hits": []}))
        .expect("empty hits ok");
    assert_eq!(filtered["hit_count"], json!(0));
    assert_eq!(filtered["hits"], json!([]));
    let projected = session
        .call("select", json!({"value": [], "fields": ["a"]}))
        .expect("empty array ok");
    assert_eq!(projected, json!([]));
}

#[test]
fn golden_canonicalize_is_idempotent_fixpoint() {
    // Metamorphic (new relation on the pass-2 function): canonicalize is a
    // fixpoint — re-canonicalizing never changes output, over a fixed corpus
    // with CRLF, tabs, unicode, blank runs, and missing terminators.
    let corpus = [
        "a  \r\nb\t\n\n",
        "",
        "x",
        "\n\n",
        "a\r\n",
        "  ",
        "héllo  \r\nwörld\t\n\n\n",
        "a\nb\n",
        "  indented\n\ttabbed  \n",
        "emoji 🔍 trailing   \r\n",
    ];
    for input in corpus {
        let once = canonicalize_text(input);
        let twice = canonicalize_text(&once);
        assert_eq!(once, twice, "not idempotent for {input:?}");
    }
    // Hand-computed fixpoints (leading whitespace preserved, trailing cut).
    assert_eq!(canonicalize_text("  indented\n\ttabbed  \n"), "  indented\n\ttabbed\n");
    assert_eq!(canonicalize_text("héllo  \r\nwörld\t\n\n\n"), "héllo\nwörld\n");
}

#[test]
fn plan_ref_adversarial_matrix_fails_invalid_args() {
    // Adversarial: self-ref (cycle of length 1), forward ref (cycle of length
    // 2 fails at the first step), missing path, array overrun, scalar descent,
    // and bare `$` all fail with InvalidArgs — never a panic or Null echo.
    let temp = tempfile::tempdir().expect("tempdir");
    let cases: &[Value] = &[
        // Self reference: $s is not yet bound when step s resolves args.
        json!({"steps": [{"id": "s", "tool": "catalog_search", "args": {"query": "$s"}}]}),
        // Forward reference: $b is not yet bound when step a resolves args.
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "$b"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "x"}},
        ]}),
        // Missing object path in a bound step.
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a.nosuch"}},
        ]}),
        // Array index out of range.
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a.tools.999.name"}},
        ]}),
        // Descent into a scalar (tools[0].name is a string).
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a.tools.0.name.first"}},
        ]}),
        // Bare `$` is an empty ref.
        json!({"steps": [{"id": "s", "tool": "catalog_search", "args": {"query": "$"}}]}),
        // Non-string ref target: $a is an object, query requires a string.
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a"}},
        ]}),
    ];
    for (i, raw) in cases.iter().enumerate() {
        let plan = parse_plan(raw).expect("adversarial plans parse");
        let mut session = session_at(temp.path());
        let err = run_plan(&mut session, &plan).expect_err("must fail");
        assert!(matches!(err, CallError::InvalidArgs(_)), "case {i}: got {err:?}");
    }
}

#[test]
fn plan_default_return_equals_explicit_last_ref() {
    // Metamorphic: omitting `return` returns the last step — identical value
    // to an explicit `$last` ref. Array-index return paths resolve by hand.
    let temp = tempfile::tempdir().expect("tempdir");
    let explicit = parse_plan(&search_plan()).expect("explicit parses");
    let mut implied_raw = search_plan();
    implied_raw.as_object_mut().expect("object").remove("return");
    let implied = parse_plan(&implied_raw).expect("implied parses");
    assert!(implied.return_ref.is_none());
    let mut s1 = session_at(temp.path());
    let mut s2 = session_at(temp.path());
    let r1 = run_plan(&mut s1, &explicit).expect("explicit runs");
    let r2 = run_plan(&mut s2, &implied).expect("implied runs");
    assert_eq!(r1.return_value, r2.return_value);
    assert_eq!(r2.return_value, r2.steps["b"]);
    // Single-step plan: default return is the only step's output.
    let solo = parse_plan(&json!({"steps": [
        {"id": "only", "tool": "catalog_search", "args": {"query": "chain"}},
    ]}))
    .expect("solo parses");
    let mut s3 = session_at(temp.path());
    let r3 = run_plan(&mut s3, &solo).expect("solo runs");
    assert_eq!(r3.return_value, r3.steps["only"]);
    // Array-index return path, hand-computed: tools[0] is "search".
    let indexed = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ], "return": "$a.tools.0.name"}))
    .expect("indexed parses");
    let mut s4 = session_at(temp.path());
    let r4 = run_plan(&mut s4, &indexed).expect("indexed runs");
    assert_eq!(r4.return_value, json!("search"));
}

#[test]
fn batch_result_shaping_echoes_ids_in_order_with_exclusive_fields() {
    // Shaping: results echo input order/ids; ok and error are exclusive;
    // call_count == n; all_ok is the AND over per-call ok. Duplicate ids both
    // execute (no dedup) — documents the no-aliasing contract.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut bad = catalog_call("m");
    bad.tool = "no-such-tool".to_string();
    let calls = vec![catalog_call("a"), bad, catalog_call("z")];
    let response = run_batch(config_at(temp.path()), &batch_request(calls)).expect("runs");
    assert!(!response.all_ok);
    assert_eq!(response.call_count, 3);
    assert_eq!(response.results.len(), 3);
    let ids: Vec<&str> = response.results.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "m", "z"]);
    for result in &response.results {
        if result.ok {
            assert!(result.value.is_some(), "ok without value: {}", result.id);
            assert!(result.error.is_none(), "ok with error: {}", result.id);
        } else {
            assert!(result.value.is_none(), "fail with value: {}", result.id);
            assert!(result.error.is_some(), "fail without error: {}", result.id);
        }
    }
    assert!(response.results[0].ok && !response.results[1].ok && response.results[2].ok);
    // Duplicate ids: both execute, both echo, order preserved.
    let dup = run_batch(
        config_at(temp.path()),
        &batch_request(vec![catalog_call("dup"), catalog_call("dup")]),
    )
    .expect("dup ids run");
    assert!(dup.all_ok);
    assert_eq!(dup.call_count, 2);
    assert_eq!(dup.results.len(), 2);
    assert!(dup.results.iter().all(|r| r.ok && r.id == "dup"));
    assert_eq!(dup.results[0].value, dup.results[1].value);
}

#[test]
fn error_taxonomy_splits_unknown_tool_from_invalid_args() {
    // Taxonomy: dispatch failures are UnknownTool; guard failures are
    // InvalidArgs; and batch dispatch failures stay per-call (Ok envelope)
    // while validation failures are top-level Err. Discriminants only.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    for unknown in ["no-such-tool", "Search", " search", ""] {
        let err = session
            .call(unknown, json!({"query": "x"}))
            .expect_err("unknown tool");
        assert!(matches!(err, CallError::UnknownTool(_)), "tool {unknown:?}: got {err:?}");
    }
    let invalid: &[(&str, Value)] = &[
        ("catalog_search", json!({})),
        ("catalog_search", json!({"query": ""})),
        ("catalog_search", json!({"query": "   "})),
        ("catalog_describe", json!({"name": "no-such-tool"})),
        ("catalog_describe", json!({})),
        ("defs", json!({})),
        ("defs", json!({"symbol": "  "})),
        ("imports", json!({})),
        ("select", json!({})),
        ("select", json!({"value": {"a": 1}})),
        ("select", json!({"value": {"a": 1}, "fields": []})),
        ("select", json!({"value": 42, "fields": ["a"]})),
        ("filter_hits", json!({})),
        ("filter_hits", json!({"hits": {"not": "hits"}})),
    ];
    for (tool, args) in invalid {
        let err = session.call(tool, args.clone()).expect_err("invalid args");
        assert!(matches!(err, CallError::InvalidArgs(_)), "tool {tool}: got {err:?}");
    }
    // Boundary: unknown tool inside a batch is a per-call failure, not Err.
    let mut bad = catalog_call("u");
    bad.tool = "no-such-tool".to_string();
    let dispatched = run_batch(config_at(temp.path()), &batch_request(vec![bad])).expect("runs");
    assert!(!dispatched.all_ok);
    assert!(!dispatched.results[0].ok);
    assert!(dispatched.results[0].error.is_some());
}

#[test]
fn unicode_and_empty_adversarial_inputs() {
    // Adversarial fixed inputs: unicode step ids roundtrip byte-exact; unicode
    // tool names are UnknownTool; unicode queries are deterministic with a
    // hand-computed empty match; batch ids echo byte-exact.
    let temp = tempfile::tempdir().expect("tempdir");
    let plan = parse_plan(&json!({"steps": [
        {"id": "étape-🔍", "tool": "catalog_search", "args": {"query": "search"}},
    ], "return": "$étape-🔍.tools.0.name"}))
    .expect("unicode plan parses");
    let mut session = session_at(temp.path());
    let result = run_plan(&mut session, &plan).expect("unicode plan runs");
    assert!(result.steps.contains_key("étape-🔍"));
    assert_eq!(result.return_value, json!("search"));
    let err = session
        .call("searχ", json!({"query": "x"}))
        .expect_err("unicode tool unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    // "héllo" matches no catalog haystack: deterministic empty tools array.
    let v1 = session
        .call("catalog_search", json!({"query": "héllo"}))
        .expect("unicode query ok");
    let v2 = session
        .call("catalog_search", json!({"query": "héllo"}))
        .expect("unicode query ok twice");
    assert_eq!(v1, v2);
    assert_eq!(v1["tools"], json!([]));
    // Direct catalog fn agrees with the session path on the same query.
    assert_eq!(catalog_search("héllo").len(), 0);
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![catalog_call("🔍-id-✓")]),
    )
    .expect("unicode batch id runs");
    assert!(batch.all_ok);
    assert_eq!(batch.results[0].id, "🔍-id-✓");
}

#[test]
fn parallel_mode_precedence_and_max_batch_order() {
    // Guard/apply boundary: explicit parallel_mode wins over the legacy
    // parallel bool; legacy true alone parallelizes pure waves. Max-size
    // batches preserve input order across all 32 results (new shape relation
    // beyond the pass-2 ceiling accept).
    let temp = tempfile::tempdir().expect("tempdir");
    let two = || vec![catalog_call("a"), catalog_call("b")];
    let mut legacy_only = batch_request(two());
    legacy_only.parallel = Some(true);
    let legacy = run_batch(config_at(temp.path()), &legacy_only).expect("legacy runs");
    assert_eq!(legacy.mode, "parallel");
    let mut override_req = batch_request(two());
    override_req.parallel = Some(true);
    override_req.parallel_mode = Some(ParallelMode::Serial);
    let overridden = run_batch(config_at(temp.path()), &override_req).expect("override runs");
    assert_eq!(overridden.mode, "serial");
    assert_eq!(legacy.results.len(), overridden.results.len());
    for (l, o) in legacy.results.iter().zip(overridden.results.iter()) {
        assert_eq!(l.value, o.value);
    }
    // Max-size batch: 32 results in input order with echoed ids.
    let full: Vec<BatchCall> = (0..32).map(|i| catalog_call(&format!("c{i}"))).collect();
    let maxed = run_batch(config_at(temp.path()), &batch_request(full)).expect("32 run");
    assert!(maxed.all_ok);
    assert_eq!(maxed.call_count, 32);
    for (i, result) in maxed.results.iter().enumerate() {
        assert_eq!(result.id, format!("c{i}"));
        assert!(result.ok);
    }
}

#[test]
fn pure_transforms_agree_across_call_plan_batch_paths() {
    // Differential: filter_hits/select agree across session.call, plan steps,
    // and batch calls — three independent dispatch paths, one semantics.
    let temp = tempfile::tempdir().expect("tempdir");
    let hits = json!([
        {"kind": "def", "file": "src/a.rs", "score": 9.0},
        {"kind": "ref", "file": "src/b.rs", "score": 1.0},
        {"kind": "def", "file": "tests/c.rs", "score": 5.0},
    ]);
    let filter_args = json!({"hits": hits, "kind": "def", "path_contains": "src/", "min_score": 2.0, "limit": 10});
    let mut session = session_at(temp.path());
    let direct = session.call("filter_hits", filter_args.clone()).expect("direct");
    assert_eq!(direct["hit_count"], json!(1));
    assert_eq!(direct["hits"][0]["file"], json!("src/a.rs"));
    // Narrowing the limit can only shrink the hit list (monotone).
    let tight = session
        .call("filter_hits", json!({"hits": hits, "limit": 1}))
        .expect("limit 1");
    let loose = session
        .call("filter_hits", json!({"hits": hits}))
        .expect("no limit");
    assert_eq!(tight["hit_count"], json!(1));
    assert_eq!(loose["hit_count"], json!(3));
    // Same filter via a plan step and via a batch call.
    let plan = parse_plan(&json!({"steps": [
        {"id": "f", "tool": "filter_hits", "args": filter_args},
    ]}))
    .expect("filter plan parses");
    let mut planned = session_at(temp.path());
    let pr = run_plan(&mut planned, &plan).expect("filter plan runs");
    assert_eq!(pr.return_value, direct);
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![BatchCall {
            id: "f".to_string(),
            tool: "filter_hits".to_string(),
            args: filter_args,
        }]),
    )
    .expect("filter batch runs");
    assert!(batch.all_ok);
    assert_eq!(batch.results[0].value.as_ref().expect("value"), &direct);
    // select drops unknown fields and truncates arrays by limit — hand shapes.
    let projected = session
        .call("select", json!({"value": {"a": 1, "b": 2}, "fields": ["a", "zzz"]}))
        .expect("select");
    assert_eq!(projected, json!({"a": 1}));
    let truncated = session
        .call("select", json!({"value": [{"a": 1}, {"a": 2}], "fields": ["a"], "limit": 1}))
        .expect("select limit");
    assert_eq!(truncated, json!([{"a": 1}]));
    // Catalog path differential: session tool payload wraps the direct fn.
    let via_session = session
        .call("catalog_search", json!({"query": "chain"}))
        .expect("catalog via session");
    let direct_tools = catalog_search("chain");
    assert_eq!(via_session["tools"].as_array().expect("array").len(), direct_tools.len());
    assert_eq!(via_session["tools"][0]["name"], json!(direct_tools[0].name));
    assert!(!direct_tools.is_empty());
    let catalog_names: Vec<&str> = tool_catalog().iter().map(|t| t.name).collect();
    assert!(direct_tools.iter().all(|t| catalog_names.contains(&t.name)));
}
