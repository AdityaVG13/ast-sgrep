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
//! - BudgetExhausted: serve fail-once discriminant
//!   -> e1_budget_exhausted_serve_returns_discriminant
//! - Json: From<serde_json::Error> (production to_value sites are infallible
//!   over in-memory values, so the From arm is the reachable constructor)
//!   -> e1_json_row_from_conversion
//! - Other: search/find/chain query validation
//!   -> e1_other_search_find_chain_query_validation
//! - Other: unknown lang (pre-IO) -> e1_other_unknown_lang_rejected_pre_io
//! - Other: read ref shape + jail -> e1_other_read_ref_shape_and_jail
//! - Other: edit shape + uniqueness -> e1_other_edit_shape_and_uniqueness
//! - Other: index_repo targeted shape -> e1_other_index_repo_targeted_shape
//! - Batch envelope: per-call mirroring of all direct discriminants
//!   -> e1_batch_per_call_mirrors_direct_discriminants

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, run_serve, BatchCall, BatchRequest, CallError,
    CodeModeSession, ServeRequest, ServeResponse, SessionConfig,
};
use ast_sgrep_core::MAX_QUERY_CHARS;
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

#[test]
fn e1_budget_exhausted_serve_returns_discriminant() {
    // Serve pins max_calls=10_000: the 10_001st pure call gets exactly one
    // Result{ok:false} and run_serve terminates with BudgetExhausted(10_000).
    // The existing serve-budget test asserts is_err only; this pins the enum.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut input = String::new();
    for i in 0..10_001 {
        input.push_str(&serve_request_line(&ServeRequest::Call {
            id: format!("c{i}"),
            tool: "select".to_string(),
            args: json!({"value": {"v": i}, "fields": ["v"]}),
        }));
    }
    let (result, lines) = serve_lines(input, temp.path());
    let err = result.expect_err("serve must stop past budget");
    assert!(
        matches!(err, CallError::BudgetExhausted(10_000)),
        "got {err:?}"
    );
    assert_eq!(lines.len(), 10_001);
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

#[test]
fn e1_json_row_from_conversion() {
    // Json row: production to_value/encoded-len sites are infallible over
    // in-memory values, so the reachable constructor is From<serde_json::Error>
    // (direct and via `?`). Both must land on CallError::Json.
    fn load(text: &str) -> Result<Value, CallError> {
        Ok(serde_json::from_str(text)?)
    }
    let direct = CallError::from(serde_json::from_str::<Value>("{oops").expect_err("bad json"));
    assert!(matches!(direct, CallError::Json(_)), "got {direct:?}");
    let err = load("{oops").expect_err("bad json via ?");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    assert!(load(r#"{"ok":true}"#).is_ok());
}

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

#[test]
fn e1_batch_per_call_mirrors_direct_discriminants() {
    // The batch envelope maps every direct-call discriminant to a per-call
    // failure (ok:false, value dropped, error set) and stays Ok itself; ids
    // echo in input order and ok/error stay exclusive per result.
    let temp = tempfile::tempdir().expect("tempdir");
    let response = run_batch(
        config_at(temp.path()),
        &batch_request(vec![
            batch_call("u", "no-such-tool", json!({})),
            batch_call("i", "select", json!({})),
            batch_call("o", "search", json!({})),
            batch_call("g", "catalog_search", json!({"query": "search"})),
        ]),
    )
    .expect("batch envelope stays Ok");
    assert!(!response.all_ok);
    assert_eq!(response.call_count, 4);
    let ids: Vec<&str> = response.results.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["u", "i", "o", "g"]);
    for result in &response.results {
        if result.ok {
            assert!(result.value.is_some(), "ok without value: {}", result.id);
            assert!(result.error.is_none(), "ok with error: {}", result.id);
        } else {
            assert!(result.value.is_none(), "fail with value: {}", result.id);
            assert!(result.error.is_some(), "fail without error: {}", result.id);
        }
    }
    assert!(!response.results[0].ok);
    assert!(!response.results[1].ok);
    assert!(!response.results[2].ok);
    assert!(response.results[3].ok);
}
