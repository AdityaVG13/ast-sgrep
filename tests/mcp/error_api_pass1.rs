//! Pass 1 (errorapi, Mission E1): error-taxonomy inventory for ast-sgrep-mcp.
//!
//! This file pins the FULL MCP error surface as a table: one test per
//! JSON-RPC code / result shape, each with a hand-built trigger. Earlier
//! suites pin scattered rows (`protocol.rs`: -32601 unknown method,
//! unknown-tool result, unparsable silence; pass 3: tool-error uniformity,
//! malformed-envelope -32601; pass 4: method-vs-tool-vs-root; recovery:
//! fault shapes and startup exit). This file pins the TABLE and the GAPS:
//! -32600 (new row), -32602 pre-init (new row), notification silence,
//! index-tool arg rejections, and the remaining per-tool bound/type rows.
//!
//! Pinned taxonomy (discriminants are codes and shapes, never message text):
//!
//! | row | trigger                                              | discriminant                          |
//! |-----|------------------------------------------------------|---------------------------------------|
//! | J1  | unknown method (`missing`, `tools/unknown`)          | -32601, id echo, no `result`            |
//! | J2  | unshaped `tools/call`/`initialize` params            | -32601, id echo, no `result`            |
//! | J3  | invalid request (no method, no jsonrpc, batch, ...) | -32600, NO `id`, no `result`            |
//! | J4  | any request before `initialize` (fatal)           | -32602, id echo, no `result`, exit != 0 |
//! | T1  | unknown tool name                                    | tool error, no top-level `error`        |
//! | T2  | search arg rejection (bounds/types/unknown keys)     | tool error, no top-level `error`        |
//! | T3  | code_read arg rejection (+ 20-id boundary control)   | tool error, no top-level `error`        |
//! | T4  | code_read node-id shape rejection                    | tool error, no top-level `error`        |
//! | T5  | index_status/index_repo arg rejection                | tool error, no top-level `error`        |
//! | T6  | per-call root escaping the workspace                 | tool error, no top-level `error`        |
//! | S1  | notification (absent/null id)                        | silence; session continues              |
//!
//! Enumerated but wire-unreachable (no row): -32603 internal error
//! (`handler.rs` join/catalog failures need a task fault, not an input),
//! -32700 parse error (unparsable lines are ignored per `protocol.rs` pin),
//! and `arguments must be an object` (rmcp rejects non-object `arguments`
//! with J2 before dispatch; `null`/absent default to `{}` per pass 3 pin).
//!
//! Transport comes from [`error_testkit`](self::error_testkit): every
//! live-session read and process wait is timeout-bounded.

#[path = "error_testkit.rs"]
mod error_testkit;

use error_testkit::*;
use serde_json::{json, Value};

/// J1: unknown methods are -32601 with the id echoed and no `result`.
/// INTENT: unknown methods -32601, id echo, no result.
/// KILLS: code-swap, id-drop.
/// ABSORBS: none.
/// OVERLAP: protocol.rs -32601 (repin in table context).
#[test]
fn taxonomy_unknown_method_is_32601_with_id_echo() {
    let responses = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"missing"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/unknown"}),
        ],
        None,
    );
    assert_eq!(responses.len(), 2);
    for (response, id) in responses.iter().zip([1, 2]) {
        assert_jsonrpc_error(response, -32601);
        assert_eq!(response["id"], id, "{response:#}");
    }
}

/// J2: rmcp maps unshaped `tools/call` (and `initialize`) params to -32601,
/// not -32602: missing `name`, mistyped `name`, non-object `arguments`,
/// null params, and empty `initialize` params. Non-object `arguments` never
/// reach dispatch, so `arguments must be an object` is wire-unreachable.
/// INTENT: unshaped tools/call + initialize params map -32601 incl. non-object arguments.
/// KILLS: code-swap(-32602), dispatch-on-unshaped.
/// ABSORBS: none.
#[test]
fn taxonomy_unshaped_envelope_is_32601_not_32602() {
    let responses = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":null}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":42,"arguments":{}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"keyword_search","arguments":[]}}),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"keyword_search","arguments":"x"}}),
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"keyword_search","arguments":42}}),
            json!({"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}),
        ],
        None,
    );
    assert_eq!(responses.len(), 7);
    for (response, id) in responses.iter().zip(1..=7) {
        assert_jsonrpc_error(response, -32601);
        assert_eq!(response["id"], id, "{response:#}");
    }
}

/// J3: invalid requests are -32600 with NO `id` member (even when the
/// request carried one) and no `result`: missing method, missing `jsonrpc`,
/// batch arrays, and mistyped `params`.
/// INTENT: invalid requests -32600 with NO id member, no result.
/// KILLS: code-swap, id-echo-on-32600.
/// ABSORBS: none.
#[test]
fn taxonomy_invalid_request_is_32600_without_id() {
    let mut session = LiveSession::spawn(None);
    session.handshake();
    for payload in [
        json!({"jsonrpc": "2.0", "id": 2}),
        json!({"id": 3, "method": "ping"}),
        json!([{"jsonrpc": "2.0", "id": 4, "method": "ping"}]),
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": "x"}),
    ] {
        session.send(&payload);
    }
    let mut responses = Vec::new();
    for _ in 0..4 {
        responses.push(session.recv());
    }
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
    assert_eq!(responses.len(), 4);
    for response in &responses {
        assert_jsonrpc_error(response, -32600);
        assert!(
            response.get("id").is_none(),
            "-32600 must not echo an id: {response:#}"
        );
    }
}

/// J4: any request before `initialize` is -32602 (missing `_meta`), with
/// the id echoed and no `result` -- and it is fatal: the process exits
/// non-zero after the single error. One process per trigger. The only
/// wire-reachable -32602.
/// INTENT: pre-init requests -32602 with id echo, fatal non-zero exit.
/// KILLS: code-swap, non-fatal-pre-init.
/// ABSORBS: none.
#[test]
fn taxonomy_pre_initialize_request_is_32602() {
    for (payload, id) in [
        (
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            1,
        ),
        (tool_call(2, "index_status", json!({})), 2),
    ] {
        let mut session = LiveSession::spawn(None);
        session.send(&payload);
        let response = session.recv();
        assert_jsonrpc_error(&response, -32602);
        assert_eq!(response["id"], id, "{response:#}");
        session.close_stdin();
        let status = session.wait_clean();
        assert!(!status.success(), "pre-init request must exit non-zero");
    }
}

/// T1: unknown tool names are tool errors, never JSON-RPC errors: `isError`
/// with no top-level `error`. Case, whitespace, and near-miss variants.
/// INTENT: unknown/case/whitespace tool names are tool errors, never JSON-RPC errors.
/// KILLS: level-escalation(tool→rpc), fuzzy-name-match.
/// ABSORBS: none.
/// OVERLAP: protocol.rs unknown-tool (adds variants).
#[test]
fn taxonomy_unknown_tool_is_tool_error_not_jsonrpc_error() {
    let temp = file_tree();
    let responses = rpc_session(
        vec![
            tool_call(1, "no_such_tool", json!({})),
            tool_call(2, "Search", json!({"query": "x"})),
            tool_call(3, "keyword_search ", json!({"query": "x"})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

/// T2: search-channel arg rejections (bound ceilings, mistyped scalars,
/// unknown keys) share the one tool-error shape. Gap rows: `budget_tokens`
/// above max / negative / mistyped, mistyped `resend_seen` / `preview` /
/// `file_filter` / `lang`, negative `limit`, mistyped `query` / `root`.
/// INTENT: 11 search bound/type/unknown-key rejections share tool-error shape.
/// KILLS: arg-accepted, level-escalation.
/// ABSORBS: none.
#[test]
fn taxonomy_search_arg_rejections_share_tool_error_shape() {
    let temp = file_tree();
    let bad_args = vec![
        json!({"query": "x", "budget_tokens": 65537}),
        json!({"query": "x", "budget_tokens": -5}),
        json!({"query": "x", "budget_tokens": "many"}),
        json!({"query": "x", "resend_seen": "yes"}),
        json!({"query": "x", "preview": 42}),
        json!({"query": "x", "file_filter": 42}),
        json!({"query": "x", "lang": 42}),
        json!({"query": "x", "limit": -1}),
        json!({"query": 42}),
        json!({"query": "x", "root": 42}),
        json!({"query": "x", "budgettokens": 8}),
    ];
    let calls: Vec<Value> = bad_args
        .into_iter()
        .enumerate()
        .map(|(i, args)| tool_call(i as u32 + 1, "keyword_search", args))
        .collect();
    let n = calls.len();
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), n);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

/// T3: code_read arg rejections share the tool-error shape, and exactly 20
/// ids still succeed (pass 3 pins 21 rejected; this pins the ceiling).
/// INTENT: 6 code_read bound/type rejections share shape + 20-id ceiling still succeeds.
/// KILLS: arg-accepted, ceiling-off-by-one.
/// ABSORBS: none.
/// OVERLAP: pass 3 pins 21 rejected (this pins the ceiling).
#[test]
fn taxonomy_code_read_arg_rejections_share_tool_error_shape() {
    let temp = file_tree();
    let twenty: Vec<Value> = (0..20).map(|_| json!("src/lib.rs#L1-L1")).collect();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "max_chars": 1000001}),
            ),
            tool_call(
                2,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "max_chars": -1}),
            ),
            tool_call(
                3,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "max_chars": "many"}),
            ),
            tool_call(
                4,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "context_lines": -1}),
            ),
            tool_call(
                5,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "context_lines": 1.5}),
            ),
            tool_call(
                6,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": 42}),
            ),
            tool_call(7, "code_read", json!({"ids": twenty})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 7);
    for response in &responses[..6] {
        assert_tool_error_shape(response);
    }
    assert_tool_success_shape(&responses[6]);
    assert_eq!(
        tool_body(&responses[6])["nodes"].as_array().map(Vec::len),
        Some(20),
        "{:#}",
        responses[6]
    );
}

/// T4: node-id shape rejections share the tool-error shape: missing range,
/// wrong separator, absolute path, directory target, unknown file.
/// INTENT: 6 node-id shape rejections share tool-error shape.
/// KILLS: shape-accepted, level-escalation.
/// ABSORBS: none.
#[test]
fn taxonomy_code_read_node_id_rows_share_tool_error_shape() {
    let temp = file_tree();
    let bad_ids = [
        "src/lib.rs",
        "src/lib.rs#1-2",
        "src/lib.rs#L1",
        "/etc/hosts#L1-L1",
        "src#L1-L1",
        "src/nope.rs#L1-L1",
    ];
    let calls: Vec<Value> = bad_ids
        .iter()
        .enumerate()
        .map(|(i, id)| tool_call(i as u32 + 1, "code_read", json!({"ids": [id]})))
        .collect();
    let n = calls.len();
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), n);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

/// T5: index-tool arg rejections share the tool-error shape. Gap rows:
/// unknown keys on both tools, mistyped `force`, mistyped `root`, and a
/// `root` pointing at a file. All fail at parse; no indexing runs.
/// INTENT: 7 index-tool unknown-key/type/file-root rejections share shape, no indexing runs.
/// KILLS: arg-accepted, index-before-validate.
/// ABSORBS: none.
#[test]
fn taxonomy_index_tool_arg_rejections_share_tool_error_shape() {
    let temp = file_tree();
    // The file-root row needs a second file; the canonical tree stays
    // single-file so `file_count` pins elsewhere stay exact.
    std::fs::write(temp.path().join("plain.txt"), "x\n").unwrap();
    let file_root = temp.path().join("plain.txt").display().to_string();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({"bogus": 1})),
            tool_call(2, "index_status", json!({"root": true})),
            tool_call(3, "index_repo", json!({"bogus": 1})),
            tool_call(4, "index_repo", json!({"force": "yes"})),
            tool_call(5, "index_repo", json!({"force": 1})),
            tool_call(6, "index_repo", json!({"root": 42})),
            tool_call(7, "index_repo", json!({"root": file_root})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 7);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

/// T6: a per-call root outside the configured workspace is a tool error on
/// every tool that takes one (`protocol.rs` pins `index_status` only).
/// INTENT: per-call root outside workspace is tool error on all 4 root-taking tools.
/// KILLS: jail-drop, per-tool-divergence.
/// ABSORBS: none.
/// OVERLAP: protocol.rs index_status-only (extends to all tools).
#[test]
fn taxonomy_sandbox_escape_is_tool_error_across_all_tools() {
    let workspace = file_tree();
    let outside = tempfile::tempdir().unwrap();
    let escaped = outside.path().display().to_string();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "x", "limit": 4, "root": escaped}),
            ),
            tool_call(
                2,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": escaped}),
            ),
            tool_call(3, "index_status", json!({"root": escaped})),
            tool_call(4, "index_repo", json!({"root": escaped})),
        ],
        Some(workspace.path()),
    );
    assert_eq!(responses.len(), 4);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

/// S1: notifications (absent id, null id) produce no response. The next
/// line after two notifications plus a ping must be the ping response, and
/// the session exits cleanly -- silence is observable without timeouts.
/// INTENT: absent/null-id notifications silent; next line is the ping; clean exit.
/// KILLS: notification-response-leak, session-stall.
/// ABSORBS: none.
#[test]
fn taxonomy_notification_produces_no_response_session_continues() {
    let mut session = LiveSession::spawn(None);
    session.handshake();
    session.send(
        &json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"index_status","arguments":{}}}),
    );
    session.send(&json!({"jsonrpc":"2.0","id":null,"method":"ping"}));
    session.send(&json!({"jsonrpc":"2.0","id":77,"method":"ping"}));
    let first = session.recv();
    assert_eq!(first["id"], 77, "notification leaked a response: {first:#}");
    assert!(first.get("error").is_none(), "{first:#}");
    assert!(first.get("result").is_some(), "{first:#}");
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
}
