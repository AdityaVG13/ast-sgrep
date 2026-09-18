//! MCP stdio session harness for `asgrep-mcp` integration tests.
//!
//! # Contract
//!
//! - One canonical copy of the ~200-line harness duplicated across the
//!   `tests/mcp` suites (tokio, recovery, error-api, invalidation, numerical,
//!   oracle). Drivers spawn the real `asgrep-mcp` binary over stdio; no
//!   in-process mocks.
//! - Payload builders return [`serde_json::Value`]; response extractors take
//!   `&Value` and panic (never `Result`) on shape mismatch, matching suite
//!   convention: a malformed envelope is a test failure, not a fallible op.
//! - `rpc_session` / `rpc_pipeline` assert a clean exit (`status.success()`).
//!   [`LiveSession::wait_clean`] returns the [`ExitStatus`](std::process::ExitStatus)
//!   so long-lived drills can assert it themselves; every other wait panics.
//! - [`LiveSession`] reads are timeout-bounded (reader thread +
//!   `recv_timeout`); a regressed server fails the test instead of hanging CI.
//! - `index_tree` / `indexed_tree` index with **default** `IndexOptions`
//!   resolution (not an explicit `index_path`) so the planted index lands
//!   exactly where the server process opens it. By design, not hermetic:
//!   do not set `ASGREP_INDEX_PATH` around these fixtures.
//! - Determinism: builders and extractors are pure functions of their inputs.
//!   Process drivers are deterministic up to server behavior; `rpc_session`
//!   preserves request order (sequential send/recv) while `rpc_pipeline`
//!   returns arrival order (the server answers concurrently by design).

use ast_sgrep_core::{Indexer, IndexOptions};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Client name the canned drivers (`rpc_session`, `rpc_pipeline`,
/// [`LiveSession::handshake`]) send in the `initialize` handshake.
/// Custom flows can pass their own name to [`init_payload`].
pub const TESTKIT_CLIENT_NAME: &str = "asgrep-testkit";

/// Per-read bound for [`LiveSession::recv`]. Slow phases use [`LiveSession::recv_timeout`].
const RECV_TIMEOUT: Duration = Duration::from_secs(15);
/// Bound for [`LiveSession::wait_clean`] before the child is killed.
const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

/// Locate the `asgrep-mcp` binary: `CARGO_BIN_EXE_asgrep-mcp` when cargo sets
/// it for the test target, else `$CARGO_TARGET_DIR/<profile>/`, else the
/// workspace `target/<profile>/` relative to this crate's manifest.
pub fn mcp_bin() -> PathBuf {
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

/// JSON-RPC `initialize` handshake payload (id `"__init"`,
/// protocolVersion `"2025-11-25"`). Pure constructor.
pub fn init_payload(client_name: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "__init",
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": client_name, "version": "0"}
        }
    })
}

/// `notifications/initialized` (no id, no response expected). Pure constructor.
pub fn initialized_notif() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// `tools/call` request envelope. Pure constructor.
pub fn tool_call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

/// `tools/list` request envelope. Pure constructor.
pub fn tools_list(id: u32) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":{}})
}

/// `ping` request envelope. Pure constructor.
pub fn ping(id: u32) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"ping"})
}

/// `notifications/cancelled` for `request_id` (no id, no response expected).
/// Pure constructor.
pub fn cancelled_notif(request_id: u32) -> Value {
    json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request_id}})
}

/// A live stdio session with timeout-bounded reads: a regressed server fails
/// the test instead of hanging the suite. Dropping the session kills the child.
pub struct LiveSession {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
}

impl LiveSession {
    /// Spawn the server; sets `ASGREP_ROOT` when `root` is `Some`.
    /// Panics when the child cannot be spawned.
    pub fn spawn(root: Option<&Path>) -> Self {
        let mut command = Command::new(mcp_bin());
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        if let Some(root) = root {
            command.env("ASGREP_ROOT", root);
        }
        let mut child = command.spawn().expect("spawn MCP");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout: ChildStdout = child.stdout.take().expect("MCP stdout");
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

    /// Run the `initialize` handshake (sends init, asserts the `"__init"`
    /// echo, sends `notifications/initialized`). Panics on mismatch.
    pub fn handshake(&mut self) {
        self.send(&init_payload(TESTKIT_CLIENT_NAME));
        let init = self.recv();
        assert_eq!(init["id"], "__init", "{init:#}");
        self.send(&initialized_notif());
    }

    /// Write one JSON line + flush. Panics when stdin is closed or the write fails.
    pub fn send(&mut self, payload: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{payload}").expect("write MCP stdin");
        stdin.flush().expect("flush MCP stdin");
    }

    /// Read one JSON response, bounded by the 15s default. Panics on timeout,
    /// EOF, or non-JSON output.
    pub fn recv(&self) -> Value {
        self.recv_timeout(RECV_TIMEOUT)
    }

    /// Read one JSON response with an explicit bound. Same panics as [`Self::recv`].
    pub fn recv_timeout(&self, timeout: Duration) -> Value {
        let line = match self.lines.recv_timeout(timeout) {
            Ok(line) => line,
            Err(_) => panic!("timed out after {timeout:?} waiting for MCP output"),
        }
        .expect("MCP closed stdout while a response was pending");
        serde_json::from_str(line.trim()).expect("server emitted JSON-RPC")
    }

    /// Close stdin (signals EOF to the server) without waiting.
    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// Wait up to 15s for exit; kills the child and panics past the deadline.
    /// Returns the exit status for the caller to assert on.
    pub fn wait_clean(&mut self) -> ExitStatus {
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

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Drive `payloads` through ONE server process, strictly sequential (send one,
/// read one), so response order matches request order. Notifications (no `id`)
/// are sent without waiting. Sets `ASGREP_ROOT` when `root` is `Some`.
/// Panics on handshake mismatch, EOF, or a non-clean exit.
pub fn rpc_session(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    rpc_session_env(payloads, root, &[])
}

/// [`rpc_session`] with extra child env applied (e.g. fault-injection knobs).
/// Extras are applied after `ASGREP_ROOT`, so they win on collision.
pub fn rpc_session_env(
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
    let mut stdin = child.stdin.take().expect("MCP stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("MCP stdout"));
    let send = |stdin: &mut ChildStdin, payload: &Value| {
        writeln!(stdin, "{payload}").expect("write MCP stdin");
        stdin.flush().expect("flush MCP stdin");
    };
    let recv = |stdout: &mut BufReader<ChildStdout>| -> Value {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout");
        serde_json::from_str(line.trim()).expect("JSON-RPC")
    };
    send(&mut stdin, &init_payload(TESTKIT_CLIENT_NAME));
    let init = recv(&mut stdout);
    assert_eq!(init["id"], "__init", "{init:#}");
    send(&mut stdin, &initialized_notif());
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

/// Fire ALL payloads without waiting, then collect one response per id-bearing
/// payload. Arrival order is timing-dependent by design (the server dispatches
/// each request on its own task); callers match by `id`. Panics on handshake
/// mismatch, mid-batch EOF, or a non-clean exit.
pub fn rpc_pipeline(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    let mut child = command.spawn().expect("spawn MCP");
    let mut stdin = child.stdin.take().expect("MCP stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("MCP stdout"));
    writeln!(stdin, "{}", init_payload(TESTKIT_CLIENT_NAME)).expect("write init");
    stdin.flush().expect("flush init");
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read init");
    assert!(!line.trim().is_empty(), "MCP closed stdout");
    writeln!(stdin, "{}", initialized_notif()).expect("write initialized");
    let expected = payloads.iter().filter(|p| p.get("id").is_some()).count();
    for payload in &payloads {
        writeln!(stdin, "{payload}").expect("write MCP stdin");
    }
    stdin.flush().expect("flush MCP stdin");
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

/// First text block of a `tools/call` result (`result.content[0].text`).
/// Panics when the envelope lacks it. Pure accessor.
pub fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text content")
}

/// Parse [`tool_text`] as JSON. Panics when the text is not JSON. Pure accessor.
pub fn tool_body(response: &Value) -> Value {
    serde_json::from_str(tool_text(response)).expect("tool body JSON")
}

/// The `isError` discriminant (`result.isError == true`). Panics when the
/// envelope lacks a boolean `isError` — a malformed envelope is a test
/// failure, never a silent `false`. Pure accessor; never inspects message text.
pub fn is_error(response: &Value) -> bool {
    response["result"]["isError"].as_bool().unwrap()
}

/// Assert the success discriminant: `isError == false`, no top-level `error`.
pub fn assert_tool_success(response: &Value) {
    assert_eq!(response["result"]["isError"], false, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
}

/// Assert a `ping` response: id echo, no top-level `error`, object `result`.
pub fn assert_ping_ok(response: &Value, id: u32) {
    assert_eq!(response["id"], id, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert!(response["result"].is_object(), "{response:#}");
}

/// Canonical `tools/list` tool names in server order.
pub fn expected_tool_names() -> Vec<&'static str> {
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

/// Assert a `tools/list` response: id echo plus exactly the canonical tool
/// set, in server order.
pub fn assert_tools_list_ok(response: &Value, id: u32) {
    assert_eq!(response["id"], id, "{response:#}");
    let names: Vec<String> = response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, expected_tool_names(), "{response:#}");
}

/// Collect exactly `n` responses, each within `per_read`. Returns arrival
/// order. Panics on timeout, EOF, or non-JSON output (via
/// [`LiveSession::recv_timeout`]).
pub fn collect_responses(session: &LiveSession, n: usize, per_read: Duration) -> Vec<Value> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(session.recv_timeout(per_read));
    }
    out
}

/// The response bearing `id`. Panics when no response carries it. Pure accessor.
pub fn response_by_id(responses: &[Value], id: u32) -> &Value {
    responses
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("missing response id {id}: {responses:#?}"))
}

/// Assert the strictest error shape: `isError == true`, no top-level `error`,
/// exactly one `text` content block, no `structuredContent`.
pub fn assert_tool_error_shape(response: &Value) {
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

/// Index `path` with default `IndexOptions` resolution — the planted index
/// lands where the server opens it. Panics on index failure. See module docs.
pub fn index_tree(path: &Path) {
    Indexer::new(IndexOptions {
        root: path.to_path_buf(),
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index_all");
}

/// Temp tree with `files` (`(relative path, body)` pairs), indexed via
/// [`index_tree`]. The caller keeps the [`TempDir`] alive. Panics on IO or
/// index failure.
pub fn indexed_tree(files: &[(&str, &str)]) -> TempDir {
    let temp = crate::fixture::file_tree(files);
    index_tree(temp.path());
    temp
}

/// Three-symbol indexed tree: one token-distinct symbol per file (no shared
/// word-pieces, since keyword search ORs query tokens) so per-query
/// attribution is exact. The caller keeps the [`TempDir`] alive.
pub fn small_tree() -> TempDir {
    indexed_tree(&[
        ("a.rs", "fn redhammer() {}\n"),
        ("b.rs", "fn blueanvil() {}\n"),
        (
            "c.rs",
            "fn greenchisel() {}\nfn greenchisel_helper() {}\n",
        ),
    ])
}

/// Wide unindexed tree with `files` single-symbol files: `index_repo` over it
/// is the reliably-slow tool call. Symbols carry a distinctive prefix so a
/// follow-up search proves the index is queryable. The caller keeps the
/// [`TempDir`] alive.
pub fn big_tree(files: usize) -> TempDir {
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

/// Overwrite the durable `<root>/.asgrep/index.db` with deterministic
/// non-SQLite bytes. Panics when no index db exists (the fault requires a
/// planted index). Returns the bytes written for post-drill comparison.
pub fn corrupt_index_db(root: &Path) -> Vec<u8> {
    let db = root.join(".asgrep").join("index.db");
    assert!(db.is_file(), "expected an index db at {}", db.display());
    crate::fault::write_garbage(&db)
}
