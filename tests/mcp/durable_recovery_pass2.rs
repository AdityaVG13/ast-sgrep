//! R2 recovery-contract oracles for ast-sgrep-mcp durable state.
//!
//! Non-overlap contract: `protocol.rs` pins handshake/negotiation, discovery,
//! structured content, tool names, per-channel kinds, single-id expansion, read
//! windows, schema rejection, sandbox escapes, byte-stability, elision,
//! miss envelopes, cancellation, and EOF-before-initialize. Pass 1 pins
//! startup config, pipelined workspace-removal, file-roots, pre-session index
//! corruption plus delete-and-reindex healing, empty-root restart determinism,
//! elision reset on restart, and pinned-index garbage. This file pins NONE of
//! those again. Instead it injects ACTIVE faults mid-session over stdio, each
//! test naming its fault class:
//!
//! * `root_deleted_mid_session`: the NEXT sequential call after deletion fails
//!   closed (vs pass 1's pipelined batch), and recreating the root heals the
//!   same live session without a restart;
//! * `index_truncated_to_stub_mid_session`: a live tear of `index.db` to a
//!   stub is refused loudly while `code_read` keeps serving files;
//! * `index_torn_half_length_mid_session`: a partial-length tear mid-session
//!   is a tool error, never a silent empty success or fabricated hits;
//! * `stdin_closed_mid_session` / `stdin_closed_mid_request`: EOF (clean and
//!   with a partial line in flight) terminates the process cleanly with no
//!   response for the partial id (vs protocol's EOF-before-initialize);
//! * `garbage_line_mid_stream`: an unparsable line mid-stream is ignored and
//!   the session continues (no hang, no echo);
//! * `invalid_envelope_mid_stream`: a well-formed unknown-method envelope
//!   mid-stream yields the JSON-RPC error shape and the session survives;
//! * `restart_after_root_deletion`: a fresh process over a restored tree
//!   reproduces the pre-fault indexed chain byte-identically.
//!
//! Discriminants are exit codes, `isError` booleans, envelope shapes (key
//! presence, error-code numericity, id echo), and byte equality -- never
//! message text. Every stdio read and process wait carries a timeout so a
//! regressed server fails the test instead of hanging the suite.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const RECV_TIMEOUT: Duration = Duration::from_secs(15);
const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

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
            "clientInfo": {"name": "asgrep-mcp-r2", "version": "0"}
        }
    })
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

/// A live stdio session with timeout-bounded reads: a regressed server fails
/// the test instead of hanging the suite.
struct LiveSession {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
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
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(None);
                        break;
                    }
                    Ok(_) => {
                        if tx.send(Some(line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(None);
                        break;
                    }
                }
            }
        });
        LiveSession {
            child,
            stdin: Some(stdin),
            lines: rx,
        }
    }

    fn handshake(&mut self) {
        self.send(&init_payload());
        let init = self.recv();
        assert_eq!(init["id"], "__init", "{init:#}");
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }

    fn send(&mut self, payload: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    }

    fn send_raw_line(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    /// Write bytes with no trailing newline: a request torn mid-flight.
    fn send_partial(&mut self, bytes: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(bytes.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn recv_raw(&self) -> Option<String> {
        match self.lines.recv_timeout(RECV_TIMEOUT) {
            Ok(line) => line,
            Err(_) => panic!("timed out after {RECV_TIMEOUT:?} waiting for MCP output"),
        }
    }

    fn recv(&self) -> Value {
        let line = self
            .recv_raw()
            .expect("MCP closed stdout while a response was pending");
        serde_json::from_str(line.trim()).expect("server emitted JSON-RPC")
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// Drain stdout until EOF or the total budget expires. Returns raw lines.
    fn drain_until_eof(&self, budget: Duration) -> Vec<String> {
        let started = Instant::now();
        let mut lines = Vec::new();
        loop {
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                panic!("timed out draining MCP stdout: got {} lines", lines.len());
            }
            match self.lines.recv_timeout(remaining) {
                Ok(Some(line)) => lines.push(line),
                Ok(None) => break,
                Err(_) => panic!("timed out draining MCP stdout: got {} lines", lines.len()),
            }
        }
        lines
    }

    fn wait_clean(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("poll MCP") {
                return status;
            }
            if started.elapsed() > WAIT_TIMEOUT {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("MCP did not exit within {WAIT_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
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

fn index_db_len(root: &Path) -> u64 {
    let db = root.join(".asgrep").join("index.db");
    assert!(db.is_file(), "expected an index db at {}", db.display());
    std::fs::metadata(&db).unwrap().len()
}

fn truncate_index_db(root: &Path, len: u64) {
    let db = root.join(".asgrep").join("index.db");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&db)
        .unwrap()
        .set_len(len)
        .unwrap();
}

#[test]
fn fault_root_deleted_mid_session_next_call_fails_closed_and_live_heal() {
    // Active fault: the workspace root vanishes mid-session. The NEXT
    // sequential call must fail closed with the uniform tool-error shape
    // (pass 1 pins a pipelined batch; this pins the immediate next call),
    // `ping` still answers, and recreating the root heals the SAME live
    // session -- no restart required.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tool_call(1, "index_status", json!({})));
    let before = session.recv();
    assert_eq!(before["result"]["isError"], false, "{before:#}");

    std::fs::remove_dir_all(temp.path()).unwrap();
    assert!(!temp.path().exists());

    session.send(&tool_call(2, "index_status", json!({})));
    let failed = session.recv();
    assert_eq!(failed["id"], 2, "{failed:#}");
    assert_tool_error_shape(&failed);

    session.send(&json!({"jsonrpc":"2.0","id":3,"method":"ping"}));
    let ping = session.recv();
    assert_eq!(ping["id"], 3, "{ping:#}");
    assert!(ping.get("error").is_none(), "{ping:#}");
    assert!(ping.get("result").is_some(), "{ping:#}");

    // Live heal: recreate the identical root; the session serves again.
    std::fs::create_dir_all(temp.path()).unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").unwrap();
    session.send(&tool_call(4, "index_status", json!({})));
    let healed = session.recv();
    assert_eq!(healed["id"], 4, "{healed:#}");
    assert_eq!(healed["result"]["isError"], false, "{healed:#}");
    assert_eq!(tool_body(&healed)["file_count"], 0);

    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
}

#[test]
fn fault_index_truncated_to_stub_mid_session_refused_loudly_reads_survive() {
    // Active fault: `index.db` is torn to a 7-byte stub under a live root
    // after a healthy search. Index-dependent calls must refuse loudly (the
    // search uses a fresh limit so it cannot ride the warm searcher cache)
    // while `code_read` keeps serving files from the healthy tree.
    let temp = indexed_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tool_call(
        1,
        "keyword_search",
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
    ));
    let before = session.recv();
    assert_eq!(before["result"]["isError"], false, "{before:#}");
    assert!(!tool_body(&before)["h"].as_array().unwrap().is_empty());

    assert!(index_db_len(temp.path()) > 4096);
    truncate_index_db(temp.path(), 7);

    session.send(&tool_call(2, "index_status", json!({})));
    let status = session.recv();
    assert_eq!(status["id"], 2, "{status:#}");
    assert_tool_error_shape(&status);

    session.send(&tool_call(
        3,
        "keyword_search",
        json!({"query": "target_symbol", "limit": 8, "resend_seen": true}),
    ));
    let search = session.recv();
    assert_eq!(search["id"], 3, "{search:#}");
    assert_tool_error_shape(&search);

    session.send(&tool_call(4, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})));
    let read = session.recv();
    assert_eq!(read["id"], 4, "{read:#}");
    assert_eq!(read["result"]["isError"], false, "{read:#}");
    assert_eq!(tool_body(&read)["nodes"].as_array().unwrap().len(), 1);

    session.close_stdin();
    let exit = session.wait_clean();
    assert!(exit.success(), "MCP exited {exit}");
}

#[test]
fn fault_index_torn_half_length_mid_session_refused_loudly_without_silent_empty() {
    // Active fault: `index.db` is torn to half its length mid-session (header
    // intact, pages missing). The next index-dependent calls must be tool
    // errors -- never a silent empty success (`isError: false` with zero
    // hits) and never fabricated hits.
    let temp = indexed_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tool_call(
        1,
        "keyword_search",
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
    ));
    let before = session.recv();
    assert_eq!(before["result"]["isError"], false, "{before:#}");

    let len = index_db_len(temp.path());
    assert!(len > 4096, "fixture too small to tear: {len}");
    truncate_index_db(temp.path(), len / 2);

    session.send(&tool_call(2, "index_status", json!({})));
    let status = session.recv();
    assert_eq!(status["id"], 2, "{status:#}");
    assert_tool_error_shape(&status);

    session.send(&tool_call(
        3,
        "keyword_search",
        json!({"query": "target_symbol", "limit": 8, "resend_seen": true}),
    ));
    let search = session.recv();
    assert_eq!(search["id"], 3, "{search:#}");
    assert_tool_error_shape(&search);

    session.close_stdin();
    let exit = session.wait_clean();
    assert!(exit.success(), "MCP exited {exit}");
}

#[test]
fn fault_stdin_closed_mid_session_exits_cleanly() {
    // Active fault: EOF right after a healthy call mid-session (protocol pins
    // only EOF-before-initialize). The process must terminate cleanly with
    // exit 0 inside the wait budget -- never hang on a half-open session.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tool_call(1, "index_status", json!({})));
    let before = session.recv();
    assert_eq!(before["result"]["isError"], false, "{before:#}");

    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
}

#[test]
fn fault_stdin_closed_mid_request_partial_line_exits_cleanly_without_response() {
    // Active fault: stdin closes with a torn request (bytes, no newline, no
    // closing brace) in flight. The process must exit 0 inside the wait
    // budget, emit no JSON-RPC response carrying the torn id, and emit no
    // non-JSON garbage on stdout.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send_partial(r#"{"jsonrpc":"2.0","id":9,"method":"ping""#);
    session.close_stdin();
    let lines = session.drain_until_eof(WAIT_TIMEOUT);
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line.trim()).expect("stdout stays JSON");
        assert_ne!(value.get("id"), Some(&json!(9)), "torn id answered: {value:#}");
    }
}

#[test]
fn fault_garbage_line_mid_stream_ignored_session_continues() {
    // Active fault: an unparsable line arrives mid-stream between healthy
    // calls. It must be ignored (no response, no echo), and the very next
    // calls must answer with their own ids inside the read budget -- the
    // session never hangs behind the garbage.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tool_call(1, "index_status", json!({})));
    assert_eq!(session.recv()["id"], 1);

    session.send_raw_line("{{not json");
    session.send(&json!({"jsonrpc":"2.0","id":2,"method":"ping"}));
    let ping = session.recv();
    assert_eq!(ping["id"], 2, "{ping:#}");
    assert!(ping.get("error").is_none(), "{ping:#}");

    session.send(&tool_call(3, "index_status", json!({})));
    let after = session.recv();
    assert_eq!(after["id"], 3, "{after:#}");
    assert_eq!(after["result"]["isError"], false, "{after:#}");

    session.close_stdin();
    // No late echo of the garbage may appear after close.
    let rest = session.drain_until_eof(WAIT_TIMEOUT);
    assert!(
        rest.iter().all(|line| line.trim().is_empty()),
        "server echoed mid-stream garbage: {rest:?}"
    );
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
}

#[test]
fn fault_invalid_envelope_mid_stream_yields_error_shape_session_survives() {
    // Active fault: a well-formed but invalid JSON-RPC envelope (unknown
    // method) arrives mid-stream. It must yield the JSON-RPC error shape --
    // echoed id, top-level `error` object with a numeric code, no `result` --
    // inside the read budget, and the session must serve the next valid call.
    // Discriminants are shape only, never the code value or message text.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tool_call(1, "index_status", json!({})));
    assert_eq!(session.recv()["id"], 1);

    session.send(&json!({"jsonrpc": "2.0", "id": 77, "method": "missing"}));
    let error = session.recv();
    assert_eq!(error["id"], 77, "{error:#}");
    assert!(error["error"].is_object(), "{error:#}");
    assert!(error["error"]["code"].is_i64(), "{error:#}");
    assert!(error.get("result").is_none(), "{error:#}");

    session.send(&json!({"jsonrpc":"2.0","id":78,"method":"ping"}));
    let ping = session.recv();
    assert_eq!(ping["id"], 78, "{ping:#}");
    assert!(ping.get("error").is_none(), "{ping:#}");
    assert!(ping.get("result").is_some(), "{ping:#}");

    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
}

#[test]
fn fault_restart_after_root_deletion_reproduces_clean_state() {
    // Active fault then restart: capture the indexed chain bytes, delete the
    // root under a live session (next call fails closed), restore the
    // identical tree, and restart. The fresh process must reproduce the
    // pre-fault chain byte-identically -- the fault leaves no durable trace.
    // Pass 1 pins restart determinism only for the empty root; this pins the
    // indexed chain across a fault-and-restore cycle.
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    index_tree(temp.path());

    let chain = || {
        let mut session = LiveSession::spawn(Some(temp.path()));
        session.handshake();
        session.send(&tool_call(1, "index_status", json!({})));
        let status = session.recv();
        session.send(&tool_call(
            2,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ));
        let search = session.recv();
        session.close_stdin();
        let exit = session.wait_clean();
        assert!(exit.success(), "MCP exited {exit}");
        (tool_text(&status).to_owned(), tool_text(&search).to_owned())
    };

    let (baseline_status, baseline_search) = chain();

    // Fault session: root deleted mid-session, next call fails closed.
    let mut faulty = LiveSession::spawn(Some(temp.path()));
    faulty.handshake();
    faulty.send(&tool_call(1, "index_status", json!({})));
    assert_eq!(faulty.recv()["result"]["isError"], false);
    std::fs::remove_dir_all(temp.path()).unwrap();
    faulty.send(&tool_call(2, "index_status", json!({})));
    let failed = faulty.recv();
    assert_eq!(failed["id"], 2, "{failed:#}");
    assert_tool_error_shape(&failed);
    faulty.close_stdin();
    assert!(faulty.wait_clean().success());

    // Restore the identical tree and reindex from source.
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    index_tree(temp.path());

    let (restored_status, restored_search) = chain();
    // `writer_generation` is a fresh random stamp per index build, so it is
    // compared by shape while every content key must reproduce exactly.
    let mut baseline_body: Value = serde_json::from_str(&baseline_status).unwrap();
    let mut restored_body: Value = serde_json::from_str(&restored_status).unwrap();
    for body in [&baseline_body, &restored_body] {
        assert!(body["writer_generation"].is_u64(), "{body:#}");
    }
    baseline_body.as_object_mut().unwrap().remove("writer_generation");
    restored_body.as_object_mut().unwrap().remove("writer_generation");
    assert_eq!(restored_body, baseline_body, "status drifted across fault");
    assert_eq!(restored_search, baseline_search, "search drifted across fault");
    let search: Value = serde_json::from_str(&restored_search).unwrap();
    let hits = search["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{search:#}");
    assert_eq!(search["zn"].as_u64().unwrap() as usize, hits.len());
}
