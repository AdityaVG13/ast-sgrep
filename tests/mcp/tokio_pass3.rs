//! K3 async-metamorphic tests for ast-sgrep-mcp.
//!
//! Pins RELATIONS over async execution through the real stdio binary: the same
//! logical work scheduled differently (sequential vs pipelined), on a fresh
//! server, repeated in-session, under soak, or with reader-path calls
//! interleaved must yield equivalent responses.
//!
//! Relations pinned (discriminants are bytes, shapes, counts, id echo, and
//! orderings -- never message text, never durations):
//!
//! * M1: sequential-vs-pipelined equivalence (tools): the same tool-call list
//!   sent one-at-a-time vs all-at-once yields per-call byte-identical bodies.
//! * M2: sequential-vs-pipelined equivalence (mixed methods): ping/list/call
//!   scripts likewise match per id.
//! * M3: sequential-vs-pipelined equivalence with errors: per-id success/error
//!   shapes and byte-identical bodies.
//! * M4: session-restart equivalence: the same transcript on a fresh server
//!   over same-content roots yields byte-identical responses.
//! * M5: repetition determinism (intra-session): the same script twice in one
//!   session yields id-normalized byte-identical transcripts.
//! * M6: repetition determinism (across restarts): the same pipelined batch on
//!   two fresh servers yields per-id byte-identical responses.
//! * M7: soak: 60 sequential mixed calls are all correct, with a clean exit.
//! * M8: soak stability: repeated identical searches stay byte-identical
//!   (no drift, no degradation).
//! * M9: ping interleaved anywhere never perturbs tool results.
//! * M10: tools/list interleaved anywhere never perturbs tool results.
//!
//! Deliberate non-pins (inherited from K1 C1): pipelined wire ARRIVAL order is
//! timing-dependent by design (rmcp dispatches each request on its own task),
//! so cross-schedule comparisons match by id, never by arrival position.
//! Likewise `index_status` bodies are compared on stable count fields only:
//! the wire struct carries absolute `root`/`index_path` plus cache counters
//! and a writer epoch, which legitimately vary across roots and restarts.
//!
//! Snippet elision (`elide_seen_snippets` in `ast-sgrep-mcp/src/lib.rs`) is
//! per-process session state keyed on hit id + content hash: the first
//! occurrence of a snippet in a process goes out in full, repeats go out as
//! the `~` marker with a `ze` counter. Pipelined attribution of the full
//! copies across overlapping queries is therefore timing-dependent by design,
//! so search bodies are compared ELISION-NORMALIZED (snippet payloads
//! redacted, `ze` dropped), pinning hit identity/order/counts/files. The
//! elision itself is pinned as a conservation relation: each process delivers
//! every unique snippet full exactly once, so the multiset of full snippets
//! and the total elided count are schedule-independent.
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
            "clientInfo": {"name": "asgrep-mcp-k3", "version": "0"}
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

    /// Handshake, returning the raw initialize response for restart-equivalence
    /// comparison.
    fn handshake(&mut self) -> Value {
        self.send(&init_payload());
        let init = self.recv();
        assert_eq!(init["id"], "__init", "{init:#}");
        self.send(&initialized_notif());
        init
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

/// Stable count fields of an `index_status` body: everything else (absolute
/// root/index paths, cache counters, writer epoch) legitimately varies across
/// roots and restarts.
fn status_counts(body: &Value) -> (u64, u64, u64, u64, u64, u64) {
    (
        body["file_count"].as_u64().expect("file_count"),
        body["line_count"].as_u64().expect("line_count"),
        body["symbol_count"].as_u64().expect("symbol_count"),
        body["caller_count"].as_u64().expect("caller_count"),
        body["import_count"].as_u64().expect("import_count"),
        body["semantic_chunk_count"]
            .as_u64()
            .expect("semantic_chunk_count"),
    )
}

fn assert_status_counts_eq(a: &Value, b: &Value) {
    assert_tool_success(a);
    assert_tool_success(b);
    assert_eq!(
        status_counts(&tool_body(a)),
        status_counts(&tool_body(b)),
        "index_status counts diverged:\n{a:#}\n{b:#}"
    );
}

/// Three-symbol indexed tree: one token-distinct symbol per file (no shared
/// word-pieces, since keyword search ORs query tokens) so per-query
/// attribution is exact.
fn write_small_files(dir: &Path) {
    std::fs::write(dir.join("a.rs"), "fn redhammer() {}\n").unwrap();
    std::fs::write(dir.join("b.rs"), "fn blueanvil() {}\n").unwrap();
    std::fs::write(
        dir.join("c.rs"),
        "fn greenchisel() {}\nfn greenchisel_helper() {}\n",
    )
    .unwrap();
}

fn index_tree(dir: &Path) {
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: dir.to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();
}

fn small_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    write_small_files(temp.path());
    index_tree(temp.path());
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

/// Run `script` strictly sequentially (send one, read one) and return the
/// responses in id order.
fn run_sequential(session: &mut LiveSession, script: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(script.len());
    for payload in script {
        session.send(payload);
        out.push(session.recv());
    }
    out
}

/// Run `script` fully pipelined (send all, then read all). Returns arrival
/// order; match by id, never by position.
fn run_pipelined(session: &mut LiveSession, script: &[Value]) -> Vec<Value> {
    for payload in script {
        session.send(payload);
    }
    collect(session, script.len(), RECV_TIMEOUT)
}

/// Canonical bytes of a response with the same id on both sides.
fn canon(response: &Value) -> String {
    serde_json::to_string(response).expect("canonical response")
}

/// Canonical bytes with the id normalized away, for comparing repeated
/// scripts that reused payload shapes under different ids.
fn canon_no_id(response: &Value) -> String {
    let mut clone = response.clone();
    if let Some(obj) = clone.as_object_mut() {
        obj.insert("id".to_string(), Value::from(0));
    }
    serde_json::to_string(&clone).expect("canonical response")
}

/// Protocol marker the server substitutes for an already-sent snippet
/// (`ELIDED_SNIPPET` in `ast-sgrep-mcp/src/lib.rs`).
const ELIDED: &str = "~";

/// Elision-normalized tool body: hit snippet payloads (row index 4) redacted
/// and the volatile `ze` elision counter dropped. Non-search bodies (code
/// read nodes, status counts) have neither and pass through unchanged.
fn norm_search_body(body: &Value) -> Value {
    let mut clone = body.clone();
    if let Some(obj) = clone.as_object_mut() {
        obj.remove("ze");
        if let Some(hits) = obj.get_mut("h").and_then(Value::as_array_mut) {
            for hit in hits.iter_mut() {
                if let Some(row) = hit.as_array_mut() {
                    if row.len() > 4 {
                        row[4] = Value::String(String::new());
                    }
                }
            }
        }
    }
    clone
}

/// Snippet payloads of a search body in hit order (may include [`ELIDED`]).
fn hit_snippets(body: &Value) -> Vec<String> {
    body.get("h")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .filter_map(|hit| {
                    hit.as_array()
                        .and_then(|row| row.get(4).and_then(Value::as_str))
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Sorted multiset of full (non-elided, non-empty) `(hit id, snippet)` pairs
/// across search responses: the content-conservation discriminant.
fn full_snippets(responses: &[&Value]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for response in responses {
        if response["result"]["isError"] == true {
            continue;
        }
        let body = tool_body(response);
        let Some(hits) = body.get("h").and_then(Value::as_array) else {
            continue;
        };
        for hit in hits {
            let Some(row) = hit.as_array() else { continue };
            let id = row
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let snippet = row
                .get(4)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !snippet.is_empty() && snippet != ELIDED {
                out.push((id, snippet));
            }
        }
    }
    out.sort();
    out
}

/// Sum of `ze` elision counters across search responses.
fn total_elided(responses: &[&Value]) -> u64 {
    responses
        .iter()
        .map(|response| {
            if response["result"]["isError"] == true {
                return 0;
            }
            tool_body(response)
                .get("ze")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .sum()
}

/// The shared tool-call script for schedule-equivalence tests. Ids 1..=8.
fn tool_script() -> Vec<Value> {
    vec![
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
    ]
}

/// Assert two per-id-matched tool responses are equivalent:
/// elision-normalized byte-identical bodies, except `index_status` which
/// compares on stable count fields.
fn assert_tool_responses_equivalent(a: &Value, b: &Value, id: u32) {
    assert_eq!(a["id"], id, "{a:#}");
    assert_eq!(b["id"], id, "{b:#}");
    // Success/error bit must agree (shape), without reading message text.
    assert_eq!(
        a["result"]["isError"], b["result"]["isError"],
        "id {id} success bit diverged:\n{a:#}\n{b:#}"
    );
    if a["result"]["isError"] == true {
        assert_tool_error_shape(a);
        assert_tool_error_shape(b);
        assert_eq!(
            tool_text(a),
            tool_text(b),
            "id {id} error bytes diverged"
        );
        return;
    }
    assert_tool_success(a);
    assert_tool_success(b);
    // index_status (ids 2 and 6 in tool_script) carries root paths and
    // counters: counts only. Everything else is byte-identical.
    if id == 2 || id == 6 {
        assert_status_counts_eq(a, b);
    } else {
        assert_eq!(
            norm_search_body(&tool_body(a)),
            norm_search_body(&tool_body(b)),
            "id {id} normalized tool bytes diverged:\n{a:#}\n{b:#}"
        );
    }
}

/// Assert snippet-content conservation across two schedules of the same
/// search set: the same full snippets delivered exactly once per process
/// (multiset-equal), with equal total elided counts.
fn assert_snippet_conservation(a: &[&Value], b: &[&Value]) {
    let full_a = full_snippets(a);
    let full_b = full_snippets(b);
    assert!(
        !full_a.is_empty(),
        "no full snippets delivered; conservation check vacuous"
    );
    assert_eq!(
        full_a, full_b,
        "delivered snippet content differs by schedule"
    );
    assert_eq!(
        total_elided(a),
        total_elided(b),
        "total elided counts differ by schedule"
    );
}

/// M1: the same tool-call list sent one-at-a-time vs all-at-once yields
/// per-call equivalent responses: elision-normalized identical bodies
/// (count-fields for index_status), matched by id, plus snippet-content
/// conservation across the two schedules.
#[test]
fn sequential_vs_pipelined_tools_equivalent() {
    let temp = small_tree();
    let script = tool_script();

    let mut sequential_session = LiveSession::spawn(Some(temp.path()));
    sequential_session.handshake();
    let sequential = run_sequential(&mut sequential_session, &script);
    sequential_session.close_stdin();
    assert!(sequential_session.wait_clean().success());

    // Sequential responses arrive in id order (ordering pinned).
    assert_eq!(sequential.len(), script.len());
    for (position, response) in sequential.iter().enumerate() {
        assert_eq!(
            response["id"],
            (position as u32) + 1,
            "sequential run out of order at position {position}: {response:#}"
        );
    }

    let mut pipelined_session = LiveSession::spawn(Some(temp.path()));
    pipelined_session.handshake();
    let pipelined = run_pipelined(&mut pipelined_session, &script);
    pipelined_session.close_stdin();
    assert!(pipelined_session.wait_clean().success());
    assert_eq!(pipelined.len(), script.len());

    for id in 1..=script.len() as u32 {
        assert_tool_responses_equivalent(by_id(&sequential, id), by_id(&pipelined, id), id);
    }

    // The h-carrying searches in tool_script (ids 1, 5, 7, 8): same content
    // delivered exactly once per process under either schedule.
    let searches = [1u32, 5, 7, 8];
    let refs_seq: Vec<&Value> = searches.iter().map(|id| by_id(&sequential, *id)).collect();
    let refs_pipe: Vec<&Value> = searches.iter().map(|id| by_id(&pipelined, *id)).collect();
    assert_snippet_conservation(&refs_seq, &refs_pipe);
}

/// M2: a mixed ping/list/call script sent sequentially vs pipelined yields
/// per-id identical responses (ping result objects and tool bytes compared
/// byte-for-byte; list names; status counts).
#[test]
fn sequential_vs_pipelined_mixed_methods_equivalent() {
    let temp = small_tree();
    let script = vec![
        ping(1),
        tools_list(2),
        tool_call(3, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        ping(4),
        tool_call(5, "index_status", json!({})),
        tools_list(6),
        tool_call(7, "code_read", json!({"ids": ["c.rs#L1-L2"]})),
        ping(8),
    ];

    let mut sequential_session = LiveSession::spawn(Some(temp.path()));
    sequential_session.handshake();
    let sequential = run_sequential(&mut sequential_session, &script);
    sequential_session.close_stdin();
    assert!(sequential_session.wait_clean().success());

    let mut pipelined_session = LiveSession::spawn(Some(temp.path()));
    pipelined_session.handshake();
    let pipelined = run_pipelined(&mut pipelined_session, &script);
    pipelined_session.close_stdin();
    assert!(pipelined_session.wait_clean().success());

    assert_eq!(sequential.len(), script.len());
    assert_eq!(pipelined.len(), script.len());
    for id in [1, 4, 8] {
        let a = by_id(&sequential, id);
        let b = by_id(&pipelined, id);
        assert_ping_ok(a, id);
        assert_ping_ok(b, id);
        assert_eq!(canon(a), canon(b), "ping id {id} diverged");
    }
    for id in [2, 6] {
        let a = by_id(&sequential, id);
        let b = by_id(&pipelined, id);
        assert_tools_list_ok(a, id);
        assert_tools_list_ok(b, id);
        assert_eq!(canon(a), canon(b), "tools/list id {id} diverged");
    }
    for (id, query) in [(3, "redhammer")] {
        let a = by_id(&sequential, id);
        let b = by_id(&pipelined, id);
        assert_tool_success(a);
        assert_tool_success(b);
        assert_eq!(tool_text(a), tool_text(b), "id {id} diverged");
        assert!(tool_text(a).contains(query), "{a:#}");
    }
    assert_status_counts_eq(by_id(&sequential, 5), by_id(&pipelined, 5));
    let read_a = by_id(&sequential, 7);
    let read_b = by_id(&pipelined, 7);
    assert_tool_success(read_a);
    assert_tool_success(read_b);
    assert_eq!(tool_text(read_a), tool_text(read_b), "{read_a:#} vs {read_b:#}");
    assert_eq!(tool_body(read_a)["nodes"][0]["id"], "c.rs#L1-L2");
}

/// M3: schedule equivalence holds with errors in the batch: per-id
/// success/error shapes agree and bodies are byte-identical, sequentially or
/// pipelined.
#[test]
fn sequential_vs_pipelined_errors_equivalent() {
    let temp = small_tree();
    let script = vec![
        tool_call(1, "no_such_tool", json!({})),
        tool_call(2, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        tool_call(3, "keyword_search", json!({"query": "x", "limit": 0})),
        tool_call(4, "index_status", json!({})),
        tool_call(5, "code_read", json!({"ids": ["zzz-missing.rs#L1-L1"]})),
        tool_call(6, "keyword_search", json!({"query": "blueanvil", "limit": 4})),
    ];

    let mut sequential_session = LiveSession::spawn(Some(temp.path()));
    sequential_session.handshake();
    let sequential = run_sequential(&mut sequential_session, &script);
    sequential_session.close_stdin();
    assert!(sequential_session.wait_clean().success());

    let mut pipelined_session = LiveSession::spawn(Some(temp.path()));
    pipelined_session.handshake();
    let pipelined = run_pipelined(&mut pipelined_session, &script);
    pipelined_session.close_stdin();
    assert!(pipelined_session.wait_clean().success());

    assert_eq!(sequential.len(), script.len());
    assert_eq!(pipelined.len(), script.len());

    // At least one error and one success on each side (counts, not text).
    for responses in [&sequential, &pipelined] {
        let errors = responses
            .iter()
            .filter(|r| r["result"]["isError"] == true)
            .count();
        let successes = responses
            .iter()
            .filter(|r| r["result"]["isError"] == false)
            .count();
        assert!(errors >= 2, "expected >=2 errors: {responses:#?}");
        assert!(successes >= 2, "expected >=2 successes: {responses:#?}");
    }

    for id in 1..=script.len() as u32 {
        let a = by_id(&sequential, id);
        let b = by_id(&pipelined, id);
        assert_eq!(a["id"], id);
        assert_eq!(b["id"], id);
        assert_eq!(
            a["result"]["isError"], b["result"]["isError"],
            "id {id} success bit diverged:\n{a:#}\n{b:#}"
        );
        if id == 4 {
            assert_status_counts_eq(a, b);
        } else if a["result"]["isError"] == true {
            assert_tool_error_shape(a);
            assert_tool_error_shape(b);
            assert_eq!(tool_text(a), tool_text(b), "id {id} error bytes diverged");
        } else {
            assert_tool_success(a);
            assert_tool_success(b);
            assert_eq!(tool_text(a), tool_text(b), "id {id} bytes diverged");
        }
    }
}

/// M4: session-restart equivalence: the same transcript on a fresh server over
/// a same-content root yields byte-identical responses (count-fields for
/// index_status), including a byte-identical initialize result.
#[test]
fn fresh_servers_same_transcript_byte_identical() {
    let first = small_tree();
    let second = tempfile::tempdir().unwrap();
    write_small_files(second.path());
    index_tree(second.path());

    let script = vec![
        ping(1),
        tools_list(2),
        tool_call(3, "keyword_search", json!({"query": "greenchisel", "limit": 8})),
        tool_call(4, "code_search", json!({"query": "blueanvil", "limit": 4})),
        tool_call(5, "code_read", json!({"ids": ["b.rs#L1-L1"]})),
        tool_call(6, "index_status", json!({})),
        tool_call(
            7,
            "ast_search",
            json!({"query": "fn $NAME() { $$$BODY }", "limit": 8}),
        ),
    ];

    let mut session_a = LiveSession::spawn(Some(first.path()));
    let init_a = session_a.handshake();
    let transcript_a = run_sequential(&mut session_a, &script);
    session_a.close_stdin();
    assert!(session_a.wait_clean().success());

    let mut session_b = LiveSession::spawn(Some(second.path()));
    let init_b = session_b.handshake();
    let transcript_b = run_sequential(&mut session_b, &script);
    session_b.close_stdin();
    assert!(session_b.wait_clean().success());

    assert_eq!(canon(&init_a), canon(&init_b), "initialize diverged");
    assert_eq!(transcript_a.len(), script.len());
    assert_eq!(transcript_b.len(), script.len());
    for id in 1..=5 {
        assert_eq!(
            canon(&transcript_a[(id - 1) as usize]),
            canon(&transcript_b[(id - 1) as usize]),
            "id {id} diverged across fresh servers"
        );
    }
    assert_status_counts_eq(&transcript_a[5], &transcript_b[5]);
    assert_eq!(
        canon(&transcript_a[6]),
        canon(&transcript_b[6]),
        "ast_search diverged across fresh servers"
    );
    // Sanity: the compared transcripts are non-vacuous (hits present).
    assert!(
        tool_body(&transcript_a[2])["h"]
            .as_array()
            .is_some_and(|h| !h.is_empty()),
        "{:#}",
        transcript_a[2]
    );
}

/// M5: repetition determinism within one session: the same script run twice
/// back-to-back yields id-normalized identical transcripts (exact for
/// ping/list, elision-normalized for searches, count-fields for
/// index_status); the repeat run's searches are fully elided, proving the
/// repetition was recognized rather than recomputed differently.
#[test]
fn same_script_twice_in_one_session_identical() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let first_run = vec![
        ping(1),
        tools_list(2),
        tool_call(3, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        tool_call(4, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
        tool_call(5, "code_search", json!({"query": "greenchisel", "limit": 4})),
        tool_call(6, "index_status", json!({})),
    ];
    // Same payloads under fresh ids: id-normalized comparison below.
    let second_run = vec![
        ping(11),
        tools_list(12),
        tool_call(13, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        tool_call(14, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
        tool_call(15, "code_search", json!({"query": "greenchisel", "limit": 4})),
        tool_call(16, "index_status", json!({})),
    ];

    let first = run_sequential(&mut session, &first_run);
    let second = run_sequential(&mut session, &second_run);
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_eq!(first.len(), first_run.len());
    assert_eq!(second.len(), second_run.len());
    for (a, b) in first.iter().zip(second.iter()).take(2) {
        assert_eq!(
            canon_no_id(a),
            canon_no_id(b),
            "repeated ping/list diverged:\n{a:#}\n{b:#}"
        );
    }
    for (a, b) in first.iter().zip(second.iter()).skip(2).take(3) {
        assert_eq!(
            a["result"]["isError"], b["result"]["isError"],
            "repeated tool success bit diverged:\n{a:#}\n{b:#}"
        );
        assert_tool_success(a);
        assert_tool_success(b);
        assert_eq!(
            norm_search_body(&tool_body(a)),
            norm_search_body(&tool_body(b)),
            "repeated tool normalized bytes diverged:\n{a:#}\n{b:#}"
        );
    }
    assert_status_counts_eq(&first[5], &second[5]);

    // Elision shape: run 1 delivered real snippets (non-vacuous baseline),
    // run 2's searches are fully elided (same hits, no recompute drift).
    for index in [2usize, 4] {
        let first_snippets = hit_snippets(&tool_body(&first[index]));
        assert!(
            !first_snippets.is_empty()
                && first_snippets.iter().any(|s| s != ELIDED && !s.is_empty()),
            "run 1 delivered no full snippets: {:#}",
            first[index]
        );
        let second_snippets = hit_snippets(&tool_body(&second[index]));
        assert_eq!(
            second_snippets.len(),
            first_snippets.len(),
            "repeat changed hit count: {:#}",
            second[index]
        );
        assert!(
            !second_snippets.is_empty()
                && second_snippets.iter().all(|s| s == ELIDED),
            "repeat run not fully elided: {:#}",
            second[index]
        );
    }
}

/// M6: repetition determinism across restarts under pipelining: the same batch
/// pipelined on two fresh servers (same root, sequential sessions) yields
/// per-id equivalent responses (elision-normalized) plus snippet-content
/// conservation.
#[test]
fn same_pipelined_batch_twice_across_restart_identical() {
    let temp = small_tree();
    let script = tool_script();

    let mut session_a = LiveSession::spawn(Some(temp.path()));
    session_a.handshake();
    let run_a = run_pipelined(&mut session_a, &script);
    session_a.close_stdin();
    assert!(session_a.wait_clean().success());

    let mut session_b = LiveSession::spawn(Some(temp.path()));
    session_b.handshake();
    let run_b = run_pipelined(&mut session_b, &script);
    session_b.close_stdin();
    assert!(session_b.wait_clean().success());

    assert_eq!(run_a.len(), script.len());
    assert_eq!(run_b.len(), script.len());
    for id in 1..=script.len() as u32 {
        assert_tool_responses_equivalent(by_id(&run_a, id), by_id(&run_b, id), id);
    }

    let searches = [1u32, 5, 7, 8];
    let refs_a: Vec<&Value> = searches.iter().map(|id| by_id(&run_a, *id)).collect();
    let refs_b: Vec<&Value> = searches.iter().map(|id| by_id(&run_b, *id)).collect();
    assert_snippet_conservation(&refs_a, &refs_b);
}

/// M7: soak: 60 sequential mixed calls are all answered correctly (per-kind
/// shape/hit checks), ids echo exactly, and the session exits clean.
#[test]
fn soak_sixty_mixed_calls_all_correct() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    const N: u32 = 60;
    let queries = ["redhammer", "blueanvil", "greenchisel"];
    let mut successes = 0u32;
    for step in 0..N {
        let id = step + 1;
        match step % 6 {
            0 => {
                let query = queries[(step / 6) as usize % queries.len()];
                session.send(&tool_call(
                    id,
                    "keyword_search",
                    json!({"query": query, "limit": 4}),
                ));
                let response = session.recv();
                assert_eq!(response["id"], id, "{response:#}");
                assert_tool_success(&response);
                assert!(
                    tool_body(&response)["h"]
                        .as_array()
                        .is_some_and(|h| !h.is_empty()),
                    "{response:#}"
                );
                assert!(tool_text(&response).contains(query), "{response:#}");
                successes += 1;
            }
            1 => {
                session.send(&tool_call(id, "index_status", json!({})));
                let response = session.recv();
                assert_eq!(response["id"], id, "{response:#}");
                assert_tool_success(&response);
                let counts = status_counts(&tool_body(&response));
                assert_eq!(counts.0, 3, "{response:#}");
                assert!(counts.2 >= 3, "{response:#}");
                successes += 1;
            }
            2 => {
                let query = queries[(step / 6) as usize % queries.len()];
                session.send(&tool_call(
                    id,
                    "code_search",
                    json!({"query": query, "limit": 4}),
                ));
                let response = session.recv();
                assert_eq!(response["id"], id, "{response:#}");
                assert_tool_success(&response);
                assert!(
                    tool_body(&response)["h"]
                        .as_array()
                        .is_some_and(|h| !h.is_empty()),
                    "{response:#}"
                );
                successes += 1;
            }
            3 => {
                session.send(&ping(id));
                let response = session.recv();
                assert_ping_ok(&response, id);
                successes += 1;
            }
            4 => {
                session.send(&tools_list(id));
                let response = session.recv();
                assert_tools_list_ok(&response, id);
                successes += 1;
            }
            _ => {
                let file = ["a.rs", "b.rs", "c.rs"][(step / 6) as usize % 3];
                let want = format!("{file}#L1-L1");
                session.send(&tool_call(id, "code_read", json!({"ids": [want]})));
                let response = session.recv();
                assert_eq!(response["id"], id, "{response:#}");
                assert_tool_success(&response);
                assert_eq!(
                    tool_body(&response)["nodes"][0]["id"],
                    format!("{file}#L1-L1"),
                    "{response:#}"
                );
                successes += 1;
            }
        }
    }
    assert_eq!(successes, N, "soak answered {successes}/{N} correctly");
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// M8: soak stability: 40 identical sequential searches carry the same hits
/// throughout (elision-normalized equal to the first: no in-session drift),
/// and every repeat after the first is byte-identical to the first repeat
/// (exact post-warmup steady state), with periodic status checks succeeding.
#[test]
fn soak_repeated_search_byte_stable() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&tool_call(
        1,
        "keyword_search",
        json!({"query": "greenchisel", "limit": 8}),
    ));
    let first = session.recv();
    assert_eq!(first["id"], 1, "{first:#}");
    assert_tool_success(&first);
    let baseline_norm = norm_search_body(&tool_body(&first));
    let baseline_snippets = hit_snippets(&tool_body(&first));
    assert!(
        !baseline_snippets.is_empty()
            && baseline_snippets.iter().any(|s| s != ELIDED && !s.is_empty()),
        "cold search delivered no full snippets: {first:#}"
    );

    const REPEATS: u32 = 39;
    let mut steady: Option<String> = None;
    for i in 0..REPEATS {
        let id = 2 + i * 2;
        session.send(&tool_call(
            id,
            "keyword_search",
            json!({"query": "greenchisel", "limit": 8}),
        ));
        let response = session.recv();
        assert_eq!(response["id"], id, "{response:#}");
        assert_tool_success(&response);
        assert_eq!(
            norm_search_body(&tool_body(&response)),
            baseline_norm,
            "repeat {i} hits drifted from baseline"
        );
        match &steady {
            None => steady = Some(tool_text(&response).to_string()),
            Some(bytes) => assert_eq!(
                tool_text(&response),
                bytes,
                "repeat {i} drifted from steady state"
            ),
        }
        let status_id = id + 1;
        session.send(&tool_call(status_id, "index_status", json!({})));
        let status = session.recv();
        assert_eq!(status["id"], status_id, "{status:#}");
        assert_tool_success(&status);
    }
    session.close_stdin();
    assert!(session.wait_clean().success());

    // The steady state is the fully-elided repeat (shape, not text).
    let steady_bytes = steady.expect("at least one repeat ran");
    let steady_body: Value = serde_json::from_str(&steady_bytes).expect("steady body JSON");
    let steady_snippets = hit_snippets(&steady_body);
    assert_eq!(
        steady_snippets.len(),
        baseline_snippets.len(),
        "steady state changed hit count"
    );
    assert!(
        !steady_snippets.is_empty() && steady_snippets.iter().all(|s| s == ELIDED),
        "steady state not fully elided: {steady_body:#}"
    );
}

/// M9: ping interleaved before and after every tool call never perturbs tool
/// results: tool responses match the uninterleaved baseline byte-for-byte.
#[test]
fn ping_interleaved_never_perturbs_tools() {
    let temp = small_tree();
    let tools = tool_script();

    let mut baseline_session = LiveSession::spawn(Some(temp.path()));
    baseline_session.handshake();
    let baseline = run_sequential(&mut baseline_session, &tools);
    baseline_session.close_stdin();
    assert!(baseline_session.wait_clean().success());

    let mut probe_session = LiveSession::spawn(Some(temp.path()));
    probe_session.handshake();
    let mut ping_count = 0u32;
    for payload in &tools {
        let id = payload["id"].as_u64().unwrap() as u32;
        let before = 1000 + id * 2;
        let after = 1000 + id * 2 + 1;
        probe_session.send(&ping(before));
        assert_ping_ok(&probe_session.recv(), before);
        ping_count += 1;
        probe_session.send(payload);
        let tool_response = probe_session.recv();
        assert_eq!(tool_response["id"], id, "{tool_response:#}");
        assert_tool_responses_equivalent(by_id(&baseline, id), &tool_response, id);
        probe_session.send(&ping(after));
        assert_ping_ok(&probe_session.recv(), after);
        ping_count += 1;
    }
    assert_eq!(ping_count, tools.len() as u32 * 2);
    probe_session.close_stdin();
    assert!(probe_session.wait_clean().success());
}

/// M10: tools/list interleaved between every tool call never perturbs tool
/// results: tool responses match the uninterleaved baseline byte-for-byte and
/// every list carries the full tool set.
#[test]
fn tools_list_interleaved_never_perturbs_tools() {
    let temp = small_tree();
    let tools = tool_script();

    let mut baseline_session = LiveSession::spawn(Some(temp.path()));
    baseline_session.handshake();
    let baseline = run_sequential(&mut baseline_session, &tools);
    baseline_session.close_stdin();
    assert!(baseline_session.wait_clean().success());

    let mut probe_session = LiveSession::spawn(Some(temp.path()));
    probe_session.handshake();
    let mut list_count = 0u32;
    for (index, payload) in tools.iter().enumerate() {
        let id = payload["id"].as_u64().unwrap() as u32;
        probe_session.send(payload);
        let tool_response = probe_session.recv();
        assert_eq!(tool_response["id"], id, "{tool_response:#}");
        assert_tool_responses_equivalent(by_id(&baseline, id), &tool_response, id);
        let list_id = 2000 + index as u32;
        probe_session.send(&tools_list(list_id));
        assert_tools_list_ok(&probe_session.recv(), list_id);
        list_count += 1;
    }
    assert_eq!(list_count, tools.len() as u32);
    probe_session.close_stdin();
    assert!(probe_session.wait_clean().success());
}
