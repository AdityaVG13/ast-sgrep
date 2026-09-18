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

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn mcp_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_asgrep-mcp") {
        return PathBuf::from(p);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let exe = format!("asgrep-mcp{}", std::env::consts::EXE_SUFFIX);
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(dir).join(profile).join(&exe);
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join(exe)
}

fn initialize_params(protocol_version: &str) -> Value {
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {},
        "clientInfo": {"name": "asgrep-mcp-test", "version": "0"}
    })
}

fn init_payload() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "__init",
        "method": "initialize",
        "params": initialize_params("2025-11-25")
    })
}

fn initialized_notif() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// Handshake, then strictly sequential send-one/read-one. Response order
/// matches request order; every payload here carries an `id`.
fn rpc_session(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    let mut child = command.spawn().expect("spawn MCP");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let send = |stdin: &mut std::process::ChildStdin, payload: &Value| {
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    };
    let recv = |stdout: &mut BufReader<std::process::ChildStdout>| -> Value {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout");
        serde_json::from_str(line.trim()).expect("JSON-RPC")
    };
    send(&mut stdin, &init_payload());
    let init = recv(&mut stdout);
    assert_eq!(init["id"], "__init", "{init:#}");
    send(&mut stdin, &initialized_notif());
    let mut responses = Vec::new();
    for payload in &payloads {
        send(&mut stdin, payload);
        responses.push(recv(&mut stdout));
    }
    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
    responses
}

/// Raw session with no handshake: send every payload, read exactly
/// `expect_lines` responses, return the exit status. For pre-init (J4),
/// which is fatal to the process.
fn rpc_raw(payloads: Vec<Value>, expect_lines: usize) -> (Vec<Value>, std::process::ExitStatus) {
    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn MCP");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for payload in &payloads {
        writeln!(stdin, "{payload}").unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut responses = Vec::new();
    for _ in 0..expect_lines {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout early");
        responses.push(serde_json::from_str(line.trim()).expect("JSON-RPC"));
    }
    let status = child.wait().expect("wait MCP");
    (responses, status)
}

fn tool_call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

/// Uniform tool-error envelope (matches the pass 3 / pass 4 / recovery
/// helper): `isError: true`, no top-level `error`, one `text` block, and
/// no `structuredContent`. Never asserts the message text.
fn assert_tool_error_shape(response: &Value) {
    assert_eq!(response["result"]["isError"], true, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert_eq!(
        response["result"]["content"][0]["type"],
        "text",
        "{response:#}"
    );
    assert!(
        response["result"].get("structuredContent").is_none(),
        "{response:#}"
    );
}

/// JSON-RPC error row: numeric `code`, no `result`.
fn assert_jsonrpc_error(response: &Value, code: i64) {
    assert_eq!(response["error"]["code"], code, "{response:#}");
    assert!(response["error"].is_object(), "{response:#}");
    assert!(response.get("result").is_none(), "{response:#}");
}

fn file_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    std::fs::write(temp.path().join("plain.txt"), "x\n").unwrap();
    temp
}

/// J1: unknown methods are -32601 with the id echoed and no `result`.
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
#[test]
fn taxonomy_invalid_request_is_32600_without_id() {
    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn MCP");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, "{}", init_payload()).unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read init");
    assert_eq!(
        serde_json::from_str::<Value>(line.trim()).unwrap()["id"],
        "__init"
    );
    writeln!(stdin, "{}", initialized_notif()).unwrap();
    for payload in [
        json!({"jsonrpc": "2.0", "id": 2}),
        json!({"id": 3, "method": "ping"}),
        json!([{"jsonrpc": "2.0", "id": 4, "method": "ping"}]),
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": "x"}),
    ] {
        writeln!(stdin, "{payload}").unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut responses = Vec::new();
    for _ in 0..4 {
        line.clear();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout early");
        responses.push(serde_json::from_str::<Value>(line.trim()).expect("JSON-RPC"));
    }
    let status = child.wait().expect("wait MCP");
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
#[test]
fn taxonomy_pre_initialize_request_is_32602() {
    for (payload, id) in [
        (
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            1,
        ),
        (tool_call(2, "index_status", json!({})), 2),
    ] {
        let (responses, status) = rpc_raw(vec![payload], 1);
        assert_eq!(responses.len(), 1);
        assert_jsonrpc_error(&responses[0], -32602);
        assert_eq!(responses[0]["id"], id, "{:#}", responses[0]);
        assert!(!status.success(), "pre-init request must exit non-zero");
    }
}

/// T1: unknown tool names are tool errors, never JSON-RPC errors: `isError`
/// with no top-level `error`. Case, whitespace, and near-miss variants.
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
    assert_eq!(responses[6]["result"]["isError"], false, "{:#}", responses[6]);
    assert_eq!(
        responses[6]["result"]["content"][0]["text"]
            .as_str()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
            .and_then(|b| b["nodes"].as_array().map(Vec::len)),
        Some(20),
        "{:#}",
        responses[6]
    );
}

/// T4: node-id shape rejections share the tool-error shape: missing range,
/// wrong separator, absolute path, directory target, unknown file.
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
#[test]
fn taxonomy_index_tool_arg_rejections_share_tool_error_shape() {
    let temp = file_tree();
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
#[test]
fn taxonomy_notification_produces_no_response_session_continues() {
    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn MCP");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let send = |stdin: &mut std::process::ChildStdin, payload: &Value| {
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    };
    send(&mut stdin, &init_payload());
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read init");
    assert_eq!(
        serde_json::from_str::<Value>(line.trim()).unwrap()["id"],
        "__init"
    );
    send(&mut stdin, &initialized_notif());
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"index_status","arguments":{}}}),
    );
    send(&mut stdin, &json!({"jsonrpc":"2.0","id":null,"method":"ping"}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","id":77,"method":"ping"}));
    stdin.flush().unwrap();
    line.clear();
    let n = stdout.read_line(&mut line).expect("read MCP line");
    assert!(n > 0, "MCP closed stdout");
    let first: Value = serde_json::from_str(line.trim()).expect("JSON-RPC");
    assert_eq!(first["id"], 77, "notification leaked a response: {first:#}");
    assert!(first.get("error").is_none(), "{first:#}");
    assert!(first.get("result").is_some(), "{first:#}");
    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
}
