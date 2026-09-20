//! Oracle CORE for codemode (consolidated suite): plan lifecycle, batch
//! envelope, and budget contracts.
//!
//! Consolidates `oracle_foundry_pass1.rs` (plan facets), `pass2.rs`
//! (plan/batch/budget facets), and `pass3.rs` (plan/batch/budget relations)
//! into 3 intent-grouped tests: each `#[test]` owns ONE intent with multiple
//! facets. Surface names/catalog/golden helpers live in `oracle_surface.rs`;
//! end-to-end compositions live in `oracle_e2e.rs`. Catalog:
//! `tests/catalog/oracle-codemode.md`.
//!
//! Discipline (inherited): hand-computed expectations; failures assert enum
//! discriminants via `matches!`, never Display text. Session/batch builders
//! come from testkit (`session_at`, `config_at`, `batch_request`,
//! `catalog_call` 2-arg canonical form), as do the shared oracle helpers
//! (`sample_search_hit`, `search_select_plan`).

use ast_sgrep_codemode::plan::single_result;
use ast_sgrep_codemode::{
    example_plan, parse_plan, run_batch, run_plan, BatchCall, CallError, ParallelMode,
    MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_plugins::budget::{plan_cost, render, select};
use ast_sgrep_plugins::{CompactBudget, DetailLevel, OutputBudget};
use ast_sgrep_testkit::{
    batch_request, catalog_call, config_at, sample_search_hit, search_select_plan, session_at,
};
use serde_json::{json, Value};

/// INTENT=plan lifecycle: example parses with shaped steps; malformed plans
/// fail parse with InvalidArgs while empty steps parse Ok; run refuses
/// empty/dup-id/dangling refs and reports ok-shape honestly; the 7-case $ref
/// adversarial matrix fails InvalidArgs (never panic/Null); omitted return
/// equals explicit $last; reruns across fresh sessions are Value-identical.
/// KILLS=fixture-rot, validation-guard-removal, empty-accept, dup-blindness,
/// ref-blindness, ok-lie, ok-flag/step-key/count-mutant (ctor), cycle and
/// path-resolution holes, default-return/path-mutant, render-cache and
/// order-nondeterminism.
/// ABSORBS=example_plan_parses_with_nonempty_steps,
/// plan_parse_failures_carry_invalid_args_discriminant,
/// run_plan_refuses_empty_duplicate_and_dangling,
/// single_result_wraps_main_with_count_1 (positive-shape leg),
/// plan_ref_adversarial_matrix_fails_invalid_args,
/// plan_default_return_equals_explicit_last_ref,
/// plan_rerun_is_deterministic_across_fresh_sessions.
#[test]
fn plan_lifecycle_parses_validates_resolves_and_reruns_deterministically() {
    // Facet 1 (example shape): the shipped example parses; every step has
    // id+tool. Sole example_plan pin.
    let plan = parse_plan(&example_plan()).expect("example parses");
    assert!(!plan.steps.is_empty());
    for step in &plan.steps {
        assert!(!step.id.is_empty(), "step id must be set");
        assert!(!step.tool.is_empty(), "step tool must be set");
    }

    // Facet 2 (parse failures): 4 malformed plans → InvalidArgs; empty steps
    // are absence (Ok), not a failure.
    for bad in [
        json!({"steps": "nope"}),
        json!({}),
        json!({"steps": [{"id": "a"}]}),
        json!({"steps": "nope", "return": "$a"}),
    ] {
        let err = parse_plan(&bad).expect_err("must fail");
        assert!(
            matches!(err, CallError::InvalidArgs(_)),
            "expected InvalidArgs, got {err:?}"
        );
    }
    let empty = parse_plan(&json!({"steps": []})).expect("empty ok");
    assert!(empty.steps.is_empty());
    assert!(empty.return_ref.is_none());

    // Facet 3 (run validation + positive shape): parse allows empty steps,
    // run must refuse; dup ids and dangling $refs refuse; a good plan runs
    // with honest ok/call_count/return. The single_result ctor leg pins the
    // same positive shape (ok, count 1, return echo, steps[main]) the run
    // path must produce.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let err = run_plan(&mut session, &empty).expect_err("empty runs never");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let dup = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "catalog_search", "args": {"query": "a"}},
        {"id": "s", "tool": "catalog_search", "args": {"query": "b"}},
    ]}))
    .expect("dup parses");
    let err = run_plan(&mut session, &dup).expect_err("dup id");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let dangling = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "catalog_search", "args": {"query": "a"}},
    ], "return": "$missing"}))
    .expect("dangling parses");
    let err = run_plan(&mut session, &dangling).expect_err("dangling ref");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let good = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "catalog_search", "args": {"query": "search"}},
    ], "return": "$s"}))
    .expect("good parses");
    let mut fresh = session_at(temp.path());
    let result = run_plan(&mut fresh, &good).expect("good runs");
    assert!(result.ok);
    assert_eq!(result.call_count, 1);
    assert!(result.return_value.get("tools").is_some());
    let value = json!({"a": 1});
    let single = single_result("search", value.clone());
    assert!(single.ok);
    assert_eq!(single.call_count, 1);
    assert_eq!(single.return_value, value);
    assert_eq!(single.steps.get("main"), Some(&value));

    // Facet 4 ($ref adversarial matrix): self-ref, forward ref, missing path,
    // array overrun, scalar descent, bare `$`, and non-string ref target all
    // fail InvalidArgs — never a panic or Null echo.
    let cases: &[Value] = &[
        json!({"steps": [{"id": "s", "tool": "catalog_search", "args": {"query": "$s"}}]}),
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "$b"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "x"}},
        ]}),
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a.nosuch"}},
        ]}),
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a.tools.999.name"}},
        ]}),
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a.tools.0.name.first"}},
        ]}),
        json!({"steps": [{"id": "s", "tool": "catalog_search", "args": {"query": "$"}}]}),
        json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": "catalog_search", "args": {"query": "$a"}},
        ]}),
    ];
    for (i, raw) in cases.iter().enumerate() {
        let plan = parse_plan(raw).expect("adversarial plans parse");
        let mut session = session_at(temp.path());
        let err = run_plan(&mut session, &plan).expect_err("must fail");
        assert!(
            matches!(err, CallError::InvalidArgs(_)),
            "case {i}: got {err:?}"
        );
    }

    // Facet 5 (return default): omitting `return` returns the last step —
    // identical to explicit `$last`; solo plans return their only step;
    // array-index return paths resolve by hand (tools[0] is "search").
    let explicit = parse_plan(&search_select_plan()).expect("explicit parses");
    let mut implied_raw = search_select_plan();
    implied_raw
        .as_object_mut()
        .expect("object")
        .remove("return");
    let implied = parse_plan(&implied_raw).expect("implied parses");
    assert!(implied.return_ref.is_none());
    let mut s1 = session_at(temp.path());
    let mut s2 = session_at(temp.path());
    let r1 = run_plan(&mut s1, &explicit).expect("explicit runs");
    let r2 = run_plan(&mut s2, &implied).expect("implied runs");
    assert_eq!(r1.return_value, r2.return_value);
    assert_eq!(r2.return_value, r2.steps["b"]);
    let solo = parse_plan(&json!({"steps": [
        {"id": "only", "tool": "catalog_search", "args": {"query": "chain"}},
    ]}))
    .expect("solo parses");
    let mut s3 = session_at(temp.path());
    let r3 = run_plan(&mut s3, &solo).expect("solo runs");
    assert_eq!(r3.return_value, r3.steps["only"]);
    let indexed = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ], "return": "$a.tools.0.name"}))
    .expect("indexed parses");
    let mut s4 = session_at(temp.path());
    let r4 = run_plan(&mut s4, &indexed).expect("indexed runs");
    assert_eq!(r4.return_value, json!("search"));

    // Facet 6 (rerun determinism): same plan × 2 fresh sessions → identical
    // steps/return/count 2, tools[0]=search. Value-level pin; the e2e suite
    // pins the complementary golden-text level.
    let plan = parse_plan(&search_select_plan()).expect("plan parses");
    let mut first = session_at(temp.path());
    let mut second = session_at(temp.path());
    let r1 = run_plan(&mut first, &plan).expect("first runs");
    let r2 = run_plan(&mut second, &plan).expect("second runs");
    assert!(r1.ok && r2.ok);
    assert_eq!(r1.call_count, 2);
    assert_eq!(r1.call_count, r2.call_count);
    assert_eq!(r1.steps, r2.steps);
    assert_eq!(r1.return_value, r2.return_value);
    assert_eq!(r1.return_value["tools"][0]["name"], json!("search"));
}

/// INTENT=batch envelope contract: validation rejects empty/oversize batches
/// before execution and accepts the exact ceilings; permuted calls permute
/// results but preserve the per-id ok-map; serial/parallel paths agree
/// per-call; results echo input order/ids with exclusive ok/error fields and
/// no dedup; explicit parallel_mode beats the legacy bool and 32-call batches
/// stay ordered.
/// KILLS=ceiling-drop, guard-removal, order-sensitive-validation,
/// path-divergence, shaping/dedup/exclusivity-mutant, precedence and
/// max-order mutants.
/// ABSORBS=batch_validation_rejects_before_execution,
/// batch_validation_and_outcome_are_order_independent,
/// batch_serial_parallel_paths_agree_per_call,
/// batch_result_shaping_echoes_ids_in_order_with_exclusive_fields,
/// parallel_mode_precedence_and_max_batch_order.
#[test]
fn batch_envelope_validates_shapes_orders_and_modes() {
    // Facet 1 (validation): ceilings pinned (32, 128, 128); empty/33-call/
    // bad-id/bad-tool reject before execution; exactly-32 and 128-byte ids
    // execute; a 128-byte unknown tool passes validation but fails per-call.
    assert_eq!(
        (MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES),
        (32, 128, 128)
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let config = config_at(temp.path());
    let err = run_batch(config.clone(), &batch_request(vec![])).expect_err("empty");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let too_many: Vec<BatchCall> = (0..33)
        .map(|i| catalog_call(&format!("c{i}"), "search"))
        .collect();
    let err = run_batch(config.clone(), &batch_request(too_many)).expect_err("33 calls");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let err = run_batch(
        config.clone(),
        &batch_request(vec![catalog_call("", "search")]),
    )
    .expect_err("id");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let err = run_batch(
        config.clone(),
        &batch_request(vec![catalog_call(&"i".repeat(129), "search")]),
    )
    .expect_err("long id");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let mut no_tool = catalog_call("t", "search");
    no_tool.tool.clear();
    let err = run_batch(config.clone(), &batch_request(vec![no_tool])).expect_err("tool");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let mut long_tool = catalog_call("t", "search");
    long_tool.tool = "t".repeat(129);
    let err = run_batch(config.clone(), &batch_request(vec![long_tool])).expect_err("long tool");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let full: Vec<BatchCall> = (0..32)
        .map(|i| catalog_call(&format!("c{i}"), "search"))
        .collect();
    let ok = run_batch(config.clone(), &batch_request(full)).expect("32 run");
    assert!(ok.all_ok);
    assert_eq!(ok.call_count, 32);
    let edge = run_batch(
        config.clone(),
        &batch_request(vec![catalog_call(&"i".repeat(128), "search")]),
    )
    .expect("128-byte id runs");
    assert!(edge.all_ok);
    let mut unknown = catalog_call("u", "search");
    unknown.tool = "u".repeat(128);
    let dispatched = run_batch(config.clone(), &batch_request(vec![unknown])).expect("dispatch");
    assert!(!dispatched.all_ok);
    assert!(!dispatched.results[0].ok);
    assert_eq!(dispatched.results[0].id, "u");

    // Facet 2 (order independence): permuting calls permutes results but the
    // per-id ok-map, all_ok, and call_count are stable; results follow input
    // order, not sorted order.
    let mut bad = catalog_call("bad", "search");
    bad.tool = "no-such-tool".to_string();
    let forward = vec![
        catalog_call("x", "search"),
        bad.clone(),
        catalog_call("y", "search"),
    ];
    let reversed: Vec<BatchCall> = forward.iter().rev().cloned().collect();
    let r1 = run_batch(config.clone(), &batch_request(forward)).expect("runs");
    let r2 = run_batch(config.clone(), &batch_request(reversed)).expect("runs");
    assert!(!r1.all_ok && !r2.all_ok);
    assert_eq!((r1.call_count, r2.call_count), (3, 3));
    let by_id = |r: &ast_sgrep_codemode::BatchResponse| {
        r.results
            .iter()
            .map(|c| (c.id.clone(), c.ok))
            .collect::<Vec<_>>()
    };
    let mut m1 = by_id(&r1);
    let mut m2 = by_id(&r2);
    m1.sort();
    m2.sort();
    assert_eq!(m1, m2);
    assert_eq!(
        m1,
        vec![
            ("bad".to_string(), false),
            ("x".to_string(), true),
            ("y".to_string(), true)
        ]
    );
    let ids: Vec<&str> = r1.results.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["x", "bad", "y"]);

    // Facet 3 (serial/parallel agreement): the two paths agree on every
    // per-id (ok, value) pair for pure calls; only the mode label differs.
    let calls = || vec![catalog_call("p", "search"), catalog_call("q", "search")];
    let mut serial_req = batch_request(calls());
    serial_req.parallel_mode = Some(ParallelMode::Serial);
    let mut parallel_req = batch_request(calls());
    parallel_req.parallel_mode = Some(ParallelMode::Parallel);
    let serial = run_batch(config.clone(), &serial_req).expect("serial");
    let parallel = run_batch(config.clone(), &parallel_req).expect("parallel");
    assert_eq!(serial.mode, "serial");
    assert_eq!(parallel.mode, "parallel");
    assert!(serial.all_ok && parallel.all_ok);
    assert_eq!(serial.results.len(), 2);
    for (s, p) in serial.results.iter().zip(parallel.results.iter()) {
        assert_eq!(s.id, p.id);
        assert!(s.ok && p.ok);
        assert_eq!(s.value, p.value);
    }

    // Facet 4 (shaping): results echo input order/ids; ok and error are
    // exclusive; call_count == n; all_ok is the AND over per-call ok.
    // Duplicate ids both execute (no dedup) — the no-aliasing contract.
    let mut bad = catalog_call("m", "search");
    bad.tool = "no-such-tool".to_string();
    let calls = vec![
        catalog_call("a", "search"),
        bad,
        catalog_call("z", "search"),
    ];
    let response = run_batch(config.clone(), &batch_request(calls)).expect("runs");
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
    let dup = run_batch(
        config.clone(),
        &batch_request(vec![
            catalog_call("dup", "search"),
            catalog_call("dup", "search"),
        ]),
    )
    .expect("dup ids run");
    assert!(dup.all_ok);
    assert_eq!(dup.call_count, 2);
    assert_eq!(dup.results.len(), 2);
    assert!(dup.results.iter().all(|r| r.ok && r.id == "dup"));
    assert_eq!(dup.results[0].value, dup.results[1].value);

    // Facet 5 (precedence + max order): explicit parallel_mode wins over the
    // legacy parallel bool; legacy true alone parallelizes pure waves with
    // identical values; 32-call batches echo all ids in input order.
    let two = || vec![catalog_call("a", "search"), catalog_call("b", "search")];
    let mut legacy_only = batch_request(two());
    legacy_only.parallel = Some(true);
    let legacy = run_batch(config.clone(), &legacy_only).expect("legacy runs");
    assert_eq!(legacy.mode, "parallel");
    let mut override_req = batch_request(two());
    override_req.parallel = Some(true);
    override_req.parallel_mode = Some(ParallelMode::Serial);
    let overridden = run_batch(config.clone(), &override_req).expect("override runs");
    assert_eq!(overridden.mode, "serial");
    assert_eq!(legacy.results.len(), overridden.results.len());
    for (l, o) in legacy.results.iter().zip(overridden.results.iter()) {
        assert_eq!(l.value, o.value);
    }
    let full: Vec<BatchCall> = (0..32)
        .map(|i| catalog_call(&format!("c{i}"), "search"))
        .collect();
    let maxed = run_batch(config, &batch_request(full)).expect("32 run");
    assert!(maxed.all_ok);
    assert_eq!(maxed.call_count, 32);
    for (i, result) in maxed.results.iter().enumerate() {
        assert_eq!(result.id, format!("c{i}"));
        assert!(result.ok);
    }
}

/// INTENT=budget contracts: CompactBudget and OutputBudget defaults are pinned
/// (96/768, 900/Block); detail levels form the ordered Metadata<Signature<
/// Block<Full lattice with exact labels; select degrades to Metadata under a
/// zero budget (never drops rows), upgrades to Full when funded, costs sum,
/// and render bodies are byte-exact; the call budget trips at calls>=max
/// (max 0 refuses immediately) and exhaustion is monotone and sticky.
/// KILLS=default-drift, order-swap/label-drift, evidence-drop,
/// upgrade-removal, cost-mutant, >=-vs->-flip, zero-budget-blindness,
/// budget-reset, off-by-one.
/// ABSORBS=compact_budget_default_matches_hand_values (budget-defaults leg),
/// detail_levels_order_and_label_by_hand,
/// output_budget_default_and_select_floor,
/// call_budget_boundary_is_calls_gte_max,
/// budget_exhaustion_is_monotone_and_sticky.
#[test]
fn budgets_default_floor_and_exhaust_monotonically() {
    // Facet 1 (compact defaults): 96/768 pinned against drift.
    let compact = CompactBudget::default();
    assert_eq!(compact.per_result_tokens, 96);
    assert_eq!(compact.response_tokens, 768);

    // Facet 2 (detail lattice): 4 levels ordered with exact labels.
    assert_eq!(DetailLevel::ALL.len(), 4);
    let labels: Vec<&str> = DetailLevel::ALL.iter().map(|d| d.as_str()).collect();
    assert_eq!(labels, vec!["metadata", "signature", "block", "full"]);
    assert!(
        DetailLevel::Metadata < DetailLevel::Signature
            && DetailLevel::Signature < DetailLevel::Block
            && DetailLevel::Block < DetailLevel::Full
    );

    // Facet 3 (output budget + select floor): default 900/Block; empty select
    // is empty; starved budgets degrade every row to Metadata (never drop);
    // funded budgets upgrade every row to Full; plan cost sums; render bodies
    // are byte-exact per level.
    let default = OutputBudget::default();
    assert_eq!(default.max_tokens, 900);
    assert_eq!(default.default_detail, DetailLevel::Block);
    assert!(select(&[], default).is_empty());
    let hits = vec![
        sample_search_hit("fn foo() {\nbar();\n}\n"),
        sample_search_hit("fn bar() {}\n"),
    ];
    let starved = select(
        &hits,
        OutputBudget {
            max_tokens: 0,
            default_detail: DetailLevel::Metadata,
        },
    );
    assert_eq!(starved.len(), 2);
    assert!(starved.iter().all(|r| r.detail == DetailLevel::Metadata));
    let funded = select(
        &hits,
        OutputBudget {
            max_tokens: 100_000,
            default_detail: DetailLevel::Full,
        },
    );
    assert!(funded.iter().all(|r| r.detail == DetailLevel::Full));
    assert_eq!(
        plan_cost(&funded),
        funded.iter().map(|r| r.cost).sum::<usize>()
    );
    let meta = render(&hits[0], DetailLevel::Metadata);
    assert_eq!(meta.body, "");
    let full = render(&hits[0], DetailLevel::Full);
    assert_eq!(full.body, "fn foo() {\nbar();\n}");
    let sig = render(&hits[0], DetailLevel::Signature);
    assert_eq!(sig.body, "fn foo() {\n… bar();");

    // Facet 4 (call boundary): 3rd call at max 2 → BudgetExhausted(2); max 0
    // refuses immediately with BudgetExhausted(0).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 2;
    let args = || json!({"query": "search"});
    assert!(session.call("catalog_search", args()).is_ok());
    assert!(session.call("catalog_search", args()).is_ok());
    assert!(session.exhausted());
    let err = session.call("catalog_search", args()).expect_err("budget");
    assert!(matches!(err, CallError::BudgetExhausted(2)), "got {err:?}");
    let mut zero = session_at(temp.path());
    zero.max_calls = 0;
    assert!(zero.exhausted());
    let err = zero
        .call("catalog_search", args())
        .expect_err("zero budget");
    assert!(matches!(err, CallError::BudgetExhausted(0)), "got {err:?}");

    // Facet 5 (monotone + sticky): a smaller budget never completes more
    // calls (tight 1 ≤ roomy 2); once exhausted the session stays exhausted
    // with the identical discriminant + payload.
    let plan = parse_plan(&search_select_plan()).expect("plan parses");
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
    let again = tight
        .call("catalog_search", json!({"query": "search"}))
        .expect_err("stays exhausted");
    assert!(
        matches!(again, CallError::BudgetExhausted(1)),
        "got {again:?}"
    );
}
