//! Error-API matrix for `ast-sgrep-codemode`.
//!
//! F35 collapse of `error_api_pass{1,2,3,4}.rs` into one parametric target.
//! Pure collapse: every test below is unchanged from its pass-file original —
//! same triggers, same assertions, same INTENT/KILLS/ABSORBS audit lines. Only
//! the file layout changed (one import block, E1-E4 sections). Per-pass module
//! docs are kept as section banners; shared setup lives in
//! `tests/codemode/error_testkit.rs` (canonical beats promoted to `ast-sgrep-testkit`).
//!
//! Matrix (mission -> test):
//! - E1 taxonomy: `e1_unknown_tool_row_direct_plan_batch_serve`, `e1_invalid_args_pure_tool_guard_gaps`, `e1_invalid_args_plan_ref_and_shape_gaps`, `e1_invalid_args_serve_envelope_not_err`, `e1_invalid_args_unknown_format_rejected_pre_io`, `e1_other_search_find_chain_query_validation`, `e1_other_unknown_lang_rejected_pre_io`, `e1_other_read_ref_shape_and_jail`, `e1_other_edit_shape_and_uniqueness`, `e1_other_index_repo_targeted_shape`
//! - E2 propagation: `e2_session_other_preserves_io_cause_chain`, `e2_plan_short_circuits_preserving_variant`, `e2_plan_parse_and_shape_never_touch_budget`, `e2_batch_envelope_validation_is_err`, `e2_budget_sticks_on_session`, `e2_plan_budget_aborts_with_discriminant`, `e2_serve_tool_failures_are_result_and_continues`, `e2_serve_batch_mixed_propagates_percall`, `e2_serve_budget_answers_once_ignores_trailing`, `e2_json_cause_preserved_via_downcast`, `e2_unknown_tool_vs_invalid_args_precedence`
//! - E3 relations: `e3_dispatch_equivalence`, `e3_repeat_bad_call_deterministic`, `e3_batch_position_independence`, `e3_batch_mode_equivalence`, `e3_plan_position_independence`, `e3_batch_mirrors_direct_outcomes`, `e3_plan_fail_closed_never_ok`, `e3_serve_stream_position_independence`
//! - E4 drills: `e4_serve_batch_fault_mid_stream_isolates_and_resumes`, `e4_unknown_tool_in_plan_resumes_with_corrected_plan`, `e4_budget_exhaustion_mid_plan_sticks_terminal`, `e4_deleted_root_fault_scoped_and_recovers_after_repair`, `e4_atomic_edit_batch_zero_writes_then_resumes`, `e4_chained_double_fault_unknown_then_budget`, `e4_oversized_response_fault_then_resumes`
//!
//! No asserted behavior changed in this collapse (see F35: keep every distinct
//! behavior assertion, drop pure duplication — no pure duplication remained;
//! net test count is unchanged).
#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_codemode::tools::call_tool;
use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, BatchCall, CallError, ParallelMode, ServeRequest,
    ServeResponse, MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_core::MAX_QUERY_CHARS;
use error_testkit::{
    assert_dispatch_equivalence, assert_other_preserves_cause, batch_call, batch_request,
    config_at, discriminant, indexed_session_at, serve_lines, serve_request_line, session_at,
};
use serde_json::{json, Value};

// ── E1 taxonomy inventory (from error_api_pass1.rs) ──
// E1 error-taxonomy inventory for ast-sgrep-codemode.
//
// Pins the full `CallError` taxonomy TABLE with hand-built triggers, one row
// per variant, discriminant assertions via `matches!` only (never message
// text). Oracle passes already pin the table's core rows (unknown-tool
// dispatch, plan/batch shape guards, budget boundary, missing/escaping root,
// oversized response); every test here adds at least one previously unpinned
// discriminant. Deterministic, tempfile fixtures, no new deps.
//
// Row map (variant -> site -> test):
// - UnknownTool: dispatch (direct/plan/batch-per-call/serve-per-request)
//   -> e1_unknown_tool_row_direct_plan_batch_serve
// - InvalidArgs: pure-tool guards -> e1_invalid_args_pure_tool_guard_gaps
// - InvalidArgs: plan $ref arms -> e1_invalid_args_plan_ref_and_shape_gaps
// - InvalidArgs: serve maps to Error envelopes, never Err
//   -> e1_invalid_args_serve_envelope_not_err
// - InvalidArgs: unknown search format (pre-IO)
//   -> e1_invalid_args_unknown_format_rejected_pre_io
// - Other: search/find/chain query validation
//   -> e1_other_search_find_chain_query_validation
// - Other: unknown lang (pre-IO) -> e1_other_unknown_lang_rejected_pre_io
// - Other: read ref shape + jail -> e1_other_read_ref_shape_and_jail
// - Other: edit shape + uniqueness -> e1_other_edit_shape_and_uniqueness
// - Other: index_repo targeted shape -> e1_other_index_repo_targeted_shape
//
// Folded out (MERGE verdicts; pinned at their anchors, not here):
// - BudgetExhausted serve discriminant -> e2_serve_budget_answers_once_ignores_trailing (pass2)
// - Json via-? arm -> e2_json_cause_preserved_via_downcast (pass2)
// - batch per-call mirroring -> e3_batch_mirrors_direct_outcomes (pass3)
//

/// INTENT=UnknownTool row on direct/plan/batch/serve surfaces.
/// KILLS=variant-swap(UnknownTool→InvalidArgs), serve-Err-instead-of-Result.
/// ABSORBS=none.
#[test]
fn e1_unknown_tool_row_direct_plan_batch_serve() {
    // UnknownTool row: direct dispatch, plan-step propagation, batch per-call
    // mirroring (Ok envelope), and the serve gap — an unknown tool on the
    // sticky path is a per-request Result{ok:false}, never an Error or Err.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let err = session
        .call("no-such-tool", json!({"query": "x"}))
        .expect_err("unknown tool");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");

    let plan = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "no-such-tool", "args": {}},
    ]}))
    .expect("plan parses");
    let mut planned = session_at(temp.path());
    let err = run_plan(&mut planned, &plan).expect_err("plan unknown step");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert_eq!(planned.call_count(), 2);

    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![batch_call("u", "no-such-tool", json!({}))]),
    )
    .expect("batch envelope stays Ok");
    assert!(!batch.all_ok);
    assert!(!batch.results[0].ok);
    assert!(batch.results[0].value.is_none());
    assert!(batch.results[0].error.is_some());

    let mut input = serve_request_line(&ServeRequest::Call {
        id: "u".to_string(),
        tool: "no-such-tool".to_string(),
        args: json!({}),
    });
    input.push_str(&serve_request_line(&ServeRequest::End));
    let (result, lines) = serve_lines(input, temp.path());
    assert!(result.is_ok(), "serve survives dispatch failures");
    assert_eq!(lines.len(), 2);
    let first: ServeResponse = serde_json::from_str(&lines[0]).expect("result line");
    assert!(
        matches!(first, ServeResponse::Result { ok: false, .. }),
        "got {first:?}"
    );
    let last: ServeResponse = serde_json::from_str(&lines[1]).expect("bye line");
    assert!(matches!(last, ServeResponse::Bye), "got {last:?}");
}

/// INTENT=7 pure-tool guard arms yield InvalidArgs.
/// KILLS=guard-drop per tool.
/// ABSORBS=none.
#[test]
fn e1_invalid_args_pure_tool_guard_gaps() {
    // Pure-tool guard arms the oracle taxonomy test does not pin: callers
    // symbol guards, blank (whitespace-only) module/name guards, select
    // fields wrong-type and all-non-string arms, scalar hits for filter_hits.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let cases: &[(&str, Value)] = &[
        ("callers", json!({})),
        ("callers", json!({"symbol": "  "})),
        ("imports", json!({"module": "   "})),
        ("catalog_describe", json!({"name": "  "})),
        ("select", json!({"value": {"a": 1}, "fields": "a"})),
        ("select", json!({"value": {"a": 1}, "fields": [1, 2]})),
        ("filter_hits", json!({"hits": 42})),
    ];
    for (tool, args) in cases {
        let err = session.call(tool, args.clone()).expect_err("guard");
        assert!(
            matches!(err, CallError::InvalidArgs(_)),
            "tool {tool}: got {err:?}"
        );
    }
}

/// INTENT=plan $ref index-required arms + null-args guard yield InvalidArgs.
/// KILLS=ref-arm-swap, null-args-panic.
/// ABSORBS=none.
/// OVERLAP=oracle ref matrix (pins gaps only).
#[test]
fn e1_invalid_args_plan_ref_and_shape_gaps() {
    // Plan arms the oracle ref matrix does not pin: non-numeric index into an
    // array ("array index required") in both args and return position, and a
    // pure tool step with default (Null) args tripping its arg guard.
    let temp = tempfile::tempdir().expect("tempdir");
    let indexed = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "$a.tools.name"}},
    ]}))
    .expect("index-required plan parses");
    let mut session = session_at(temp.path());
    let err = run_plan(&mut session, &indexed).expect_err("args index required");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");

    let via_return = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ], "return": "$a.tools.name"}))
    .expect("return index-required plan parses");
    let mut session = session_at(temp.path());
    let err = run_plan(&mut session, &via_return).expect_err("return index required");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");

    let null_args = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search"},
    ]}))
    .expect("null-args plan parses");
    let mut session = session_at(temp.path());
    let err = run_plan(&mut session, &null_args).expect_err("null args guard");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
}

/// INTENT=serve maps bad identity/batch shape to Error envelopes, keeps serving.
/// KILLS=serve-abort-on-validation, Error-vs-Result-swap.
/// ABSORBS=none.
#[test]
fn e1_invalid_args_serve_envelope_not_err() {
    // Serve maps request validation failures (bad identity, bad batch shape)
    // to Error envelopes and keeps serving; only budget/IO ends the worker.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut bad_identity = serve_request_line(&ServeRequest::Call {
        id: String::new(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "x"}),
    });
    bad_identity.push_str(&serve_request_line(&ServeRequest::End));
    let (result, lines) = serve_lines(bad_identity, temp.path());
    assert!(result.is_ok(), "serve survives bad identity");
    assert_eq!(lines.len(), 2);
    let first: ServeResponse = serde_json::from_str(&lines[0]).expect("error line");
    assert!(
        matches!(first, ServeResponse::Error { .. }),
        "got {first:?}"
    );

    let mut bad_batch = serve_request_line(&ServeRequest::Batch {
        id: "b0".to_string(),
        calls: vec![],
        parallel_mode: None,
    });
    bad_batch.push_str(&serve_request_line(&ServeRequest::End));
    let (result, lines) = serve_lines(bad_batch, temp.path());
    assert!(result.is_ok(), "serve survives bad batch");
    assert_eq!(lines.len(), 2);
    let first: ServeResponse = serde_json::from_str(&lines[0]).expect("error line");
    match first {
        ServeResponse::Error { id, .. } => assert_eq!(id.as_deref(), Some("b0")),
        other => panic!("got {other:?}"),
    }
}

/// INTENT=missing/overlong query on search/find/chain yields Other pre-IO.
/// KILLS=variant-swap(Other→InvalidArgs), validation-after-IO.
/// ABSORBS=none.
#[test]
fn e1_other_search_find_chain_query_validation() {
    // Bound-tool query validation surfaces as Other (anyhow), pre-IO: missing
    // query for search/find/chain plus overlong (>MAX_QUERY_CHARS) queries.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let overlong = "x".repeat(MAX_QUERY_CHARS + 1);
    let cases: &[(&str, Value)] = &[
        ("search", json!({})),
        ("search", json!({"query": overlong})),
        ("find", json!({})),
        ("find", json!({"query": overlong})),
        ("chain", json!({})),
        ("chain", json!({"query": overlong})),
    ];
    for (tool, args) in cases {
        let err = session.call(tool, args.clone()).expect_err("query guard");
        assert!(
            matches!(err, CallError::Other(_)),
            "tool {tool}: got {err:?}"
        );
    }
}

/// INTENT=unknown search format fails closed as InvalidArgs, pre-IO.
/// KILLS=format-fallback-swallow (typo silently served as session default).
/// ABSORBS=none.
/// OVERLAP=oracle_surface format table (pins OutputFormat::parse only).
#[test]
fn e1_invalid_args_unknown_format_rejected_pre_io() {
    // Unknown `format` on the search path fails closed as InvalidArgs
    // before any index work, on both the search path and the
    // find-forwards-to-search path. Absent `format` still means the
    // session default (no error); every shipped spelling still renders.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    for tool in ["search", "find"] {
        let err = session
            .call(tool, json!({"query": "auth", "format": "notafmt"}))
            .expect_err("unknown format");
        assert!(
            matches!(err, CallError::InvalidArgs(_)),
            "tool {tool}: got {err:?}"
        );
        assert!(
            err.to_string().contains("unknown format"),
            "tool {tool}: {err}"
        );
    }
    // Shipped spellings still render: one indexed fixture, all five arms.
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("hello.py"), "def hello():\n    return 1\n")
        .expect("write");
    let (_idx, mut rendered) = indexed_session_at(fixture.path());
    for format in ["agent", "capsule", "agent-capsule", "llm", "ai"] {
        rendered
            .call("search", json!({"query": "hello", "format": format}))
            .expect("shipped format must render");
    }
}

/// INTENT=unknown lang on search/find yields Other before index work.
/// KILLS=lang-fallback-swallow.
/// ABSORBS=none.
/// OVERLAP=text pinned elsewhere (discriminant new).
#[test]
fn e1_other_unknown_lang_rejected_pre_io() {
    // Unknown lang fails closed as Other before any index work, on both the
    // search path and the find-forwards-to-search path. Existing coverage
    // asserts message text only.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    for tool in ["search", "find"] {
        let err = session
            .call(tool, json!({"query": "auth", "lang": "notalang"}))
            .expect_err("unknown lang");
        assert!(
            matches!(err, CallError::Other(_)),
            "tool {tool}: got {err:?}"
        );
    }
}

/// INTENT=read ref-shape + jail arms yield Other.
/// KILLS=guard-drop, jail-drop.
/// ABSORBS=none.
#[test]
fn e1_other_read_ref_shape_and_jail() {
    // Read ref-shape arms surface as Other: >32 windows, unparseable ref
    // start/end/range, non-string non-object ref, and '..' jail rejection.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let many: Vec<Value> = (0..33)
        .map(|i| json!({"path": format!("f{i}.rs"), "start": 1, "end": 1}))
        .collect();
    let cases: &[Value] = &[
        json!({"refs": many}),
        json!({"ref": "f.rs#Lx"}),
        json!({"ref": "f.rs#L5-L2"}),
        json!({"ref": 42}),
        json!({"path": "../escape.rs", "start": 1, "end": 1}),
    ];
    for args in cases {
        let err = session.call("read", args.clone()).expect_err("read guard");
        assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    }
}

/// INTENT=edit shape/uniqueness/jail arms yield Other, file untouched.
/// KILLS=guard-drop, write-before-validate.
/// ABSORBS=none.
#[test]
fn e1_other_edit_shape_and_uniqueness() {
    // Edit shape + uniqueness arms surface as Other: empty oldText, zero
    // matches, 2+ matches (discriminant; text pinned elsewhere), >16 edits,
    // non-object edits[] entry, and '..' jail rejection.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("hello.py"), "def hello():\n    return 1\n").expect("write");
    let mut session = session_at(temp.path());
    let many: Vec<Value> = (0..17)
        .map(|_| json!({"path": "hello.py", "oldText": "return 1", "newText": "x"}))
        .collect();
    let cases: &[Value] = &[
        json!({"path": "hello.py", "oldText": "", "newText": "x"}),
        json!({"path": "hello.py", "oldText": "not-present-anywhere", "newText": "x"}),
        json!({"path": "hello.py", "oldText": "e", "newText": "x"}),
        json!({"edits": many}),
        json!({"edits": [42]}),
        json!({"path": "../x.py", "oldText": "a", "newText": "b"}),
    ];
    for args in cases {
        let err = session.call("edit", args.clone()).expect_err("edit guard");
        assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    }
    let body = std::fs::read_to_string(temp.path().join("hello.py")).expect("reread");
    assert_eq!(body, "def hello():\n    return 1\n");
}

/// INTENT=index_repo targeted-shape arms yield Other.
/// KILLS=guard-drop, traversal-accept.
/// ABSORBS=none.
#[test]
fn e1_other_index_repo_targeted_shape() {
    // index_repo targeted-shape arms surface as Other: force+paths exclusive,
    // non-array/empty/non-string/empty-entry paths, traversal, outside root.
    // Traversal/outside-root discriminants are new (text pinned elsewhere).
    let root = tempfile::tempdir().expect("root");
    std::fs::write(root.path().join("a.rs"), "fn a() {}\n").expect("write");
    let outside = tempfile::tempdir().expect("outside");
    let escaped = outside.path().join("outside.rs");
    let mut session = session_at(root.path());
    let cases: &[Value] = &[
        json!({"force": true, "paths": ["a.rs"]}),
        json!({"paths": "a.rs"}),
        json!({"paths": []}),
        json!({"paths": [42]}),
        json!({"paths": [""]}),
        json!({"paths": ["../t.rs"]}),
        json!({"paths": [escaped.to_str().expect("utf8")]}),
    ];
    for args in cases {
        let err = session
            .call("index_repo", args.clone())
            .expect_err("paths guard");
        assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    }
}

// ── E2 propagation oracles (from error_api_pass2.rs) ──
// E2 error-propagation oracles for ast-sgrep-codemode.
//
// E1 pins the `CallError` taxonomy TABLE (one row per variant). E2 pins
// PROPAGATION: a failure raised at any layer must surface at every boundary
// as the documented variant — never confused, never wrapped into a different
// variant, never smuggled through the Ok channel. Discriminant assertions via
// `matches!` / `match` arms / downcast only (never message text).
// Deterministic, tempfile fixtures, no new deps.
//
// Path map (propagation path -> test):
// - session anyhow cause preserved (io cause in chain, source Some)
//   -> e2_session_other_preserves_io_cause_chain
// - plan short-circuit preserving the direct-call variant (InvalidArgs + Other
//   legs parameterized; later steps never run)
//   -> e2_plan_short_circuits_preserving_variant
// - plan parse/shape failures are InvalidArgs, consume zero budget
//   -> e2_plan_parse_and_shape_never_touch_budget
// - batch envelope validation is Err (never Ok with all_ok:false)
//   -> e2_batch_envelope_validation_is_err
// - session budget sticks (same payload, frozen count, exhausted)
//   -> e2_budget_sticks_on_session
// - plan aborts on budget with the BudgetExhausted discriminant
//   -> e2_plan_budget_aborts_with_discriminant
// - serve maps tool failures to Result (not Error) and keeps serving
//   -> e2_serve_tool_failures_are_result_and_continues
// - serve BatchResult mirrors per-call ok/fail, serve continues
//   -> e2_serve_batch_mixed_propagates_percall
// - serve budget answers once, ignores trailing input, returns discriminant
//   (absorbs the E1 serve-budget discriminant run)
//   -> e2_serve_budget_answers_once_ignores_trailing
// - Json cause preserved (inner discriminant methods + source; absorbs the E1
//   via-? constructor arm as the single Json pin)
//   -> e2_json_cause_preserved_via_downcast
// - UnknownTool vs InvalidArgs precedence (name first, catalog entry second)
//   -> e2_unknown_tool_vs_invalid_args_precedence
//
// Folded out / deleted (not here):
// - e2_plan_other_short_circuits -> e2_plan_short_circuits_preserving_variant (same file)
// - e2_batch_parallel_readonly_isolates_failures -> e3_batch_mode_equivalence (pass3)
// - e2_ok_channel_never_carries_failure -> DELETED (no unique error mutant)
//

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
        assert_eq!(
            planned.call_count(),
            2,
            "tool {tool}: later steps never run"
        );
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
        vec![batch_call(
            &long_id,
            "catalog_search",
            json!({"query": "x"}),
        )],
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
        assert!(session
            .call("catalog_search", json!({"query": "search"}))
            .is_ok());
    }
    assert_eq!(session.call_count(), 2);
    assert!(session.exhausted());
    for _ in 0..2 {
        let err = session
            .call("catalog_search", json!({"query": "search"}))
            .expect_err("spent budget must fail");
        assert!(matches!(err, CallError::BudgetExhausted(2)), "got {err:?}");
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
            ServeResponse::Result {
                ok, value, error, ..
            } => {
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
    // Pin the raw envelope's tag and field shapes. Typed BatchResult decoding
    // is independently covered in batch.rs.
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

// ── E3 metamorphic relations (from error_api_pass3.rs) ──
// E3 negative-path metamorphic tests for ast-sgrep-codemode errors.
//
// E1 pins the `CallError` taxonomy TABLE (one row per variant) and E2 pins
// PROPAGATION (a failure keeps its variant across layers). E3 pins RELATIONS:
// the same fault observed through two different surfaces must agree — never
// asserted twice as absolute pins, only as equivalences. Discriminant
// assertions via `matches!` / `match` arms only (never message text).
// Deterministic, tempfile fixtures, no new deps.
//
// Relation map (metamorphic relation -> test):
// - dispatch equivalence: session.call == tools::call_tool across the
//   UnknownTool + InvalidArgs + Other fault matrices (plus the documented
//   budget-bump divergence)
//   -> e3_dispatch_equivalence
// - repeat determinism: identical bad call twice == identical discriminant
//   sequence, same session and fresh sessions (plus budget-bypass divergence)
//   -> e3_repeat_bad_call_deterministic
// - batch position independence: bad call at any index, good rows unaffected
//   and bit-identical across rotations
//   -> e3_batch_position_independence
// - batch mode equivalence: serial vs parallel mixed batch, same per-id
//   ok pattern and good values (absorbs the E2 parallel isolation legs)
//   -> e3_batch_mode_equivalence
// - plan position independence: failing step at any index, same discriminant
//   as the direct call, call_count pins the executed prefix
//   -> e3_plan_position_independence
// - batch mirrors direct outcomes: per-call ok == direct is_ok, ok/error
//   exclusivity per row (fail-closed, never Ok-carried; absorbs the E1
//   per-call mirroring legs)
//   -> e3_batch_mirrors_direct_outcomes
// - plan fail-closed: failure is Err never Ok, discriminant == direct,
//   documented prefix state (applied-before-fail vs untouched-on-fail),
//   remainder resumes (absorbs the E4 plan-prefix drill)
//   -> e3_plan_fail_closed_never_ok
// - serve stream position independence: bad request at any stream index,
//   neighbors unaffected, worker reaches Bye
//   -> e3_serve_stream_position_independence
//

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

// ── E4 end-to-end drills (from error_api_pass4.rs) ──
// E4 end-to-end error drills for ast-sgrep-codemode.
//
// E1 pins the `CallError` taxonomy TABLE, E2 pins PROPAGATION, E3 pins
// RELATIONS. E4 pins DRILLS: full session flows — a working session executing
// calls/plans/batches, an injected fault mid-flow, then the end-to-end
// `CallError` discriminant AND the documented post-fault state (sticky
// budget, atomic batch, resumable plan). Discriminant assertions via
// `matches!` only (never message text). Deterministic, tempfile fixtures,
// no new deps.
//
// Drill map (fault -> post-fault state -> test):
// - bad tool args mid-batch over sticky serve; per-call isolation, Bye
//   -> e4_serve_batch_fault_mid_stream_isolates_and_resumes
// - unknown tool mid-plan; corrected plan runs (resumable plan)
//   -> e4_unknown_tool_in_plan_resumes_with_corrected_plan
// - budget exhaustion mid-plan; frozen count, repeat payload (sticky budget)
//   -> e4_budget_exhaustion_mid_plan_sticks_terminal
// - session root deleted mid-flow; pure tools unaffected, repair resumes
//   -> e4_deleted_root_fault_scoped_and_recovers_after_repair
// - edits[] batch with a failing entry; zero writes (atomic batch), retry ok
//   -> e4_atomic_edit_batch_zero_writes_then_resumes
// - chained double fault: unknown-tool then budget on one session
//   -> e4_chained_double_fault_unknown_then_budget
// - oversized response mid-flow; session resumes after the rejection
//   -> e4_oversized_response_fault_then_resumes
//
// Folded out (MERGE verdict; pinned at its anchor, not here):
// - e4_plan_prefix_applied_then_remainder_resumes -> e3_plan_fail_closed_never_ok (pass3)
//

/// INTENT=mid-stream batch fault isolated per-call, stream reaches Bye.
/// KILLS=isolation-breach, stream-abort.
/// ABSORBS=none.
#[test]
fn e4_serve_batch_fault_mid_stream_isolates_and_resumes() {
    // Full sticky-serve flow: good call, batch with bad args in the middle,
    // good call, End. The fault stays per-call isolated (neighbors ok), the
    // worker keeps serving, and the stream reaches Bye (resumable).
    let temp = tempfile::tempdir().expect("tempdir");
    let good = |id: &str| ServeRequest::Call {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    };
    let mut input = serve_request_line(&good("g0"));
    input.push_str(&serve_request_line(&ServeRequest::Batch {
        id: "b0".to_string(),
        calls: vec![
            batch_call("bg0", "catalog_search", json!({"query": "search"})),
            batch_call("bb", "select", json!({})),
            batch_call("bg1", "catalog_search", json!({"query": "find"})),
        ],
        parallel_mode: None,
    }));
    input.push_str(&serve_request_line(&good("g1")));
    input.push_str(&serve_request_line(&ServeRequest::End));

    let (result, lines) = serve_lines(input, temp.path());
    assert!(result.is_ok(), "serve survives the mid-stream fault");
    assert_eq!(lines.len(), 4);

    for (line, expect_id) in [(&lines[0], "g0"), (&lines[2], "g1")] {
        let response: ServeResponse = serde_json::from_str(line).expect("result line");
        match response {
            ServeResponse::Result {
                id,
                ok,
                value,
                error,
            } => {
                assert_eq!(id, expect_id);
                assert!(ok, "good call {expect_id} tainted by the fault");
                assert!(value.is_some());
                assert!(error.is_none());
            }
            other => panic!("tool outcome must be Result, got {other:?}"),
        }
    }

    // Pin the raw envelope's tag and field shapes. Typed BatchResult decoding
    // is independently covered in batch.rs.
    let batch: Value = serde_json::from_str(&lines[1]).expect("batch line");
    assert_eq!(
        batch.get("type").and_then(Value::as_str),
        Some("batch_result")
    );
    assert_eq!(batch.get("id").and_then(Value::as_str), Some("b0"));
    assert_eq!(batch.get("all_ok").and_then(Value::as_bool), Some(false));
    assert_eq!(batch.get("mode").and_then(Value::as_str), Some("serial"));
    let rows = batch
        .get("results")
        .and_then(Value::as_array)
        .expect("rows");
    assert_eq!(rows.len(), 3);
    let pattern: Vec<bool> = rows
        .iter()
        .map(|row| row.get("ok").and_then(Value::as_bool).expect("ok flag"))
        .collect();
    assert_eq!(pattern, vec![true, false, true]);
    for row in rows {
        let ok = row.get("ok").and_then(Value::as_bool).expect("ok flag");
        assert_eq!(row.get("value").is_some(), ok);
        assert_eq!(row.get("error").is_some(), !ok);
    }

    let last: ServeResponse = serde_json::from_str(&lines[3]).expect("bye line");
    assert!(matches!(last, ServeResponse::Bye), "got {last:?}");
}

/// INTENT=UnknownTool mid-plan fails, same session resumes corrected plan.
/// KILLS=session-poison-after-UnknownTool.
/// ABSORBS=none.
#[test]
fn e4_unknown_tool_in_plan_resumes_with_corrected_plan() {
    // Working session runs a good plan, then a plan with an unknown tool
    // mid-plan fails end-to-end as UnknownTool with the executed prefix pinned
    // — and the same session resumes with the corrected plan (resumable plan).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let warmup = parse_plan(&json!({"steps": [
        {"id": "w", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("warmup plan parses");
    let ok = run_plan(&mut session, &warmup).expect("warmup plan succeeds");
    assert!(ok.ok);
    assert_eq!(session.call_count(), 1);

    let faulty = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "no-such-tool", "args": {}},
        {"id": "c", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("faulty plan parses");
    let err = run_plan(&mut session, &faulty).expect_err("unknown step must fail");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert_eq!(session.call_count(), 3);
    assert!(!session.exhausted());

    let corrected = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "c", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("corrected plan parses");
    let ok = run_plan(&mut session, &corrected).expect("session resumes after UnknownTool");
    assert!(ok.ok);
    assert_eq!(session.call_count(), 5);
}

/// INTENT=budget aborts mid-plan then sticks terminal (non-resumable).
/// KILLS=budget-recovery-after-exhaustion.
/// ABSORBS=none.
/// OVERLAP=e2 budget pins (adds mid-plan flow).
#[test]
fn e4_budget_exhaustion_mid_plan_sticks_terminal() {
    // Working session nearly spends its budget on a good plan; the next plan
    // aborts mid-flow with BudgetExhausted — then the budget sticks terminal:
    // same payload on repeat, frozen counter, exhausted (sticky budget, the
    // one fault class that is NOT resumable).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 3;
    let warmup = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("warmup plan parses");
    assert!(run_plan(&mut session, &warmup).expect("warmup succeeds").ok);
    assert_eq!(session.call_count(), 2);
    assert!(!session.exhausted());

    let over = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("over-budget plan parses");
    let err = run_plan(&mut session, &over).expect_err("plan must stop at budget");
    assert!(matches!(err, CallError::BudgetExhausted(3)), "got {err:?}");
    assert_eq!(session.call_count(), 3);
    assert!(session.exhausted());

    for _ in 0..2 {
        let err = session
            .call("catalog_search", json!({"query": "search"}))
            .expect_err("spent budget must keep failing");
        assert!(matches!(err, CallError::BudgetExhausted(3)), "got {err:?}");
    }
    assert_eq!(session.call_count(), 3);
    assert!(session.exhausted());
}

/// INTENT=root deletion fails bound calls as Other, pure tools green, reindex resumes.
/// KILLS=fault-scope-breach, stale-searcher-after-repair.
/// ABSORBS=none.
#[test]
fn e4_deleted_root_fault_scoped_and_recovers_after_repair() {
    // Working session over an indexed root; the root is deleted mid-flow so
    // root-touching calls fail end-to-end as Other while pure tools stay
    // green (fault scoping) — then files are restored plus a reindex (which
    // drops the stale Searcher) and the same session serves reads again
    // (resumable after repair).
    let temp = tempfile::tempdir().expect("tempdir");
    let proj = temp.path().join("proj");
    std::fs::create_dir_all(&proj).expect("mkdir");
    std::fs::write(proj.join("a.rs"), "fn alpha() {}\nfn beta() {}\n").expect("write");
    let mut session = session_at(&proj);
    assert!(session.call("index_repo", json!({})).is_ok());
    let window = session
        .call("read", json!({"path": "a.rs", "start": 1, "end": 2}))
        .expect("read before fault");
    assert_eq!(window.get("count").and_then(Value::as_u64), Some(1));
    assert_eq!(session.call_count(), 2);

    std::fs::remove_dir_all(&proj).expect("delete root");
    assert!(!proj.exists());
    let err = session
        .call("search", json!({"query": "alpha"}))
        .expect_err("root-touching call must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert!(
        session
            .call("catalog_search", json!({"query": "search"}))
            .is_ok(),
        "pure tools must survive root deletion"
    );
    assert_eq!(session.call_count(), 4);

    std::fs::create_dir_all(&proj).expect("repair root");
    std::fs::write(proj.join("a.rs"), "fn alpha() {}\nfn beta() {}\n").expect("rewrite");
    assert!(
        session.call("index_repo", json!({})).is_ok(),
        "reindex must rebuild the repaired root"
    );
    let window = session
        .call("read", json!({"path": "a.rs", "start": 1, "end": 2}))
        .expect("repaired root must serve reads");
    assert_eq!(window.get("count").and_then(Value::as_u64), Some(1));
    let text = window["windows"][0]["text"].as_str().expect("text");
    assert!(text.contains("fn alpha"), "{text}");
    assert_eq!(session.call_count(), 6);
    assert!(!session.exhausted());
}

/// INTENT=failing edits[] entry fails whole call, zero writes, retry applies.
/// KILLS=partial-batch-write.
/// ABSORBS=none.
#[test]
fn e4_atomic_edit_batch_zero_writes_then_resumes() {
    // A working session fires one edit call with an edits[] batch whose second
    // entry cannot match: the whole call fails as Other and NEITHER file is
    // touched (atomic batch — two-phase commit, zero writes on any failure).
    // The corrected batch then applies on the same session (resumable).
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.txt"), "alpha one\n").expect("write a");
    std::fs::write(temp.path().join("b.txt"), "beta two\n").expect("write b");
    let mut session = session_at(temp.path());
    assert!(session
        .call("catalog_search", json!({"query": "search"}))
        .is_ok());

    let err = session
        .call(
            "edit",
            json!({"edits": [
                {"path": "a.txt", "oldText": "alpha one", "newText": "alpha 1"},
                {"path": "b.txt", "oldText": "not-present-anywhere", "newText": "x"},
            ]}),
        )
        .expect_err("partially failing edits[] must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(
        std::fs::read_to_string(temp.path().join("a.txt")).expect("reread a"),
        "alpha one\n",
        "atomic batch must leave the valid entry unapplied"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("b.txt")).expect("reread b"),
        "beta two\n"
    );

    let applied = session
        .call(
            "edit",
            json!({"edits": [
                {"path": "a.txt", "oldText": "alpha one", "newText": "alpha 1"},
                {"path": "b.txt", "oldText": "beta two", "newText": "beta 2"},
            ]}),
        )
        .expect("corrected batch must apply");
    assert_eq!(applied.get("ok").and_then(Value::as_bool), Some(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("a.txt")).expect("reread a"),
        "alpha 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("b.txt")).expect("reread b"),
        "beta 2\n"
    );
    assert_eq!(session.call_count(), 3);
}

/// INTENT=UnknownTool then, after resume spends budget, terminal BudgetExhausted in order.
/// KILLS=fault-order-swap, non-sticky-terminal.
/// ABSORBS=none.
#[test]
fn e4_chained_double_fault_unknown_then_budget() {
    // Chained double fault on one session: an unknown-tool plan fault first
    // (UnknownTool, resumable), then — after resuming work spends the rest of
    // the budget — a terminal BudgetExhausted (sticky). Both discriminants in
    // order, then the frozen terminal state.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 5;
    let warmup = parse_plan(&json!({"steps": [
        {"id": "w", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("warmup plan parses");
    assert!(run_plan(&mut session, &warmup).expect("warmup succeeds").ok);
    assert_eq!(session.call_count(), 1);

    let faulty = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "no-such-tool", "args": {}},
    ]}))
    .expect("faulty plan parses");
    let first = run_plan(&mut session, &faulty).expect_err("fault 1 must fail");
    assert!(matches!(first, CallError::UnknownTool(_)), "got {first:?}");
    assert_eq!(session.call_count(), 3);
    assert!(!session.exhausted());

    let resume = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("resume plan parses");
    assert!(run_plan(&mut session, &resume).expect("resume succeeds").ok);
    assert_eq!(session.call_count(), 5);
    assert!(session.exhausted());

    let second = session
        .call("catalog_search", json!({"query": "search"}))
        .expect_err("fault 2 must fail");
    assert!(
        matches!(second, CallError::BudgetExhausted(5)),
        "got {second:?}"
    );
    assert_eq!(session.call_count(), 5);
}

/// INTENT=oversized response fails Other closed (never truncated-ok), session resumes.
/// KILLS=truncated-ok, session-poison.
/// ABSORBS=none.
/// OVERLAP=oracle oversized pin (adds resume + budget-charge).
#[test]
fn e4_oversized_response_fault_then_resumes() {
    // A working session whose call would return more than the per-call cap
    // fails end-to-end as Other (fail closed, never truncated-ok) — and the
    // same session keeps serving afterwards (resumable; the rejection still
    // consumes its budget unit like any other dispatched call).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    assert!(session
        .call("select", json!({"value": {"v": 1}, "fields": ["v"]}))
        .is_ok());

    let blob = "x".repeat(ast_sgrep_core::MAX_STDIN_LINE_BYTES + 1024);
    let err = session
        .call(
            "select",
            json!({"value": {"blob": blob}, "fields": ["blob"]}),
        )
        .expect_err("oversized response must fail closed");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(session.call_count(), 2);

    assert!(
        session
            .call("catalog_search", json!({"query": "search"}))
            .is_ok(),
        "session must resume after the oversized rejection"
    );
    assert_eq!(session.call_count(), 3);
    assert!(!session.exhausted());
}
