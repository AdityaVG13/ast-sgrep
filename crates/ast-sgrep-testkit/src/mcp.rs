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
//! - `rpc_session` / `rpc_session_env` / `rpc_pipeline` / `rpc_session_raw`
//!   assert a clean exit (`status.success()`).
//!   [`LiveSession::wait_clean`] returns the [`ExitStatus`](std::process::ExitStatus)
//!   so long-lived drills can assert it themselves; every other wait panics.
//! - [`LiveSession`] reads are timeout-bounded (reader thread +
//!   `recv_timeout`); a regressed server fails the test instead of hanging CI.
//!   The one-shot drivers ride [`LiveSession`], so every driver read and
//!   every driver wait is bounded the same way (15s).
//! - `index_tree` / `indexed_tree` plant at the explicit default-state path
//!   ([`crate::index_db_path`]), so the planted index lands exactly where the
//!   server process opens it and ambient `ASGREP_INDEX_PATH` cannot redirect
//!   it. Embed stays on (deliberate delta vs
//!   [`crate::hermetic_index_options`]): the planted DB carries the active
//!   embed identity so the server's first refresh stays incremental — see
//!   [`index_tree`].
//! - Determinism: builders and extractors are pure functions of their inputs.
//!   Process drivers are deterministic up to server behavior; `rpc_session`
//!   preserves request order (sequential send/recv) while `rpc_pipeline`
//!   returns arrival order (the server answers concurrently by design).

use ast_sgrep_core::{IndexOptions, Indexer};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Client name the canned drivers (`rpc_session`, `rpc_pipeline`,
/// [`LiveSession::handshake`]) send in the `initialize` handshake.
/// Custom flows can pass their own name to [`init_payload`].
pub const TESTKIT_CLIENT_NAME: &str = "asgrep-testkit";

use crate::spawn::{SPAWN_POLL_INTERVAL, SPAWN_RECV_TIMEOUT, SPAWN_WAIT_TIMEOUT};

/// Locate the `asgrep-mcp` binary: `CARGO_BIN_EXE_asgrep-mcp` when cargo sets
/// it for the test target, else `$CARGO_TARGET_DIR/<profile>/`, else the
/// workspace `target/<profile>/` relative to this crate's manifest.
pub fn mcp_bin() -> PathBuf {
    crate::locate_test_bin(option_env!("CARGO_BIN_EXE_asgrep-mcp"), "asgrep-mcp")
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
/// the test instead of hanging the suite. Dropping kills and reaps the child,
/// including on assertion unwind; explicit waits may already have reaped it.
pub struct LiveSession {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
}

impl LiveSession {
    /// Spawn the server; sets `ASGREP_ROOT` when `root` is `Some`.
    /// Panics when the child cannot be spawned.
    pub fn spawn(root: Option<&Path>) -> Self {
        Self::spawn_env(root, &[])
    }

    /// INTENT: [`spawn`](Self::spawn) with extra child env applied (e.g.
    /// fault-injection knobs like `ASGREP_NEURAL_EMBED=1` on the child only —
    /// process-global `set_var` would leak across parallel tests). Extras are
    /// applied after `ASGREP_ROOT`, so they win on collision. Panics when the
    /// child cannot be spawned.
    pub fn spawn_env(root: Option<&Path>, extra_env: &[(&str, &str)]) -> Self {
        let mut command = Command::new(mcp_bin());
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        if let Some(root) = root {
            command.env("ASGREP_ROOT", root);
        }
        for (key, value) in extra_env {
            command.env(key, value);
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

    /// Write one raw line + flush. Panics when stdin is closed or the write fails.
    fn write_line(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{line}").expect("write MCP stdin");
        stdin.flush().expect("flush MCP stdin");
    }

    /// Write one JSON line + flush. Panics when stdin is closed or the write fails.
    pub fn send(&mut self, payload: &Value) {
        self.write_line(&payload.to_string());
    }

    /// Read one JSON response, bounded by the 15s default. Panics on timeout,
    /// EOF, or non-JSON output.
    pub fn recv(&self) -> Value {
        self.recv_timeout(SPAWN_RECV_TIMEOUT)
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

    /// INTENT: read one raw response line (without trailing CR/LF), bounded
    /// by the 15s default — the byte-identity primitive for determinism
    /// relations that compare wire bytes, not re-serialized values. Panics
    /// on timeout or EOF.
    pub fn recv_raw(&self) -> String {
        let line = match self.lines.recv_timeout(SPAWN_RECV_TIMEOUT) {
            Ok(line) => line,
            Err(_) => panic!("timed out after {SPAWN_RECV_TIMEOUT:?} waiting for MCP output"),
        }
        .expect("MCP closed stdout while a response was pending");
        line.trim_end_matches(['\r', '\n']).to_owned()
    }

    /// INTENT: plant a byte-level stream fault — write one raw line + flush.
    /// Unparsable lines must be ignored (no echo, no hang); the next valid
    /// call still answers. Panics when stdin is closed or the write fails.
    pub fn send_raw_line(&mut self, line: &str) {
        self.write_line(line);
    }

    /// INTENT: plant a torn request — write bytes with no trailing newline,
    /// as from a writer killed mid-line. The server must never answer the
    /// torn id. Panics when stdin is closed or the write fails.
    pub fn send_partial(&mut self, bytes: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(bytes.as_bytes()).expect("write MCP stdin");
        stdin.flush().expect("flush MCP stdin");
    }

    /// INTENT: collect post-fault stdout as raw lines until EOF (or the total
    /// budget, which panics past the deadline): proves a torn stream emits no
    /// torn-id response and no non-JSON garbage.
    pub fn drain_until_eof(&self, budget: Duration) -> Vec<String> {
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

    /// INTENT: crash the server mid-session — SIGKILL without a clean stdin
    /// close. Returns the terminal status; the caller asserts the crash
    /// discriminant. Panics when the killed child does not terminate in budget.
    pub fn crash_kill(&mut self) -> ExitStatus {
        self.stdin.take();
        let _ = self.child.kill();
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("poll MCP") {
                return status;
            }
            if started.elapsed() > SPAWN_WAIT_TIMEOUT {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("killed MCP did not terminate within {SPAWN_WAIT_TIMEOUT:?}");
            }
            std::thread::sleep(SPAWN_POLL_INTERVAL);
        }
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
            if started.elapsed() > SPAWN_WAIT_TIMEOUT {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("MCP did not exit within {SPAWN_WAIT_TIMEOUT:?}");
            }
            std::thread::sleep(SPAWN_POLL_INTERVAL);
        }
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Drive `payloads` through ONE server process, strictly sequential (send one,
/// read one), so response order matches request order. Notifications (no `id`)
/// are sent without waiting. Sets `ASGREP_ROOT` when `root` is `Some`. Every
/// read and the exit wait are timeout-bounded (15s): a regressed server fails
/// the test instead of hanging the suite. Panics on handshake mismatch, EOF,
/// timeout, or a non-clean exit.
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
    let mut session = LiveSession::spawn_env(root, extra_env);
    session.handshake();
    let mut responses = Vec::new();
    for payload in &payloads {
        session.send(payload);
        if payload.get("id").is_some() {
            responses.push(session.recv());
        }
    }
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
    responses
}

/// INTENT: [`rpc_session`] returning the RAW response lines (one per payload,
/// post-handshake) so determinism relations compare wire bytes, not
/// re-serialized values. Positional 1:1 — every payload consumes exactly one
/// line, so callers pass id-bearing payloads only. Every read and the exit
/// wait are timeout-bounded (15s). Panics on handshake mismatch, EOF,
/// timeout, or a non-clean exit.
pub fn rpc_session_raw(payloads: Vec<Value>, root: Option<&Path>) -> Vec<String> {
    let mut session = LiveSession::spawn(root);
    session.handshake();
    let mut responses = Vec::new();
    for payload in &payloads {
        session.send(payload);
        responses.push(session.recv_raw());
    }
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
    responses
}

/// INTENT: startup-failure flows — spawn the server with NO handshake, write
/// each payload as one stdin line, close stdin, and return the full [`Output`]
/// (the canned drivers assert a clean exit, so they cannot express nonzero
/// startup). Sets `ASGREP_ROOT` when `root` is `Some`; `extra_env` wins on
/// collision. Panics when the child cannot be spawned or waited on.
pub fn spawn_raw_no_handshake(
    root: Option<&Path>,
    extra_env: &[(&str, &str)],
    stdin_lines: &[Value],
) -> Output {
    let mut command = Command::new(mcp_bin());
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn MCP");
    {
        let mut stdin = child.stdin.take().expect("MCP stdin");
        for line in stdin_lines {
            writeln!(stdin, "{line}").expect("write MCP stdin");
        }
    }
    child.wait_with_output().expect("wait MCP")
}

/// Fire ALL payloads without waiting, then collect one response per id-bearing
/// payload. Arrival order is timing-dependent by design (the server dispatches
/// each request on its own task); callers match by `id`. Every read and the
/// exit wait are timeout-bounded (15s): a regressed server fails the test
/// instead of hanging the suite. Panics on handshake mismatch, mid-batch EOF,
/// timeout, or a non-clean exit.
pub fn rpc_pipeline(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    let mut session = LiveSession::spawn(root);
    session.handshake();
    for payload in &payloads {
        session.send(payload);
    }
    let expected = payloads.iter().filter(|p| p.get("id").is_some()).count();
    let mut responses = Vec::new();
    for _ in 0..expected {
        responses.push(session.recv());
    }
    session.close_stdin();
    let status = session.wait_clean();
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

/// INTENT: per-hit snippet byte lengths (hit row index 4) — elision and
/// budget accounting re-sum rendered snippet bytes from this projection.
/// Pure accessor (panics when the body lacks the compact hit rows).
pub fn snippet_bytes(body: &Value) -> Vec<usize> {
    body["h"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit.as_array().unwrap()[4].as_str().unwrap().len())
        .collect()
}

/// The `isError` discriminant (`result.isError == true`). Under fastmcp's
/// auto facade the field is omitted when false (both eras), so ABSENT means
/// success; a present-but-non-boolean is a malformed envelope and panics.
pub fn is_error(response: &Value) -> bool {
    match &response["result"]["isError"] {
        Value::Bool(true) => true,
        Value::Bool(false) => false,
        Value::Null => false,
        other => panic!("malformed isError member: {other} in {response:#}"),
    }
}

/// Assert the success discriminant: `isError` false-or-absent (fastmcp
/// omits the member when false on both eras), no top-level `error`.
pub fn assert_tool_success(response: &Value) {
    assert!(!is_error(response), "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
}

/// INTENT: strict tool-success envelope — no error discriminant AND a
/// machine-readable body. The body's member is era syntax: the modern
/// (2026-07-28) era lifts it into `structuredContent`; the exact-legacy
/// (2024-11-05) projection carries it in the text block, which is itself
/// the compact JSON envelope. Machine readability is the contract; either
/// member satisfies it. Pure assertion.
pub fn assert_tool_success_shape(response: &Value) {
    assert_tool_success(response);
    if response["result"].get("structuredContent").is_none() {
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        let parses_as_object = serde_json::from_str::<Value>(text)
            .map(|body| body.is_object())
            .unwrap_or(false);
        assert!(
            parses_as_object,
            "no structuredContent and text block is not a JSON object: {response:#}"
        );
    }
}

/// INTENT: JSON-RPC error row — numeric `code`, object `error`, no
/// `result`. Pure assertion.
pub fn assert_jsonrpc_error(response: &Value, code: i64) {
    assert_eq!(response["error"]["code"], code, "{response:#}");
    assert!(response["error"].is_object(), "{response:#}");
    assert!(response.get("result").is_none(), "{response:#}");
}

/// INTENT: machine-readable error discriminant — everything about a tool
/// error EXCEPT the human message text: `(isError, content len, block-0
/// type, block-0 text-is-string, has structuredContent, has top-level
/// error)`. Equal discriminants mean the same error code/shape reached
/// the caller. Pure projection.
pub fn tool_error_discriminant(response: &Value) -> (bool, usize, String, bool, bool, bool) {
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

/// Assert the error shape: `isError == true`, no top-level `error`, exactly
/// one `text` content block. `structuredContent` is era syntax (modern error
/// results carry the admitted miss envelope; exact-legacy omits the member),
/// so its presence is not asserted either way here. Pure assertion.
pub fn assert_tool_error_shape(response: &Value) {
    assert!(is_error(response), "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert_eq!(
        response["result"]["content"][0]["type"], "text",
        "{response:#}"
    );
    assert_eq!(
        response["result"]["content"].as_array().map(Vec::len),
        Some(1),
        "{response:#}"
    );
}

/// INTENT: plant the default-state index pair — explicit
/// [`crate::index_db_path`] (immune to ambient `ASGREP_INDEX_PATH`), default
/// [`IndexOptions`] otherwise — so the planted index lands exactly where the
/// server opens it. Deliberate delta vs [`crate::hermetic_index_options`]:
/// embed stays ON: the planted DB carries the active embed identity, so the
/// server's first refresh is incremental (a single-file modify reindexes 1
/// file). Planting embed-off forces a one-time full semantic rewrite on first
/// refresh (measured: `files_indexed` 2 vs 1 on the modify phase) and breaks
/// the refresh-stat pins in `tests/mcp/invalidation_matrix.rs`. Panics on
/// index failure.
pub fn index_tree(path: &Path) {
    Indexer::new(IndexOptions {
        root: path.to_path_buf(),
        index_path: Some(crate::index_db_path(path)),
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index_all");
}

/// Temp tree with `files` (`(relative path, body)` pairs; parents created),
/// indexed via [`index_tree`]. Accepts borrowed or owned pairs (`&str` or
/// `String` bodies alike), so generated-name fixtures pass owned strings
/// directly. The caller keeps the [`TempDir`] alive. Panics on IO or
/// index failure.
pub fn indexed_tree<A: AsRef<str>, B: AsRef<str>>(files: &[(A, B)]) -> TempDir {
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
        ("c.rs", "fn greenchisel() {}\nfn greenchisel_helper() {}\n"),
    ])
}

/// Wide unindexed tree with `files` single-symbol files: `index_repo` over it
/// is the reliably-slow tool call. Symbols carry a distinctive prefix so a
/// follow-up search proves the index is queryable. Generated names are owned
/// `String`s, routed through [`crate::fixture::file_tree`]. The caller keeps
/// the [`TempDir`] alive.
pub fn big_tree(files: usize) -> TempDir {
    let owned: Vec<(String, String)> = (0..files)
        .map(|index| {
            (
                format!("f{index}.rs"),
                format!("pub fn k2requelch_{index}() {{}}\n"),
            )
        })
        .collect();
    crate::fixture::file_tree(&owned)
}

/// INTENT: distinct `<path_id>` prefixes across compact hit ids — the only
/// cheap "distinct files" discriminant on the wire. Pure projection; panics
/// when a hit id is not a `path:span` string.
pub fn distinct_hit_paths(hits: &[Value]) -> std::collections::HashSet<String> {
    hits.iter()
        .map(|hit| {
            hit[0]
                .as_str()
                .expect("hit id is a string")
                .rsplit_once(':')
                .expect("compact id carries a path id")
                .0
                .to_owned()
        })
        .collect()
}

/// INTENT: generated-name indexed fixture — six single-symbol files plus the
/// target file, so limit 2 vs limit 8 differ. Bodies carry the shared
/// `shared_token` stem the surface suites query. [`indexed_tree`] takes
/// borrowed or owned pairs, so generated names pass straight through. The
/// caller keeps the [`TempDir`] alive. Panics on IO or index failure.
pub fn multi_hit_tree() -> TempDir {
    let mut owned: Vec<(String, String)> = (1..=6)
        .map(|index| {
            (
                format!("src/m{index}.rs"),
                format!("fn shared_token_{index}() {{}}\n"),
            )
        })
        .collect();
    owned.push((
        "src/lib.rs".to_owned(),
        "fn target_symbol() {}\n".to_owned(),
    ));
    indexed_tree(&owned)
}

/// Overwrite the durable `<root>/.asgrep/index.db` with deterministic
/// non-SQLite bytes. Panics when no index db exists (the fault requires a
/// planted index). Returns the bytes written for post-drill comparison.
/// Sidecars are left behind by design (cf. [`crate::corrupt_db_total`], the
/// WAL-total variant).
pub fn corrupt_index_db(root: &Path) -> Vec<u8> {
    let db = crate::index_db_path(root);
    assert!(db.is_file(), "expected an index db at {}", db.display());
    crate::fault::write_garbage(&db)
}

/// INTENT: one-shot single-call fresh-process convenience over
/// [`rpc_session`]. One canonical copy of the helper duplicated across the
/// `tests/mcp/invalidation_*` suites.
pub fn rpc_at(payload: Value, root: &Path) -> Value {
    let mut responses = rpc_session(vec![payload], Some(root));
    responses.pop().expect("one response")
}

/// INTENT: canonical `keyword_search` args (limit 8, resend_seen) shared by
/// [`search_call`] and [`CallSession::search_text`] so the query shape cannot
/// drift per call site. Pure constructor.
pub fn keyword_search_args(query: &str) -> Value {
    json!({"query": query, "limit": 8, "resend_seen": true})
}

/// INTENT: canonical `keyword_search` envelope (limit 8, resend_seen) shared
/// by every invalidation phase so query shape cannot drift per test. One
/// canonical copy of the helper duplicated across the suites.
pub fn search_call(id: u32, query: &str) -> Value {
    tool_call(id, "keyword_search", keyword_search_args(query))
}

/// INTENT: hit body-shape discriminant (no `why` + `zn >= 1` + nonempty `h`).
/// One canonical copy of the helper triplicated across the suites.
pub fn assert_hit_envelope(body: &Value) {
    assert!(body.get("why").is_none(), "hit must carry no why: {body:#}");
    assert!(
        body["zn"].as_u64().unwrap_or(0) >= 1,
        "hit must count >= 1: {body:#}"
    );
    assert!(
        !body["h"].as_array().expect("hit h array").is_empty(),
        "hit h must be nonempty: {body:#}"
    );
}

/// INTENT: miss body-shape discriminant for one `why` code
/// (`why` + `zn == 0` + empty `h`). One canonical copy of the helper
/// triplicated across the suites.
pub fn assert_miss_envelope(body: &Value, why: &str) {
    assert_eq!(body["why"], why, "{body:#}");
    assert_eq!(body["zn"], 0, "{body:#}");
    assert_eq!(body["h"], json!([]), "{body:#}");
}

/// INTENT: id-tracking live session over [`LiveSession`] — the strictest of
/// the three suite copies (spawn/call/finish plus the `search_text` /
/// `status_text` / `refresh` convenience the drills add). Reads ride the 15s
/// `recv` bound, `finish` rides the 15s `wait_clean` bound.
pub struct CallSession {
    inner: LiveSession,
    next_id: u32,
}

impl CallSession {
    /// Spawn the server at `root` and run the `initialize` handshake.
    pub fn spawn(root: &Path) -> Self {
        let mut inner = LiveSession::spawn(Some(root));
        inner.handshake();
        Self { inner, next_id: 1 }
    }

    /// Send one `tools/call` with the next id, read its response, assert the
    /// id echo.
    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.inner.send(&tool_call(id, name, arguments));
        let response = self.inner.recv();
        assert_eq!(response["id"], id, "{response:#}");
        response
    }

    /// Successful lexical search; returns the raw body bytes.
    pub fn search_text(&mut self, query: &str) -> String {
        let response = self.call("keyword_search", keyword_search_args(query));
        assert_tool_success(&response);
        tool_text(&response).to_owned()
    }

    /// Successful status call; returns the raw body bytes.
    pub fn status_text(&mut self) -> String {
        let response = self.call("index_status", json!({}));
        assert_tool_success(&response);
        tool_text(&response).to_owned()
    }

    /// Successful in-session refresh; returns the stats body.
    pub fn refresh(&mut self) -> Value {
        let response = self.call("index_repo", json!({}));
        assert_tool_success(&response);
        tool_body(&response)
    }

    /// Close stdin, wait (bounded), and assert a clean exit.
    pub fn finish(mut self) {
        self.inner.close_stdin();
        let status = self.inner.wait_clean();
        assert!(status.success(), "MCP exited {status}");
    }
}

/// INTENT: hit paths projected through the compact `p` table exactly as the
/// server resolves them, pinning `zn ==` row-count plus table `==` hit-paths
/// (the old-path-leaves-table discriminant). One canonical copy of the helper
/// duplicated across the suites (comment-only drift; strictest comment kept).
/// Needs the `plugins` feature for the compact-path resolver.
#[cfg(feature = "plugins")]
pub fn hit_path_set(body: &Value) -> std::collections::BTreeSet<String> {
    let table: std::collections::HashMap<String, String> =
        ast_sgrep_plugins::resolve_compact_paths(body)
            .into_iter()
            .collect();
    let table_paths: std::collections::BTreeSet<String> = table.values().cloned().collect();
    let hits = body["h"].as_array().expect("hit h array");
    assert_eq!(
        body["zn"].as_u64().unwrap_or(u64::MAX),
        hits.len() as u64,
        "zn must equal hit rows: {body:#}"
    );
    let mut paths = std::collections::BTreeSet::new();
    for row in hits {
        let id = row[0].as_str().expect("hit id string");
        let (path_id, _) = id.rsplit_once(':').expect("compact id carries range");
        let path = table
            .get(path_id)
            .unwrap_or_else(|| panic!("path id {path_id} must resolve in p table: {body:#}"))
            .clone();
        paths.insert(path);
    }
    assert_eq!(
        table_paths, paths,
        "p table must name exactly the hit paths: {body:#}"
    );
    paths
}

/// INTENT: hit envelope whose every hit resolves under exactly `expected`
/// paths. One canonical copy of the helper duplicated across the suites.
/// Needs the `plugins` feature (see [`hit_path_set`]).
#[cfg(feature = "plugins")]
pub fn assert_hit_path_set(body: &Value, expected: &[&str]) {
    assert_hit_envelope(body);
    let want: std::collections::BTreeSet<String> =
        expected.iter().map(ToString::to_string).collect();
    assert_eq!(hit_path_set(body), want, "{body:#}");
}
