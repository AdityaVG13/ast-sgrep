//! E1 error-taxonomy inventory for ast-sgrep-codemode.
//!
//! Pins the full `CallError` taxonomy TABLE with hand-built triggers, one row
//! per variant, discriminant assertions via `matches!` only (never message
//! text). Oracle passes already pin the table's core rows (unknown-tool
//! dispatch, plan/batch shape guards, budget boundary, missing/escaping root,
//! oversized response); every test here adds at least one previously unpinned
//! discriminant. Deterministic, tempfile fixtures, no new deps.
//!
//! Row map (variant -> site -> test):
//! - UnknownTool: dispatch (direct/plan/batch-per-call/serve-per-request)
//!   -> e1_unknown_tool_row_direct_plan_batch_serve
//! - InvalidArgs: pure-tool guards -> e1_invalid_args_pure_tool_guard_gaps
//! - InvalidArgs: plan $ref arms -> e1_invalid_args_plan_ref_and_shape_gaps
//! - InvalidArgs: serve maps to Error envelopes, never Err
//!   -> e1_invalid_args_serve_envelope_not_err
//! - Other: search/find/chain query validation
//!   -> e1_other_search_find_chain_query_validation
//! - Other: unknown lang (pre-IO) -> e1_other_unknown_lang_rejected_pre_io
//! - Other: read ref shape + jail -> e1_other_read_ref_shape_and_jail
//! - Other: edit shape + uniqueness -> e1_other_edit_shape_and_uniqueness
//! - Other: index_repo targeted shape -> e1_other_index_repo_targeted_shape
//!
//! Folded out (MERGE verdicts; pinned at their anchors, not here):
//! - BudgetExhausted serve discriminant -> e2_serve_budget_answers_once_ignores_trailing (pass2)
//! - Json via-? arm -> e2_json_cause_preserved_via_downcast (pass2)
//! - batch per-call mirroring -> e3_batch_mirrors_direct_outcomes (pass3)

#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, CallError, ServeRequest, ServeResponse,
};
use ast_sgrep_core::MAX_QUERY_CHARS;
use error_testkit::{
    batch_call, batch_request, config_at, serve_lines, serve_request_line, session_at,
};
use serde_json::{json, Value};

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
        assert!(matches!(err, CallError::Other(_)), "tool {tool}: got {err:?}");
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
        assert!(matches!(err, CallError::Other(_)), "tool {tool}: got {err:?}");
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
        let err = session.call("index_repo", args.clone()).expect_err("paths guard");
        assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    }
}
