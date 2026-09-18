//! R3 metamorphic-recovery oracles for ast-sgrep-mcp durable state.
//!
//! Non-overlap contract: `protocol.rs` pins handshake/negotiation, discovery,
//! structured content, tool names, per-channel kinds, single-id expansion, read
//! windows, schema rejection, sandbox escapes, byte-stability, elision,
//! miss envelopes, cancellation, and EOF-before-initialize. Pass 1 pins
//! startup config, pipelined workspace-removal, file-roots, pre-session index
//! corruption plus delete-and-reindex healing, empty-root restart determinism,
//! elision reset on restart, and pinned-index garbage. Pass 2 pins ACTIVE
//! mid-session faults (root deletion + live heal, stub/half tears, EOF,
//! garbage lines, invalid envelopes, restart-after-restore). This file pins
//! NONE of those again. Instead it pins RELATIONS over recovery --
//! equalities/inequalities between calls, sessions, and restarts:
//!
//! * restart determinism of the INDEXED multi-tool chain (pass 1 pins only the
//!   empty root; pass 2 pins only fault-and-restore, not pure restart);
//! * fault -> restart -> verify roundtrip on SOURCE bytes (pass 1/2 corrupt
//!   only `index.db` or the root itself, never file content): reads track the
//!   tree under fault, the fault reproduces across restarts, and restore plus
//!   restart reproduces the baseline;
//! * `tools/list` stability across stream, index, and root fault cycles
//!   (protocol pins only the fault-free case);
//! * search-then-read link consistency preserved across restarts: the compact
//!   id resolves to the same stable node with the same bytes in every fresh
//!   session;
//! * full-transcript identity across three identical sessions;
//! * `resend_seen` stateless-encoding invariance under call position and
//!   restart (protocol pins within-session bytes; pass 1 pins elision reset;
//!   this pins the position x restart product);
//! * error-envelope determinism across restarts: invalid calls fail with
//!   byte-identical tool errors in every fresh process;
//! * explicit-root equivalence: omitted vs explicit default root are
//!   byte-identical within each session and across restarts.
//!
//! Discriminants are exit codes, `isError` booleans, envelope shapes (key
//! presence, tuple widths, counts, id echo), and byte (in)equality -- never
//! message text. Every live-session read and process wait carries a timeout
//! so a regressed server fails the test instead of hanging the suite.

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
            "clientInfo": {"name": "asgrep-mcp-r3", "version": "0"}
        }
    })
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

/// Drive several requests through ONE fresh server process, strictly
/// sequential (send one, read one), so response order matches request order.
fn rpc_session(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    let mut command = Command::new(mcp_bin());
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(root) = root {
        command.env("ASGREP_ROOT", root);
    }
    let mut child = command.spawn().expect("spawn MCP");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let send = |stdin: &mut std::process::ChildStdin, payload: &Value| {
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    };
    let recv = |stdout: &mut BufReader<std::process::ChildStdout>| -> Value {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout");
        serde_json::from_str(line.trim()).expect("JSON-RPC")
    };
    send(&mut stdin, &init_payload());
    let init = recv(&mut stdout);
    assert_eq!(init["id"], "__init", "{init:#}");
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
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

const FIXTURE_SOURCE: &str = "fn target_symbol() { helper(); }\nfn helper() {}\n";
const FAULT_SOURCE: &str = "fn mutated_symbol() { changed(); }\nfn changed() {}\n// R3-fault-sentinel\n";

/// Two-symbol indexed tree: two distinct searchable queries plus a
/// multi-line file for read-window relations.
fn indexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), FIXTURE_SOURCE).unwrap();
    index_tree(temp.path());
    temp
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

fn search_call(id: u32, query: &str) -> Value {
    tool_call(
        id,
        "keyword_search",
        json!({"query": query, "limit": 4, "resend_seen": true}),
    )
}

/// tools/list catalog bytes: the discovery region must be restart-stable.
fn catalog_bytes(response: &Value) -> String {
    serde_json::to_string(&response["result"]).unwrap()
}

#[test]
fn restart_indexed_chain_reproduces_byte_identical_responses() {
    // Relation: chain(session N) == chain(session N+1) byte-for-byte over an
    // indexed tree with no fault involved. Pass 1 pins this only for the
    // empty root; the indexed chain adds hit tuples, path tables, and reads.
    let temp = indexed_tree();
    let chain = || {
        vec![
            tools_list(1),
            tool_call(2, "index_status", json!({})),
            search_call(3, "target_symbol"),
            search_call(4, "helper"),
            tool_call(5, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 5);
    assert_eq!(second.len(), 5);
    assert_eq!(
        catalog_bytes(&first[0]),
        catalog_bytes(&second[0]),
        "tools/list drifted across restarts"
    );
    assert_eq!(first[0]["result"]["tools"].as_array().unwrap().len(), 8);
    for (index, response) in first.iter().enumerate().skip(1) {
        assert_eq!(
            tool_text(response),
            tool_text(&second[index]),
            "response {index} drifted across restarts"
        );
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    assert_eq!(tool_body(&first[1])["file_count"], 1);
    for response in [&first[2], &first[3]] {
        let envelope = tool_body(response);
        let hits = envelope["h"].as_array().unwrap();
        assert!(!hits.is_empty(), "{envelope:#}");
        assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
        for hit in hits {
            assert_eq!(hit.as_array().unwrap().len(), 5, "{hit:#}");
        }
    }
    assert_eq!(tool_body(&first[4])["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(tool_body(&first[4])["nodes"][0]["id"], "src/lib.rs#L1-L1");
}

#[test]
fn fault_source_corruption_restart_roundtrip_restores_baseline() {
    // Relation over a SOURCE-byte fault (pass 1/2 corrupt only `index.db` or
    // the root itself): reads track the live tree (fault bytes != baseline
    // bytes), the fault reproduces identically across restarts (a restart
    // neither heals nor masks durable bytes), and restoring the exact bytes
    // plus a restart reproduces the baseline search and read byte-identically.
    let temp = indexed_tree();
    let lib = temp.path().join("src").join("lib.rs");

    let baseline = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    let baseline_search = tool_text(&baseline[0]).to_owned();
    let baseline_read = tool_text(&baseline[1]).to_owned();

    std::fs::write(&lib, FAULT_SOURCE).unwrap();

    // Fault observed from two independent fresh processes: identical.
    let mut faulty = Vec::new();
    for _ in 0..2 {
        let responses = rpc_session(
            vec![tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}))],
            Some(temp.path()),
        );
        faulty.push(tool_text(&responses[0]).to_owned());
        assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
        assert_eq!(tool_body(&responses[0])["nodes"][0]["id"], "src/lib.rs#L1-L1");
    }
    assert_eq!(faulty[0], faulty[1], "fault must reproduce across restarts");
    assert_ne!(
        faulty[0], baseline_read,
        "reads must track the live tree under fault"
    );

    std::fs::write(&lib, FIXTURE_SOURCE).unwrap();
    let healed = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(
        tool_text(&healed[0]),
        baseline_search,
        "search drifted after fault roundtrip"
    );
    assert_eq!(
        tool_text(&healed[1]),
        baseline_read,
        "read drifted after fault roundtrip"
    );
}

#[test]
fn tools_list_stable_across_stream_index_and_root_fault_cycles() {
    // Relation: tools/list bytes are invariant under every fault class --
    // mid-stream garbage plus an invalid envelope, a torn index db, and a
    // deleted workspace root -- and across the restarts that follow. Protocol
    // pins stability only for the fault-free case.
    let temp = indexed_tree();

    // Stream faults plus an index tear inside one live session.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tools_list(1));
    let list = session.recv();
    assert_eq!(list["id"], 1, "{list:#}");
    let baseline = catalog_bytes(&list);
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 8);

    // Stream-fault cycle: garbage line plus an invalid envelope. The garbage
    // draws no response (the next recv is the envelope error with its own
    // id), and the session survives.
    session.send_raw_line("{{r3-not-json");
    session.send(&json!({"jsonrpc": "2.0", "id": 77, "method": "missing"}));
    let error = session.recv();
    assert_eq!(error["id"], 77, "{error:#}");
    assert!(error["error"].is_object(), "{error:#}");
    assert!(error["error"]["code"].is_i64(), "{error:#}");
    assert!(error.get("result").is_none(), "{error:#}");
    session.send(&tools_list(2));
    let after_stream = session.recv();
    assert_eq!(after_stream["id"], 2, "{after_stream:#}");
    assert_eq!(
        catalog_bytes(&after_stream),
        baseline,
        "tools/list drifted across stream faults"
    );

    // Index-fault cycle: tear the db to a stub. Discovery must not notice,
    // while an index-dependent call proves the fault is active.
    let db = temp.path().join(".asgrep").join("index.db");
    assert!(std::fs::metadata(&db).unwrap().len() > 4096);
    truncate_index_db(temp.path(), 7);
    session.send(&tools_list(3));
    let during_tear = session.recv();
    assert_eq!(during_tear["id"], 3, "{during_tear:#}");
    assert_eq!(
        catalog_bytes(&during_tear),
        baseline,
        "tools/list drifted across index tear"
    );
    session.send(&tool_call(4, "index_status", json!({})));
    let status = session.recv();
    assert_eq!(status["id"], 4, "{status:#}");
    assert_tool_error_shape(&status);
    session.close_stdin();
    assert!(session.wait_clean().success());

    // Heal the durable index out of band, restart: catalog still identical.
    std::fs::remove_file(temp.path().join(".asgrep").join("index.db")).unwrap();
    index_tree(temp.path());
    let restarted = rpc_session(vec![tools_list(1)], Some(temp.path()));
    assert_eq!(
        catalog_bytes(&restarted[0]),
        baseline,
        "tools/list drifted across heal-plus-restart"
    );

    // Root-fault cycle on a second tree: discovery answers identically while
    // the workspace itself is deleted, after live recreation, and after a
    // restart over the recreated tree.
    let temp2 = tempfile::tempdir().unwrap();
    std::fs::write(temp2.path().join("a.rs"), "fn hey() {}\n").unwrap();
    let mut root_session = LiveSession::spawn(Some(temp2.path()));
    root_session.handshake();
    root_session.send(&tools_list(1));
    let root_baseline = catalog_bytes(&root_session.recv());
    std::fs::remove_dir_all(temp2.path()).unwrap();
    assert!(!temp2.path().exists());
    root_session.send(&tools_list(2));
    let during_delete = root_session.recv();
    assert_eq!(during_delete["id"], 2, "{during_delete:#}");
    assert_eq!(
        catalog_bytes(&during_delete),
        root_baseline,
        "tools/list drifted while root deleted"
    );
    root_session.send(&ping(3));
    let ping_during = root_session.recv();
    assert_eq!(ping_during["id"], 3, "{ping_during:#}");
    assert!(ping_during.get("error").is_none(), "{ping_during:#}");
    assert!(ping_during.get("result").is_some(), "{ping_during:#}");
    std::fs::create_dir_all(temp2.path()).unwrap();
    std::fs::write(temp2.path().join("a.rs"), "fn hey() {}\n").unwrap();
    root_session.send(&tools_list(4));
    assert_eq!(
        catalog_bytes(&root_session.recv()),
        root_baseline,
        "tools/list drifted after live root heal"
    );
    root_session.close_stdin();
    assert!(root_session.wait_clean().success());
    let root_restarted = rpc_session(vec![tools_list(1)], Some(temp2.path()));
    assert_eq!(
        catalog_bytes(&root_restarted[0]),
        root_baseline,
        "tools/list drifted across root-fault restart"
    );
    // One build serves both trees, so both catalogs are one catalog.
    assert_eq!(root_baseline, baseline, "catalog differs between trees");
}

#[test]
fn search_then_read_consistency_preserved_across_restarts() {
    // Relation: in EVERY fresh session the search -> compact-id -> code_read
    // link resolves to the same stable node with the same bytes, and the
    // whole link reproduces byte-identically across restarts. Protocol pins
    // the single-session link; this pins the link as a restart invariant.
    let temp = indexed_tree();
    let stable = "src/lib.rs#L1-L1";

    // Session pair A: a lone search (compact-id source), then the linked
    // search-plus-reads chain in a fresh process.
    let probe_a = rpc_session(vec![search_call(1, "target_symbol")], Some(temp.path()));
    let compact_a = tool_body(&probe_a[0])["h"][0][0]
        .as_str()
        .expect("compact id")
        .to_owned();
    let linked_a = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": [compact_a]})),
            tool_call(3, "code_read", json!({"ids": [stable]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(linked_a.len(), 3);
    assert_eq!(
        tool_text(&linked_a[0]),
        tool_text(&probe_a[0]),
        "first-search bytes differ between fresh sessions"
    );

    // Session pair B: the identical shape after another restart.
    let probe_b = rpc_session(vec![search_call(1, "target_symbol")], Some(temp.path()));
    assert_eq!(
        tool_text(&probe_b[0]),
        tool_text(&probe_a[0]),
        "search drifted across restarts"
    );
    let compact_b = tool_body(&probe_b[0])["h"][0][0]
        .as_str()
        .expect("compact id")
        .to_owned();
    let linked_b = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": [compact_b]})),
            tool_call(3, "code_read", json!({"ids": [stable]})),
        ],
        Some(temp.path()),
    );

    // The whole link reproduces across restarts.
    for (index, response) in linked_a.iter().enumerate() {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
        assert_eq!(
            tool_text(&linked_b[index]),
            tool_text(response),
            "linked response {index} drifted across restarts"
        );
    }
    // Within each session the compact id and the stable ref resolve to the
    // same node with the same bytes.
    for linked in [&linked_a, &linked_b] {
        let via_compact = tool_body(&linked[1]);
        let via_stable = tool_body(&linked[2]);
        assert_eq!(via_compact["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(via_stable["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(via_compact["nodes"][0]["id"], stable);
        assert_eq!(via_stable["nodes"][0]["id"], stable);
        assert_eq!(
            via_compact["nodes"][0]["content"], via_stable["nodes"][0]["content"],
            "compact and stable reads disagree within one session"
        );
    }
}

#[test]
fn repeated_identical_sessions_produce_identical_transcripts() {
    // Relation: transcript(session) == transcript(session) == transcript
    // across three independent fresh processes -- full response values, not
    // just shapes. Every call is first-in-session, so no session memory may
    // perturb any byte.
    let temp = indexed_tree();
    let chain = || {
        vec![
            tools_list(1),
            ping(2),
            tool_call(3, "index_status", json!({})),
            search_call(4, "target_symbol"),
            search_call(5, "helper"),
            tool_call(6, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ]
    };
    let runs: Vec<Vec<Value>> = (0..3).map(|_| rpc_session(chain(), Some(temp.path()))).collect();
    for run in &runs {
        assert_eq!(run.len(), 6);
    }
    assert_eq!(runs[1], runs[0], "second transcript differs");
    assert_eq!(runs[2], runs[0], "third transcript differs");
    // Discriminant spot-checks on the canonical transcript.
    assert_eq!(runs[0][0]["result"]["tools"].as_array().unwrap().len(), 8);
    assert_eq!(runs[0][1]["id"], 2);
    assert!(runs[0][1].get("error").is_none(), "{:#}", runs[0][1]);
    assert!(runs[0][1].get("result").is_some(), "{:#}", runs[0][1]);
    for response in runs[0].iter().skip(2) {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
}

#[test]
fn resend_seen_encoding_invariant_under_position_and_restart() {
    // Relation: with `resend_seen` the search encoding is a pure function of
    // (query, index) -- identical at every call position, interleaved with
    // other calls, and across restarts. Protocol pins within-session bytes;
    // pass 1 pins elision reset; this pins the position x restart product.
    let temp = indexed_tree();
    let pair = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            search_call(2, "target_symbol"),
        ],
        Some(temp.path()),
    );
    assert_eq!(
        tool_text(&pair[0]),
        tool_text(&pair[1]),
        "resend_seen bytes differ by position"
    );
    let restarted = rpc_session(vec![search_call(1, "target_symbol")], Some(temp.path()));
    assert_eq!(
        tool_text(&restarted[0]),
        tool_text(&pair[0]),
        "resend_seen bytes differ across restart"
    );
    let interleaved = rpc_session(
        vec![
            tool_call(1, "index_status", json!({})),
            search_call(2, "target_symbol"),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            search_call(4, "target_symbol"),
        ],
        Some(temp.path()),
    );
    assert_eq!(
        tool_text(&interleaved[1]),
        tool_text(&pair[0]),
        "resend_seen bytes differ when interleaved"
    );
    assert_eq!(
        tool_text(&interleaved[3]),
        tool_text(&pair[0]),
        "resend_seen bytes differ at a later position"
    );
    // No elision markers anywhere in the stateless encoding.
    for response in [&pair[0], &pair[1], &restarted[0], &interleaved[1], &interleaved[3]] {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
        let envelope = tool_body(response);
        let hits = envelope["h"].as_array().unwrap();
        assert!(!hits.is_empty(), "{envelope:#}");
        assert!(
            envelope.get("ze").is_none(),
            "stateless encoding must not elide: {envelope:#}"
        );
        for hit in hits {
            assert_ne!(hit[4], "~", "snippet elided despite resend_seen: {hit:#}");
        }
    }
}

#[test]
fn error_envelopes_reproduce_identically_across_restarts() {
    // Relation: error(call, session N) == error(call, session N+1) as bytes.
    // Recovery must not leak error state between processes, and every
    // failure keeps the uniform tool-error shape. Discriminants are shape
    // plus byte equality, never message text.
    let temp = indexed_tree();
    let chain = || {
        vec![
            tool_call(1, "keyword_search", json!({"query": "target_symbol", "limit": 0})),
            tool_call(2, "keyword_search", json!({"query": "", "limit": 4})),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
            tool_call(4, "no_such_tool", json!({})),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 4);
    assert_eq!(second.len(), 4);
    for (index, response) in first.iter().enumerate() {
        assert_tool_error_shape(response);
        assert_tool_error_shape(&second[index]);
        assert_eq!(
            tool_text(response),
            tool_text(&second[index]),
            "error {index} drifted across restarts"
        );
    }
}

#[test]
fn explicit_root_equivalent_to_default_across_restarts() {
    // Relation: f(root omitted) == f(root = default) byte-identically within
    // each session, and both sides reproduce across restarts. The equivalence
    // class itself is the restart invariant.
    let temp = indexed_tree();
    let root = temp.path().display().to_string();
    let chain = || {
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(2, "index_status", json!({"root": root.as_str()})),
            search_call(3, "target_symbol"),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "root": root.as_str()}),
            ),
            tool_call(5, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(
                6,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": root.as_str()}),
            ),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 6);
    assert_eq!(second.len(), 6);
    for (omitted, explicit) in [(0usize, 1usize), (2, 3), (4, 5)] {
        assert_eq!(first[omitted]["result"]["isError"], false, "{:#}", first[omitted]);
        assert_eq!(
            tool_text(&first[explicit]),
            tool_text(&first[omitted]),
            "explicit root differs from default in the first session"
        );
        assert_eq!(
            tool_text(&second[explicit]),
            tool_text(&second[omitted]),
            "explicit root differs from default in the second session"
        );
        assert_eq!(
            tool_text(&second[omitted]),
            tool_text(&first[omitted]),
            "default-root response drifted across restarts"
        );
    }
}
