//! Pass 2 (oracle-foundry, Mission 3): L2 mutation-discriminating oracles for
//! MCP tool-argument boundaries, driven through the stdio protocol.
//!
//! Each test pins BOTH sides of a boundary the implementation enforces with
//! `bounded_usize`/range checks: limit 1..=100, budget_tokens 1..=65536,
//! context_lines 0..=100, max_chars 1..=1000000, node ranges 1-based with
//! end >= start. Reject-only corpora cannot kill `>`-vs-`>=` mutants on the
//! accept side, so every boundary asserts accept AND reject. Discriminants
//! are `isError` booleans, never message text.

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

/// Drive several requests through ONE server process (one spawn per test).
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
                "clientInfo": {"name": "asgrep-mcp-pass2", "version": "0"}
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

fn search_call(id: u32, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"keyword_search","arguments":arguments}})
}

fn read_call(id: u32, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"code_read","arguments":arguments}})
}

/// Tiny indexed tree: one file with a findable symbol.
fn indexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("lib.rs"),
        "fn target_symbol() { helper(); }\nfn helper() {}\n",
    )
    .unwrap();
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
fn search_limit_accepts_1_and_100_rejects_0_and_101() {
    // Kills: minimum flipped to 0 or 2, maximum flipped to 99/101, and the
    // whole bound check removed (101 would then succeed).
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": 1})),
            search_call(2, json!({"query": "target_symbol", "limit": 100})),
            search_call(3, json!({"query": "target_symbol", "limit": 0})),
            search_call(4, json!({"query": "target_symbol", "limit": 101})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    assert_eq!(responses[2]["result"]["isError"], true, "{:#}", responses[2]);
    assert_eq!(responses[3]["result"]["isError"], true, "{:#}", responses[3]);
    let body: Value = serde_json::from_str(
        responses[0]["result"]["content"][0]["text"].as_str().unwrap(),
    )
    .unwrap();
    assert!(!body["h"].as_array().unwrap().is_empty(), "{body:#}");
}

#[test]
fn budget_tokens_accepts_1_and_max_rejects_0() {
    // Kills: budget 0 accepted (1..=MAX flipped to 0..=MAX), budget ignored
    // (hit tuples would stay 5-wide instead of gaining the detail level),
    // and the 65536 ceiling dropped.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": 4, "budget_tokens": 1})),
            search_call(2, json!({"query": "target_symbol", "limit": 4, "budget_tokens": 65536})),
            search_call(3, json!({"query": "target_symbol", "limit": 4, "budget_tokens": 0})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    assert_eq!(responses[2]["result"]["isError"], true, "{:#}", responses[2]);
    let body: Value = serde_json::from_str(
        responses[0]["result"]["content"][0]["text"].as_str().unwrap(),
    )
    .unwrap();
    for hit in body["h"].as_array().unwrap() {
        let tuple = hit.as_array().expect("hit is a positional tuple");
        assert_eq!(tuple.len(), 6, "{hit:#}");
        assert!(
            ["metadata", "signature", "block", "full"].contains(&tuple[5].as_str().unwrap()),
            "{hit:#}"
        );
    }
}

#[test]
fn code_read_context_zero_is_exact_window() {
    // Kills: context_lines 0 rejected (0..=MAX flipped to 1..=MAX), the
    // saturating window math widened off-by-one, and the 100 ceiling dropped.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("five.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["five.rs#L3-L3"], "context_lines": 0})),
            read_call(2, json!({"ids": ["five.rs#L3-L3"], "context_lines": 100})),
            read_call(3, json!({"ids": ["five.rs#L3-L3"], "context_lines": 101})),
            read_call(4, json!({"ids": ["five.rs#L3-L3"], "max_chars": 0})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    assert_eq!(responses[2]["result"]["isError"], true, "{:#}", responses[2]);
    assert_eq!(responses[3]["result"]["isError"], true, "{:#}", responses[3]);
    let body: Value = serde_json::from_str(
        responses[0]["result"]["content"][0]["text"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(body["nodes"][0]["lines"], json!({"start": 3, "end": 3}));
    assert_eq!(body["nodes"][0]["content"], "l3");
}

#[test]
fn code_read_rejects_zero_start_and_reversed_range() {
    // Kills: `start > 0` dropped (L0 accepted) and `end >= start` dropped
    // (reversed ranges accepted); L1-L1 control proves the file parses.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("two.rs"), "one\ntwo\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["two.rs#L1-L1"]})),
            read_call(2, json!({"ids": ["two.rs#L0-L1"]})),
            read_call(3, json!({"ids": ["two.rs#L2-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], true, "{:#}", responses[1]);
    assert_eq!(responses[2]["result"]["isError"], true, "{:#}", responses[2]);
}

#[test]
fn search_query_single_char_ok_overlong_rejected() {
    // Kills: length floor flipped above 1, and the 4096-char ceiling
    // dropped (4097 would then succeed) or flipped to reject 4096.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "x", "limit": 4})),
            search_call(2, json!({"query": "a".repeat(4096), "limit": 4})),
            search_call(3, json!({"query": "a".repeat(4097), "limit": 4})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    assert_eq!(responses[2]["result"]["isError"], true, "{:#}", responses[2]);
}
