//! Pass 3 (oracle-foundry, Mission 3): L3 metamorphic / differential /
//! adversarial oracles for ast-sgrep-mcp, driven through the stdio protocol.
//!
//! Non-overlap contract: pass 2 pins limit/query/budget/code_read-window
//! boundaries and `protocol.rs` pins handshake, tools/list stability, elision,
//! miss envelopes, sandbox roots, and cancellation. This file pins NONE of
//! those again. Instead it pins relations BETWEEN calls:
//!
//! * differential: `code_search` alias agrees with `keyword_search` exactly;
//! * metamorphic: limit growth is prefix-stable; query trim, preview case,
//!   and preview-none preserve ids; repeats are byte-identical;
//! * adversarial: fixed invalid inputs (missing fields, wrong types, huge
//!   numbers, empty/unicode/overlong ids) are tool errors with a uniform
//!   shape, pipelined batches lose nothing, and the server survives.
//!
//! Discriminants are `isError` booleans, JSON-RPC codes, and envelope shapes
//! (key presence, tuple widths, id multisets) -- never message text.

use serde_json::{json, Value};
use std::collections::HashMap;
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

/// Drive several requests through ONE server process, strictly sequential
/// (send one, read one), so response order matches request order.
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
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": "__init",
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "asgrep-mcp-pass3", "version": "0"}
            }
        }),
    );
    let init = recv(&mut stdout);
    assert_eq!(init["id"], "__init", "{init:#}");
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    let mut responses = Vec::new();
    for payload in &payloads {
        send(&mut stdin, payload);
        if payload.get("id").is_some() {
            responses.push(recv(&mut stdout));
        }
    }
    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
    responses
}

/// Fire every payload without reading, then collect exactly one response per
/// id-bearing request. Delivery order is NOT pinned (the server answers
/// concurrently); id-multiset agreement is.
fn rpc_pipeline(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    let mut child = command.spawn().expect("spawn MCP");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": "__init",
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "asgrep-mcp-pass3-pipe", "version": "0"}
            }
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read init");
    assert!(!line.trim().is_empty(), "MCP closed stdout");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .unwrap();
    let expected = payloads
        .iter()
        .filter(|p| p.get("id").is_some())
        .count();
    for payload in &payloads {
        writeln!(stdin, "{payload}").unwrap();
    }
    stdin.flush().unwrap();
    let mut responses = Vec::new();
    for _ in 0..expected {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout mid-batch");
        responses.push(serde_json::from_str::<Value>(line.trim()).expect("JSON-RPC"));
    }
    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
    responses
}

fn tool_call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text content")
}

fn tool_body(response: &Value) -> Value {
    serde_json::from_str(tool_text(response)).expect("tool body JSON")
}

/// Every tool-level failure has the same envelope: `isError: true`, no
/// top-level `error`, a single `text` content block, and no structured body.
fn assert_tool_error_shape(response: &Value) {
    assert_eq!(response["result"]["isError"], true, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert_eq!(response["result"]["content"][0]["type"], "text", "{response:#}");
    assert!(
        response["result"].get("structuredContent").is_none(),
        "{response:#}"
    );
}

/// Single-file indexed tree with one findable symbol.
fn indexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: temp.path().to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();
    temp
}

/// Six files sharing one lexical token, so limit 2 vs limit 8 differ.
fn multi_hit_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for index in 1..=6 {
        std::fs::write(
            source.join(format!("m{index}.rs")),
            format!("fn shared_token_{index}() {{}}\n"),
        )
        .unwrap();
    }
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: temp.path().to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();
    temp
}

#[test]
fn code_search_alias_matches_keyword_search_exactly() {
    // Differential: `code_search` is a compat alias for Keyword mode, so the
    // same query must produce byte-identical text AND structured bodies.
    // `resend_seen` keeps the second call out of snippet-elision state.
    let temp = multi_hit_tree();
    let args = json!({"query": "shared_token", "limit": 8, "resend_seen": true});
    let responses = rpc_session(
        vec![
            tool_call(1, "keyword_search", args.clone()),
            tool_call(2, "code_search", args),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    assert_eq!(
        tool_text(&responses[0]),
        tool_text(&responses[1]),
        "alias drift between code_search and keyword_search"
    );
    assert_eq!(
        responses[0]["result"]["structuredContent"],
        responses[1]["result"]["structuredContent"]
    );
}

#[test]
fn repeated_index_status_and_code_read_are_byte_identical() {
    // Metamorphic idempotence: pure reads repeat byte-for-byte in one session.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(2, "index_status", json!({})),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(4, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    assert_eq!(tool_text(&responses[0]), tool_text(&responses[1]));
    assert_eq!(tool_text(&responses[2]), tool_text(&responses[3]));
    assert_eq!(tool_body(&responses[0])["file_count"], 1);
    assert_eq!(
        tool_body(&responses[2])["nodes"][0]["content"],
        "fn target_symbol() {}"
    );
}

#[test]
fn limit_growth_is_prefix_stable() {
    // Metamorphic: raising the limit extends the hit list without reordering
    // or rewriting the shared prefix. `resend_seen` disables elision so both
    // calls carry full tuples.
    let temp = multi_hit_tree();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "shared_token", "limit": 2, "resend_seen": true}),
            ),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "shared_token", "limit": 8, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    let small = tool_body(&responses[0]);
    let large = tool_body(&responses[1]);
    assert_eq!(small["zn"], 2, "{small:#}");
    assert_eq!(small["h"].as_array().unwrap().len(), 2);
    assert!(
        large["h"].as_array().unwrap().len() > 2,
        "need headroom to test prefix stability: {large:#}"
    );
    assert_eq!(&large["h"].as_array().unwrap()[..2], small["h"].as_array().unwrap());
    assert_eq!(small["q"], large["q"]);
    for (id, path) in small["p"].as_object().unwrap() {
        assert_eq!(&large["p"][id], path, "path table shrank for {id}");
    }
}

#[test]
fn query_trim_and_preview_case_are_normalized() {
    // Metamorphic: surrounding whitespace is trimmed (echoed `q` proves it)
    // and preview names are case-insensitive; both pairs are byte-identical.
    let temp = multi_hit_tree();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "shared_token", "limit": 8, "resend_seen": true}),
            ),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "  shared_token  ", "limit": 8, "resend_seen": true}),
            ),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "shared_token", "limit": 8, "resend_seen": true, "preview": "full"}),
            ),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "shared_token", "limit": 8, "resend_seen": true, "preview": "FULL"}),
            ),
        ],
        Some(temp.path()),
    );
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    assert_eq!(tool_text(&responses[0]), tool_text(&responses[1]));
    assert_eq!(tool_body(&responses[1])["q"], "shared_token");
    assert_eq!(tool_text(&responses[2]), tool_text(&responses[3]));
}

#[test]
fn preview_none_keeps_ids_drops_snippets() {
    // Metamorphic across preview modes: `none` blanks snippets but keeps the
    // same ranking and ids as `short`.
    let temp = multi_hit_tree();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "shared_token", "limit": 8, "resend_seen": true}),
            ),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "shared_token", "limit": 8, "resend_seen": true, "preview": "none"}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    let short = tool_body(&responses[0]);
    let none = tool_body(&responses[1]);
    let short_hits = short["h"].as_array().unwrap();
    let none_hits = none["h"].as_array().unwrap();
    assert!(!short_hits.is_empty());
    assert_eq!(none_hits.len(), short_hits.len());
    for (full, blank) in short_hits.iter().zip(none_hits.iter()) {
        assert_eq!(blank[0], full[0], "id drift across preview modes");
        assert_eq!(blank[4], "", "preview=none must blank snippets: {blank:#}");
        assert!(
            !full[4].as_str().unwrap().is_empty(),
            "control call must carry snippets: {full:#}"
        );
    }
}

#[test]
fn unknown_tool_and_bad_search_args_share_tool_error_shape() {
    // Unknown tools (empty, absent, case-shifted) and malformed search args
    // (missing query, float/huge limit, bad preview, empty/overlong filters)
    // are all tool errors with one uniform shape. A unicode query is valid
    // input and must run (miss envelope, not an error).
    let temp = multi_hit_tree();
    let bad_calls = vec![
        tool_call(1, "", json!({})),
        tool_call(2, "no_such_tool", json!({})),
        tool_call(3, "Keyword_Search", json!({"query": "x", "limit": 4})),
        tool_call(4, "keyword_search", json!({"limit": 4})),
        tool_call(5, "keyword_search", json!({"query": "x", "limit": 1.5})),
        tool_call(
            6,
            "keyword_search",
            json!({"query": "x", "limit": u64::MAX}),
        ),
        tool_call(7, "keyword_search", json!({"query": "   ", "limit": 4})),
        tool_call(
            8,
            "keyword_search",
            json!({"query": "x", "limit": 4, "preview": "huge"}),
        ),
        tool_call(
            9,
            "keyword_search",
            json!({"query": "x", "limit": 4, "file_filter": ""}),
        ),
        tool_call(
            10,
            "keyword_search",
            json!({"query": "x", "limit": 4, "file_filter": "a".repeat(4097)}),
        ),
        tool_call(11, "keyword_search", json!({"query": "x", "limit": 4, "lang": "  "})),
        tool_call(
            12,
            "keyword_search",
            json!({"query": "日本語🔍", "limit": 4, "resend_seen": true}),
        ),
    ];
    let responses = rpc_session(bad_calls, Some(temp.path()));
    assert_eq!(responses.len(), 12);
    for response in &responses[..11] {
        assert_tool_error_shape(response);
    }
    // Unicode is a runnable query: a miss envelope, not a tool error.
    assert_eq!(responses[11]["result"]["isError"], false, "{:#}", responses[11]);
    let body = tool_body(&responses[11]);
    assert_eq!(body["why"], "no_match", "{body:#}");
    assert_eq!(body["h"].as_array().unwrap().len(), 0);
}

#[test]
fn adversarial_code_read_ids_rejected_server_survives() {
    // Fixed adversarial ids: empty list, empty/unicode/overlong ids, oversize
    // lists, wrong JSON types, missing fields, null arguments. All are tool
    // errors; a valid read afterwards proves the server is unpoisoned.
    let temp = indexed_tree();
    let bad_calls = vec![
        tool_call(1, "code_read", json!({"ids": []})),
        tool_call(2, "code_read", json!({"ids": [""]})),
        tool_call(3, "code_read", json!({"ids": ["日本.rs#L1-L1"]})),
        tool_call(
            4,
            "code_read",
            json!({"ids": [format!("{}#L1-L1", "a".repeat(5000))]}),
        ),
        tool_call(
            5,
            "code_read",
            json!({"ids": vec!["src/lib.rs#L1-L1"; 21]}),
        ),
        tool_call(6, "code_read", json!({"ids": "src/lib.rs#L1-L1"})),
        tool_call(7, "code_read", json!({"ids": [42]})),
        tool_call(8, "code_read", json!({"ids": Value::Null})),
        tool_call(9, "code_read", json!({})),
        tool_call(10, "code_read", Value::Null),
    ];
    let responses = rpc_session(bad_calls, Some(temp.path()));
    assert_eq!(responses.len(), 10);
    for response in &responses {
        assert_tool_error_shape(response);
    }
    // Liveness: the same session still serves a valid read.
    let responses = rpc_session(
        vec![tool_call(11, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}))],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(
        tool_body(&responses[0])["nodes"][0]["content"],
        "fn target_symbol() {}"
    );
}

#[test]
fn index_repo_twice_keeps_status_file_count_stable() {
    // Metamorphic idempotence: reindexing an unchanged tree succeeds and
    // indexes nothing new; status file counts agree across the second run.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_repo", json!({})),
            tool_call(2, "index_status", json!({})),
            tool_call(3, "index_repo", json!({})),
            tool_call(4, "index_status", json!({})),
        ],
        Some(temp.path()),
    );
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    assert_eq!(tool_body(&responses[2])["files_indexed"], 0);
    assert_eq!(
        tool_body(&responses[1])["file_count"],
        tool_body(&responses[3])["file_count"]
    );
    assert_eq!(tool_body(&responses[1])["file_count"], 1);
}

#[test]
fn pipelined_batch_returns_every_id_exactly_once() {
    // Adversarial concurrency: six mixed requests fired without waiting. The
    // server answers concurrently (order not pinned), but every id must come
    // back exactly once, each response carries exactly one of result/error,
    // and error topology still holds per response.
    let temp = indexed_tree();
    let responses = rpc_pipeline(
        vec![
            json!({"jsonrpc":"2.0","id":101,"method":"tools/list","params":{}}),
            tool_call(102, "index_status", json!({})),
            tool_call(
                103,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(104, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(105, "no_such_tool", json!({})),
            json!({"jsonrpc":"2.0","id":106,"method":"missing"}),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 6, "{responses:#?}");
    let mut by_id: HashMap<i64, &Value> = HashMap::new();
    for response in &responses {
        let id = response["id"].as_i64().expect("numeric id echo");
        assert!(by_id.insert(id, response).is_none(), "duplicate id {id}");
        let has_result = response.get("result").is_some();
        let has_error = response.get("error").is_some();
        assert!(
            has_result ^ has_error,
            "response must carry exactly one of result/error: {response:#}"
        );
    }
    assert_eq!(
        by_id.len(),
        6,
        "id multiset mismatch: {responses:#?}"
    );
    for id in 101..=106 {
        assert!(by_id.contains_key(&id), "missing id {id}");
    }
    assert_eq!(by_id[&106]["error"]["code"], -32601);
    assert_eq!(by_id[&105]["result"]["isError"], true);
    assert!(by_id[&105].get("error").is_none());
    assert_eq!(by_id[&103]["result"]["isError"], false, "{:#}", by_id[&103]);
    assert_eq!(by_id[&104]["result"]["isError"], false, "{:#}", by_id[&104]);
}

#[test]
fn malformed_envelope_topology_method_vs_tool_errors() {
    // JSON-RPC topology: unknown methods and unshaped tools/call envelopes
    // are top-level -32601 with no `result`; calls that reach dispatch (even
    // with absent/null arguments, which default to `{}`) are tool errors.
    let responses = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"missing"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"keyword_search","arguments":[]}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"keyword_search"}}),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"keyword_search","arguments":null}}),
        ],
        None,
    );
    assert_eq!(responses.len(), 5);
    for response in &responses[..3] {
        assert_eq!(response["error"]["code"], -32601, "{response:#}");
        assert!(response.get("result").is_none(), "{response:#}");
    }
    assert_tool_error_shape(&responses[3]);
    assert_tool_error_shape(&responses[4]);
}
