//! R4 end-to-end crash drills for ast-sgrep-mcp durable state.
//!
//! Non-overlap contract: `protocol.rs` pins handshake/negotiation, discovery,
//! structured content, tool names, per-channel kinds, single-id expansion, read
//! windows, schema rejection, sandbox escapes, byte-stability, elision,
//! miss envelopes, cancellation, and EOF-before-initialize. Pass 1 pins
//! startup config, pipelined workspace-removal, file-roots, pre-session index
//! corruption plus delete-and-reindex healing, empty-root restart determinism,
//! elision reset on restart, and pinned-index garbage. Pass 2 pins ACTIVE
//! mid-session faults (root deletion + live heal, stub/half tears, EOF,
//! garbage lines, invalid envelopes, restart-after-restore). Pass 3 pins
//! RELATIONS over recovery (restart determinism, source roundtrips,
//! catalog stability, link consistency, transcript identity, stateless
//! encodings, error determinism, root equivalence). This file pins NONE of
//! those again. Instead it pins FULL crash→recover→serve DRILLS over stdio:
//!
//! * every drill starts from a WORKING session serving status/search/read,
//!   captures the pre-crash baseline bytes, then CRASHES (SIGKILL idle,
//!   SIGKILL with pipelined requests in flight, clean stdin EOF, stdin
//!   aborted mid-request, root deleted + kill, index corrupted + kill,
//!   index deleted + kill);
//! * every drill then starts a FRESH session that RECOVERS (directly, or via
//!   filesystem restore plus `index_repo` healing) and SERVES the identical
//!   chain;
//! * the SERVE proof is byte equality of the recovered search/read (and
//!   status, scrubbed only across rebuilds) against the pre-crash baseline;
//! * one chained double-crash drill proves kill → recover → tear → kill →
//!   heal → serve.
//!
//! Discriminants are exit codes, `isError` booleans, envelope shapes (key
//! presence, tuple widths, counts, id echo), filesystem facts (existence,
//! lengths), and byte (in)equality -- never message text. Every stdio read
//! and process wait carries a timeout so a regressed server fails the test
//! instead of hanging the suite.

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
            "clientInfo": {"name": "asgrep-mcp-r4", "version": "0"}
        }
    })
}

fn tool_call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

fn status_call(id: u32) -> Value {
    tool_call(id, "index_status", json!({}))
}

fn search_call(id: u32) -> Value {
    tool_call(
        id,
        "keyword_search",
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
    )
}

fn read_call(id: u32) -> Value {
    tool_call(id, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}))
}

fn reindex_call(id: u32) -> Value {
    tool_call(id, "index_repo", json!({}))
}

fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text content")
}

fn tool_body(response: &Value) -> Value {
    serde_json::from_str(tool_text(response)).expect("tool body JSON")
}

/// A live stdio session with timeout-bounded reads: a regressed server fails
/// the test instead of hanging the suite.
struct LiveSession {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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

    /// Write bytes with no trailing newline: a request torn mid-flight.
    fn send_partial(&mut self, bytes: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(bytes.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&self) -> Value {
        let line = match self.lines.recv_timeout(RECV_TIMEOUT) {
            Ok(line) => line,
            Err(_) => panic!("timed out after {RECV_TIMEOUT:?} waiting for MCP output"),
        }
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

    /// Crash the server mid-session: SIGKILL without a clean stdin close.
    /// Returns the terminal status; the caller asserts the crash discriminant.
    fn crash_kill(&mut self) -> ExitStatus {
        self.stdin.take();
        let _ = self.child.kill();
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("poll MCP") {
                return status;
            }
            if started.elapsed() > WAIT_TIMEOUT {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("killed MCP did not terminate within {WAIT_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
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

const FIXTURE_SOURCE: &str = "fn target_symbol() { helper(); }\nfn helper() {}\n";

/// Two-symbol indexed tree: one searchable target plus a second symbol and a
/// multi-line file for read-window drills.
fn indexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), FIXTURE_SOURCE).unwrap();
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
    std::fs::write(&db, "R4-corrupt-index-sentinel;".repeat(256)).unwrap();
}

fn truncate_index_db(root: &Path, len: u64) {
    let db = index_db_path(root);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&db)
        .unwrap()
        .set_len(len)
        .unwrap();
}

/// Serve the canonical drill chain: status, search, read. Returns the three
/// responses in order; asserts id echo so pipelined order cannot hide.
fn serve_chain(session: &mut LiveSession, base_id: u32) -> (Value, Value, Value) {
    session.send(&status_call(base_id));
    let status = session.recv();
    assert_eq!(status["id"], base_id, "{status:#}");
    session.send(&search_call(base_id + 1));
    let search = session.recv();
    assert_eq!(search["id"], base_id + 1, "{search:#}");
    session.send(&read_call(base_id + 2));
    let read = session.recv();
    assert_eq!(read["id"], base_id + 2, "{read:#}");
    (status, search, read)
}

/// The working-session proof: every leg serves with the expected shape.
/// Discriminants are `isError`, counts, tuple widths, and key presence.
fn assert_serve_valid(status: &Value, search: &Value, read: &Value) {
    assert_eq!(status["result"]["isError"], false, "{status:#}");
    assert_eq!(tool_body(status)["file_count"], 1, "{status:#}");
    assert_eq!(search["result"]["isError"], false, "{search:#}");
    let envelope = tool_body(search);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    for hit in hits {
        assert_eq!(hit.as_array().unwrap().len(), 5, "{hit:#}");
    }
    assert_eq!(read["result"]["isError"], false, "{read:#}");
    let body = tool_body(read);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1, "{body:#}");
    assert_eq!(body["nodes"][0]["id"], "src/lib.rs#L1-L1", "{body:#}");
}

/// Status across a rebuild: `writer_generation` is a fresh unique stamp per
/// index build, so it is compared by shape while every content key must match.
fn assert_status_equal_across_rebuild(recovered: &str, baseline: &str) {
    let mut baseline_body: Value = serde_json::from_str(baseline).unwrap();
    let mut recovered_body: Value = serde_json::from_str(recovered).unwrap();
    for body in [&baseline_body, &recovered_body] {
        assert!(body["writer_generation"].is_u64(), "{body:#}");
    }
    baseline_body.as_object_mut().unwrap().remove("writer_generation");
    recovered_body.as_object_mut().unwrap().remove("writer_generation");
    assert_eq!(recovered_body, baseline_body, "status drifted across rebuild");
}

#[test]
fn drill_kill_idle_server_recovers_identical_serve() {
    // Crash kind: SIGKILL of an idle working session after a healthy serve.
    // The fresh session must serve the pre-crash chain byte-identically: the
    // crash leaves no durable trace because the tree and index are intact.
    let temp = indexed_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();

    let crash = work.crash_kill();
    assert!(!crash.success(), "SIGKILL must terminate the server: {crash}");

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 1);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_eq!(tool_text(&rstatus), baseline_status, "status drifted across kill");
    assert_eq!(tool_text(&rsearch), baseline_search, "search drifted across kill");
    assert_eq!(tool_text(&rread), baseline_read, "read drifted across kill");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

#[test]
fn drill_kill_with_pipelined_requests_in_flight_recovers() {
    // Crash kind: SIGKILL with pipelined requests in flight (three sends, no
    // reads) after a healthy baseline. The killed session's pending output is
    // nondeterministic and asserted nothing about; the fresh session must
    // serve the baseline chain byte-identically.
    let temp = indexed_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();

    work.send(&status_call(11));
    work.send(&search_call(12));
    work.send(&read_call(13));
    let crash = work.crash_kill();
    assert!(!crash.success(), "SIGKILL must terminate the server: {crash}");

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 1);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_eq!(tool_text(&rstatus), baseline_status, "status drifted across kill");
    assert_eq!(tool_text(&rsearch), baseline_search, "search drifted across kill");
    assert_eq!(tool_text(&rread), baseline_read, "read drifted across kill");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

#[test]
fn drill_stdin_eof_mid_session_fresh_session_serves_identical() {
    // Crash kind: clean stdin EOF mid-session after a healthy serve. Pass 2
    // pins only the clean exit; this drill pins the recovery SERVE: the fresh
    // session reproduces the pre-EOF chain byte-identically.
    let temp = indexed_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();
    work.close_stdin();
    let exit = work.wait_clean();
    assert!(exit.success(), "clean EOF must exit 0: {exit}");

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 1);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_eq!(tool_text(&rstatus), baseline_status, "status drifted across EOF");
    assert_eq!(tool_text(&rsearch), baseline_search, "search drifted across EOF");
    assert_eq!(tool_text(&rread), baseline_read, "read drifted across EOF");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

#[test]
fn drill_stdin_aborted_mid_request_fresh_session_serves() {
    // Crash kind: stdin closes with a torn request (bytes, no newline, no
    // closing brace) in flight after a healthy serve. The crashed process
    // must exit 0 with no response for the torn id and no non-JSON stdout;
    // the fresh session must serve the baseline chain byte-identically.
    let temp = indexed_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();

    work.send_partial(r#"{"jsonrpc":"2.0","id":99,"method":"ping""#);
    work.close_stdin();
    let lines = work.drain_until_eof(WAIT_TIMEOUT);
    let exit = work.wait_clean();
    assert!(exit.success(), "torn EOF must exit 0: {exit}");
    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line.trim()).expect("stdout stays JSON");
        assert_ne!(value.get("id"), Some(&json!(99)), "torn id answered: {value:#}");
    }

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 1);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_eq!(tool_text(&rstatus), baseline_status, "status drifted across abort");
    assert_eq!(tool_text(&rsearch), baseline_search, "search drifted across abort");
    assert_eq!(tool_text(&rread), baseline_read, "read drifted across abort");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

#[test]
fn drill_root_deleted_then_killed_restore_and_reindex_recovers() {
    // Crash kind: the workspace root is deleted under a working session and
    // the server is SIGKILLed during the durable loss (pass 2 kills nothing
    // and heals live; this drill crashes, then restores out of band and heals
    // via a fresh session's `index_repo`). Search/read must reproduce
    // byte-identically; status matches across the rebuild stamp.
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), FIXTURE_SOURCE).unwrap();
    index_tree(temp.path());

    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();

    std::fs::remove_dir_all(temp.path()).unwrap();
    assert!(!temp.path().exists());
    let crash = work.crash_kill();
    assert!(!crash.success(), "SIGKILL must terminate the server: {crash}");

    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("lib.rs"), FIXTURE_SOURCE).unwrap();
    assert!(source.join("lib.rs").is_file());

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    fresh.send(&reindex_call(1));
    let rebuilt = fresh.recv();
    assert_eq!(rebuilt["id"], 1, "{rebuilt:#}");
    assert_eq!(rebuilt["result"]["isError"], false, "{rebuilt:#}");
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 2);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_status_equal_across_rebuild(tool_text(&rstatus), &baseline_status);
    assert_eq!(tool_text(&rsearch), baseline_search, "search drifted across crash");
    assert_eq!(tool_text(&rread), baseline_read, "read drifted across crash");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

#[test]
fn drill_index_corrupted_then_killed_delete_and_reindex_heals() {
    // Crash kind: `index.db` is overwritten with deterministic garbage under
    // a working session, then the server is SIGKILLed (pass 1 corrupts only
    // pre-session and never kills). Recovery deletes the corrupt inode and a
    // fresh session's `index_repo` rebuilds: search/read reproduce the
    // baseline byte-identically, status matches across the rebuild stamp.
    let temp = indexed_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();

    corrupt_index_db(temp.path());
    assert!(index_db_path(temp.path()).is_file());
    let crash = work.crash_kill();
    assert!(!crash.success(), "SIGKILL must terminate the server: {crash}");

    std::fs::remove_file(index_db_path(temp.path())).unwrap();
    assert!(!index_db_path(temp.path()).exists());

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    fresh.send(&reindex_call(1));
    let rebuilt = fresh.recv();
    assert_eq!(rebuilt["id"], 1, "{rebuilt:#}");
    assert_eq!(rebuilt["result"]["isError"], false, "{rebuilt:#}");
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 2);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_status_equal_across_rebuild(tool_text(&rstatus), &baseline_status);
    assert_eq!(tool_text(&rsearch), baseline_search, "search drifted across crash");
    assert_eq!(tool_text(&rread), baseline_read, "read drifted across crash");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

#[test]
fn drill_index_deleted_then_killed_reindex_recovers() {
    // Crash kind: `index.db` is deleted outright under a working session,
    // then the server is SIGKILLed (distinct from corruption: no inode, no
    // bytes, only the source tree remains). A fresh session's `index_repo`
    // rebuilds from source: search/read reproduce byte-identically, status
    // matches across the rebuild stamp.
    let temp = indexed_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();

    let db = index_db_path(temp.path());
    assert!(db.is_file());
    std::fs::remove_file(&db).unwrap();
    assert!(!db.exists());
    let crash = work.crash_kill();
    assert!(!crash.success(), "SIGKILL must terminate the server: {crash}");

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    fresh.send(&reindex_call(1));
    let rebuilt = fresh.recv();
    assert_eq!(rebuilt["id"], 1, "{rebuilt:#}");
    assert_eq!(rebuilt["result"]["isError"], false, "{rebuilt:#}");
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 2);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_status_equal_across_rebuild(tool_text(&rstatus), &baseline_status);
    assert_eq!(tool_text(&rsearch), baseline_search, "search drifted across crash");
    assert_eq!(tool_text(&rread), baseline_read, "read drifted across crash");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

#[test]
fn drill_double_crash_kill_then_tear_then_kill_recovers() {
    // Chained double crash: kill #1 with durable state intact, an interim
    // fresh session proves recovery, then the index is torn to a stub and
    // kill #2 crashes during the fault. Recovery deletes the stub and a final
    // fresh session's `index_repo` rebuilds: the final SERVE reproduces the
    // original pre-crash baseline (search/read byte-identical, status across
    // the rebuild stamp).
    let temp = indexed_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let (status, search, read) = serve_chain(&mut work, 1);
    assert_serve_valid(&status, &search, &read);
    let baseline_status = tool_text(&status).to_owned();
    let baseline_search = tool_text(&search).to_owned();
    let baseline_read = tool_text(&read).to_owned();
    let crash_one = work.crash_kill();
    assert!(!crash_one.success(), "kill #1 must terminate: {crash_one}");

    let mut interim = LiveSession::spawn(Some(temp.path()));
    interim.handshake();
    let (istatus, isearch, iread) = serve_chain(&mut interim, 1);
    assert_serve_valid(&istatus, &isearch, &iread);
    assert_eq!(tool_text(&istatus), baseline_status, "interim status drifted");
    assert_eq!(tool_text(&isearch), baseline_search, "interim search drifted");
    assert_eq!(tool_text(&iread), baseline_read, "interim read drifted");

    let db = index_db_path(temp.path());
    assert!(std::fs::metadata(&db).unwrap().len() > 4096);
    truncate_index_db(temp.path(), 7);
    assert_eq!(std::fs::metadata(&db).unwrap().len(), 7);
    let crash_two = interim.crash_kill();
    assert!(!crash_two.success(), "kill #2 must terminate: {crash_two}");

    std::fs::remove_file(index_db_path(temp.path())).unwrap();
    assert!(!index_db_path(temp.path()).exists());

    let mut healed = LiveSession::spawn(Some(temp.path()));
    healed.handshake();
    healed.send(&reindex_call(1));
    let rebuilt = healed.recv();
    assert_eq!(rebuilt["id"], 1, "{rebuilt:#}");
    assert_eq!(rebuilt["result"]["isError"], false, "{rebuilt:#}");
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    let (rstatus, rsearch, rread) = serve_chain(&mut healed, 2);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_status_equal_across_rebuild(tool_text(&rstatus), &baseline_status);
    assert_eq!(tool_text(&rsearch), baseline_search, "final search drifted");
    assert_eq!(tool_text(&rread), baseline_read, "final read drifted");
    healed.close_stdin();
    assert!(healed.wait_clean().success());
}
