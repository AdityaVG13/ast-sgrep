//! E3 negative-path metamorphic tests for ast-sgrep-codemode errors.
//!
//! E1 pins the `CallError` taxonomy TABLE (one row per variant) and E2 pins
//! PROPAGATION (a failure keeps its variant across layers). E3 pins RELATIONS:
//! the same fault observed through two different surfaces must agree — never
//! asserted twice as absolute pins, only as equivalences. Discriminant
//! assertions via `matches!` / `match` arms only (never message text).
//! Deterministic, tempfile fixtures, no new deps.
//!
//! Relation map (metamorphic relation -> test):
//! - dispatch equivalence: session.call == tools::call_tool across the
//!   UnknownTool + InvalidArgs + Other fault matrices (plus the documented
//!   budget-bump divergence)
//!   -> e3_dispatch_equivalence
//! - repeat determinism: identical bad call twice == identical discriminant
//!   sequence, same session and fresh sessions (plus budget-bypass divergence)
//!   -> e3_repeat_bad_call_deterministic
//! - batch position independence: bad call at any index, good rows unaffected
//!   and bit-identical across rotations
//!   -> e3_batch_position_independence
//! - batch mode equivalence: serial vs parallel mixed batch, same per-id
//!   ok pattern and good values (absorbs the E2 parallel isolation legs)
//!   -> e3_batch_mode_equivalence
//! - plan position independence: failing step at any index, same discriminant
//!   as the direct call, call_count pins the executed prefix
//!   -> e3_plan_position_independence
//! - batch mirrors direct outcomes: per-call ok == direct is_ok, ok/error
//!   exclusivity per row (fail-closed, never Ok-carried; absorbs the E1
//!   per-call mirroring legs)
//!   -> e3_batch_mirrors_direct_outcomes
//! - plan fail-closed: failure is Err never Ok, discriminant == direct,
//!   documented prefix state (applied-before-fail vs untouched-on-fail),
//!   remainder resumes (absorbs the E4 plan-prefix drill)
//!   -> e3_plan_fail_closed_never_ok
//! - serve stream position independence: bad request at any stream index,
//!   neighbors unaffected, worker reaches Bye
//!   -> e3_serve_stream_position_independence

#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_codemode::tools::call_tool;
use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, BatchCall, CallError, ParallelMode, ServeRequest,
    ServeResponse,
};
use error_testkit::{
    assert_dispatch_equivalence, batch_call, batch_request, config_at, discriminant, serve_lines,
    serve_request_line, session_at,
};
use serde_json::{json, Value};

/// INTENT=session.call == call_tool discriminant on UnknownTool + InvalidArgs + Other fault matrices (budget-bump divergence pinned).
/// KILLS=dispatch-surface-divergence.
/// ABSORBS=e3_dispatch_equivalence_unknown_tool, e3_dispatch_equivalence_other (anchor: e3_dispatch_equivalence_invalid_args).
#[test]
fn e3_dispatch_equivalence() {
    // One parameterized equivalence across all three fault matrices: both
    // dispatch surfaces reject the same bad payload with the same variant —
    // the variant lives in dispatch, not in the caller. Unknown names
    // (including the empty name), pure-tool guard faults (InvalidArgs), and
    // bound-tool pre-IO validation faults (Other, anyhow-wrapped at dispatch).
    let temp = tempfile::tempdir().expect("tempdir");
    let cases: &[(&str, Value, &str)] = &[
        ("no-such-tool", json!({}), "unknown_tool"),
        ("", json!({}), "unknown_tool"),
        ("select", json!({}), "invalid_args"),
        (
            "select",
            json!({"value": {"a": 1}, "fields": "a"}),
            "invalid_args",
        ),
        ("filter_hits", json!({}), "invalid_args"),
        ("filter_hits", json!({"hits": 42}), "invalid_args"),
        ("catalog_search", json!({}), "invalid_args"),
        (
            "catalog_describe",
            json!({"name": "no-such-tool"}),
            "invalid_args",
        ),
        ("callers", json!({}), "invalid_args"),
        ("imports", json!({"module": "   "}), "invalid_args"),
        ("search", json!({}), "other"),
        ("find", json!({}), "other"),
        ("chain", json!({}), "other"),
        (
            "search",
            json!({"query": "auth", "lang": "notalang"}),
            "other",
        ),
        ("read", json!({"ref": 42}), "other"),
    ];
    for (tool, args, expected) in cases {
        assert_dispatch_equivalence(temp.path(), tool, args.clone());
        let mut probe = session_at(temp.path());
        let err = probe.call(tool, args.clone()).expect_err("fault must fail");
        assert_eq!(discriminant(&err), *expected, "tool {tool}");
        match *expected {
            "unknown_tool" => assert!(
                matches!(err, CallError::UnknownTool(_)),
                "tool {tool}: got {err:?}"
            ),
            "invalid_args" => assert!(
                matches!(err, CallError::InvalidArgs(_)),
                "tool {tool}: got {err:?}"
            ),
            "other" => assert!(
                matches!(err, CallError::Other(_)),
                "tool {tool}: got {err:?}"
            ),
            _ => unreachable!("unknown variant leg"),
        }
    }
}

/// INTENT=repeat bad call same discriminant same+fresh session; budget bypass pinned.
/// KILLS=nondeterministic-discriminant, bypass-regression.
/// ABSORBS=none.
#[test]
fn e3_repeat_bad_call_deterministic() {
    // The same bad call twice on one session yields the identical discriminant
    // sequence (failures consume budget deterministically, one unit each), and
    // a fresh session reproduces the same discriminant for the same payload.
    // Budget exhaustion itself repeats identically; tools::call_tool bypasses
    // the budget gate, so it still dispatches on an exhausted session.
    let temp = tempfile::tempdir().expect("tempdir");
    let cases: &[(&str, Value)] = &[
        ("no-such-tool", json!({})),
        ("select", json!({})),
        ("search", json!({})),
    ];
    for (tool, args) in cases {
        let mut session = session_at(temp.path());
        let first = session
            .call(tool, args.clone())
            .expect_err("bad call must fail");
        let second = session
            .call(tool, args.clone())
            .expect_err("repeat must fail identically");
        assert_eq!(
            discriminant(&first),
            discriminant(&second),
            "tool {tool}: repeat changed discriminant"
        );
        assert_eq!(session.call_count(), 2);

        let mut fresh = session_at(temp.path());
        let replay = fresh
            .call(tool, args.clone())
            .expect_err("fresh session must fail identically");
        assert_eq!(
            discriminant(&first),
            discriminant(&replay),
            "tool {tool}: fresh session changed discriminant"
        );
    }

    let mut spent = session_at(temp.path());
    spent.max_calls = 0;
    let first = spent
        .call("catalog_search", json!({"query": "search"}))
        .expect_err("zero budget must fail");
    let second = spent
        .call("catalog_search", json!({"query": "search"}))
        .expect_err("zero budget must fail identically");
    assert!(
        matches!(first, CallError::BudgetExhausted(0)),
        "got {first:?}"
    );
    assert_eq!(discriminant(&first), discriminant(&second));
    assert_eq!(spent.call_count(), 0);
    let bypass = call_tool(&mut spent, "no-such-tool", json!({})).expect_err("bypass dispatches");
    assert!(
        matches!(bypass, CallError::UnknownTool(_)),
        "exhausted session via call_tool: got {bypass:?}"
    );
}

/// INTENT=bad call at index 0/1/2: same envelope, good values bit-identical.
/// KILLS=position-dependent-routing, good-row-taint.
/// ABSORBS=none.
#[test]
fn e3_batch_position_independence() {
    // The bad call rides at index 0, 1, then 2 among good calls: the envelope
    // stays Ok with all_ok:false every time, the bad row always fails closed
    // (no value, error set), good rows always succeed — and each good row's
    // value is bit-identical across all three rotations.
    let temp = tempfile::tempdir().expect("tempdir");
    let good0 = batch_call("g0", "catalog_search", json!({"query": "search"}));
    let good1 = batch_call("g1", "catalog_search", json!({"query": "find"}));
    let bad = batch_call("b", "select", json!({}));
    let rotations = [
        vec![bad.clone(), good0.clone(), good1.clone()],
        vec![good0.clone(), bad.clone(), good1.clone()],
        vec![good0.clone(), good1.clone(), bad],
    ];
    let mut good_values: Vec<(Value, Value)> = Vec::new();
    for (position, calls) in rotations.into_iter().enumerate() {
        let response =
            run_batch(config_at(temp.path()), &batch_request(calls)).expect("envelope stays Ok");
        assert!(!response.all_ok, "position {position}");
        assert_eq!(response.call_count, 3, "position {position}");
        let by_id = |id: &str| {
            response
                .results
                .iter()
                .find(|r| r.id == id)
                .unwrap_or_else(|| panic!("missing row {id} at position {position}"))
        };
        let bad_row = by_id("b");
        assert!(!bad_row.ok, "position {position}");
        assert!(bad_row.value.is_none(), "position {position}");
        assert!(bad_row.error.is_some(), "position {position}");
        let mut values = Vec::new();
        for id in ["g0", "g1"] {
            let row = by_id(id);
            assert!(row.ok, "position {position}: good row {id} tainted");
            assert!(row.error.is_none(), "position {position}: {id}");
            values.push(row.value.clone().expect("good row carries value"));
        }
        good_values.push((values.remove(0), values.remove(0)));
    }
    assert_eq!(good_values[0].0, good_values[1].0);
    assert_eq!(good_values[0].0, good_values[2].0);
    assert_eq!(good_values[0].1, good_values[1].1);
    assert_eq!(good_values[0].1, good_values[2].1);
}

/// INTENT=serial vs parallel mixed batch: same per-id pattern, values, order.
/// KILLS=mode-divergence.
/// ABSORBS=e2_batch_parallel_readonly_isolates_failures (mode pin + per-row isolation legs).
#[test]
fn e3_batch_mode_equivalence() {
    // The same mixed read-only batch under Serial and Parallel resolves to the
    // same per-id ok pattern in the same order with the same good values; only
    // the reported mode string differs.
    let temp = tempfile::tempdir().expect("tempdir");
    let calls = vec![
        batch_call("g0", "catalog_search", json!({"query": "search"})),
        batch_call("i0", "select", json!({})),
        batch_call("o0", "search", json!({})),
        batch_call("g1", "catalog_search", json!({"query": "find"})),
    ];
    let mut serial = batch_request(calls.clone());
    serial.parallel_mode = Some(ParallelMode::Serial);
    let mut parallel = batch_request(calls);
    parallel.parallel_mode = Some(ParallelMode::Parallel);
    let serial = run_batch(config_at(temp.path()), &serial).expect("serial envelope stays Ok");
    let parallel =
        run_batch(config_at(temp.path()), &parallel).expect("parallel envelope stays Ok");
    assert_eq!(serial.mode, "serial");
    assert_eq!(parallel.mode, "parallel");
    assert_eq!(serial.all_ok, parallel.all_ok);
    assert!(!serial.all_ok);
    assert_eq!(serial.results.len(), parallel.results.len());
    for (left, right) in serial.results.iter().zip(parallel.results.iter()) {
        assert_eq!(left.id, right.id);
        assert_eq!(left.ok, right.ok, "row {} disagrees across modes", left.id);
        assert_eq!(left.value, right.value, "row {} value differs", left.id);
        assert_eq!(
            left.error.is_some(),
            right.error.is_some(),
            "row {} error presence differs",
            left.id
        );
    }
    let ok_pattern: Vec<bool> = serial.results.iter().map(|r| r.ok).collect();
    assert_eq!(ok_pattern, vec![true, false, false, true]);
    // Absorbed from e2_batch_parallel_readonly_isolates_failures: ok/error
    // exclusivity per row on both modes — failures stay isolated, ok siblings
    // keep value + no error.
    for response in [&serial, &parallel] {
        for row in &response.results {
            if row.ok {
                assert!(row.value.is_some(), "ok without value: {}", row.id);
                assert!(row.error.is_none(), "ok with error: {}", row.id);
            } else {
                assert!(row.value.is_none(), "fail with value: {}", row.id);
                assert!(row.error.is_some(), "fail without error: {}", row.id);
            }
        }
    }
}

/// INTENT=failing step at any index: same discriminant as direct, count pins prefix.
/// KILLS=position-dependent-discriminant, count-drift.
/// ABSORBS=none.
#[test]
fn e3_plan_position_independence() {
    // The failing step rides at index 0, 1, then 2 among good steps: every plan
    // fails with the same discriminant the direct call raises, call_count pins
    // the executed prefix (fail index + 1), and later steps never run.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut direct = session_at(temp.path());
    let direct_err = direct.call("select", json!({})).expect_err("guard");
    assert!(
        matches!(direct_err, CallError::InvalidArgs(_)),
        "got {direct_err:?}"
    );
    for position in 0..3 {
        let mut steps = Vec::new();
        for index in 0..3 {
            if index == position {
                steps.push(json!({"id": format!("s{index}"), "tool": "select", "args": {}}));
            } else {
                steps.push(
                    json!({"id": format!("s{index}"), "tool": "catalog_search", "args": {"query": "search"}}),
                );
            }
        }
        let plan = parse_plan(&json!({"steps": steps})).expect("plan parses");
        let mut session = session_at(temp.path());
        let err = run_plan(&mut session, &plan).expect_err("plan must fail");
        assert_eq!(
            discriminant(&err),
            discriminant(&direct_err),
            "position {position}: plan discriminant drifted from direct"
        );
        assert_eq!(session.call_count(), position + 1, "position {position}");
    }
}

/// INTENT=per-call ok == direct is_ok, exclusivity per row, all_ok conjunction.
/// KILLS=mirror-divergence, partial-value-smuggle.
/// ABSORBS=e1_batch_per_call_mirrors_direct_discriminants (call_count + id-order legs).
#[test]
fn e3_batch_mirrors_direct_outcomes() {
    // Fail-closed across the batch boundary: each per-call ok flag equals the
    // direct session.call outcome for the same payload, ok/error stay exclusive
    // per row, and all_ok is exactly the conjunction of the direct outcomes.
    let temp = tempfile::tempdir().expect("tempdir");
    let calls: &[(&str, &str, Value)] = &[
        ("g0", "catalog_search", json!({"query": "search"})),
        ("u", "no-such-tool", json!({})),
        ("i", "select", json!({})),
        ("o", "search", json!({})),
        ("g1", "catalog_search", json!({"query": "find"})),
    ];
    let mut session = session_at(temp.path());
    let mut direct_ok = Vec::new();
    for (id, tool, args) in calls {
        let outcome = session.call(tool, args.clone()).is_ok();
        direct_ok.push((*id, outcome));
    }
    let batch_calls: Vec<BatchCall> = calls
        .iter()
        .map(|(id, tool, args)| batch_call(id, tool, args.clone()))
        .collect();
    let response =
        run_batch(config_at(temp.path()), &batch_request(batch_calls)).expect("envelope stays Ok");
    assert_eq!(response.results.len(), direct_ok.len());
    // Absorbed from e1_batch_per_call_mirrors_direct_discriminants: the batch
    // charges one unit per call and echoes ids in input order.
    assert_eq!(response.call_count, 5);
    let ids: Vec<&str> = response.results.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["g0", "u", "i", "o", "g1"]);
    for ((id, expected), row) in direct_ok.iter().zip(response.results.iter()) {
        assert_eq!(&row.id, id);
        assert_eq!(row.ok, *expected, "row {id}: batch disagrees with direct");
        if *expected {
            assert!(row.value.is_some(), "row {id}: ok without value");
            assert!(row.error.is_none(), "row {id}: ok with error");
        } else {
            assert!(row.value.is_none(), "row {id}: fail smuggled a value");
            assert!(row.error.is_some(), "row {id}: fail without error");
        }
    }
    assert_eq!(response.all_ok, direct_ok.iter().all(|(_, ok)| *ok));
    assert!(!response.all_ok);
}

/// INTENT=failed plan is Err with direct discriminant; prefix applied vs failing-edit untouched; remainder resumes.
/// KILLS=Ok-carried-plan-failure, prefix-rollback, partial-edit-write.
/// ABSORBS=e4_plan_prefix_applied_then_remainder_resumes (first half was a duplicate; remainder-resume half appended).
#[test]
fn e3_plan_fail_closed_never_ok() {
    // A failed plan is an Err with the direct call's discriminant — never an Ok
    // carrying failure — and leaves documented state: steps before the failure
    // applied (sequential prefix semantics), the failing edit applied nothing,
    // call_count pins the boundary in both cases.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.txt"), "hello world\n").expect("write");

    let mut direct = session_at(temp.path());
    let direct_err = direct.call("select", json!({})).expect_err("guard");
    let plan = parse_plan(&json!({"steps": [
        {"id": "e", "tool": "edit", "args": {"path": "a.txt", "oldText": "hello world", "newText": "hello mars"}},
        {"id": "s", "tool": "select", "args": {}},
    ]}))
    .expect("plan parses");
    let mut session = session_at(temp.path());
    let err = run_plan(&mut session, &plan).expect_err("plan must fail, not half-succeed");
    assert_eq!(
        discriminant(&err),
        discriminant(&direct_err),
        "plan discriminant drifted from direct"
    );
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    assert_eq!(session.call_count(), 2);
    let body = std::fs::read_to_string(temp.path().join("a.txt")).expect("reread");
    assert!(
        body.contains("hello mars"),
        "executed prefix must apply: {body:?}"
    );
    assert!(
        !body.contains("hello world"),
        "prefix must apply fully: {body:?}"
    );

    // Absorbed remainder-resume half of e4_plan_prefix_applied_then_remainder_resumes
    // (its first half was a line-for-line duplicate of the prefix block above):
    // the same session resumes with the corrected remainder, prefix kept.
    let remainder = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "select", "args": {"value": {"v": 7}, "fields": ["v"]}},
        {"id": "g", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("remainder plan parses");
    let ok = run_plan(&mut session, &remainder).expect("remainder must resume");
    assert!(ok.ok);
    assert_eq!(session.call_count(), 4);
    let body = std::fs::read_to_string(temp.path().join("a.txt")).expect("reread");
    assert!(
        body.contains("hello mars"),
        "resume must keep the prefix: {body:?}"
    );

    std::fs::write(temp.path().join("b.txt"), "keep me\n").expect("write");
    let mut direct = session_at(temp.path());
    let direct_err = direct
        .call(
            "edit",
            json!({"path": "b.txt", "oldText": "not-present-anywhere", "newText": "x"}),
        )
        .expect_err("edit guard");
    assert!(
        matches!(direct_err, CallError::Other(_)),
        "got {direct_err:?}"
    );
    let plan = parse_plan(&json!({"steps": [
        {"id": "g", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "e", "tool": "edit", "args": {"path": "b.txt", "oldText": "not-present-anywhere", "newText": "x"}},
        {"id": "never", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("plan parses");
    let mut session = session_at(temp.path());
    let err = run_plan(&mut session, &plan).expect_err("plan must fail");
    assert_eq!(
        discriminant(&err),
        discriminant(&direct_err),
        "plan discriminant drifted from direct"
    );
    assert_eq!(session.call_count(), 2);
    let body = std::fs::read_to_string(temp.path().join("b.txt")).expect("reread");
    assert_eq!(body, "keep me\n");
}

/// INTENT=bad request at stream index 0/1/2: Result{ok:false}, neighbors identical, Bye reached.
/// KILLS=stream-position-dependence, worker-abort.
/// ABSORBS=none.
#[test]
fn e3_serve_stream_position_independence() {
    // The bad request rides at stream index 0, 1, then 2 among good requests:
    // it always surfaces as Result{ok:false} (never an Error envelope, never a
    // worker abort), neighbors always succeed with identical values across
    // rotations, and the worker always reaches Bye.
    let temp = tempfile::tempdir().expect("tempdir");
    let good = |id: &str| ServeRequest::Call {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    };
    let mut good_values: Vec<Value> = Vec::new();
    for position in 0..3 {
        let mut calls = vec![good("g0"), good("g1"), good("g2")];
        calls.insert(
            position,
            ServeRequest::Call {
                id: "b".to_string(),
                tool: "select".to_string(),
                args: json!({}),
            },
        );
        let mut input = String::new();
        for request in &calls {
            input.push_str(&serve_request_line(request));
        }
        input.push_str(&serve_request_line(&ServeRequest::End));
        let (result, lines) = serve_lines(input, temp.path());
        assert!(result.is_ok(), "position {position}: serve must survive");
        assert_eq!(lines.len(), 5, "position {position}");
        for (line, request) in lines.iter().take(4).zip(calls.iter()) {
            let expected_id = match request {
                ServeRequest::Call { id, .. } => id.as_str(),
                _ => unreachable!("only Call lines here"),
            };
            let response: ServeResponse = serde_json::from_str(line).expect("result line");
            match response {
                ServeResponse::Result {
                    id,
                    ok,
                    value,
                    error,
                } => {
                    assert_eq!(id, expected_id, "position {position}");
                    if expected_id == "b" {
                        assert!(!ok, "position {position}: bad call surfaced as ok");
                        assert!(value.is_none(), "position {position}");
                        assert!(error.is_some(), "position {position}");
                    } else {
                        assert!(ok, "position {position}: good call {id} tainted");
                        assert!(error.is_none(), "position {position}: {id}");
                        if expected_id == "g0" {
                            good_values.push(value.expect("good row carries value"));
                        }
                    }
                }
                other => panic!("position {position}: tool outcome must be Result, got {other:?}"),
            }
        }
        let last: ServeResponse = serde_json::from_str(&lines[4]).expect("bye line");
        assert!(
            matches!(last, ServeResponse::Bye),
            "position {position}: got {last:?}"
        );
    }
    assert_eq!(good_values.len(), 3);
    assert_eq!(good_values[0], good_values[1]);
    assert_eq!(good_values[0], good_values[2]);
}
