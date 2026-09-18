//! R1 recovery-contract oracles for ast-sgrep-mcp durable state.
//!
//! Non-overlap contract: `protocol.rs` pins handshake/negotiation, discovery,
//! structured content, tool names, per-channel kinds, single-id expansion, read
//! windows, schema rejection, sandbox escapes, byte-stability, elision,
//! miss envelopes, and cancellation. Pass 2 pins argument boundaries, pass 3
//! pins relations between calls, and pass 4 pins end-to-end flows (including
//! missing *per-call* roots and single-session empty trees). This file pins
//! NONE of those again. Instead it pins DURABLE-STATE recovery:
//!
//! * startup config: a missing `ASGREP_ROOT` fails the process closed with no
//!   JSON-RPC on stdout (vs pass 4's per-call missing root, which is a tool
//!   error inside a live session);
//! * workspace removal mid-session: every tool fails closed with the uniform
//!   tool-error shape while `ping` still answers and the process exits 0;
//! * per-call root pointing at a regular file (not a dir, not missing, not
//!   escaping): uniform tool errors;
//! * corrupt `index.db` under the root: index-dependent tools refuse loudly
//!   (tool errors, never silent empty success or fabricated hits) while
//!   `code_read` keeps serving files; deleting the corrupt db plus `index_repo`
//!   heals search and status;
//! * empty-root restart determinism: tools/list, status, and the zero-hit miss
//!   reproduce byte-identically across fresh processes;
//! * session restart clears snippet-elision memory (session state is ephemeral,
//!   not durable);
//! * pinned `ASGREP_INDEX_PATH` garbage is refused loudly, and the default
//!   root index is unaffected once the pin is gone.
//!
//! Discriminants are exit codes, `isError` booleans, envelope shapes (key
//! presence, tuple widths, counts), and byte equality -- never message text.

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

fn init_payload() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "__init",
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "asgrep-mcp-r1", "version": "0"}
        }
    })
}

/// Drive several requests through ONE server process, strictly sequential
/// (send one, read one), so response order matches request order.
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

fn index_tree(path: &Path) {
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: path.to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();
}

/// Single-file indexed tree with one findable symbol.
fn indexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    index_tree(temp.path());
    temp
}

/// Overwrite the durable index db with deterministic non-SQLite bytes.
fn corrupt_index_db(root: &Path) {
    let db = root.join(".asgrep").join("index.db");
    assert!(db.is_file(), "expected an index db at {}", db.display());
    let mut garbage = Vec::new();
    for _ in 0..128 {
        garbage.extend_from_slice(b"R1-corrupt-index-sentinel;");
    }
    std::fs::write(&db, garbage).unwrap();
}

#[test]
fn startup_with_missing_asgrep_root_exits_without_json() {
    // Durable-config recovery: the configured workspace itself does not
    // exist, so there is no session to serve. The process must fail closed
    // (nonzero exit) with zero JSON-RPC responses on stdout -- never a
    // half-initialized server that answers tools/list against nothing.
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("does_not_exist");
    assert!(!missing.exists());
    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("ASGREP_ROOT", &missing)
        .spawn()
        .expect("spawn MCP");
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{}", init_payload()).unwrap();
    }
    let output = child.wait_with_output().expect("wait MCP");
    assert!(
        !output.status.success(),
        "missing ASGREP_ROOT must fail the process, got {}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let rpc_lines = stdout
        .lines()
        .filter(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v.get("jsonrpc").cloned())
                .is_some()
        })
        .count();
    assert_eq!(rpc_lines, 0, "no JSON-RPC may escape a failed startup: {stdout:?}");
}

#[test]
fn workspace_root_removed_mid_session_fails_tools_closed_server_survives() {
    // The workspace root is durable state the server re-validates per call.
    // Removing it mid-session must fail every tool closed with the uniform
    // shape while `ping` still answers and the process exits cleanly.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").unwrap();
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
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );

    send(&mut stdin, &tool_call(1, "index_status", json!({})));
    let before = recv(&mut stdout);
    assert_eq!(before["result"]["isError"], false, "{before:#}");

    std::fs::remove_dir_all(temp.path()).unwrap();
    assert!(!temp.path().exists());

    send(&mut stdin, &tool_call(2, "index_status", json!({})));
    send(
        &mut stdin,
        &tool_call(3, "keyword_search", json!({"query": "hey", "limit": 4})),
    );
    send(
        &mut stdin,
        &tool_call(4, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
    );
    // The three tool calls above were pipelined; collect all three before ping.
    let after_status = recv(&mut stdout);
    let after_search = recv(&mut stdout);
    let after_read = recv(&mut stdout);
    // Order is pinned by the tool lock, but match by id to be exact.
    let mut by_id = std::collections::HashMap::new();
    for r in [after_status, after_search, after_read] {
        by_id.insert(r["id"].as_u64().unwrap(), r);
    }
    for id in [2u64, 3, 4] {
        assert_tool_error_shape(&by_id[&id]);
    }
    send(&mut stdin, &json!({"jsonrpc":"2.0","id":5,"method":"ping"}));
    let ping = recv(&mut stdout);
    assert_eq!(ping["id"], 5, "{ping:#}");
    assert!(ping.get("error").is_none(), "{ping:#}");
    assert!(ping.get("result").is_some(), "{ping:#}");
    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
}

#[test]
fn per_call_root_pointing_at_file_fails_closed() {
    // A per-call root that exists but is a regular file (not a directory) is
    // neither the missing-root nor the escaping-root case: every tool must
    // still fail closed with the uniform tool-error shape.
    let temp = indexed_tree();
    let file_root = temp.path().join("src").join("lib.rs");
    assert!(file_root.is_file());
    let file_root = file_root.display().to_string();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({"root": file_root})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "root": file_root}),
            ),
            tool_call(
                3,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": file_root}),
            ),
            tool_call(4, "index_repo", json!({"root": file_root})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

#[test]
fn corrupt_index_db_refused_loudly_across_tools_while_reads_survive() {
    // A garbage `index.db` under an otherwise healthy root must be refused
    // loudly: every index-dependent tool (status, search, reindex with and
    // without force) is a tool error -- never a silent empty success, never
    // fabricated hits. `code_read` serves files directly, so it is unaffected.
    let temp = indexed_tree();
    corrupt_index_db(temp.path());
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4}),
            ),
            tool_call(3, "index_repo", json!({})),
            tool_call(4, "index_repo", json!({"force": true})),
            tool_call(5, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 5);
    for response in &responses[..4] {
        assert_tool_error_shape(response);
    }
    assert_eq!(responses[4]["result"]["isError"], false, "{:#}", responses[4]);
    assert_eq!(tool_body(&responses[4])["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn deleting_corrupt_db_then_reindex_heals_search_and_status() {
    // Recovery path for a corrupt db: remove the corrupt inode, `index_repo`
    // rebuilds from source, and status plus search serve the healed index.
    let temp = indexed_tree();
    corrupt_index_db(temp.path());
    std::fs::remove_file(temp.path().join(".asgrep").join("index.db")).unwrap();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_repo", json!({})),
            tool_call(2, "index_status", json!({})),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    assert_eq!(tool_body(&responses[0])["files_indexed"], 1);
    assert_eq!(tool_body(&responses[1])["file_count"], 1);
    let envelope = tool_body(&responses[2]);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
}

#[test]
fn empty_root_chain_reproduces_identically_across_restarts() {
    // An empty root (no files, no index) serves deterministic zero-hit
    // responses: tools/list, status, and the miss envelope reproduce
    // byte-identically in a fresh process. Pass 4 pins the single-session
    // shapes; this pins restart determinism of the empty-root chain.
    let temp = tempfile::tempdir().unwrap();
    let chain = || {
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            tool_call(2, "index_status", json!({})),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "anything", "limit": 4, "resend_seen": true}),
            ),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 3);
    assert_eq!(second.len(), 3);
    assert_eq!(
        serde_json::to_string(&first[0]["result"]).unwrap(),
        serde_json::to_string(&second[0]["result"]).unwrap(),
        "tools/list drifted across restarts"
    );
    assert_eq!(first[0]["result"]["tools"].as_array().unwrap().len(), 8);
    assert_eq!(tool_text(&first[1]), tool_text(&second[1]), "status drifted");
    assert_eq!(first[1]["result"]["isError"], false);
    assert_eq!(tool_body(&first[1])["file_count"], 0);
    assert_eq!(tool_text(&first[2]), tool_text(&second[2]), "miss drifted");
    assert_eq!(first[2]["result"]["isError"], false);
    let miss = tool_body(&first[2]);
    assert_eq!(miss["why"], "empty_index", "{miss:#}");
    assert_eq!(miss["zn"], 0);
    assert_eq!(miss["h"].as_array().unwrap().len(), 0);
    assert!(miss.get("p").is_none(), "miss carries no path table: {miss:#}");
}

#[test]
fn session_restart_clears_snippet_elision_state() {
    // Snippet elision is session memory, not durable state: a fresh process
    // re-sends full snippets, byte-identical to the first session's first
    // response. Protocol pins elision-until-reindex within one session; this
    // pins that a restart (not just a reindex) resets it.
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("lib.rs"),
        "fn target_symbol() { helper(); }\nfn helper() {}\n",
    )
    .unwrap();
    index_tree(temp.path());
    let search = || {
        tool_call(
            1,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4}),
        )
    };
    let first_session = rpc_session(vec![search(), search()], Some(temp.path()));
    assert_eq!(first_session.len(), 2);
    let first_bytes = tool_text(&first_session[0]).to_owned();
    let elided_bytes = tool_text(&first_session[1]).to_owned();
    let elided = tool_body(&first_session[1]);
    assert!(
        elided["h"]
            .as_array()
            .unwrap()
            .iter()
            .all(|hit| hit[4] == "~"),
        "expected every snippet elided: {elided:#}"
    );
    assert!(elided["ze"].as_u64().unwrap() > 0, "{elided:#}");
    assert!(
        elided_bytes.len() < first_bytes.len(),
        "elided response must be smaller"
    );

    let second_session = rpc_session(vec![search()], Some(temp.path()));
    assert_eq!(
        tool_text(&second_session[0]),
        first_bytes,
        "restart must restore full snippets"
    );
    assert!(
        tool_body(&second_session[0]).get("ze").is_none(),
        "fresh session must not elide: {:#}",
        second_session[0]
    );
}

#[test]
fn pinned_index_path_garbage_refused_loudly_default_root_unaffected() {
    // `ASGREP_INDEX_PATH` pins the durable db location. A garbage pinned db
    // must be refused loudly, and dropping the pin must serve the healthy
    // default index again -- the pin never poisons the root's own db.
    let temp = indexed_tree();
    let pin_dir = tempfile::tempdir().unwrap();
    let pinned = pin_dir.path().join("pinned.db");
    std::fs::write(&pinned, "R1-pinned-garbage;".repeat(256)).unwrap();
    let pinned = pinned.display().to_string();
    let responses = rpc_session_env(
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4}),
            ),
        ],
        Some(temp.path()),
        &[("ASGREP_INDEX_PATH", pinned.as_str())],
    );
    assert_eq!(responses.len(), 2);
    for response in &responses {
        assert_tool_error_shape(response);
    }

    let responses = rpc_session(
        vec![
            tool_call(3, "index_status", json!({})),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(tool_body(&responses[0])["file_count"], 1);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    assert!(!tool_body(&responses[1])["h"].as_array().unwrap().is_empty());
}
