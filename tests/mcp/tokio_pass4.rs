//! K4 tokio runtime drills for ast-sgrep-mcp: hostile-timing end-to-end
//! sessions through the real stdio binary.
//!
//! Where K1 pins concurrency contracts, K2 pins cancel/EOF contracts, and K3
//! pins schedule-equivalence relations, K4 runs whole sessions under hostile
//! client timing and asserts the server still answers correctly, stays silent
//! exactly when it must, and shuts down with the documented code.
//!
//! Drills pinned (discriminants are exit codes, shapes, counts, id echo,
//! byte (in)equality, and orderings -- never message text, never durations):
//!
//! * D1 (burst): 24 mixed calls fired at once are all answered, each correct
//!   per id against a sequential baseline, with the full id set echoed back.
//!   Arrival order is NOT pinned (K1 C1: rmcp dispatches each request on its
//!   own task); id-set completeness IS pinned.
//! * D2 (interleave): ping / tools/list / stray-cancel notifications woven
//!   between pipelined tool calls leave every tool result unperturbed
//!   (baseline-equivalent) and produce no extra output of their own.
//! * D3 (abort): stdin slammed shut mid-burst (no reads at all) ends the
//!   server with exit 0 within a bounded wait -- no hang. (Exit-0-on-EOF is
//!   the K2 K10 contract; D3 drills it under an unread burst, not a single
//!   slow call.)
//! * D4 (restart): SIGKILL mid-session, then a fresh server on the same root
//!   serves a byte-identical baseline transcript. The killed server reports
//!   non-success; both clean sessions exit 0.
//! * D5 (slow client): byte-trickle writes (multi-byte chunks at a paced
//!   cadence, newline last) still assemble into correct requests, one at a
//!   time and pipelined back-to-back in a single trickle stream.
//! * D6 (chaos): burst + interleave + cancel + SIGKILL chained in one
//!   session, then a fresh server on the same root serves the canonical
//!   transcript byte-identical to baseline: final state correct.
//! * D7 (burst errors): a burst mixing unknown tools, invalid arguments, and
//!   stray cancels yields exactly one response per request id -- error shapes
//!   for the bad ids, successes for the good ones, silence for the cancels.
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
            "clientInfo": {"name": "asgrep-mcp-k4", "version": "0"}
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
/// the test instead of hanging the suite. The reader thread drains stdout
/// into an unbounded channel, so unread bursts never wedge the server on a
/// full pipe.
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

    /// Slow-client write: the JSON line goes out in `chunk`-byte pieces with
    /// a paced sleep between pieces, newline last. The server must reassemble
    /// the framed request exactly.
    fn send_trickle(&mut self, payload: &Value, chunk: usize, pace: Duration) {
        let line = payload.to_string();
        let bytes = line.as_bytes();
        let stdin = self.stdin.as_mut().expect("stdin open");
        for piece in bytes.chunks(chunk.max(1)) {
            stdin.write_all(piece).unwrap();
            stdin.flush().unwrap();
            std::thread::sleep(pace);
        }
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    /// Several framed requests trickled back-to-back as one byte stream: the
    /// server must split frames on newlines and answer every id.
    fn send_trickle_stream(&mut self, payloads: &[Value], chunk: usize, pace: Duration) {
        let mut stream = String::new();
        for payload in payloads {
            stream.push_str(&payload.to_string());
            stream.push('\n');
        }
        let stdin = self.stdin.as_mut().expect("stdin open");
        for piece in stream.as_bytes().chunks(chunk.max(1)) {
            stdin.write_all(piece).unwrap();
            stdin.flush().unwrap();
            std::thread::sleep(pace);
        }
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

    /// SIGKILL-equivalent (`Child::kill`): hostile restart drill. Reaps the
    /// child and returns its status, which must be non-success.
    fn kill9(&mut self) -> ExitStatus {
        self.stdin.take();
        self.child.kill().expect("kill MCP");
        self.child.wait().expect("reap MCP")
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

/// Stable count fields of an `index_status` body (K3 M4: root paths, cache
/// counters, and writer epoch legitimately vary across roots and restarts).
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

/// Elision-normalized search body (K3: per-process snippet elision attributes
/// full copies timing-dependently under pipelining, so payloads are redacted
/// and the volatile `ze` counter dropped; hit identity/order/counts stay).
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

/// Three-symbol indexed tree: one token-distinct symbol per file so per-query
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

fn run_sequential(session: &mut LiveSession, script: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(script.len());
    for payload in script {
        session.send(payload);
        out.push(session.recv());
    }
    out
}

fn run_pipelined(session: &mut LiveSession, script: &[Value]) -> Vec<Value> {
    for payload in script {
        session.send(payload);
    }
    collect(session, script.len(), RECV_TIMEOUT)
}

fn canon(response: &Value) -> String {
    serde_json::to_string(response).expect("canonical response")
}

const QUERIES: [&str; 3] = ["redhammer", "blueanvil", "greenchisel"];
const READ_IDS: [&str; 3] = ["a.rs#L1-L1", "b.rs#L1-L1", "c.rs#L1-L2"];

/// 24 mixed requests (ids 1..=24): keyword/code searches cycling the three
/// symbols, index_status, code_read cycling the three files, ping, tools/list.
fn burst_script() -> Vec<Value> {
    let mut script = Vec::with_capacity(24);
    for (i, id) in (1..=24u32).enumerate() {
        let cycle = QUERIES[i % QUERIES.len()];
        let payload = match i % 6 {
            0 => tool_call(id, "keyword_search", json!({"query": cycle, "limit": 4})),
            1 => tool_call(id, "index_status", json!({})),
            2 => tool_call(id, "code_search", json!({"query": cycle, "limit": 4})),
            3 => ping(id),
            4 => tools_list(id),
            _ => tool_call(
                id,
                "code_read",
                json!({"ids": [READ_IDS[i % READ_IDS.len()]]}),
            ),
        };
        script.push(payload);
    }
    script
}

/// Canonical short transcript for restart/chaos drills (ids 1..=8).
fn canonical_script() -> Vec<Value> {
    vec![
        ping(1),
        tools_list(2),
        tool_call(3, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        tool_call(4, "index_status", json!({})),
        tool_call(5, "code_search", json!({"query": "blueanvil", "limit": 4})),
        tool_call(6, "code_read", json!({"ids": ["c.rs#L1-L2"]})),
        tool_call(7, "keyword_search", json!({"query": "greenchisel", "limit": 8})),
        ping(8),
    ]
}

/// Assert a probe response matches its sequential-baseline twin for the same
/// request: exact bytes for ping/list/reads, stable counts for index_status,
/// elision-normalized bodies for searches (plus non-empty hits on both).
fn assert_matches_baseline(request: &Value, baseline: &Value, probe: &Value) {
    let id = request["id"].as_u64().unwrap() as u32;
    assert_eq!(baseline["id"], id, "{baseline:#}");
    assert_eq!(probe["id"], id, "{probe:#}");
    match request["method"].as_str().unwrap() {
        "ping" => {
            assert_ping_ok(baseline, id);
            assert_ping_ok(probe, id);
            assert_eq!(canon(baseline), canon(probe), "ping id {id} diverged");
        }
        "tools/list" => {
            assert_tools_list_ok(baseline, id);
            assert_tools_list_ok(probe, id);
            assert_eq!(canon(baseline), canon(probe), "tools/list id {id} diverged");
        }
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap();
            match name {
                "index_status" => assert_status_counts_eq(baseline, probe),
                "code_read" => {
                    assert_tool_success(baseline);
                    assert_tool_success(probe);
                    assert_eq!(
                        tool_text(baseline),
                        tool_text(probe),
                        "code_read id {id} diverged"
                    );
                }
                _ => {
                    assert_tool_success(baseline);
                    assert_tool_success(probe);
                    for response in [baseline, probe] {
                        assert!(
                            tool_body(response)["h"]
                                .as_array()
                                .is_some_and(|h| !h.is_empty()),
                            "id {id} ({name}) has no hits: {response:#}"
                        );
                    }
                    assert_eq!(
                        norm_search_body(&tool_body(baseline)),
                        norm_search_body(&tool_body(probe)),
                        "id {id} ({name}) normalized bytes diverged"
                    );
                }
            }
        }
        other => panic!("unexpected method {other} in drill script"),
    }
}

/// Assert the id set of `responses` is exactly 1..=n (ordering by id, not by
/// arrival: arrival order is timing-dependent by design per K1 C1).
fn assert_id_set_complete(responses: &[Value], n: u32) {
    let mut ids: Vec<u32> = responses
        .iter()
        .map(|r| r["id"].as_u64().expect("numeric id") as u32)
        .collect();
    ids.sort_unstable();
    let expected: Vec<u32> = (1..=n).collect();
    assert_eq!(ids, expected, "burst id set incomplete or duplicated");
}

/// D1: 24 mixed calls fired at once are all answered, each correct per id
/// against a sequential baseline, with the full id set echoed and a clean exit.
#[test]
fn burst_24_mixed_calls_all_correct_id_ordered() {
    let temp = small_tree();
    let script = burst_script();

    let mut baseline_session = LiveSession::spawn(Some(temp.path()));
    baseline_session.handshake();
    let baseline = run_sequential(&mut baseline_session, &script);
    baseline_session.close_stdin();
    assert!(baseline_session.wait_clean().success());

    let mut burst_session = LiveSession::spawn(Some(temp.path()));
    burst_session.handshake();
    let burst = run_pipelined(&mut burst_session, &script);
    burst_session.close_stdin();
    assert!(burst_session.wait_clean().success());

    assert_eq!(burst.len(), script.len());
    assert_id_set_complete(&burst, script.len() as u32);
    for request in &script {
        let id = request["id"].as_u64().unwrap() as u32;
        assert_matches_baseline(request, by_id(&baseline, id), by_id(&burst, id));
    }
}

/// D2: ping / tools/list / stray-cancel notifications woven between pipelined
/// tool calls leave every tool result baseline-equivalent; the notifications
/// themselves add exactly the ping/list responses and nothing else. One
/// pre-cancel (cancel sent before its id is issued) proves ordering holds
/// inside the burst: that call still succeeds (K2 K7 precedent).
#[test]
fn interleave_ping_list_cancel_leaves_tools_unperturbed() {
    let temp = small_tree();
    let tools = vec![
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

    let mut baseline_session = LiveSession::spawn(Some(temp.path()));
    baseline_session.handshake();
    let baseline = run_sequential(&mut baseline_session, &tools);
    baseline_session.close_stdin();
    assert!(baseline_session.wait_clean().success());

    let mut probe = LiveSession::spawn(Some(temp.path()));
    probe.handshake();
    let mut ping_ids = Vec::new();
    let mut list_ids = Vec::new();
    for request in &tools {
        let id = request["id"].as_u64().unwrap() as u32;
        let ping_id = 100 + id;
        probe.send(&ping(ping_id));
        ping_ids.push(ping_id);
        if id == 5 {
            // Pre-cancel: arrives before id 5 is ever issued; the call must
            // still succeed (K2 K7).
            probe.send(&cancelled(5));
        }
        probe.send(request);
        let list_id = 200 + id;
        probe.send(&tools_list(list_id));
        list_ids.push(list_id);
        // Stray cancel for a never-issued id: must stay silent (K2 K6).
        probe.send(&cancelled(9000 + id));
    }
    // Exactly one response per request; the 9 stray cancels add nothing.
    let expected = tools.len() + ping_ids.len() + list_ids.len();
    let responses = collect(&probe, expected, RECV_TIMEOUT);
    probe.close_stdin();
    assert!(probe.wait_clean().success());

    assert_eq!(responses.len(), expected);
    for request in &tools {
        let id = request["id"].as_u64().unwrap() as u32;
        assert_matches_baseline(request, by_id(&baseline, id), by_id(&responses, id));
    }
    // The pre-cancelled id 5 is an explicit success, not just "equivalent".
    assert_tool_success(by_id(&responses, 5));
    for id in ping_ids {
        assert_ping_ok(by_id(&responses, id), id);
    }
    for id in list_ids {
        assert_tools_list_ok(by_id(&responses, id), id);
    }
}

/// D3: stdin slammed shut mid-burst with zero reads still ends the server with
/// exit 0 inside the bounded wait -- no hang, no failure code.
#[test]
fn abort_stdin_slammed_mid_burst_exits_ok() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    for request in burst_script() {
        session.send(&request);
    }
    // Slam: close stdin immediately without reading a single response.
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "abort mid-burst exited {status}");
}

/// D4: SIGKILL mid-session, then a fresh server on the same root serves the
/// canonical transcript byte-identical to baseline. The killed server reports
/// non-success; both clean sessions exit 0.
#[test]
fn restart_kill9_fresh_server_serves_identical_baseline() {
    let temp = small_tree();
    let script = canonical_script();

    let mut baseline_session = LiveSession::spawn(Some(temp.path()));
    baseline_session.handshake();
    let baseline = run_sequential(&mut baseline_session, &script);
    baseline_session.close_stdin();
    assert!(baseline_session.wait_clean().success());
    assert_eq!(baseline.len(), script.len());

    let mut victim = LiveSession::spawn(Some(temp.path()));
    victim.handshake();
    for request in script.iter().take(4) {
        victim.send(request);
        let response = victim.recv();
        assert_eq!(
            response["id"],
            request["id"],
            "pre-kill response mismatch: {response:#}"
        );
    }
    let kill_status = victim.kill9();
    assert!(
        !kill_status.success(),
        "SIGKILLed server reported success: {kill_status}"
    );

    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    let rerun = run_sequential(&mut fresh, &script);
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());

    // Sequential runs preserve position order; compare positionally.
    assert_eq!(rerun.len(), script.len());
    for (index, request) in script.iter().enumerate() {
        assert_matches_baseline(request, &baseline[index], &rerun[index]);
    }
}

/// D5: byte-trickle writes assemble into correct requests: a trickled
/// handshake plus one-at-a-time trickled calls, then three framed requests
/// trickled back-to-back as a single byte stream (newline framing), all
/// answered correctly with a clean exit.
#[test]
fn slow_client_byte_trickle_assembles_correct_requests() {
    const CHUNK: usize = 3;
    const PACE: Duration = Duration::from_millis(1);

    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));

    session.send_trickle(&init_payload(), CHUNK, PACE);
    let init = session.recv();
    assert_eq!(init["id"], "__init", "{init:#}");
    session.send_trickle(&initialized_notif(), CHUNK, PACE);

    // One at a time: each trickled request answered before the next starts.
    // First occurrences go out full, so query attribution is exact.
    session.send_trickle(
        &tool_call(1, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        CHUNK,
        PACE,
    );
    let first = session.recv();
    assert_eq!(first["id"], 1, "{first:#}");
    assert_tool_success(&first);
    assert!(
        tool_body(&first)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{first:#}"
    );
    assert!(tool_text(&first).contains("redhammer"), "{first:#}");

    session.send_trickle(&ping(2), CHUNK, PACE);
    assert_ping_ok(&session.recv(), 2);

    session.send_trickle(&tool_call(3, "code_read", json!({"ids": ["b.rs#L1-L1"]})), CHUNK, PACE);
    let read = session.recv();
    assert_eq!(read["id"], 3, "{read:#}");
    assert_tool_success(&read);
    assert_eq!(tool_body(&read)["nodes"][0]["id"], "b.rs#L1-L1");

    // Back-to-back: three frames in one trickle stream split on newlines.
    let stream = vec![
        tool_call(4, "keyword_search", json!({"query": "blueanvil", "limit": 4})),
        tools_list(5),
        tool_call(6, "code_search", json!({"query": "greenchisel", "limit": 4})),
    ];
    session.send_trickle_stream(&stream, CHUNK, PACE);
    let responses = collect(&session, stream.len(), RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_eq!(responses.len(), 3);
    let blue = by_id(&responses, 4);
    assert_tool_success(blue);
    assert!(
        tool_body(blue)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{blue:#}"
    );
    assert!(tool_text(blue).contains("blueanvil"), "{blue:#}");
    assert_tools_list_ok(by_id(&responses, 5), 5);
    let green = by_id(&responses, 6);
    assert_tool_success(green);
    assert!(
        tool_body(green)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{green:#}"
    );
}

/// D6: mixed chaos -- burst + interleave + cancel + SIGKILL chained in one
/// session -- then a fresh server on the same root serves the canonical
/// transcript byte-identical to baseline: final state correct.
#[test]
fn mixed_chaos_burst_interleave_cancel_restart_final_state_correct() {
    let temp = small_tree();
    let script = canonical_script();

    let mut baseline_session = LiveSession::spawn(Some(temp.path()));
    baseline_session.handshake();
    let baseline = run_sequential(&mut baseline_session, &script);
    baseline_session.close_stdin();
    assert!(baseline_session.wait_clean().success());

    // Chaos session: a 12-request burst with pings, lists, and stray cancels
    // woven in. Only the first three responses are read; a cancel for a
    // possibly-pending id goes out; then SIGKILL -- no timing assertions on
    // the chaos session itself, only that the kill lands.
    let mut chaos = LiveSession::spawn(Some(temp.path()));
    chaos.handshake();
    let burst = burst_script();
    let wave: Vec<Value> = burst.into_iter().take(12).collect();
    for request in &wave {
        let id = request["id"].as_u64().unwrap() as u32;
        chaos.send(&ping(500 + id));
        chaos.send(request);
        chaos.send(&tools_list(600 + id));
        chaos.send(&cancelled(9500 + id));
    }
    for _ in 0..3 {
        let response = chaos.recv();
        assert!(response.get("id").is_some(), "{response:#}");
    }
    chaos.send(&cancelled(12));
    let kill_status = chaos.kill9();
    assert!(
        !kill_status.success(),
        "chaos server kill reported success: {kill_status}"
    );

    // Fresh server on the same root: canonical transcript matches baseline.
    let mut fresh = LiveSession::spawn(Some(temp.path()));
    fresh.handshake();
    let rerun = run_sequential(&mut fresh, &script);
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());

    assert_eq!(rerun.len(), script.len());
    for (index, request) in script.iter().enumerate() {
        assert_matches_baseline(request, &baseline[index], &rerun[index]);
    }
}

/// D7: a burst mixing unknown tools, invalid arguments, and stray cancels
/// yields exactly one response per request id: error shapes for the bad ids,
/// successes for the good ones, silence for the cancels -- with the session
/// still usable afterwards and a clean exit.
#[test]
fn burst_errors_and_stray_cancels_exact_count() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let requests = vec![
        tool_call(1, "no_such_tool", json!({})),
        tool_call(2, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        tool_call(3, "keyword_search", json!({"query": "x", "limit": 0})),
        tool_call(4, "index_status", json!({})),
        tool_call(5, "code_read", json!({"ids": ["zzz-missing.rs#L1-L1"]})),
        tool_call(6, "keyword_search", json!({"query": "blueanvil", "limit": 4})),
        tool_call(7, "no_such_tool", json!({})),
        tool_call(8, "code_search", json!({"query": "greenchisel", "limit": 4})),
        ping(9),
        tools_list(10),
        tool_call(11, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
        tool_call(12, "keyword_search", json!({"query": "x", "limit": 0})),
    ];
    for request in &requests {
        session.send(request);
        // A stray cancel after every request: all must stay silent.
        let id = request["id"].as_u64().unwrap() as u32;
        session.send(&cancelled(8000 + id));
    }
    let responses = collect(&session, requests.len(), RECV_TIMEOUT);

    // Exact count: 12 requests in, 12 responses out, 12 stray cancels silent.
    assert_eq!(responses.len(), requests.len());
    assert_id_set_complete(&responses, requests.len() as u32);

    // Error ids carry tool-error shapes with byte-stable bodies per bad kind.
    for id in [1, 7] {
        assert_tool_error_shape(by_id(&responses, id));
    }
    assert_eq!(
        tool_text(by_id(&responses, 1)),
        tool_text(by_id(&responses, 7)),
        "unknown-tool error bytes diverged"
    );
    for id in [3, 12] {
        assert_tool_error_shape(by_id(&responses, id));
    }
    assert_eq!(
        tool_text(by_id(&responses, 3)),
        tool_text(by_id(&responses, 12)),
        "invalid-args error bytes diverged"
    );

    // Good ids succeed with their own correct results.
    let red = by_id(&responses, 2);
    assert_tool_success(red);
    assert!(
        tool_body(red)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{red:#}"
    );
    assert_tool_success(by_id(&responses, 4));
    let blue = by_id(&responses, 6);
    assert_tool_success(blue);
    assert!(
        tool_body(blue)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{blue:#}"
    );
    let green = by_id(&responses, 8);
    assert_tool_success(green);
    assert!(
        tool_body(green)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{green:#}"
    );
    assert_ping_ok(by_id(&responses, 9), 9);
    assert_tools_list_ok(by_id(&responses, 10), 10);
    let read = by_id(&responses, 11);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "a.rs#L1-L1");

    // Id 5 (missing file) is either a tool error or an empty success; pin
    // only that it answered with a well-formed tool shape, then prove the
    // session is still usable and exits clean.
    let missing = by_id(&responses, 5);
    assert_eq!(missing["id"], 5, "{missing:#}");
    assert!(missing.get("error").is_none(), "{missing:#}");
    assert!(missing["result"]["content"][0]["type"] == "text", "{missing:#}");

    session.send(&ping(13));
    assert_ping_ok(&session.recv(), 13);
    session.close_stdin();
    assert!(session.wait_clean().success());
}
