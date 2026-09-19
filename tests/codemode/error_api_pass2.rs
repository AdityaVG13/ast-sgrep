//! E2 error-propagation oracles for ast-sgrep-codemode.
//!
//! E1 pins the `CallError` taxonomy TABLE (one row per variant). E2 pins
//! PROPAGATION: a failure raised at any layer must surface at every boundary
//! as the documented variant — never confused, never wrapped into a different
//! variant, never smuggled through the Ok channel. Discriminant assertions via
//! `matches!` / `match` arms / downcast only (never message text).
//! Deterministic, tempfile fixtures, no new deps.
//!
//! Path map (propagation path -> test):
//! - session anyhow cause preserved (io cause in chain, source Some)
//!   -> e2_session_other_preserves_io_cause_chain
//! - plan short-circuit preserving the direct-call variant (InvalidArgs + Other
//!   legs parameterized; later steps never run)
//!   -> e2_plan_short_circuits_preserving_variant
//! - plan parse/shape failures are InvalidArgs, consume zero budget
//!   -> e2_plan_parse_and_shape_never_touch_budget
//! - batch envelope validation is Err (never Ok with all_ok:false)
//!   -> e2_batch_envelope_validation_is_err
//! - session budget sticks (same payload, frozen count, exhausted)
//!   -> e2_budget_sticks_on_session
//! - plan aborts on budget with the BudgetExhausted discriminant
//!   -> e2_plan_budget_aborts_with_discriminant
//! - serve maps tool failures to Result (not Error) and keeps serving
//!   -> e2_serve_tool_failures_are_result_and_continues
//! - serve BatchResult mirrors per-call ok/fail, serve continues
//!   -> e2_serve_batch_mixed_propagates_percall
//! - serve budget answers once, ignores trailing input, returns discriminant
//!   (absorbs the E1 serve-budget discriminant run)
//!   -> e2_serve_budget_answers_once_ignores_trailing
//! - Json cause preserved (inner discriminant methods + source; absorbs the E1
//!   via-? constructor arm as the single Json pin)
//!   -> e2_json_cause_preserved_via_downcast
//! - UnknownTool vs InvalidArgs precedence (name first, catalog entry second)
//!   -> e2_unknown_tool_vs_invalid_args_precedence
//!
//! Folded out / deleted (not here):
//! - e2_plan_other_short_circuits -> e2_plan_short_circuits_preserving_variant (same file)
//! - e2_batch_parallel_readonly_isolates_failures -> e3_batch_mode_equivalence (pass3)
//! - e2_ok_channel_never_carries_failure -> DELETED (no unique error mutant)

#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, BatchCall, CallError, ServeRequest, ServeResponse,
    MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use error_testkit::{
    assert_other_preserves_cause, batch_call, batch_request, config_at, serve_lines,
    serve_request_line, session_at,
};
use serde_json::{json, Value};

/// INTENT=Other keeps std source + io::Error reachable in anyhow chain.
/// KILLS=cause-flatten-to-string, source-drop.
/// ABSORBS=none.
#[test]
fn e2_session_other_preserves_io_cause_chain() {
    // Two pre-execution IO failures with typed causes: a search against a
    // missing session root (canonicalize fails) and an edit of a missing file
    // (read fails). Both must be Other with the io::Error reachable in-chain.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut missing_root = session_at(&temp.path().join("no-such-dir"));
    let err = missing_root
        .call("search", json!({"query": "hello"}))
        .expect_err("missing root must fail");
    assert_other_preserves_cause::<std::io::Error>(&err);

    let mut session = session_at(temp.path());
    let err = session
        .call(
            "edit",
            json!({"path": "no-such-file.rs", "oldText": "a", "newText": "b"}),
        )
        .expect_err("missing file must fail");
    assert_other_preserves_cause::<std::io::Error>(&err);
}

/// INTENT=mid-plan failure keeps its direct-call variant and later steps never run (InvalidArgs + Other).
/// KILLS=variant-rewrap, no-short-circuit.
/// ABSORBS=e2_plan_other_short_circuits (same short-circuit contract, second variant; parameterized).
#[test]
fn e2_plan_short_circuits_preserving_variant() {
    // A mid-plan tool failure propagates as the same variant the direct call
    // raises, and later steps never execute (call_count pins the short-circuit;
    // the third step would otherwise succeed). One leg per variant family: a
    // pure-tool guard failure (InvalidArgs) and an anyhow-path failure (Other,
    // which must surface as Err — never Ok with ok:false).
    let temp = tempfile::tempdir().expect("tempdir");
    let cases: &[(&str, Value, &str)] = &[
        ("select", json!({}), "invalid_args"),
        ("search", json!({}), "other"),
    ];
    for (tool, args, expected) in cases {
        let mut direct = session_at(temp.path());
        let direct_err = direct
            .call(tool, args.clone())
            .expect_err("direct guard must fail");
        match *expected {
            "invalid_args" => assert!(
                matches!(direct_err, CallError::InvalidArgs(_)),
                "got {direct_err:?}"
            ),
            "other" => assert!(
                matches!(direct_err, CallError::Other(_)),
                "got {direct_err:?}"
            ),
            _ => unreachable!("unknown variant leg"),
        }

        let plan = parse_plan(&json!({"steps": [
            {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
            {"id": "b", "tool": tool, "args": args},
            {"id": "c", "tool": "catalog_search", "args": {"query": "search"}},
        ]}))
        .expect("plan parses");
        let mut planned = session_at(temp.path());
        let err = run_plan(&mut planned, &plan).expect_err("plan must fail");
        match *expected {
            "invalid_args" => {
                assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}")
            }
            "other" => assert!(matches!(err, CallError::Other(_)), "got {err:?}"),
            _ => unreachable!("unknown variant leg"),
        }
        assert_eq!(planned.call_count(), 2, "tool {tool}: later steps never run");
    }
}

/// INTENT=malformed/empty plans fail InvalidArgs with zero budget consumed.
/// KILLS=parse-failure-charges-budget, variant-swap.
/// ABSORBS=none.
#[test]
fn e2_plan_parse_and_shape_never_touch_budget() {
    // Malformed plans fail at parse time as InvalidArgs (never Other/Json),
    // and the empty-steps shape guard fails in run_plan the same way. None of
    // them may consume session budget: call_count stays zero throughout.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    for bad in [
        json!({"steps": "nope"}),
        json!({}),
        json!({"steps": [{"id": "a"}]}),
    ] {
        let err = parse_plan(&bad).expect_err("malformed plan must fail");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    }
    let empty = parse_plan(&json!({"steps": []})).expect("empty plan parses");
    let err = run_plan(&mut session, &empty).expect_err("empty plan must fail");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    assert_eq!(session.call_count(), 0);
}

/// INTENT=envelope-shape violations are Err InvalidArgs, never Ok+all_ok:false.
/// KILLS=envelope-violation-as-per-call-row, Ok-carried-failure.
/// ABSORBS=none.
#[test]
fn e2_batch_envelope_validation_is_err() {
    // Envelope-shape failures (empty/oversize calls, bad identity) are Err
    // InvalidArgs from run_batch itself — never Ok with all_ok:false and never
    // a per-call row. Each arm pins the same discriminant direct guards use.
    let temp = tempfile::tempdir().expect("tempdir");
    let many: Vec<BatchCall> = (0..MAX_BATCH_CALLS + 1)
        .map(|i| batch_call(&format!("c{i}"), "catalog_search", json!({"query": "x"})))
        .collect();
    let long_id = "i".repeat(MAX_BATCH_ID_BYTES + 1);
    let long_tool = "t".repeat(MAX_BATCH_TOOL_BYTES + 1);
    let cases: Vec<Vec<BatchCall>> = vec![
        vec![],
        many,
        vec![batch_call("", "catalog_search", json!({"query": "x"}))],
        vec![batch_call("ok", "", json!({}))],
        vec![batch_call(&long_id, "catalog_search", json!({"query": "x"}))],
        vec![batch_call("ok", &long_tool, json!({}))],
    ];
    for calls in &cases {
        let err = run_batch(config_at(temp.path()), &batch_request(calls.clone()))
            .expect_err("envelope violation must be Err");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    }
}

/// INTENT=spent budget sticky: same payload, frozen count, exhausted.
/// KILLS=budget-degrade-to-Other, counter-drift.
/// ABSORBS=none.
#[test]
fn e2_budget_sticks_on_session() {
    // A spent session budget is sticky: every further call fails with the same
    // BudgetExhausted payload (never degrading to Other/InvalidArgs), the
    // counter freezes, and exhausted() reports the terminal state.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 2;
    assert!(!session.exhausted());
    for _ in 0..2 {
        assert!(
            session
                .call("catalog_search", json!({"query": "search"}))
                .is_ok()
        );
    }
    assert_eq!(session.call_count(), 2);
    assert!(session.exhausted());
    for _ in 0..2 {
        let err = session
            .call("catalog_search", json!({"query": "search"}))
            .expect_err("spent budget must fail");
        assert!(
            matches!(err, CallError::BudgetExhausted(2)),
            "got {err:?}"
        );
    }
    assert_eq!(session.call_count(), 2);
    assert!(session.exhausted());
}

/// INTENT=budget exhaustion inside plan aborts BudgetExhausted, unstarted step uncharged.
/// KILLS=budget-wrap-as-Other, overcharge.
/// ABSORBS=none.
#[test]
fn e2_plan_budget_aborts_with_discriminant() {
    // Budget exhaustion inside a plan aborts with BudgetExhausted (matching
    // the direct-call discriminant), not wrapped as Other — and the unstarted
    // step never consumes budget.
    let temp = tempfile::tempdir().expect("tempdir");
    let plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("plan parses");
    let mut session = session_at(temp.path());
    session.max_calls = 1;
    let err = run_plan(&mut session, &plan).expect_err("plan must stop at budget");
    assert!(matches!(err, CallError::BudgetExhausted(1)), "got {err:?}");
    assert_eq!(session.call_count(), 1);
    assert!(session.exhausted());
}

/// INTENT=serve maps tool failures to Result{ok:false}, keeps serving to Bye.
/// KILLS=Result-vs-Error-swap, worker-abort.
/// ABSORBS=none.
#[test]
fn e2_serve_tool_failures_are_result_and_continues() {
    // Tool-level InvalidArgs and Other failures over serve are per-request
    // Result{ok:false} lines (never Error envelopes, never Err), the worker
    // keeps serving the mixed stream, and End still gets its Bye.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut input = serve_request_line(&ServeRequest::Call {
        id: "i0".to_string(),
        tool: "select".to_string(),
        args: json!({}),
    });
    input.push_str(&serve_request_line(&ServeRequest::Call {
        id: "o0".to_string(),
        tool: "search".to_string(),
        args: json!({}),
    }));
    input.push_str(&serve_request_line(&ServeRequest::Call {
        id: "g0".to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    }));
    input.push_str(&serve_request_line(&ServeRequest::End));
    let (result, lines) = serve_lines(input, temp.path());
    assert!(result.is_ok(), "serve survives tool failures");
    assert_eq!(lines.len(), 4);
    for (line, expect_ok) in lines.iter().take(3).zip([false, false, true]) {
        let response: ServeResponse = serde_json::from_str(line).expect("result line");
        match response {
            ServeResponse::Result { ok, value, error, .. } => {
                assert_eq!(ok, expect_ok);
                assert_eq!(value.is_some(), expect_ok);
                assert_eq!(error.is_some(), !expect_ok);
            }
            other => panic!("tool failure must be Result, got {other:?}"),
        }
    }
    let last: ServeResponse = serde_json::from_str(&lines[3]).expect("bye line");
    assert!(matches!(last, ServeResponse::Bye), "got {last:?}");
}

/// INTENT=serve BatchResult mirrors per-call ok/fail, worker continues to Bye.
/// KILLS=batch-abort-on-first-failure, percall-flatten.
/// ABSORBS=none.
/// OVERLAP=e4 serve-batch drill (E2 is the cheap field-level pin).
#[test]
fn e2_serve_batch_mixed_propagates_percall() {
    // A well-shaped mixed batch over serve yields one BatchResult with
    // all_ok:false and per-call ok flags mirroring direct success/failure;
    // the worker continues to Bye instead of terminating on the failure.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut input = serve_request_line(&ServeRequest::Batch {
        id: "b0".to_string(),
        calls: vec![
            batch_call("g0", "catalog_search", json!({"query": "search"})),
            batch_call("i0", "select", json!({})),
        ],
        parallel_mode: None,
    });
    input.push_str(&serve_request_line(&ServeRequest::End));
    let (result, lines) = serve_lines(input, temp.path());
    assert!(result.is_ok(), "serve survives mixed batch");
    assert_eq!(lines.len(), 2);
    // BatchResult carries wall_ms: u128, which serde_json cannot deserialize;
    // pin the envelope via its `type` tag discriminant plus field shapes.
    let first: Value = serde_json::from_str(&lines[0]).expect("batch line");
    assert_eq!(
        first.get("type").and_then(Value::as_str),
        Some("batch_result")
    );
    assert_eq!(first.get("id").and_then(Value::as_str), Some("b0"));
    assert_eq!(first.get("all_ok").and_then(Value::as_bool), Some(false));
    assert_eq!(first.get("mode").and_then(Value::as_str), Some("serial"));
    let results = first
        .get("results")
        .and_then(Value::as_array)
        .expect("results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].get("ok").and_then(Value::as_bool), Some(true));
    assert!(results[0].get("value").is_some());
    assert!(results[0].get("error").is_none());
    assert_eq!(results[1].get("ok").and_then(Value::as_bool), Some(false));
    assert!(results[1].get("value").is_none());
    assert!(results[1].get("error").is_some());
    let last: ServeResponse = serde_json::from_str(&lines[1]).expect("bye line");
    assert!(matches!(last, ServeResponse::Bye), "got {last:?}");
}

/// INTENT=serve fail-once: one Result{ok:false}, BudgetExhausted return, trailing input silent.
/// KILLS=error-flood-past-budget, trailing-Bye.
/// ABSORBS=e1_budget_exhausted_serve_returns_discriminant (first-line ok:true pin; duplicate 10k run deleted).
#[test]
fn e2_serve_budget_answers_once_ignores_trailing() {
    // Fail-once: the 10_001st call gets exactly one Result{ok:false}, the
    // worker returns BudgetExhausted(10_000), and trailing input (further
    // calls plus End) produces no more lines — no flood, no Bye.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut input = String::new();
    for i in 0..10_001 {
        input.push_str(&serve_request_line(&ServeRequest::Call {
            id: format!("c{i}"),
            tool: "select".to_string(),
            args: json!({"value": {"v": i}, "fields": ["v"]}),
        }));
    }
    input.push_str(&serve_request_line(&ServeRequest::Call {
        id: "trailing".to_string(),
        tool: "select".to_string(),
        args: json!({"value": {"v": 1}, "fields": ["v"]}),
    }));
    input.push_str(&serve_request_line(&ServeRequest::End));
    let (result, lines) = serve_lines(input, temp.path());
    let err = result.expect_err("serve must stop past budget");
    assert!(
        matches!(err, CallError::BudgetExhausted(10_000)),
        "got {err:?}"
    );
    assert_eq!(lines.len(), 10_001);
    // Absorbed from e1_budget_exhausted_serve_returns_discriminant (the one pin
    // its deleted 10k run added beyond this anchor): the stream opens with a
    // success row before the single fail-once row.
    let first: ServeResponse = serde_json::from_str(&lines[0]).expect("first line");
    assert!(
        matches!(first, ServeResponse::Result { ok: true, .. }),
        "got {first:?}"
    );
    let last: ServeResponse = serde_json::from_str(&lines[10_000]).expect("last line");
    assert!(
        matches!(last, ServeResponse::Result { ok: false, .. }),
        "got {last:?}"
    );
}

/// INTENT=Json keeps inner serde error: downcast, syntax/eof discriminants, transparent Display, via-? arm.
/// KILLS=BEHAVIOR-ONLY (synthetic: constructs From over synthetic serde errors; no production call path yields Json).
/// ABSORBS=e1_json_row_from_conversion (via-? arm + ok control; the sole Json pin).
#[test]
fn e2_json_cause_preserved_via_downcast() {
    // The Json variant keeps the original serde_json::Error intact: callers can
    // downcast to it and read its own discriminants (syntax vs eof via both
    // is_* and classify Category), and transparent Display delegates to the
    // preserved cause. Never confused with InvalidArgs/Other.
    let syntax = serde_json::from_str::<Value>(r#"{"a": }"#).expect_err("bad json");
    assert!(syntax.is_syntax());
    let err = CallError::from(syntax);
    match &err {
        CallError::Json(inner) => {
            assert!(inner.is_syntax());
            assert!(matches!(
                inner.classify(),
                serde_json::error::Category::Syntax
            ));
            // Transparent Display delegates to the preserved cause.
            assert_eq!(err.to_string(), inner.to_string());
        }
        other => panic!("expected Json, got {other:?}"),
    }

    let eof = serde_json::from_str::<Value>("").expect_err("empty json");
    assert!(eof.is_eof());
    let err = CallError::from(eof);
    match &err {
        CallError::Json(inner) => {
            assert!(inner.is_eof());
            assert!(matches!(inner.classify(), serde_json::error::Category::Eof));
            assert_eq!(err.to_string(), inner.to_string());
        }
        other => panic!("expected Json, got {other:?}"),
    }

    // Absorbed from e1_json_row_from_conversion: the `?` conversion path lands
    // on Json too, and well-formed input stays Ok (control).
    fn load(text: &str) -> Result<Value, CallError> {
        Ok(serde_json::from_str(text)?)
    }
    let err = load("{oops").expect_err("bad json via ?");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    assert!(load(r#"{"ok":true}"#).is_ok());
}

/// INTENT=unknown name beats garbage args; known tool + bad args (incl. unknown catalog entry) is InvalidArgs.
/// KILLS=dispatch-order-swap, catalog-entry-as-UnknownTool.
/// ABSORBS=none.
#[test]
fn e2_unknown_tool_vs_invalid_args_precedence() {
    // Dispatch order pins the confusion matrix: an unknown name wins over any
    // arg shape (UnknownTool even with garbage args), while a known tool with
    // bad args — including an unknown *catalog entry* name — is InvalidArgs.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let err = session
        .call("no-such-tool", json!({"fields": 42}))
        .expect_err("unknown tool");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");

    let err = session.call("select", json!({})).expect_err("guard");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");

    let err = session
        .call("catalog_describe", json!({"name": "no-such-tool"}))
        .expect_err("unknown catalog entry");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
}
