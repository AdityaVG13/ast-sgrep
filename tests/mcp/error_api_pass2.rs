//! Pass 2 (errorapi, Mission E2): backend-failure propagation oracles.
//!
//! Pass 1 pins the error TAXONOMY (which trigger yields which row). This file
//! pins PROPAGATION: every backend failure must surface as the documented
//! envelope, never as a success result and never as a different level's
//! error. Method: hold the call arguments constant and vary only backend
//! state (index absent/corrupt/pinned-at-dir, env-gated refusal, filesystem
//! mutation, session registry state). Outcome differences are then
//! backend-attributed by construction.
//!
//! Pinned propagation map (discriminants are codes and shapes, never message
//! text; `success` = `isError: false` + `structuredContent`, `toolerr` = the
//! uniform tool-error envelope with no top-level `error`):
//!
//! | row | backend state (args held valid)            | mapping                                  |
//! |-----|--------------------------------------------|------------------------------------------|
//! | P1  | no index, all 5 search channels            | success all; miss x4, native hits x1       |
//! | P2  | corrupt index db, all 5 search channels    | toolerr; `code_read` still success         |
//! | P3  | `ASGREP_NEURAL_EMBED=1`, neural unshipped  | `semantic_search` toolerr; unset: success  |
//! | P4  | read-target file deleted mid-session       | same id flips success->toolerr->success    |
//! | P5  | symlink escapes workspace (unix)           | `code_read` toolerr; session survives      |
//! | P6  | multi-id read with one bad id              | whole-call toolerr; no partial `nodes`     |
//! | P7  | compact id before/after search registration| toolerr, then success for the same shape   |
//! | P8  | sequential mixed-outcome transcript         | per-call envelopes, ids echo 1..=7 in order |
//! | P9  | pipelined mixed batch incl. backend failure| every id exactly once; good results intact |
//! | P10 | empty vs corrupt index, identical query    | success miss vs toolerr (state decides)      |
//!
//! Non-duplication: pass 1 pins parse-level rows (arg bounds/types, unknown
//! keys, sandbox escape, JSON-RPC codes); recovery pins corrupt/pinned-garbage
//! index faults and heal flows; protocol pins single-channel miss/binary/EOF
//! rows and `isError` bits. E2 adds cross-channel uniformity, env-gated and
//! path-state backend refusals, read atomicity, registry-dependent resolution,
//! and mixed-outcome transcripts with a backend-failure row (prior mixed
//! transcripts carry only unknown-tool/method rows).

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

fn init_payload() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "__init",
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "asgrep-mcp-test", "version": "0"}
        }
    })
}

fn initialized_notif() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// Handshake, then strictly sequential send-one/read-one.
fn rpc_session(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    rpc_session_env(payloads, root, &[])
}

fn rpc_session_env(
    payloads: Vec<Value>,
    root: Option<&Path>,
    extra_env: &[(&str, &str)],
) -> Vec<Value> {
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    for (key, value) in extra_env {
        command.env(key, value);
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

/// Handshake, then fire every payload without waiting and read exactly one
/// response per request. Order is NOT pinned (the server answers
/// concurrently); callers key by id.
fn rpc_pipeline(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    let mut child = Command::new(mcp_bin());
    child.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        child.env("ASGREP_ROOT", root);
    }
    let mut child = child.spawn().expect("spawn MCP");
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
    for payload in &payloads {
        writeln!(stdin, "{payload}").unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut responses = Vec::new();
    for _ in 0..payloads.len() {
        line.clear();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout early");
        responses.push(serde_json::from_str(line.trim()).expect("JSON-RPC"));
    }
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

/// Uniform tool-error envelope: `isError: true`, no top-level `error`, one
/// `text` block, no `structuredContent`. Never asserts message text.
fn assert_tool_error_shape(response: &Value) {
    assert_eq!(response["result"]["isError"], true, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert_eq!(
        response["result"]["content"][0]["type"],
        "text",
        "{response:#}"
    );
    assert_eq!(
        response["result"]["content"].as_array().map(Vec::len),
        Some(1),
        "{response:#}"
    );
    assert!(
        response["result"].get("structuredContent").is_none(),
        "{response:#}"
    );
}

/// Tool-success envelope: `isError: false`, no top-level `error`, and a
/// machine-readable `structuredContent` body mirroring the text block.
fn assert_tool_success_shape(response: &Value) {
    assert_eq!(response["result"]["isError"], false, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert!(
        response["result"].get("structuredContent").is_some(),
        "{response:#}"
    );
}

fn assert_jsonrpc_error(response: &Value, code: i64) {
    assert_eq!(response["error"]["code"], code, "{response:#}");
    assert!(response["error"].is_object(), "{response:#}");
    assert!(response.get("result").is_none(), "{response:#}");
}

fn index_tree(path: &Path) {
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: path.to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();
}

/// Single-file tree with one findable symbol, not yet indexed.
fn file_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    temp
}

/// Single-file tree with one findable symbol, indexed.
fn indexed_tree() -> tempfile::TempDir {
    let temp = file_tree();
    index_tree(temp.path());
    temp
}

/// Overwrite the durable index db with deterministic non-SQLite bytes.
fn corrupt_index_db(root: &Path) {
    let db = root.join(".asgrep").join("index.db");
    assert!(db.is_file(), "expected an index db at {}", db.display());
    std::fs::write(&db, "E2-corrupt-index-sentinel;".repeat(128)).unwrap();
}

const SEARCH_CHANNELS: [&str; 5] = [
    "search",
    "keyword_search",
    "ast_search",
    "semantic_search",
    "code_search",
];

/// P1: no index is a success on EVERY search channel, never a tool error --
/// but the shape follows each channel's contract. Index-backed channels
/// (`search`, `keyword_search`, `semantic_search`, `code_search`) return the
/// `empty_index` miss (`why`, `zn: 0`, empty `h`); native `ast_search` needs
/// no index and returns hits. Args are valid; backend state alone decides.
#[test]
fn backend_empty_index_is_success_miss_on_every_channel() {
    let temp = file_tree();
    let calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| {
            tool_call(
                i as u32 + 1,
                channel,
                json!({"query": "target_symbol", "limit": 4}),
            )
        })
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), SEARCH_CHANNELS.len());
    for (response, channel) in responses.iter().zip(SEARCH_CHANNELS) {
        assert_tool_success_shape(response);
        let body = tool_body(response);
        if channel == "ast_search" {
            assert!(
                body["zn"].as_u64().unwrap_or(0) >= 1,
                "{channel}: {body:#}"
            );
            assert!(
                !body["h"].as_array().unwrap().is_empty(),
                "{channel}: {body:#}"
            );
        } else {
            assert_eq!(body["why"], "empty_index", "{channel}: {body:#}");
            assert_eq!(body["zn"], 0, "{channel}: {body:#}");
            assert_eq!(body["h"], json!([]), "{channel}: {body:#}");
        }
    }
}

/// P2: a corrupt index db is a tool error on EVERY search channel -- never a
/// silent empty success, never fabricated hits -- while `code_read` serves
/// files directly and stays a success.
#[test]
fn backend_corrupt_index_is_tool_error_on_every_channel() {
    let temp = indexed_tree();
    corrupt_index_db(temp.path());
    let mut calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| {
            tool_call(
                i as u32 + 1,
                channel,
                json!({"query": "target_symbol", "limit": 4}),
            )
        })
        .collect();
    calls.push(tool_call(
        6,
        "code_read",
        json!({"ids": ["src/lib.rs#L1-L1"]}),
    ));
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), 6);
    for (response, id) in responses[..5].iter().zip(1..=5) {
        assert_tool_error_shape(response);
        assert_eq!(response["id"], id, "{response:#}");
    }
    assert_tool_success_shape(&responses[5]);
    assert_eq!(
        tool_body(&responses[5])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[5]
    );
}

/// P3: identical valid `semantic_search` args succeed by default but are a
/// tool error (never a JSON-RPC error, never a success) when the backend is
/// configured for neural embed this build cannot serve. The refusal is
/// backend-attributed: only the env differs.
#[test]
fn backend_neural_gate_refusal_is_tool_error_not_success() {
    let temp = indexed_tree();
    let args = json!({"query": "target_symbol", "limit": 4});
    let ok = rpc_session(
        vec![tool_call(1, "semantic_search", args.clone())],
        Some(temp.path()),
    );
    assert_tool_success_shape(&ok[0]);
    let refused = rpc_session_env(
        vec![tool_call(1, "semantic_search", args)],
        Some(temp.path()),
        &[("ASGREP_NEURAL_EMBED", "1")],
    );
    assert_eq!(refused.len(), 1);
    assert_tool_error_shape(&refused[0]);
    assert_eq!(refused[0]["id"], 1, "{:#}", refused[0]);
}

/// P4: the SAME well-formed node id flips success -> tool error -> success
/// within one session as the backend file is deleted and restored. The id
/// parses identically every time, so the middle failure is purely
/// backend-attributed (read-time, not parse-time), and it never surfaces as
/// a success with empty nodes.
#[test]
fn backend_file_deleted_mid_session_flips_read_to_tool_error() {
    let temp = file_tree();
    let victim = temp.path().join("src").join("lib.rs");
    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .env("ASGREP_ROOT", temp.path())
        .spawn()
        .expect("spawn MCP");
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
    assert_eq!(recv(&mut stdout)["id"], "__init");
    send(&mut stdin, &initialized_notif());

    let read = || tool_call(0, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}));
    let mut call = |id: u32| -> Value {
        let mut payload = read();
        payload["id"] = json!(id);
        send(&mut stdin, &payload);
        recv(&mut stdout)
    };

    let before = call(1);
    assert_tool_success_shape(&before);
    assert_eq!(
        tool_body(&before)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{before:#}"
    );

    std::fs::remove_file(&victim).unwrap();
    let during = call(2);
    assert_tool_error_shape(&during);
    assert_eq!(during["id"], 2, "{during:#}");

    std::fs::write(&victim, "fn target_symbol() {}\n").unwrap();
    let after = call(3);
    assert_tool_success_shape(&after);
    assert_eq!(
        tool_body(&after)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{after:#}"
    );

    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
}

/// P5: a symlink inside the workspace pointing outside passes id-shape parse
/// and fails at filesystem containment -- a backend failure, hence a tool
/// error -- and the session serves a valid read right after.
#[cfg(unix)]
#[test]
fn backend_symlink_escape_read_is_tool_error_session_survives() {
    let workspace = file_tree();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.rs"), "fn secret() {}\n").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.rs"),
        workspace.path().join("link.rs"),
    )
    .unwrap();
    let responses = rpc_session(
        vec![
            tool_call(1, "code_read", json!({"ids": ["link.rs#L1-L1"]})),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(workspace.path()),
    );
    assert_eq!(responses.len(), 2);
    assert_tool_error_shape(&responses[0]);
    assert_eq!(responses[0]["id"], 1, "{:#}", responses[0]);
    assert_tool_success_shape(&responses[1]);
    assert_eq!(
        tool_body(&responses[1])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[1]
    );
}

/// P6: a multi-id `code_read` with one unreadable id fails the WHOLE call as
/// a tool error in either position -- no partial `nodes` leak through any
/// channel -- and a lone good read succeeds in the same session.
#[test]
fn backend_multi_id_read_fails_atomically_no_partial_nodes() {
    let temp = file_tree();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1", "src/lib.rs#L1-L99"]}),
            ),
            tool_call(
                2,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L99", "src/lib.rs#L1-L1"]}),
            ),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for (response, id) in responses[..2].iter().zip(1..=2) {
        assert_tool_error_shape(response);
        assert_eq!(response["id"], id, "{response:#}");
    }
    assert_tool_success_shape(&responses[2]);
    assert_eq!(
        tool_body(&responses[2])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[2]
    );
}

/// P7: a well-shaped compact id with an empty session registry is a tool
/// error; after a search registers that registry, the hit's own compact id
/// resolves to a success. Same id shape, opposite outcomes -- the failure is
/// session-backend state, not syntax.
#[test]
fn backend_compact_id_resolves_only_after_search_registration() {
    let temp = indexed_tree();
    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .env("ASGREP_ROOT", temp.path())
        .spawn()
        .expect("spawn MCP");
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
    assert_eq!(recv(&mut stdout)["id"], "__init");
    send(&mut stdin, &initialized_notif());

    send(
        &mut stdin,
        &tool_call(1, "code_read", json!({"ids": ["0:1-1"]})),
    );
    let unregistered = recv(&mut stdout);
    assert_tool_error_shape(&unregistered);
    assert_eq!(unregistered["id"], 1, "{unregistered:#}");

    send(
        &mut stdin,
        &tool_call(
            2,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ),
    );
    let search = recv(&mut stdout);
    assert_tool_success_shape(&search);
    let hit_id = tool_body(&search)["h"][0][0]
        .as_str()
        .expect("compact hit id")
        .to_owned();
    assert!(!hit_id.contains("#L"), "expected a compact id, got {hit_id}");

    send(&mut stdin, &tool_call(3, "code_read", json!({"ids": [hit_id]})));
    let resolved = recv(&mut stdout);
    assert_tool_success_shape(&resolved);
    assert_eq!(
        tool_body(&resolved)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{resolved:#}"
    );

    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
}

/// P8: one sequential transcript, five outcome classes. Unknown-tool,
/// invalid-args, and backend-failure rows are uniform tool errors (never
/// top-level `error`, never success); the unknown method is -32601 with no
/// `result`; successes bracket the failures with ids echoing 1..=7 in order.
/// No bad call drops or reorders any other result.
#[test]
fn propagation_sequence_reports_per_call_errors_without_dropping_results() {
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
            tool_call(3, "no_such_tool", json!({})),
            tool_call(4, "keyword_search", json!({"query": "x", "limit": 0})),
            json!({"jsonrpc":"2.0","id":5,"method":"missing"}),
            tool_call(
                6,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(7, "index_status", json!({})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 7);
    for (response, id) in responses.iter().zip(1..=7) {
        assert_eq!(response["id"], id, "{response:#}");
    }
    // Successes: full success shape, real payloads.
    assert_tool_success_shape(&responses[0]);
    assert_eq!(
        tool_body(&responses[0])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[0]
    );
    assert_tool_success_shape(&responses[5]);
    assert!(
        !tool_body(&responses[5])["h"].as_array().unwrap().is_empty(),
        "{:#}",
        responses[5]
    );
    assert_tool_success_shape(&responses[6]);
    assert_eq!(tool_body(&responses[6])["file_count"], 1, "{:#}", responses[6]);
    // The three tool-level failure classes share one envelope, mutually
    // indistinguishable by shape -- and none leaks to another level.
    for response in &responses[1..4] {
        assert_tool_error_shape(response);
    }
    // Unknown method is the only JSON-RPC-level error.
    assert_jsonrpc_error(&responses[4], -32601);
}

/// P9: a pipelined batch with a backend failure plus an invalid-args row:
/// every id returns exactly once, each response carries exactly one of
/// `result`/`error`, per-id classes hold, and the good results are intact.
/// Order is not pinned (concurrent service).
#[test]
fn propagation_pipelined_batch_with_backend_failure_keeps_every_id() {
    let temp = indexed_tree();
    let responses = rpc_pipeline(
        vec![
            tool_call(11, "index_status", json!({})),
            tool_call(
                12,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(13, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
            tool_call(14, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(15, "no_such_tool", json!({})),
            json!({"jsonrpc":"2.0","id":16,"method":"missing"}),
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
            "exactly one of result/error: {response:#}"
        );
    }
    for id in 11..=16 {
        assert!(by_id.contains_key(&id), "missing id {id}");
    }
    assert_tool_success_shape(by_id[&11]);
    assert_eq!(tool_body(by_id[&11])["file_count"], 1);
    assert_tool_success_shape(by_id[&12]);
    assert!(!tool_body(by_id[&12])["h"].as_array().unwrap().is_empty());
    assert_tool_error_shape(by_id[&13]);
    assert_tool_error_shape(by_id[&14]);
    assert_tool_error_shape(by_id[&15]);
    assert_jsonrpc_error(by_id[&16], -32601);
}

/// P10: the same valid query against an empty index is a success miss while
/// against a corrupt index it is a tool error. Backend state alone flips the
/// documented mapping; neither failure mode is confused with the other.
#[test]
fn backend_empty_vs_corrupt_index_map_to_miss_vs_tool_error() {
    let empty = file_tree();
    let corrupt = indexed_tree();
    corrupt_index_db(corrupt.path());
    let args = json!({"query": "target_symbol", "limit": 4});
    let miss = rpc_session(
        vec![tool_call(1, "search", args.clone())],
        Some(empty.path()),
    );
    let failure = rpc_session(vec![tool_call(1, "search", args)], Some(corrupt.path()));
    assert_tool_success_shape(&miss[0]);
    let body = tool_body(&miss[0]);
    assert_eq!(body["why"], "empty_index", "{body:#}");
    assert_eq!(body["zn"], 0, "{body:#}");
    assert_tool_error_shape(&failure[0]);
    assert_eq!(failure[0]["id"], 1, "{:#}", failure[0]);
}
