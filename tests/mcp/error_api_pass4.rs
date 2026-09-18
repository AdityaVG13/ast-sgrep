//! Pass 4 (errorapi, Mission E4): end-to-end error DRILLS on live sessions.
//!
//! Pass 1 pins the error TAXONOMY, pass 2 pins backend PROPAGATION, pass 3
//! pins cross-call RELATIONS, and recovery pins crash/restart durability.
//! This file pins live-session DRILLS: a working stdio session takes a fault
//! injection mid-stream (no kill, no restart), each failing call reports its
//! documented per-call code/shape, and the SAME session keeps serving good
//! calls -- during the fault where the contract allows it, and after a live
//! heal otherwise.
//!
//! Pinned drills (discriminants are codes, envelope shapes, and payload
//! discriminants such as `why`/`zn`/`h`, never message text; every drill
//! ends with good calls succeeding and a clean exit):
//!
//! | row | live fault injection                        | per-call codes/shapes + usability |
//! |-----|---------------------------------------------|-----------------------------------|
//! | D1  | `index.db` deleted mid-session              | miss mapping, live `index_repo` heals |
//! | D2  | `index.db` overwritten with garbage live    | toolerr x3, read survives, live heal |
//! | D3  | `.asgrep/` dir deleted mid-session          | miss mapping, live rebuild recreates |
//! | D4  | malformed envelopes mid-session             | -32601/-32600 codes, session usable |
//! | D5  | bad-argument storm mid-session              | uniform toolerr, session usable     |
//! | D6  | sandbox-escape roots mid-session            | toolerr x3, session unpoisoned      |
//! | D7  | CHAINED double fault (garbage db + deleted source) | staged heal, faults independent |
//!
//! Non-duplication: recovery pins mid-session stub/half index tears, root
//! deletion + live heal, EOF, garbage lines, and shape-only invalid envelopes,
//! plus kill+restart crash drills over corrupt/deleted indexes. E2 pins
//! pre-session corrupt/empty mappings and a mid-session file flip. E4 pins
//! only live-session injections with per-call CODE assertions and usability
//! afterward that no earlier suite states: deletion/miss mapping live (D1,
//! D3), overwrite + live heal without restart (D2), mid-session -32600 and
//! code values (D4), storm uniformity + usability (D5), escape + usability
//! (D6), and a staged-heal double fault (D7).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};

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

/// One live stdio session: handshake once, then sequential send-one/read-one
/// with filesystem fault injection between calls. No kills, no restarts.
struct LiveSession {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl LiveSession {
    fn spawn(root: Option<&Path>) -> Self {
        let mut command = Command::new(mcp_bin());
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        if let Some(root) = root {
            command.env("ASGREP_ROOT", root);
        }
        let mut child = command.spawn().expect("spawn MCP");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin: Some(stdin),
            stdout,
        }
    }

    fn handshake(&mut self) {
        self.send(&init_payload());
        assert_eq!(self.recv()["id"], "__init");
        self.send(&initialized_notif());
    }

    fn send(&mut self, payload: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout");
        serde_json::from_str(line.trim()).expect("JSON-RPC")
    }

    /// Send one call, read its response, assert the id echo.
    fn call(&mut self, payload: &Value, id: u32) -> Value {
        self.send(payload);
        let response = self.recv();
        assert_eq!(response["id"], id, "{response:#}");
        response
    }

    fn finish(mut self) -> ExitStatus {
        drop(self.stdin.take());
        self.child.wait().expect("wait MCP")
    }
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

/// JSON-RPC error row: numeric `code`, no `result`.
fn assert_jsonrpc_error(response: &Value, code: i64) {
    assert_eq!(response["error"]["code"], code, "{response:#}");
    assert!(response["error"].is_object(), "{response:#}");
    assert!(response.get("result").is_none(), "{response:#}");
}

/// Machine-readable error discriminant: everything about a tool error EXCEPT
/// the human message text.
fn tool_error_discriminant(response: &Value) -> (bool, usize, String, bool, bool, bool) {
    (
        response["result"]["isError"].as_bool().unwrap_or(false),
        response["result"]["content"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(usize::MAX),
        response["result"]["content"][0]["type"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        response["result"]["content"][0]["text"].is_string(),
        response["result"].get("structuredContent").is_some(),
        response.get("error").is_some(),
    )
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

fn index_db_path(root: &Path) -> PathBuf {
    root.join(".asgrep").join("index.db")
}

/// Overwrite the durable index db with deterministic non-SQLite bytes.
fn corrupt_index_db(root: &Path) {
    let db = index_db_path(root);
    assert!(db.is_file(), "expected an index db at {}", db.display());
    std::fs::write(&db, "E4-corrupt-index-sentinel;".repeat(128)).unwrap();
}

fn search_call(id: u32, channel: &str, limit: u32) -> Value {
    tool_call(
        id,
        channel,
        json!({"query": "target_symbol", "limit": limit, "resend_seen": true}),
    )
}

fn read_call(id: u32) -> Value {
    tool_call(id, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}))
}

/// D1: `index.db` is deleted under a live session. The next search is the
/// documented empty-miss SUCCESS (never a tool error, never stale hits),
/// status reports zero files, reads keep serving, and a live `index_repo`
/// rebuild heals search in the SAME session. Recovery pins deletion only
/// under kill+restart crash drills, never the live per-call mapping.
#[test]
fn drill_index_db_deleted_mid_session_miss_then_live_reindex_heals() {
    let temp = indexed_tree();
    assert!(index_db_path(temp.path()).is_file());
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&search_call(1, "keyword_search", 4), 1);
    assert_tool_success_shape(&before);
    assert!(
        !tool_body(&before)["h"].as_array().unwrap().is_empty(),
        "{before:#}"
    );

    std::fs::remove_file(index_db_path(temp.path())).unwrap();
    assert!(!index_db_path(temp.path()).exists());

    // Fresh limit so the call cannot ride the warm searcher cache.
    let miss = session.call(&search_call(2, "keyword_search", 8), 2);
    assert_tool_success_shape(&miss);
    let body = tool_body(&miss);
    assert_eq!(body["why"], "empty_index", "{body:#}");
    assert_eq!(body["zn"], 0, "{body:#}");
    assert_eq!(body["h"], json!([]), "{body:#}");

    let status = session.call(&tool_call(3, "index_status", json!({})), 3);
    assert_tool_success_shape(&status);
    assert_eq!(tool_body(&status)["file_count"], 0, "{status:#}");

    let read = session.call(&read_call(4), 4);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    let rebuilt = session.call(&tool_call(5, "index_repo", json!({})), 5);
    assert_tool_success_shape(&rebuilt);
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");

    let after = session.call(&search_call(6, "keyword_search", 12), 6);
    assert_tool_success_shape(&after);
    assert!(
        !tool_body(&after)["h"].as_array().unwrap().is_empty(),
        "{after:#}"
    );

    assert!(session.finish().success());
}

/// D2: `index.db` is overwritten with garbage under a live session. Search,
/// status, and even reindex-on-garbage are tool errors (never silent empty
/// successes), reads keep serving files, and deleting the corrupt inode plus
/// a live `index_repo` heals the SAME session. Recovery pins stub/half tears
/// live without healin-session, and overwrite only under kill+restart.
#[test]
fn drill_index_db_overwritten_mid_session_refused_then_live_heal() {
    let temp = indexed_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&search_call(1, "keyword_search", 4), 1);
    assert_tool_success_shape(&before);
    assert!(
        !tool_body(&before)["h"].as_array().unwrap().is_empty(),
        "{before:#}"
    );

    corrupt_index_db(temp.path());

    // Fresh limit so the call cannot ride the warm searcher cache.
    let search = session.call(&search_call(2, "keyword_search", 8), 2);
    assert_tool_error_shape(&search);
    let status = session.call(&tool_call(3, "index_status", json!({})), 3);
    assert_tool_error_shape(&status);
    let reindex_refused = session.call(&tool_call(4, "index_repo", json!({})), 4);
    assert_tool_error_shape(&reindex_refused);

    let read = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    std::fs::remove_file(index_db_path(temp.path())).unwrap();
    let rebuilt = session.call(&tool_call(6, "index_repo", json!({})), 6);
    assert_tool_success_shape(&rebuilt);
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");

    let after = session.call(&search_call(7, "keyword_search", 12), 7);
    assert_tool_success_shape(&after);
    assert!(
        !tool_body(&after)["h"].as_array().unwrap().is_empty(),
        "{after:#}"
    );

    assert!(session.finish().success());
}

/// D3: the whole `.asgrep/` directory is deleted under a live session. The
/// next search is the empty-miss success on the `search` channel, status
/// reports zero files, reads keep serving, and a live `index_repo` recreates
/// the directory and heals search. Dir-level removal is a distinct fault from
/// the file-level tears and deletions pinned elsewhere.
#[test]
fn drill_asgrep_dir_deleted_mid_session_miss_then_live_rebuild() {
    let temp = indexed_tree();
    assert!(temp.path().join(".asgrep").is_dir());
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&search_call(1, "search", 4), 1);
    assert_tool_success_shape(&before);
    assert!(
        !tool_body(&before)["h"].as_array().unwrap().is_empty(),
        "{before:#}"
    );

    std::fs::remove_dir_all(temp.path().join(".asgrep")).unwrap();
    assert!(!temp.path().join(".asgrep").exists());

    let miss = session.call(&search_call(2, "search", 8), 2);
    assert_tool_success_shape(&miss);
    let body = tool_body(&miss);
    assert_eq!(body["why"], "empty_index", "{body:#}");
    assert_eq!(body["zn"], 0, "{body:#}");
    assert_eq!(body["h"], json!([]), "{body:#}");

    let status = session.call(&tool_call(3, "index_status", json!({})), 3);
    assert_tool_success_shape(&status);
    assert_eq!(tool_body(&status)["file_count"], 0, "{status:#}");

    let read = session.call(&read_call(4), 4);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    let rebuilt = session.call(&tool_call(5, "index_repo", json!({})), 5);
    assert_tool_success_shape(&rebuilt);
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    assert!(index_db_path(temp.path()).is_file());

    let after = session.call(&search_call(6, "search", 12), 6);
    assert_tool_success_shape(&after);
    assert!(
        !tool_body(&after)["h"].as_array().unwrap().is_empty(),
        "{after:#}"
    );

    assert!(session.finish().success());
}

/// D4: malformed envelopes arrive mid-session between good calls. Each one
/// reports its documented JSON-RPC CODE (-32601 with id echo for unknown
/// method and unshaped `tools/call` params; -32600 with NO id for a request
/// with no method), and the session serves a read and a ping right after.
/// Recovery pins one unknown-method envelope mid-stream shape-only; E4 pins
/// the code values and the -32600 row live.
#[test]
fn drill_malformed_envelopes_mid_session_codes_then_session_usable() {
    let temp = file_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&read_call(1), 1);
    assert_tool_success_shape(&before);
    assert_eq!(
        tool_body(&before)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{before:#}"
    );

    let unknown = session.call(&json!({"jsonrpc":"2.0","id":2,"method":"missing"}), 2);
    assert_jsonrpc_error(&unknown, -32601);

    session.send(&json!({"jsonrpc": "2.0", "id": 3}));
    let invalid = session.recv();
    assert_jsonrpc_error(&invalid, -32600);
    assert!(
        invalid.get("id").is_none(),
        "-32600 must not echo an id: {invalid:#}"
    );

    let unshaped = session.call(
        &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{}}),
        4,
    );
    assert_jsonrpc_error(&unshaped, -32601);

    let read = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    let ping = session.call(&json!({"jsonrpc":"2.0","id":6,"method":"ping"}), 6);
    assert!(ping.get("error").is_none(), "{ping:#}");
    assert!(ping.get("result").is_some(), "{ping:#}");

    assert!(session.finish().success());
}

/// D5: a storm of bad-argument calls hits a live session: bound, sign, type,
/// unknown-key, shape, overflow, and unknown-tool faults. Every one is the
/// uniform tool-error envelope with its id echoed (one code, not seven), and
/// a search plus a read succeed immediately after. Pass 2 P8 pins one static
/// mixed sequence; this pins a parse-level storm with uniformity plus
/// usability-after on a live session.
#[test]
fn drill_bad_argument_storm_mid_session_uniform_then_usable() {
    let temp = indexed_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&search_call(1, "keyword_search", 4), 1);
    assert_tool_success_shape(&before);
    assert!(
        !tool_body(&before)["h"].as_array().unwrap().is_empty(),
        "{before:#}"
    );

    let twenty_one: Vec<Value> = (0..21).map(|_| json!("src/lib.rs#L1-L1")).collect();
    let storm = vec![
        tool_call(10, "keyword_search", json!({"query": "x", "limit": 0})),
        tool_call(11, "keyword_search", json!({"query": "x", "limit": -1})),
        tool_call(12, "keyword_search", json!({"query": "x", "limit": "many"})),
        tool_call(13, "keyword_search", json!({"query": "x", "bogus": 1})),
        tool_call(14, "keyword_search", json!({"query": "x", "root": 42})),
        tool_call(15, "code_read", json!({"ids": ["src/lib.rs#1-2"]})),
        tool_call(16, "code_read", json!({"ids": twenty_one})),
        tool_call(17, "index_repo", json!({"force": "yes"})),
        tool_call(18, "no_such_tool", json!({})),
    ];
    let mut discriminants = Vec::new();
    for (i, payload) in storm.iter().enumerate() {
        let id = 10 + i as u32;
        let response = session.call(payload, id);
        assert_tool_error_shape(&response);
        discriminants.push(tool_error_discriminant(&response));
    }
    for discriminant in &discriminants[1..] {
        assert_eq!(
            *discriminant, discriminants[0],
            "storm fault diverged from the uniform tool-error code"
        );
    }

    let search = session.call(&search_call(19, "keyword_search", 8), 19);
    assert_tool_success_shape(&search);
    assert!(
        !tool_body(&search)["h"].as_array().unwrap().is_empty(),
        "{search:#}"
    );
    let read = session.call(&read_call(20), 20);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    assert!(session.finish().success());
}

/// D6: per-call roots escaping the workspace hit a live session on three
/// tools. Each is a tool error with its id echoed, and the session is
/// unpoisoned: a read, a status, and a ping succeed right after. Pass 1 T6
/// pins the escape rows statically; this pins live injection plus usability.
#[test]
fn drill_sandbox_escape_mid_session_refused_session_unpoisoned() {
    let workspace = file_tree();
    let outside = tempfile::tempdir().unwrap();
    let escaped = outside.path().display().to_string();
    let mut session = LiveSession::spawn(Some(workspace.path()));
    session.handshake();

    let before = session.call(&read_call(1), 1);
    assert_tool_success_shape(&before);
    assert_eq!(
        tool_body(&before)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{before:#}"
    );

    let attempts = vec![
        tool_call(
            2,
            "keyword_search",
            json!({"query": "x", "limit": 4, "root": escaped}),
        ),
        tool_call(
            3,
            "code_read",
            json!({"ids": ["src/lib.rs#L1-L1"], "root": escaped}),
        ),
        tool_call(4, "index_repo", json!({"root": escaped})),
    ];
    for (i, payload) in attempts.iter().enumerate() {
        let response = session.call(payload, 2 + i as u32);
        assert_tool_error_shape(&response);
    }

    let read = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );
    let status = session.call(&tool_call(6, "index_status", json!({})), 6);
    assert_tool_success_shape(&status);
    let ping = session.call(&json!({"jsonrpc":"2.0","id":7,"method":"ping"}), 7);
    assert!(ping.get("error").is_none(), "{ping:#}");
    assert!(ping.get("result").is_some(), "{ping:#}");

    assert!(session.finish().success());
}

/// D7 (chained double fault): garbage overwrites `index.db` AND the read
/// target is deleted under one live session. Both faults report tool errors
/// on the same session; a staged heal (restore the file first) revives reads
/// while search still fails -- the faults are independent, neither heal
/// masks the other -- and deleting the corrupt inode plus a live `index_repo`
/// heals search. Recovery pins a kill+kill double crash with restarts; this
/// pins a live double fault with a staged live heal and per-call codes.
#[test]
fn drill_chained_double_fault_staged_heal_restores_live_session() {
    let temp = indexed_tree();
    let victim = temp.path().join("src").join("lib.rs");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let search0 = session.call(&search_call(1, "keyword_search", 4), 1);
    assert_tool_success_shape(&search0);
    assert!(
        !tool_body(&search0)["h"].as_array().unwrap().is_empty(),
        "{search0:#}"
    );
    let read0 = session.call(&read_call(2), 2);
    assert_tool_success_shape(&read0);
    assert_eq!(
        tool_body(&read0)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read0:#}"
    );

    // Inject both faults at once.
    corrupt_index_db(temp.path());
    std::fs::remove_file(&victim).unwrap();

    let search1 = session.call(&search_call(3, "keyword_search", 8), 3);
    assert_tool_error_shape(&search1);
    let read1 = session.call(&read_call(4), 4);
    assert_tool_error_shape(&read1);

    // Staged heal, step 1: restore the file only.
    std::fs::write(&victim, "fn target_symbol() {}\n").unwrap();
    let read2 = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read2);
    assert_eq!(
        tool_body(&read2)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read2:#}"
    );
    let search2 = session.call(&search_call(6, "keyword_search", 9), 6);
    assert_tool_error_shape(&search2);

    // Staged heal, step 2: drop the corrupt inode and rebuild live.
    std::fs::remove_file(index_db_path(temp.path())).unwrap();
    let rebuilt = session.call(&tool_call(7, "index_repo", json!({})), 7);
    assert_tool_success_shape(&rebuilt);
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    let search3 = session.call(&search_call(8, "keyword_search", 12), 8);
    assert_tool_success_shape(&search3);
    assert!(
        !tool_body(&search3)["h"].as_array().unwrap().is_empty(),
        "{search3:#}"
    );

    assert!(session.finish().success());
}
