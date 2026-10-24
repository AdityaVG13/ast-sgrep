//! Pass 1 (errorapi, Mission E1): error-taxonomy inventory for ast-sgrep-mcp.
//!
//! This file pins the FULL MCP error surface as a table: one test per
//! JSON-RPC code / result shape, each with a hand-built trigger. Earlier
//! suites pin scattered rows (`protocol.rs`: -32601 unknown method,
//! unknown-tool result, parse-error fail-closed; recovery: fault shapes and
//! startup exit). This file pins the TABLE and the GAPS: -32600 rows,
//! pre-init admission, notification silence, index-tool arg rejections, and
//! the remaining per-tool bound/type rows.
//!
//! Pinned taxonomy under fastmcp-rust 0.13 (auto facade, legacy era;
//! discriminants are codes and shapes, never message text):
//!
//! | row | trigger                                              | discriminant                          |
//! |-----|------------------------------------------------------|---------------------------------------|
//! | J1  | unknown method (`missing`, `tools/unknown`)          | -32601, id echo, no `result`            |
//! | J2  | unshaped `tools/call`/`initialize` params            | -32602, id echo, no `result`            |
//! | J3  | invalid request (no method, no jsonrpc, batch, ...) | -32600, id echoed when sent, no `result`|
//! | J4  | any request before `initialize` (fatal)           | -32600, id echo, no `result`, exit != 0 |
//! | T0  | inputSchema violation (bounds/types/unknown keys)    | -32602 (framework admission), no `result` |
//! | T1  | unknown tool name                                    | -32601 JSON-RPC, no `result`            |
//! | T4  | code_read node-id shape rejection                    | tool error, no top-level `error`        |
//! | T5b | schema-valid but semantically bad root (file root)   | tool error, no top-level `error`        |
//! | T6  | per-call root escaping the workspace                 | tool error, no top-level `error`        |
//! | S1  | notification (absent/null id)                        | silence; session continues              |
//!
//! The admission line moved with the port: rmcp let every `tools/call`
//! through to our handler (arg rejections were tool errors); fastmcp
//! validates arguments against the declared inputSchema BEFORE dispatch and
//! admits violations as -32602 "Input validation failed". Everything the
//! schema cannot see (node-id shapes, file roots, sandbox escapes) still
//! fails inside dispatch as a tool error.
//!
//! Enumerated but wire-unreachable (no row): -32603 internal error
//! (`handler.rs` join/catalog failures need a task fault, not an input).
//! -32700 parse error is pinned in `protocol.rs` (fastmcp fails closed with
//! the spec-mandated code and a nonzero exit, where rmcp ignored the line).
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

/// J2: fastmcp maps unshaped `tools/call` params to -32602 ("invalid exact
/// MCP 2024-11-05 parameters"): missing `name`, mistyped `name`,
/// non-object/null `arguments`, and null params. A mid-session
/// `initialize` with empty params is envelope-class -32600 instead.
/// rmcp answered -32601 here — the pin moved with the serve layer.
/// INTENT: unshaped tools/call params map -32602 incl. non-object arguments; initialize params map -32600.
/// KILLS: code-swap(-32601), dispatch-on-unshaped.
/// ABSORBS: none.
#[test]
fn taxonomy_unshaped_envelope_is_32602() {
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
    for (response, id) in responses.iter().zip(1..=6) {
        assert_jsonrpc_error(response, -32602);
        assert_eq!(response["id"], id, "{response:#}");
    }
    // initialize params are envelope-class, not parameters-class.
    assert_jsonrpc_error(&responses[6], -32600);
    assert_eq!(responses[6]["id"], 7, "{:#}", responses[6]);
}

/// J3: invalid requests are -32600 with no `result`: missing method,
/// missing `jsonrpc`, batch arrays, and mistyped `params`. fastmcp ECHOES
/// the request id on -32600 (rmcp dropped it) — the pin is the code and the
/// absent `result`, not the id handling.
/// INTENT: invalid requests -32600, no result member.
/// KILLS: code-swap, result-on-invalid.
/// ABSORBS: none.
#[test]
fn taxonomy_invalid_request_is_32600() {
    let mut session = LiveSession::spawn(None);
    session.handshake();
    for payload in [
        json!({ "jsonrpc": "2.0", "id": 2 }),
        json!({ "id": 3, "method": "ping" }),
        json!([{ "jsonrpc": "2.0", "id": 4, "method": "ping" }]),
        json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": "x" }),
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
    }
}

/// J4: any request before `initialize` lacking modern `_meta` is refused by
/// lifecycle admission as -32600 ("initialize is the only request allowed
/// before lifecycle admission"), with the id echoed and no `result`. rmcp
/// answered -32602 here — the pin moved with the serve layer. The error is
/// answered in-band and the server terminates CLEANLY on EOF — the failure
/// lives on the wire, not in the process exit status. One process per
/// trigger.
/// INTENT: pre-init requests -32600 with id echo, in-band, clean exit.
/// KILLS: code-swap, silent-drop.
/// ABSORBS: none.
#[test]
fn taxonomy_pre_initialize_request_is_32600() {
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
        assert_jsonrpc_error(&response, -32600);
        assert_eq!(response["id"], id, "{response:#}");
        session.close_stdin();
        let status = session.wait_clean();
        assert!(
            status.success(),
            "in-band admission error must exit cleanly: {status}"
        );
    }
}

/// T1: unknown tool names are JSON-RPC -32601 method-not-found on the
/// legacy era (rmcp answered a tool error; fastmcp's auto facade owns tool
/// routing). Case, whitespace, and near-miss variants. No `result`.
/// INTENT: unknown/case/whitespace tool names are -32601 JSON-RPC, never a fabricated result.
/// KILLS: tool-error-shape, fuzzy-name-match.
/// ABSORBS: none.
/// OVERLAP: protocol.rs unknown-tool (adds case/whitespace variants).
#[test]
fn taxonomy_unknown_tool_is_32601_jsonrpc_error() {
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
    for (response, id) in responses.iter().zip(1..=3) {
        assert_jsonrpc_error(response, -32601);
        assert_eq!(response["id"], id, "{response:#}");
    }
}

/// T2 (now T0): search-channel arg rejections (bound ceilings, mistyped
/// scalars, unknown keys) are admitted by fastmcp's inputSchema validation
/// BEFORE dispatch as -32602 — the arguments never reach our handler. Gap
/// rows: `budget_tokens` above max / negative / mistyped, mistyped
/// `resend_seen` / `preview` / `file_filter` / `lang`, negative `limit`,
/// mistyped `query` / `root`, unknown key.
/// INTENT: 11 search bound/type/unknown-key rejections admit as -32602.
/// KILLS: arg-accepted, dispatch-on-invalid.
/// ABSORBS: none.
#[test]
fn taxonomy_search_arg_rejections_share_32602_admission() {
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
        assert_jsonrpc_error(response, -32602);
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
    // Rows 1-6: inputSchema violations — fastmcp admits them as -32602
    // before dispatch.
    for response in &responses[..6] {
        assert_jsonrpc_error(response, -32602);
    }
    // Row 7: exactly 20 ids still succeeds (pass 3 pins 21 rejected; this
    // pins the ceiling).
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
    // Round-12 ref parity: single-line `#Lstart` and LSP `path:start` are now
    // VALID, so they left the bad set; an unresolved compact-id shape
    // (`:start-end` with no registry entry) keeps a row.
    let bad_ids = [
        "src/lib.rs",
        "src/lib.rs#1-2",
        "src/lib.rs:1-2",
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

/// T5: index-tool arg rejections. Schema violations (unknown keys on both
/// tools, mistyped `force`, mistyped `root`) admit as -32602 before
/// dispatch; the schema-valid-but-semantic row (a `root` pointing at a
/// FILE) still fails inside dispatch as a tool error. No indexing runs.
/// INTENT: 6 index-tool schema violations admit as -32602; file-root stays a tool error.
/// KILLS: arg-accepted, index-before-validate.
/// ABSORBS: none.
#[test]
fn taxonomy_index_tool_arg_rejections_share_32602_admission() {
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
    for response in &responses[..6] {
        assert_jsonrpc_error(response, -32602);
    }
    assert_tool_error_shape(&responses[6]);
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

/// S1: notifications produce no response. rmcp 3.7.0 splits the two shapes:
/// an id-OMITTED message stays silent (a true notification), while `"id":
/// null` is rejected in-band as -32600 with no id echo (JSON-RPC: the id
/// member, when present, must be a string or number — null is reserved for
/// server parse-error replies). The ping (id 77) reply still arrives, and
/// the session exits cleanly.
/// INTENT: id-omitted notification silent; id:null draws -32600 (no echo); ping replies; clean exit.
/// KILLS: notification-response-leak, session-stall, null-id tolerated as notification.
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
    assert_jsonrpc_error(&first, -32600);
    assert!(
        first.get("id").map(|v| v.is_null()).unwrap_or(true),
        "-32600 must not echo an id: {first:#}"
    );
    let second = session.recv();
    assert_eq!(second["id"], 77, "ping reply displaced: {second:#}");
    assert!(second.get("error").is_none(), "{second:#}");
    assert!(second.get("result").is_some(), "{second:#}");
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
}
