//! Tokio equivalence/chaos suite for ast-sgrep-mcp (consolidates K3 + K4).
//!
//! One test per intent; each test folds every catalog facet for that intent.
//! Sequential-vs-pipelined comparisons use the testkit one-shot drivers
//! (`rpc_session` preserves request order; `rpc_pipeline` returns arrival
//! order, matched by id). Long-lived drills use testkit `LiveSession`. Only
//! testkit gaps stay local: schedule comparators, elision normalization,
//! and [`RawSession`] -- a minimal raw driver covering
//! exactly the three gaps in testkit's transport (SIGKILL mid-session,
//! byte-trickle writes, initialize-response capture). Discriminants are exit
//! codes, shapes, counts, id echo, and byte (in)equality -- never message
//! text, never durations.

use ast_sgrep_testkit::{
    TESTKIT_CLIENT_NAME, assert_ping_ok, assert_tool_error_shape, assert_tool_success,
    assert_tools_list_ok, cancelled_notif, collect_responses, index_tree, init_payload,
    initialized_notif, mcp_bin, ping, response_by_id, small_tree, tool_body, tool_call, tool_text,
    tools_list, LiveSession,
};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const RECV_TIMEOUT: Duration = Duration::from_secs(15);
const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

/// Protocol marker the server substitutes for an already-sent snippet
/// (`ELIDED_SNIPPET` in `ast-sgrep-mcp/src/lib.rs`).
const ELIDED: &str = "~";

/// Minimal raw stdio driver filling exactly three testkit transport gaps:
/// SIGKILL mid-session, byte-trickle writes, and initialize-response capture.
/// Everything else (builders, extractors, asserts) is testkit.
struct RawSession {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
}

impl RawSession {
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
        RawSession {
            child,
            stdin: Some(stdin),
            lines: rx,
        }
    }

    /// Handshake, returning the raw initialize response for
    /// restart-equivalence comparison.
    fn handshake(&mut self) -> Value {
        self.send(&init_payload(TESTKIT_CLIENT_NAME));
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

    /// Slow-client write: the JSON line goes out in `chunk`-byte pieces with
    /// a paced sleep between pieces, newline last.
    fn send_trickle(&mut self, payload: &Value, chunk: usize, pace: Duration) {
        let line = payload.to_string();
        let stdin = self.stdin.as_mut().expect("stdin open");
        for piece in line.as_bytes().chunks(chunk.max(1)) {
            stdin.write_all(piece).unwrap();
            stdin.flush().unwrap();
            std::thread::sleep(pace);
        }
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    /// Several framed requests trickled back-to-back as one byte stream.
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

impl Drop for RawSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn write_small_files(dir: &Path) {
    std::fs::write(dir.join("a.rs"), "fn redhammer() {}\n").unwrap();
    std::fs::write(dir.join("b.rs"), "fn blueanvil() {}\n").unwrap();
    std::fs::write(
        dir.join("c.rs"),
        "fn greenchisel() {}\nfn greenchisel_helper() {}\n",
    )
    .unwrap();
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

/// Elision-normalized tool body: hit snippet payloads (row index 4) redacted
/// and the volatile `ze` elision counter dropped. Snippet elision is
/// per-process session state, so pipelined attribution of the full copies is
/// timing-dependent by design; hit identity/order/counts/files are pinned.
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

/// The shared tool-call script for schedule-equivalence tests. Ids 1..=8
/// (`index_status` at ids 2 and 6).
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

/// Assert two per-id-matched [`tool_script`] responses are equivalent:
/// elision-normalized byte-identical bodies, except `index_status` (ids 2, 6)
/// which compares on stable count fields.
fn assert_tool_responses_equivalent(a: &Value, b: &Value, id: u32) {
    assert_eq!(a["id"], id, "{a:#}");
    assert_eq!(b["id"], id, "{b:#}");
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
/// arrival: arrival order is timing-dependent by design).
fn assert_id_set_complete(responses: &[Value], n: u32) {
    let mut ids: Vec<u32> = responses
        .iter()
        .map(|r| r["id"].as_u64().expect("numeric id") as u32)
        .collect();
    ids.sort_unstable();
    let expected: Vec<u32> = (1..=n).collect();
    assert_eq!(ids, expected, "burst id set incomplete or duplicated");
}

/// INTENT: delivery order never changes results.
/// KILLS: order-dependent (schedule-sensitive) mutants.
/// Facets: tool scripts (K3.1) + mixed-method scripts (K3.2) + scripts with
/// errors (K3.3). Cross-schedule comparisons match by id, never by arrival
/// position.
#[test]
fn sequential_vs_pipelined_equivalence() {
    let temp = small_tree();

    // Facet 1 (K3.1): tool script sequential vs pipelined, per-id equivalent
    // plus snippet-content conservation across the two schedules.
    let script = tool_script();
    let sequential = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    assert_eq!(sequential.len(), script.len());
    for (position, response) in sequential.iter().enumerate() {
        assert_eq!(
            response["id"],
            (position as u32) + 1,
            "sequential run out of order at position {position}: {response:#}"
        );
    }
    let pipelined = ast_sgrep_testkit::rpc_pipeline(script.clone(), Some(temp.path()));
    assert_eq!(pipelined.len(), script.len());
    for id in 1..=script.len() as u32 {
        assert_tool_responses_equivalent(response_by_id(&sequential, id), response_by_id(&pipelined, id), id);
    }
    let searches = [1u32, 5, 7, 8];
    let refs_seq: Vec<&Value> = searches.iter().map(|id| response_by_id(&sequential, *id)).collect();
    let refs_pipe: Vec<&Value> = searches.iter().map(|id| response_by_id(&pipelined, *id)).collect();
    assert_snippet_conservation(&refs_seq, &refs_pipe);

    // Facet 2 (K3.2): mixed ping/list/call script, per-id identical.
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
    let sequential = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    let pipelined = ast_sgrep_testkit::rpc_pipeline(script.clone(), Some(temp.path()));
    assert_eq!(sequential.len(), script.len());
    assert_eq!(pipelined.len(), script.len());
    for id in [1, 4, 8] {
        let a = response_by_id(&sequential, id);
        let b = response_by_id(&pipelined, id);
        assert_ping_ok(a, id);
        assert_ping_ok(b, id);
        assert_eq!(canon(a), canon(b), "ping id {id} diverged");
    }
    for id in [2, 6] {
        let a = response_by_id(&sequential, id);
        let b = response_by_id(&pipelined, id);
        assert_tools_list_ok(a, id);
        assert_tools_list_ok(b, id);
        assert_eq!(canon(a), canon(b), "tools/list id {id} diverged");
    }
    let a = response_by_id(&sequential, 3);
    let b = response_by_id(&pipelined, 3);
    assert_tool_success(a);
    assert_tool_success(b);
    assert_eq!(tool_text(a), tool_text(b), "id 3 diverged");
    assert!(tool_text(a).contains("redhammer"), "{a:#}");
    assert_status_counts_eq(response_by_id(&sequential, 5), response_by_id(&pipelined, 5));
    let read_a = response_by_id(&sequential, 7);
    let read_b = response_by_id(&pipelined, 7);
    assert_tool_success(read_a);
    assert_tool_success(read_b);
    assert_eq!(tool_text(read_a), tool_text(read_b), "{read_a:#} vs {read_b:#}");
    assert_eq!(tool_body(read_a)["nodes"][0]["id"], "c.rs#L1-L2");

    // Facet 3 (K3.3): schedule equivalence holds with errors in the batch:
    // per-id success/error shapes agree and bodies are byte-identical.
    let script = vec![
        tool_call(1, "no_such_tool", json!({})),
        tool_call(2, "keyword_search", json!({"query": "redhammer", "limit": 4})),
        tool_call(3, "keyword_search", json!({"query": "x", "limit": 0})),
        tool_call(4, "index_status", json!({})),
        tool_call(5, "code_read", json!({"ids": ["zzz-missing.rs#L1-L1"]})),
        tool_call(6, "keyword_search", json!({"query": "blueanvil", "limit": 4})),
    ];
    let sequential = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    let pipelined = ast_sgrep_testkit::rpc_pipeline(script.clone(), Some(temp.path()));
    assert_eq!(sequential.len(), script.len());
    assert_eq!(pipelined.len(), script.len());
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
        let a = response_by_id(&sequential, id);
        let b = response_by_id(&pipelined, id);
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

/// INTENT: repeating the same work yields identical results, rapid-fire or rerun.
/// KILLS: nondeterminism / in-session state-rot mutants.
/// Facets: rapid-fire identical calls byte-identical (K1.4) + same script
/// twice in one session id-normalized identical (K3.5).
#[test]
fn repeat_determinism_rapid_fire_and_rerun() {
    // Facet 1 (K1.4): 12 rapid-fire identical calls, every body byte-identical.
    let temp = small_tree();
    let script: Vec<Value> = (1..=12u32)
        .map(|id| tool_call(id, "index_status", json!({})))
        .collect();
    let responses = ast_sgrep_testkit::rpc_pipeline(script, Some(temp.path()));
    assert_eq!(responses.len(), 12);
    let first = tool_text(response_by_id(&responses, 1)).to_string();
    assert_tool_success(response_by_id(&responses, 1));
    for id in 2..=12 {
        let response = response_by_id(&responses, id);
        assert_tool_success(response);
        assert_eq!(
            tool_text(response),
            first,
            "id {id} differs from id 1: {response:#}"
        );
    }

    // Facet 2 (K3.5): the same script run twice back-to-back in one session
    // yields id-normalized identical transcripts; the repeat run's searches
    // are fully elided, proving the repetition was recognized rather than
    // recomputed differently.
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
            !second_snippets.is_empty() && second_snippets.iter().all(|s| s == ELIDED),
            "repeat run not fully elided: {:#}",
            second[index]
        );
    }
}

/// INTENT: a fresh server over the same content serves byte-identical results.
/// KILLS: state-leak-across-restart mutants.
/// Facets: fresh servers over same-content roots incl byte-identical
/// initialize (K3.4) + same pipelined batch twice across restart (K3.6).
#[test]
fn restart_equivalence_fresh_servers_and_batches() {
    // Facet 1 (K3.4): the same transcript on a fresh server over a
    // same-content root yields byte-identical responses (count-fields for
    // index_status), including a byte-identical initialize result.
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
    let mut session_a = RawSession::spawn(Some(first.path()));
    let init_a = session_a.handshake();
    session_a.close_stdin();
    assert!(session_a.wait_clean().success());
    let mut session_b = RawSession::spawn(Some(second.path()));
    let init_b = session_b.handshake();
    session_b.close_stdin();
    assert!(session_b.wait_clean().success());
    assert_eq!(canon(&init_a), canon(&init_b), "initialize diverged");
    let transcript_a = ast_sgrep_testkit::rpc_session(script.clone(), Some(first.path()));
    let transcript_b = ast_sgrep_testkit::rpc_session(script.clone(), Some(second.path()));
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
    assert!(
        tool_body(&transcript_a[2])["h"]
            .as_array()
            .is_some_and(|h| !h.is_empty()),
        "{:#}",
        transcript_a[2]
    );

    // Facet 2 (K3.6): the same batch pipelined on two fresh servers (same
    // root, sequential sessions) yields per-id equivalent responses plus
    // snippet-content conservation.
    let temp = small_tree();
    let script = tool_script();
    let run_a = ast_sgrep_testkit::rpc_pipeline(script.clone(), Some(temp.path()));
    let run_b = ast_sgrep_testkit::rpc_pipeline(script.clone(), Some(temp.path()));
    assert_eq!(run_a.len(), script.len());
    assert_eq!(run_b.len(), script.len());
    for id in 1..=script.len() as u32 {
        assert_tool_responses_equivalent(response_by_id(&run_a, id), response_by_id(&run_b, id), id);
    }
    let searches = [1u32, 5, 7, 8];
    let refs_a: Vec<&Value> = searches.iter().map(|id| response_by_id(&run_a, *id)).collect();
    let refs_b: Vec<&Value> = searches.iter().map(|id| response_by_id(&run_b, *id)).collect();
    assert_snippet_conservation(&refs_a, &refs_b);
}

/// INTENT: long sessions stay correct and byte-stable, with no drift or
/// degradation.
/// KILLS: leak/degradation/drift mutants.
/// Facets: 60-call mixed soak all correct (K3.7 KEEP) + repeated-search byte
/// stability with an exact post-warmup steady state (K3.8).
#[test]
fn soak_mixed_calls_and_repeated_search_stable() {
    // Facet 1 (K3.7): 60 sequential mixed calls all answered correctly, ids
    // echo exactly, clean exit.
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

    // Facet 2 (K3.8): 40 identical sequential searches carry the same hits
    // throughout (elision-normalized equal to the first), every repeat after
    // the first byte-identical to the first repeat (exact post-warmup steady
    // state), with periodic status checks succeeding.
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

/// INTENT: reader-path traffic woven through tool work never perturbs tool
/// results.
/// KILLS: interleave-corruption mutants.
/// Facets: ping interleaved (K3.9) + tools/list interleaved (K3.10) +
/// ping/list/stray-cancel woven through a pipelined burst incl a pre-cancel
/// ordering proof (K4.2).
#[test]
fn interleave_stability_ping_list_and_cancel() {
    let temp = small_tree();
    let tools = tool_script();
    let baseline = ast_sgrep_testkit::rpc_session(tools.clone(), Some(temp.path()));

    // Facet 1 (K3.9): ping before and after every tool call never perturbs.
    let mut probe = LiveSession::spawn(Some(temp.path()));
    probe.handshake();
    let mut ping_count = 0u32;
    for payload in &tools {
        let id = payload["id"].as_u64().unwrap() as u32;
        let before = 1000 + id * 2;
        let after = 1000 + id * 2 + 1;
        probe.send(&ping(before));
        assert_ping_ok(&probe.recv(), before);
        ping_count += 1;
        probe.send(payload);
        let tool_response = probe.recv();
        assert_eq!(tool_response["id"], id, "{tool_response:#}");
        assert_tool_responses_equivalent(response_by_id(&baseline, id), &tool_response, id);
        probe.send(&ping(after));
        assert_ping_ok(&probe.recv(), after);
        ping_count += 1;
    }
    assert_eq!(ping_count, tools.len() as u32 * 2);
    probe.close_stdin();
    assert!(probe.wait_clean().success());

    // Facet 2 (K3.10): tools/list between every tool call never perturbs, and
    // every list carries the full tool set.
    let mut probe = LiveSession::spawn(Some(temp.path()));
    probe.handshake();
    let mut list_count = 0u32;
    for (index, payload) in tools.iter().enumerate() {
        let id = payload["id"].as_u64().unwrap() as u32;
        probe.send(payload);
        let tool_response = probe.recv();
        assert_eq!(tool_response["id"], id, "{tool_response:#}");
        assert_tool_responses_equivalent(response_by_id(&baseline, id), &tool_response, id);
        let list_id = 2000 + index as u32;
        probe.send(&tools_list(list_id));
        assert_tools_list_ok(&probe.recv(), list_id);
        list_count += 1;
    }
    assert_eq!(list_count, tools.len() as u32);
    probe.close_stdin();
    assert!(probe.wait_clean().success());

    // Facet 3 (K4.2): ping / tools/list / stray-cancel notifications woven
    // between pipelined tool calls leave every tool result
    // baseline-equivalent; the notifications add exactly the ping/list
    // responses and nothing else. A pre-cancel (cancel before its id is
    // issued) proves ordering holds: that call still succeeds.
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
            probe.send(&cancelled_notif(5));
        }
        probe.send(request);
        let list_id = 200 + id;
        probe.send(&tools_list(list_id));
        list_ids.push(list_id);
        probe.send(&cancelled_notif(9000 + id));
    }
    let expected = tools.len() + ping_ids.len() + list_ids.len();
    let responses = collect_responses(&probe, expected, RECV_TIMEOUT);
    probe.close_stdin();
    assert!(probe.wait_clean().success());
    assert_eq!(responses.len(), expected);
    for request in &tools {
        let id = request["id"].as_u64().unwrap() as u32;
        assert_matches_baseline(request, response_by_id(&baseline, id), response_by_id(&responses, id));
    }
    assert_tool_success(response_by_id(&responses, 5));
    for id in ping_ids {
        assert_ping_ok(response_by_id(&responses, id), id);
    }
    for id in list_ids {
        assert_tools_list_ok(response_by_id(&responses, id), id);
    }
}

/// INTENT: bursts are answered completely and account every id exactly --
/// successes, error shapes, and cancel silence.
/// KILLS: burst-drop/reorder and error-miscount mutants.
/// Facets: 24-call mixed burst vs sequential baseline with full id set
/// (K4.1 KEEP) + burst error accounting with stray-cancel silence (K4.7).
#[test]
fn burst_mixed_calls_and_error_accounting() {
    // Facet 1 (K4.1): 24 mixed calls fired at once, each correct per id
    // against a sequential baseline, full id set echoed, clean exit.
    let temp = small_tree();
    let script = burst_script();
    let baseline = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    let burst = ast_sgrep_testkit::rpc_pipeline(script.clone(), Some(temp.path()));
    assert_eq!(burst.len(), script.len());
    assert_id_set_complete(&burst, script.len() as u32);
    for request in &script {
        let id = request["id"].as_u64().unwrap() as u32;
        assert_matches_baseline(request, response_by_id(&baseline, id), response_by_id(&burst, id));
    }

    // Facet 2 (K4.7): a burst mixing unknown tools, invalid arguments, and
    // stray cancels yields exactly one response per request id -- error
    // shapes for the bad ids, successes for the good ones, silence for the
    // cancels -- with the session still usable afterwards.
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
        let id = request["id"].as_u64().unwrap() as u32;
        session.send(&cancelled_notif(8000 + id));
    }
    let responses = collect_responses(&session, requests.len(), RECV_TIMEOUT);
    assert_eq!(responses.len(), requests.len());
    assert_id_set_complete(&responses, requests.len() as u32);
    for id in [1, 7] {
        assert_tool_error_shape(response_by_id(&responses, id));
    }
    assert_eq!(
        tool_text(response_by_id(&responses, 1)),
        tool_text(response_by_id(&responses, 7)),
        "unknown-tool error bytes diverged"
    );
    for id in [3, 12] {
        assert_tool_error_shape(response_by_id(&responses, id));
    }
    assert_eq!(
        tool_text(response_by_id(&responses, 3)),
        tool_text(response_by_id(&responses, 12)),
        "invalid-args error bytes diverged"
    );
    let red = response_by_id(&responses, 2);
    assert_tool_success(red);
    assert!(
        tool_body(red)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{red:#}"
    );
    assert_tool_success(response_by_id(&responses, 4));
    let blue = response_by_id(&responses, 6);
    assert_tool_success(blue);
    assert!(
        tool_body(blue)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{blue:#}"
    );
    let green = response_by_id(&responses, 8);
    assert_tool_success(green);
    assert!(
        tool_body(green)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{green:#}"
    );
    assert_ping_ok(response_by_id(&responses, 9), 9);
    assert_tools_list_ok(response_by_id(&responses, 10), 10);
    let read = response_by_id(&responses, 11);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "a.rs#L1-L1");
    // Id 5 (missing file) is either a tool error or an empty success; pin
    // only that it answered with a well-formed tool shape.
    let missing = response_by_id(&responses, 5);
    assert_eq!(missing["id"], 5, "{missing:#}");
    assert!(missing.get("error").is_none(), "{missing:#}");
    assert!(missing["result"]["content"][0]["type"] == "text", "{missing:#}");
    session.send(&ping(13));
    assert_ping_ok(&session.recv(), 13);
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// INTENT: SIGKILL mid-session loses nothing observable: a fresh server on
/// the same root serves the canonical transcript baseline-identical.
/// KILLS: unclean-restart mutants.
/// Catalog: K4.4 KEEP.
#[test]
fn restart_kill9_fresh_server_serves_identical_baseline() {
    let temp = small_tree();
    let script = canonical_script();
    let baseline = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    assert_eq!(baseline.len(), script.len());

    let mut victim = RawSession::spawn(Some(temp.path()));
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

    let rerun = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    assert_eq!(rerun.len(), script.len());
    for (index, request) in script.iter().enumerate() {
        assert_matches_baseline(request, &baseline[index], &rerun[index]);
    }
}

/// INTENT: framing is robust to slow clients: byte-trickle writes assemble
/// into correct requests, one at a time and back-to-back in one stream.
/// KILLS: framing-assumption mutants.
/// Catalog: K4.5 KEEP.
#[test]
fn slow_client_byte_trickle_assembles_correct_requests() {
    const CHUNK: usize = 3;
    const PACE: Duration = Duration::from_millis(1);

    let temp = small_tree();
    let mut session = RawSession::spawn(Some(temp.path()));
    session.send_trickle(&init_payload(TESTKIT_CLIENT_NAME), CHUNK, PACE);
    let init = session.recv();
    assert_eq!(init["id"], "__init", "{init:#}");
    session.send_trickle(&initialized_notif(), CHUNK, PACE);

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

    session.send_trickle(
        &tool_call(3, "code_read", json!({"ids": ["b.rs#L1-L1"]})),
        CHUNK,
        PACE,
    );
    let read = session.recv();
    assert_eq!(read["id"], 3, "{read:#}");
    assert_tool_success(&read);
    assert_eq!(tool_body(&read)["nodes"][0]["id"], "b.rs#L1-L1");

    let stream = vec![
        tool_call(4, "keyword_search", json!({"query": "blueanvil", "limit": 4})),
        tools_list(5),
        tool_call(6, "code_search", json!({"query": "greenchisel", "limit": 4})),
    ];
    session.send_trickle_stream(&stream, CHUNK, PACE);
    let mut responses = Vec::new();
    for _ in 0..stream.len() {
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 3);
    let blue = response_by_id(&responses, 4);
    assert_tool_success(blue);
    assert!(
        tool_body(blue)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{blue:#}"
    );
    assert!(tool_text(blue).contains("blueanvil"), "{blue:#}");
    assert_tools_list_ok(response_by_id(&responses, 5), 5);
    let green = response_by_id(&responses, 6);
    assert_tool_success(green);
    assert!(
        tool_body(green)["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{green:#}"
    );
}

/// INTENT: chaos composition converges: burst + interleave + cancel + SIGKILL
/// chained in one session, then a fresh server serves the canonical
/// transcript baseline-identical -- final state correct.
/// KILLS: composition mutants.
/// Catalog: K4.6 KEEP.
#[test]
fn mixed_chaos_burst_interleave_cancel_restart_final_state_correct() {
    let temp = small_tree();
    let script = canonical_script();
    let baseline = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    assert_eq!(baseline.len(), script.len());

    // Chaos session: a 12-request burst with pings, lists, and stray cancels
    // woven in. Only the first three responses are read; a cancel for a
    // possibly-pending id goes out; then SIGKILL -- no timing assertions on
    // the chaos session itself, only that the kill lands.
    let mut chaos = RawSession::spawn(Some(temp.path()));
    chaos.handshake();
    let burst = burst_script();
    let wave: Vec<Value> = burst.into_iter().take(12).collect();
    for request in &wave {
        let id = request["id"].as_u64().unwrap() as u32;
        chaos.send(&ping(500 + id));
        chaos.send(request);
        chaos.send(&tools_list(600 + id));
        chaos.send(&cancelled_notif(9500 + id));
    }
    for _ in 0..3 {
        let response = chaos.recv();
        assert!(response.get("id").is_some(), "{response:#}");
    }
    chaos.send(&cancelled_notif(12));
    let kill_status = chaos.kill9();
    assert!(
        !kill_status.success(),
        "chaos server kill reported success: {kill_status}"
    );

    let rerun = ast_sgrep_testkit::rpc_session(script.clone(), Some(temp.path()));
    assert_eq!(rerun.len(), script.len());
    for (index, request) in script.iter().enumerate() {
        assert_matches_baseline(request, &baseline[index], &rerun[index]);
    }
}
