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
//! - plan short-circuit on InvalidArgs (variant kept, later steps never run)
//!   -> e2_plan_invalid_args_short_circuits
//! - plan short-circuit on Other (variant kept, later steps never run)
//!   -> e2_plan_other_short_circuits
//! - plan parse/shape failures are InvalidArgs, consume zero budget
//!   -> e2_plan_parse_and_shape_never_touch_budget
//! - batch envelope validation is Err (never Ok with all_ok:false)
//!   -> e2_batch_envelope_validation_is_err
//! - batch parallel path isolates per-call failures, keeps order
//!   -> e2_batch_parallel_readonly_isolates_failures
//! - session budget sticks (same payload, frozen count, exhausted)
//!   -> e2_budget_sticks_on_session
//! - plan aborts on budget with the BudgetExhausted discriminant
//!   -> e2_plan_budget_aborts_with_discriminant
//! - serve maps tool failures to Result (not Error) and keeps serving
//!   -> e2_serve_tool_failures_are_result_and_continues
//! - serve BatchResult mirrors per-call ok/fail, serve continues
//!   -> e2_serve_batch_mixed_propagates_percall
//! - serve budget answers once, ignores trailing input, returns discriminant
//!   -> e2_serve_budget_answers_once_ignores_trailing
//! - Json cause preserved (inner discriminant methods + source)
//!   -> e2_json_cause_preserved_via_downcast
//! - UnknownTool vs InvalidArgs precedence (name first, catalog entry second)
//!   -> e2_unknown_tool_vs_invalid_args_precedence
//! - Ok channel never carries failure (ok:true / all_ok:true on success)
//!   -> e2_ok_channel_never_carries_failure

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, run_serve, BatchCall, BatchRequest, CallError,
    CodeModeSession, ParallelMode, ServeRequest, ServeResponse, SessionConfig,
    MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_plugins::OutputFormat;
use serde_json::{json, Value};
use std::io::Cursor;

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

fn batch_call(id: &str, tool: &str, args: Value) -> BatchCall {
    BatchCall {
        id: id.to_string(),
        tool: tool.to_string(),
        args,
    }
}

fn serve_lines(input: String, root: &std::path::Path) -> (Result<(), CallError>, Vec<String>) {
    let mut out = Vec::new();
    let result = run_serve(config_at(root), Cursor::new(input), &mut out);
    let text = String::from_utf8(out).expect("serve output is utf8");
    let lines = text.lines().map(str::to_string).collect();
    (result, lines)
}

fn serve_request_line(request: &ServeRequest) -> String {
    format!("{}\n", serde_json::to_string(request).expect("request serializes"))
}

/// An anyhow failure wrapped as `CallError::Other` must keep its cause: the
/// std source is present and the original typed cause (here `io::Error`) is
/// still reachable by walking the chain — never flattened to a bare string.
fn assert_other_preserves_io_cause(err: &CallError) {
    let inner = match err {
        CallError::Other(inner) => inner,
        other => panic!("expected Other, got {other:?}"),
    };
    assert!(
        std::error::Error::source(err).is_some(),
        "Other must keep a source"
    );
    assert!(
        inner
            .chain()
            .any(|cause| cause.downcast_ref::<std::io::Error>().is_some()),
        "io cause must survive the wrap"
    );
}

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
    assert_other_preserves_io_cause(&err);

    let mut session = session_at(temp.path());
    let err = session
        .call(
            "edit",
            json!({"path": "no-such-file.rs", "oldText": "a", "newText": "b"}),
        )
        .expect_err("missing file must fail");
    assert_other_preserves_io_cause(&err);
}

#[test]
fn e2_plan_invalid_args_short_circuits() {
    // A mid-plan tool guard failure propagates as the same InvalidArgs variant
    // the direct call raises, and later steps never execute (call_count pins
    // the short-circuit; the third step would otherwise succeed).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut direct = session_at(temp.path());
    let direct_err = direct
        .call("select", json!({}))
        .expect_err("select guard");
    assert!(
        matches!(direct_err, CallError::InvalidArgs(_)),
        "got {direct_err:?}"
    );

    let plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "select", "args": {}},
        {"id": "c", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("plan parses");
    let mut planned = session_at(temp.path());
    let err = run_plan(&mut planned, &plan).expect_err("plan must fail");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    assert_eq!(planned.call_count(), 2);
}

#[test]
fn e2_plan_other_short_circuits() {
    // Same short-circuit contract for the anyhow path: a mid-plan search with
    // no query propagates as Other (matching the direct call), step three
    // never runs, and the failure is an Err — never Ok with ok:false.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut direct = session_at(temp.path());
    let direct_err = direct.call("search", json!({})).expect_err("query guard");
    assert!(
        matches!(direct_err, CallError::Other(_)),
        "got {direct_err:?}"
    );

    let plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "search", "args": {}},
        {"id": "c", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("plan parses");
    let mut planned = session_at(temp.path());
    let err = run_plan(&mut planned, &plan).expect_err("plan must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(planned.call_count(), 2);
}

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

#[test]
fn e2_batch_parallel_readonly_isolates_failures() {
    // The rayon path (all read-only tools, explicit Parallel) keeps the serial
    // propagation contract: envelope stays Ok, per-call failures stay isolated
    // (ok siblings keep value+no-error), ids echo in input order.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut request = batch_request(vec![
        batch_call("g0", "catalog_search", json!({"query": "search"})),
        batch_call("i0", "select", json!({})),
        batch_call("o0", "search", json!({})),
        batch_call("g1", "catalog_search", json!({"query": "find"})),
    ]);
    request.parallel_mode = Some(ParallelMode::Parallel);
    let response = run_batch(config_at(temp.path()), &request).expect("envelope stays Ok");
    assert_eq!(response.mode, "parallel");
    assert!(!response.all_ok);
    assert_eq!(response.call_count, 4);
    let ids: Vec<&str> = response.results.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["g0", "i0", "o0", "g1"]);
    assert!(response.results[0].ok);
    assert!(!response.results[1].ok);
    assert!(!response.results[2].ok);
    assert!(response.results[3].ok);
    for result in &response.results {
        if result.ok {
            assert!(result.value.is_some(), "ok without value: {}", result.id);
            assert!(result.error.is_none(), "ok with error: {}", result.id);
        } else {
            assert!(result.value.is_none(), "fail with value: {}", result.id);
            assert!(result.error.is_some(), "fail without error: {}", result.id);
        }
    }
}

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
    let last: ServeResponse = serde_json::from_str(&lines[10_000]).expect("last line");
    assert!(
        matches!(last, ServeResponse::Result { ok: false, .. }),
        "got {last:?}"
    );
}

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
}

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

#[test]
fn e2_ok_channel_never_carries_failure() {
    // Success shapes attest success: an ok plan returns PlanResult{ok:true},
    // an ok batch returns all_ok:true with value-bearing per-call rows — and
    // the same-shaped failing plan is an Err, never Ok with ok:false.
    let temp = tempfile::tempdir().expect("tempdir");
    let ok_plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("plan parses");
    let mut session = session_at(temp.path());
    let ok = run_plan(&mut session, &ok_plan).expect("ok plan succeeds");
    assert!(ok.ok);
    assert_eq!(ok.call_count, 1);

    let response = run_batch(
        config_at(temp.path()),
        &batch_request(vec![batch_call(
            "g0",
            "catalog_search",
            json!({"query": "search"}),
        )]),
    )
    .expect("ok batch succeeds");
    assert!(response.all_ok);
    assert!(response.results[0].ok);
    assert!(response.results[0].value.is_some());
    assert!(response.results[0].error.is_none());

    let bad_plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "select", "args": {}},
    ]}))
    .expect("plan parses");
    let mut failing = session_at(temp.path());
    assert!(run_plan(&mut failing, &bad_plan).is_err());
}
