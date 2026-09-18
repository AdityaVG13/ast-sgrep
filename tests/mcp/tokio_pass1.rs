//! K1 tokio concurrency-contract tests for ast-sgrep-mcp.
//!
//! Pins the async surface in `crates/ast-sgrep-mcp/src/handler.rs` through the
//! real stdio binary: tool calls serialize on `tool_lock` and run on
//! `spawn_blocking`, while `ping` and `tools/list` stay on the tokio reader.
//!
//! Contracts pinned (discriminants are codes, shapes, counts, id echo, and
//! byte (in)equality -- never message text, never durations):
//!
//! * C1: a pipelined `tools/call` batch is fully answered, each response
//!   matched by id with its own correct result (serialization, no interleave
//!   corruption). Wire order is NOT pinned: rmcp dispatches each request on
//!   its own task, so arrival order is timing-dependent by design.
//! * C2: distinct pipelined searches show no cross-talk (each body carries its
//!   own query symbol only).
//! * C3: a pipelined error neither poisons the batch nor the session.
//! * C4: rapid-fire identical calls are all answered identically (byte-equal).
//! * C5: `ping` is answered promptly while a slow tool holds the lock.
//! * C6: `tools/list` succeeds during an active tool call.
//! * C7: mixed-method pipelines (ping/list/call) are all matched by id.
//! * C8: concurrent writer threads are all answered (whole-line atomicity).
//! * C9: the session survives back-to-back batches and phases with a clean exit.
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
/// Generous promptness bound for ping-while-busy (seconds, not ms): a reader
/// blocked behind a 1200-file index would miss this by an order of magnitude.
const PING_WHILE_BUSY_BOUND: Duration = Duration::from_secs(10);

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
            "clientInfo": {"name": "asgrep-mcp-k1", "version": "0"}
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
        let line = match self.lines.recv_timeout(timeout) {
            Ok(line) => line,
            Err(_) => panic!("timed out after {timeout:?} waiting for MCP output"),
        }
        .expect("MCP closed stdout while a response was pending");
        serde_json::from_str(line.trim()).expect("server emitted JSON-RPC")
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
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

fn assert_tool_error_shape(response: &Value) {
    assert_eq!(response["result"]["isError"], true, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
    assert_eq!(
        response["result"]["content"][0]["type"],
        "text",
        "{response:#}"
    );
    assert!(
        response["result"].get("structuredContent").is_none(),
        "{response:#}"
    );
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

/// Three-symbol indexed tree: one token-distinct symbol per file (no shared
/// word-pieces, since keyword search ORs query tokens) so per-query
/// attribution is exact.
fn small_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn redhammer() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn blueanvil() {}\n").unwrap();
    std::fs::write(
        temp.path().join("c.rs"),
        "fn greenchisel() {}\nfn greenchisel_helper() {}\n",
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

/// Wide unindexed tree: `index_repo` over it is the reliably-slow tool call.
fn big_tree(files: usize) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..files {
        std::fs::write(
            temp.path().join(format!("f{index}.rs")),
            format!("pub fn slow_fn_{index}() {{}}\n"),
        )
        .unwrap();
    }
    temp
}

/// Collect exactly `n` responses, each within `per_read`. Returns arrival order.
fn collect(session: &LiveSession, n: usize, per_read: Duration) -> Vec<Value> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(session.recv_timeout(per_read));
    }
    out
}

fn by_id(responses: &[Value], id: u32) -> &Value {
    responses
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("missing response id {id}: {responses:#?}"))
}

/// C1: a pipelined batch of mixed tool calls is fully answered, each response
/// matched by id with its own correct result (no interleave corruption).
#[test]
fn pipelined_tool_batch_all_answered_with_matching_ids() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let batch = vec![
        tool_call(1, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        tool_call(2, "index_status", json!({})),
        tool_call(3, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
        tool_call(4, "keyword_search", json!({"query": "blueanvil", "limit": 4})),
        tool_call(5, "code_search", json!({"query": "greenchisel", "limit": 4})),
        tool_call(6, "index_status", json!({})),
        tool_call(7, "keyword_search", json!({"query": "greenchisel", "limit": 4})),
        tool_call(
            8,
            "ast_search",
            json!({"query": "fn $NAME() { $$$BODY }", "limit": 8}),
        ),
    ];
    for payload in &batch {
        session.send(payload);
    }
    let responses = collect(&session, batch.len(), RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_eq!(responses.len(), 8);
    for (id, name, query) in [
        (1, "keyword_search", "redhammer"),
        (4, "keyword_search", "blueanvil"),
        (5, "code_search", "greenchisel"),
        (7, "keyword_search", "greenchisel"),
    ] {
        let response = by_id(&responses, id);
        assert_tool_success(response);
        let body = tool_body(response);
        assert!(
            body["h"].as_array().is_some_and(|h| !h.is_empty()),
            "{name} {query}: {body:#}"
        );
        assert!(
            tool_text(response).contains(query),
            "{name} {query}: result does not carry its own query: {response:#}"
        );
    }
    for id in [2, 6] {
        let response = by_id(&responses, id);
        assert_tool_success(response);
        assert!(tool_body(response).is_object(), "{response:#}");
    }
    let read = by_id(&responses, 3);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "a.rs#L1-L1");
    let ast = by_id(&responses, 8);
    assert_tool_success(ast);
    assert!(
        tool_body(ast)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{ast:#}"
    );
}

/// C2: distinct pipelined searches show no cross-talk: each body carries its
/// own query symbol and none of the others.
#[test]
fn pipelined_distinct_searches_have_no_crosstalk() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let queries = ["redhammer", "blueanvil", "greenchisel"];
    for (i, query) in queries.iter().enumerate() {
        session.send(&tool_call(
            i as u32 + 1,
            "keyword_search",
            json!({"query": query, "limit": 8}),
        ));
    }
    let responses = collect(&session, queries.len(), RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_eq!(responses.len(), 3);
    for (i, query) in queries.iter().enumerate() {
        let id = i as u32 + 1;
        let response = by_id(&responses, id);
        assert_tool_success(response);
        let text = tool_text(response).to_string();
        assert!(text.contains(query), "id {id}: {response:#}");
        for other in queries.iter().filter(|q| *q != query) {
            assert!(
                !text.contains(other),
                "id {id} ({query}) leaked {other}: {response:#}"
            );
        }
    }
}

/// C3: a pipelined error neither poisons the batch nor the session: the bad
/// ids carry tool-error shapes, the good ids succeed, and follow-up calls
/// still work.
#[test]
fn pipelined_error_does_not_poison_batch_or_session() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "no_such_tool", json!({})));
    session.send(&tool_call(2, "keyword_search", json!({"query": "x", "limit": 0})));
    session.send(&tool_call(
        3,
        "keyword_search",
        json!({"query": "redhammer", "limit": 4}),
    ));
    session.send(&tool_call(4, "index_status", json!({})));
    let responses = collect(&session, 4, RECV_TIMEOUT);

    assert_tool_error_shape(by_id(&responses, 1));
    assert_tool_error_shape(by_id(&responses, 2));
    let good = by_id(&responses, 3);
    assert_tool_success(good);
    assert!(
        tool_body(good)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{good:#}"
    );
    assert_tool_success(by_id(&responses, 4));

    session.send(&ping(5));
    session.send(&tool_call(
        6,
        "keyword_search",
        json!({"query": "blueanvil", "limit": 4}),
    ));
    let followup = collect(&session, 2, RECV_TIMEOUT);
    assert_ping_ok(by_id(&followup, 5), 5);
    assert_tool_success(by_id(&followup, 6));
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// C4: rapid-fire identical calls are all answered, and every body is
/// byte-identical (serialization without result drift).
#[test]
fn rapid_fire_identical_calls_answered_identically() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    const N: u32 = 12;
    for id in 1..=N {
        session.send(&tool_call(id, "index_status", json!({})));
    }
    let responses = collect(&session, N as usize, RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_eq!(responses.len(), N as usize);
    let first = tool_text(by_id(&responses, 1)).to_string();
    assert_tool_success(by_id(&responses, 1));
    for id in 2..=N {
        let response = by_id(&responses, id);
        assert_tool_success(response);
        assert_eq!(
            tool_text(response),
            first,
            "id {id} differs from id 1: {response:#}"
        );
    }
}

/// C5: `ping` is answered promptly while a slow `index_repo` holds the tool
/// lock: ping arrives first, with no error, well within a generous bound, and
/// the index still completes.
#[test]
fn ping_answered_while_slow_tool_runs() {
    let temp = big_tree(1200);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    let ping_sent = Instant::now();
    session.send(&ping(2));

    let first = session.recv_timeout(SLOW_RECV_TIMEOUT);
    let ping_latency;
    let index_response;
    if first["id"] == 2 {
        ping_latency = ping_sent.elapsed();
        assert_ping_ok(&first, 2);
        index_response = session.recv_timeout(SLOW_RECV_TIMEOUT);
        assert_eq!(index_response["id"], 1, "{index_response:#}");
    } else {
        // The index won the race: still accept the run only if ping follows
        // promptly behind it (reader not wedged), but record the order.
        assert_eq!(first["id"], 1, "{first:#}");
        index_response = first;
        let second = session.recv_timeout(SLOW_RECV_TIMEOUT);
        ping_latency = ping_sent.elapsed();
        assert_ping_ok(&second, 2);
    }
    assert!(
        ping_latency < PING_WHILE_BUSY_BOUND,
        "ping waited {ping_latency:?} behind index_repo"
    );
    assert_tool_success(&index_response);
    assert!(tool_body(&index_response).is_object());
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// C6: `tools/list` succeeds during an active tool call (it takes no lock),
/// and the slow call still completes afterwards.
#[test]
fn tools_list_succeeds_during_active_tool_call() {
    let temp = big_tree(1200);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    session.send(&tools_list(2));
    let responses = collect(&session, 2, SLOW_RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_tools_list_ok(by_id(&responses, 2), 2);
    let index = by_id(&responses, 1);
    assert_tool_success(index);
    assert!(tool_body(index).is_object());
}

/// C7: a mixed-method pipeline (ping/list/call interleaved) is fully
/// answered, each response matched by id with the right shape.
#[test]
fn mixed_method_pipeline_all_matched_by_id() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut next: u32 = 1;
    let mut ping_ids = Vec::new();
    let mut list_ids = Vec::new();
    let mut call_ids = Vec::new();
    for round in 0..4 {
        session.send(&ping(next));
        ping_ids.push(next);
        next += 1;
        session.send(&tools_list(next));
        list_ids.push(next);
        next += 1;
        let query = ["redhammer", "blueanvil", "greenchisel", "redhammer"][round];
        session.send(&tool_call(
            next,
            "keyword_search",
            json!({"query": query, "limit": 4}),
        ));
        call_ids.push((next, query));
        next += 1;
    }
    let total = (next - 1) as usize;
    let responses = collect(&session, total, RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_eq!(responses.len(), total);
    for id in ping_ids {
        assert_ping_ok(by_id(&responses, id), id);
    }
    for id in list_ids {
        assert_tools_list_ok(by_id(&responses, id), id);
    }
    for (id, query) in call_ids {
        let response = by_id(&responses, id);
        assert_tool_success(response);
        assert!(
            tool_text(response).contains(query),
            "id {id}: {response:#}"
        );
    }
}

/// C8: concurrent writer threads piping through one stdin are all answered:
/// whole-line writes stay atomic and every id gets its own correct result.
#[test]
fn concurrent_writer_threads_all_answered() {
    use std::sync::{Arc, Mutex};

    let temp = small_tree();
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    command.env("ASGREP_ROOT", temp.path());
    let mut child = command.spawn().expect("spawn MCP");
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let stdout: ChildStdout = child.stdout.take().unwrap();
    let (tx, rx): (mpsc::Sender<Option<String>>, Receiver<Option<String>>) = mpsc::channel();
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
    let send = |payload: &Value| {
        let stdin = stdin.clone();
        let line = payload.to_string();
        let mut guard = stdin.lock().unwrap();
        writeln!(*guard, "{line}").unwrap();
        guard.flush().unwrap();
    };
    let recv = |timeout: Duration| -> Value {
        let line = match rx.recv_timeout(timeout) {
            Ok(line) => line,
            Err(_) => panic!("timed out after {timeout:?} waiting for MCP output"),
        }
        .expect("MCP closed stdout while a response was pending");
        serde_json::from_str(line.trim()).expect("server emitted JSON-RPC")
    };

    send(&init_payload());
    assert_eq!(recv(RECV_TIMEOUT)["id"], "__init");
    send(&initialized_notif());

    const THREADS: u32 = 4;
    const PER_THREAD: u32 = 5;
    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let stdin = stdin.clone();
            scope.spawn(move || {
                for i in 0..PER_THREAD {
                    let id = 1000 + thread * 100 + i;
                    let payload = if i % 2 == 0 {
                        ping(id)
                    } else {
                        tool_call(id, "index_status", json!({}))
                    };
                    let line = payload.to_string();
                    let mut guard = stdin.lock().unwrap();
                    writeln!(*guard, "{line}").unwrap();
                    guard.flush().unwrap();
                }
            });
        }
    });

    let total = (THREADS * PER_THREAD) as usize;
    let mut responses = Vec::with_capacity(total);
    for _ in 0..total {
        responses.push(recv(RECV_TIMEOUT));
    }
    drop(stdin);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll MCP") {
            break status;
        }
        if started.elapsed() > WAIT_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("MCP did not exit within {WAIT_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "MCP exited {status}");

    assert_eq!(responses.len(), total);
    for thread in 0..THREADS {
        for i in 0..PER_THREAD {
            let id = 1000 + thread * 100 + i;
            let response = by_id(&responses, id);
            if i % 2 == 0 {
                assert_ping_ok(response, id);
            } else {
                assert_tool_success(response);
            }
        }
    }
}

/// C9a: the session survives back-to-back pipelined batches over one
/// handshake: every batch is fully answered.
#[test]
fn session_survives_back_to_back_batches() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    for id in 1..=3 {
        session.send(&tool_call(id, "index_status", json!({})));
    }
    let batch1 = collect(&session, 3, RECV_TIMEOUT);
    for id in 1..=3 {
        assert_tool_success(by_id(&batch1, id));
    }

    for (id, query) in [(4, "redhammer"), (5, "blueanvil"), (6, "greenchisel")] {
        session.send(&tool_call(
            id,
            "keyword_search",
            json!({"query": query, "limit": 4}),
        ));
    }
    let batch2 = collect(&session, 3, RECV_TIMEOUT);
    for (id, query) in [(4, "redhammer"), (5, "blueanvil"), (6, "greenchisel")] {
        let response = by_id(&batch2, id);
        assert_tool_success(response);
        assert!(tool_text(response).contains(query), "{response:#}");
    }

    session.send(&ping(7));
    session.send(&tools_list(8));
    session.send(&tool_call(9, "code_read", json!({"ids": ["c.rs#L1-L2"]})));
    let batch3 = collect(&session, 3, RECV_TIMEOUT);
    assert_ping_ok(by_id(&batch3, 7), 7);
    assert_tools_list_ok(by_id(&batch3, 8), 8);
    let read = by_id(&batch3, 9);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "c.rs#L1-L2");

    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// C9b: initialize -> calls -> more calls, including an error mid-stream:
/// every phase is answered and the process exits clean.
#[test]
fn session_survives_initialize_calls_more_calls() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tools_list(1));
    session.send(&tool_call(2, "index_status", json!({})));
    let phase1 = collect(&session, 2, RECV_TIMEOUT);
    assert_tools_list_ok(by_id(&phase1, 1), 1);
    assert_tool_success(by_id(&phase1, 2));

    session.send(&tool_call(
        3,
        "keyword_search",
        json!({"query": "redhammer", "limit": 4}),
    ));
    session.send(&tool_call(4, "code_read", json!({"ids": ["b.rs#L1-L1"]})));
    let phase2 = collect(&session, 2, RECV_TIMEOUT);
    let search = by_id(&phase2, 3);
    assert_tool_success(search);
    assert!(tool_text(search).contains("redhammer"), "{search:#}");
    let read = by_id(&phase2, 4);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "b.rs#L1-L1");

    session.send(&tool_call(5, "no_such_tool", json!({})));
    session.send(&ping(6));
    session.send(&tool_call(
        7,
        "code_search",
        json!({"query": "greenchisel", "limit": 4}),
    ));
    let phase3 = collect(&session, 3, RECV_TIMEOUT);
    assert_tool_error_shape(by_id(&phase3, 5));
    assert_ping_ok(by_id(&phase3, 6), 6);
    let late = by_id(&phase3, 7);
    assert_tool_success(late);
    assert!(tool_text(late).contains("greenchisel"), "{late:#}");

    session.close_stdin();
    assert!(session.wait_clean().success());
}
