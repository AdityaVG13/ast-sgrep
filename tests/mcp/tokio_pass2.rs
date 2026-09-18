//! K2 cancellation/timeout-contract tests for ast-sgrep-mcp.
//!
//! Pins the cancel paths in `crates/ast-sgrep-mcp/src/handler.rs` (`cancel`
//! token watcher spawn in `call_tool`, dispatch cancel flag) and the EOF
//! handling in `serve_stdio` through the real stdio binary.
//!
//! Contracts pinned (discriminants are exit codes, response shapes, id echo,
//! and presence/absence of responses -- never message text):
//!
//! * K1: `notifications/cancelled` mid-`index_repo` suppresses that call's
//!   response: the cancelled id is never answered (probed: rmcp aborts the
//!   request task, so no success, no error, nothing -- the silence IS the
//!   contract), while ping served in the same window answers promptly.
//! * K2: a call cancelled while queued on `tool_lock` is never answered, and
//!   the slow tool ahead of it still completes with success.
//! * K3: cancelling one queued call leaves a sibling queued call intact: the
//!   sibling and the slow tool both succeed.
//! * K4: the session stays usable after a cancel: a re-run `index_repo`
//!   succeeds and a follow-up search returns hits.
//! * K5: ping and `tools/list` are both served promptly during the cancel
//!   window of a slow tool.
//! * K6: stray cancels (unknown id, post-completion id, double cancel) emit
//!   no output and do not disturb the session.
//! * K7: a cancel sent before its id is ever issued does not poison the
//!   later call reusing that id (the call succeeds normally).
//! * K8: EOF before `initialize` exits 0 (not a failure).
//! * K9: EOF mid-session (idle) ends gracefully with exit 0.
//! * K10: client abort (stdin close) mid-tool-call terminates the server
//!   with exit 0 within a bounded wait; likewise for EOF right after a
//!   cancel.
//!
//! Every read carries a timeout (reader thread + `recv_timeout`) and every
//! wait carries a kill deadline, so a regressed server fails the test instead
//! of hanging CI. Plain `#[test]`s only; no tokio dev-dependency.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const RECV_TIMEOUT: Duration = Duration::from_secs(15);
const SLOW_RECV_TIMEOUT: Duration = Duration::from_secs(120);
const WAIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Bound for abort-mid-call shutdown: the server must wind down on EOF even
/// with a tool in flight (probed ~6s for a 3000-file index; 60s is generous).
const ABORT_WAIT_TIMEOUT: Duration = Duration::from_secs(60);
/// Generous promptness bound for ping/list-while-cancelling (seconds, not
/// ms): a reader wedged behind index teardown would miss this by an order of
/// magnitude.
const PROMPT_WHILE_BUSY_BOUND: Duration = Duration::from_secs(10);
/// Quiet window asserting the cancelled in-flight id is never answered. Must
/// exceed the remaining uncancelled index time (~5s for [`BIG_FILES`]) so a
/// missed cancel still lands inside the window and fails the test.
const CANCEL_QUIET_WINDOW: Duration = Duration::from_secs(20);
/// Quiet window for a cancelled queued call: had the cancel missed, the fast
/// queued tool would answer within milliseconds of the slow tool finishing.
const QUEUED_QUIET_WINDOW: Duration = Duration::from_secs(5);
/// Quiet window for stray cancels: a (buggy) reply to one would be instant,
/// since no work backs it.
const STRAY_QUIET_WINDOW: Duration = Duration::from_secs(1);
/// Delay before sending `cancelled` so the slow tool is genuinely mid-call.
/// Small on purpose: a faster machine only shortens the index, and the reader
/// preserves request-before-notification order.
const CANCEL_DELAY: Duration = Duration::from_millis(500);
/// Tree width whose uncancelled `index_repo` takes ~5s in debug (calibrated),
/// giving the 500ms cancel a wide mid-call margin on fast machines.
const BIG_FILES: usize = 2500;

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
            "clientInfo": {"name": "asgrep-mcp-k2", "version": "0"}
        }
    })
}

fn initialized_notif() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

fn tool_call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

fn tools_list(id: u32) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":{}})
}

fn ping(id: u32) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"ping"})
}

fn cancelled(request_id: u32) -> Value {
    json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request_id}})
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
        self.send(&initialized_notif());
    }

    fn send(&mut self, payload: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&self) -> Value {
        self.recv_timeout(RECV_TIMEOUT)
    }

    fn recv_timeout(&self, timeout: Duration) -> Value {
        self.try_recv(timeout)
            .unwrap_or_else(|| panic!("timed out after {timeout:?} waiting for MCP output"))
    }

    /// `None` on timeout only; a closed stdout while the session must be live
    /// is a failure, not a quiet window.
    fn try_recv(&self, timeout: Duration) -> Option<Value> {
        let line = match self.lines.recv_timeout(timeout) {
            Ok(line) => line,
            Err(_) => return None,
        }
        .expect("MCP closed stdout while the session must be live");
        Some(serde_json::from_str(line.trim()).expect("server emitted JSON-RPC"))
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn wait_clean(&mut self) -> ExitStatus {
        self.wait_bounded(WAIT_TIMEOUT)
    }

    fn wait_bounded(&mut self, bound: Duration) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("poll MCP") {
                return status;
            }
            if started.elapsed() > bound {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("MCP did not exit within {bound:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text content")
}

fn tool_body(response: &Value) -> Value {
    serde_json::from_str(tool_text(response)).expect("tool body JSON")
}

fn assert_tool_success(response: &Value) {
    assert_eq!(response["result"]["isError"], false, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
}

fn assert_ping_ok(response: &Value, id: u32) {
    assert_eq!(response["id"], id, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert!(response["result"].is_object(), "{response:#}");
}

fn expected_tool_names() -> Vec<&'static str> {
    vec![
        "search",
        "keyword_search",
        "ast_search",
        "semantic_search",
        "code_search",
        "code_read",
        "index_status",
        "index_repo",
    ]
}

fn assert_tools_list_ok(response: &Value, id: u32) {
    assert_eq!(response["id"], id, "{response:#}");
    let names: Vec<String> = response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, expected_tool_names(), "{response:#}");
}

/// Wide unindexed tree: `index_repo` over it is the reliably-slow tool call.
/// Symbols carry a distinctive prefix so a follow-up search proves the index
/// is queryable.
fn big_tree(files: usize) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..files {
        std::fs::write(
            temp.path().join(format!("f{index}.rs")),
            format!("pub fn k2requelch_{index}() {{}}\n"),
        )
        .unwrap();
    }
    temp
}

/// Read until the response with `want` arrives; any response bearing a
/// `forbidden` (cancelled) id fails the test immediately.
fn recv_until(session: &LiveSession, want: u32, forbidden: &[u32]) -> Value {
    let started = Instant::now();
    loop {
        let elapsed = started.elapsed();
        assert!(
            elapsed < SLOW_RECV_TIMEOUT,
            "timed out waiting for id {want} after {elapsed:?}"
        );
        let response = session.recv_timeout(SLOW_RECV_TIMEOUT - elapsed);
        if let Some(id) = response.get("id").and_then(Value::as_u64) {
            assert!(
                !forbidden.contains(&(id as u32)),
                "cancelled id {id} was answered (silence is the contract): {response:#}"
            );
            if id as u32 == want {
                return response;
            }
        }
    }
}

/// Read until every id in `wants` arrives; any `forbidden` id fails the test.
fn recv_until_all(
    session: &LiveSession,
    wants: &[u32],
    forbidden: &[u32],
) -> Vec<Value> {
    let mut out = Vec::with_capacity(wants.len());
    let mut pending: Vec<u32> = wants.to_vec();
    let started = Instant::now();
    while !pending.is_empty() {
        let elapsed = started.elapsed();
        assert!(
            elapsed < SLOW_RECV_TIMEOUT,
            "timed out waiting for ids {pending:?} after {elapsed:?}"
        );
        let response = session.recv_timeout(SLOW_RECV_TIMEOUT - elapsed);
        if let Some(id) = response.get("id").and_then(Value::as_u64) {
            let id = id as u32;
            assert!(
                !forbidden.contains(&id),
                "cancelled id {id} was answered (silence is the contract): {response:#}"
            );
            if let Some(pos) = pending.iter().position(|w| *w == id) {
                pending.remove(pos);
                out.push(response);
            }
        }
    }
    out
}

/// Assert no response bearing any `forbidden` id arrives within `window`.
/// Anything else arriving is ignored: only the cancelled ids are the
/// contract under test.
fn assert_no_id_for(session: &LiveSession, forbidden: &[u32], window: Duration) {
    let started = Instant::now();
    while started.elapsed() < window {
        let remaining = window - started.elapsed();
        match session.try_recv(remaining) {
            None => return,
            Some(response) => {
                if let Some(id) = response.get("id").and_then(Value::as_u64) {
                    assert!(
                        !forbidden.contains(&(id as u32)),
                        "cancelled id {id} answered {} after cancel (silence is the contract): {response:#}",
                        started.elapsed().as_secs_f32()
                    );
                }
            }
        }
    }
}

/// Assert the server emits nothing at all within `window`.
fn assert_silent(session: &LiveSession, window: Duration) {
    assert_eq!(
        session.try_recv(window),
        None,
        "server emitted output for a stray cancel"
    );
}

/// K8: EOF before `initialize` exits 0 -- the client closed stdio; not a
/// server failure (`serve_stdio` maps `ConnectionClosed` to `Ok`).
#[test]
fn eof_before_initialize_exits_ok() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "EOF before initialize exited {status}");
}

/// K9: EOF mid-session while idle ends gracefully with exit 0.
#[test]
fn eof_mid_session_idle_exits_ok() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&ping(1));
    assert_ping_ok(&session.recv(), 1);
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "EOF mid-session exited {status}");
}

/// K10a: client abort (stdin close) mid-tool-call terminates the server with
/// exit 0 within a bounded wait (probed: no failure, no hang).
#[test]
fn client_abort_mid_call_exits_ok() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.close_stdin();
    let status = session.wait_bounded(ABORT_WAIT_TIMEOUT);
    assert!(status.success(), "abort mid-call exited {status}");
}

/// K10b: EOF immediately after cancelling an in-flight call still exits 0.
#[test]
fn eof_immediately_after_cancel_exits_ok() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled(1));
    session.close_stdin();
    let status = session.wait_bounded(ABORT_WAIT_TIMEOUT);
    assert!(status.success(), "EOF after cancel exited {status}");
}

/// K1: cancelling an in-flight `index_repo` suppresses that call's response
/// entirely (probed: rmcp aborts the request task, so the cancelled id gets
/// no success, no error, nothing), while a ping sent in the same window is
/// answered promptly and the session stays usable.
#[test]
fn cancel_mid_call_suppresses_response() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    let ping_sent = Instant::now();
    session.send(&cancelled(1));
    session.send(&ping(2));

    let ping_response = recv_until(&session, 2, &[1]);
    let ping_latency = ping_sent.elapsed();
    assert_ping_ok(&ping_response, 2);
    assert!(
        ping_latency < PROMPT_WHILE_BUSY_BOUND,
        "ping waited {ping_latency:?} behind the cancelled call"
    );

    // The cancelled id must stay silent well past the point where an
    // uncancelled index would have answered.
    assert_no_id_for(&session, &[1], CANCEL_QUIET_WINDOW);

    // The cancelled call did not poison the session.
    session.send(&tool_call(3, "index_status", json!({})));
    let status = recv_until(&session, 3, &[1]);
    assert_tool_success(&status);
    assert!(tool_body(&status).is_object(), "{status:#}");

    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// K2: a call cancelled while queued on `tool_lock` is never answered, and
/// the slow tool ahead of it still completes with success (probed).
#[test]
fn cancel_queued_call_behind_slow_tool() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&tool_call(2, "index_status", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled(2));

    let index = recv_until(&session, 1, &[2]);
    assert_tool_success(&index);
    assert!(tool_body(&index).is_object(), "{index:#}");

    // Had the cancel missed, the fast queued call would answer within
    // milliseconds of the index finishing.
    assert_no_id_for(&session, &[2], QUEUED_QUIET_WINDOW);

    session.send(&ping(3));
    assert_ping_ok(&recv_until(&session, 3, &[2]), 3);
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// K3: cancelling one queued call leaves a sibling queued call intact: the
/// slow tool and the sibling both succeed, the cancelled id stays silent.
#[test]
fn cancel_one_queued_call_leaves_sibling_intact() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&tool_call(2, "index_status", json!({})));
    session.send(&tool_call(3, "index_status", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled(2));

    let responses = recv_until_all(&session, &[1, 3], &[2]);
    assert_eq!(responses.len(), 2);
    for response in &responses {
        assert_tool_success(response);
        assert!(tool_body(response).is_object(), "{response:#}");
    }

    assert_no_id_for(&session, &[2], QUEUED_QUIET_WINDOW);
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// K4: the session stays usable after a cancel: a re-run `index_repo`
/// succeeds and a follow-up search over the rebuilt index returns hits
/// (probed: the cancelled attempt neither wedges the lock nor poisons the
/// store for the next attempt).
#[test]
fn session_usable_after_cancel_reindex_and_search() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled(1));
    session.send(&ping(2));
    assert_ping_ok(&recv_until(&session, 2, &[1]), 2);

    session.send(&tool_call(3, "index_repo", json!({})));
    let reindex = recv_until(&session, 3, &[1]);
    assert_tool_success(&reindex);
    assert!(tool_body(&reindex).is_object(), "{reindex:#}");

    session.send(&tool_call(
        4,
        "keyword_search",
        json!({"query": "k2requelch_7", "limit": 4}),
    ));
    let search = recv_until(&session, 4, &[1]);
    assert_tool_success(&search);
    let body = tool_body(&search);
    assert!(
        body["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{search:#}"
    );
    assert!(
        tool_text(&search).contains("k2requelch_7"),
        "{search:#}"
    );

    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// K5: ping and `tools/list` are both served promptly during the cancel
/// window of a slow tool (neither takes `tool_lock`, neither waits for the
/// cancelled call's teardown).
#[test]
fn ping_and_list_served_during_cancel_window() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    let sent = Instant::now();
    session.send(&cancelled(1));
    session.send(&ping(2));
    session.send(&tools_list(3));

    let responses = recv_until_all(&session, &[2, 3], &[1]);
    let latency = sent.elapsed();
    assert_eq!(responses.len(), 2);
    for response in &responses {
        match response["id"].as_u64().expect("numeric id") {
            2 => assert_ping_ok(response, 2),
            3 => assert_tools_list_ok(response, 3),
            other => panic!("unexpected id {other}: {response:#}"),
        }
    }
    assert!(
        latency < PROMPT_WHILE_BUSY_BOUND,
        "ping/list waited {latency:?} behind the cancelled call"
    );

    session.close_stdin();
    assert!(session.wait_bounded(ABORT_WAIT_TIMEOUT).success());
}

/// K6+K7: stray cancels are silent and harmless -- unknown id,
/// post-completion id, and double cancel emit no output -- and a cancel sent
/// before its id is ever issued does not poison the later call reusing that
/// id (the call succeeds normally; all probed).
#[test]
fn stray_cancels_are_silent_and_harmless() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&cancelled(999));
    assert_silent(&session, STRAY_QUIET_WINDOW);
    session.send(&ping(10));
    assert_ping_ok(&session.recv(), 10);

    session.send(&tool_call(11, "index_status", json!({})));
    let status = session.recv();
    assert_eq!(status["id"], 11, "{status:#}");
    assert_tool_success(&status);
    session.send(&cancelled(11));
    assert_silent(&session, STRAY_QUIET_WINDOW);

    session.send(&cancelled(11));
    session.send(&cancelled(11));
    session.send(&cancelled(4242));
    assert_silent(&session, STRAY_QUIET_WINDOW);

    // Pre-cancel for an id never issued: the later call with that id still
    // runs normally.
    session.send(&cancelled(12));
    session.send(&tool_call(12, "index_status", json!({})));
    let late = session.recv();
    assert_eq!(late["id"], 12, "{late:#}");
    assert_tool_success(&late);

    session.send(&ping(13));
    assert_ping_ok(&session.recv(), 13);
    session.close_stdin();
    assert!(session.wait_clean().success());
}
