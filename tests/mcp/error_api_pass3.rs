//! Pass 3 (errorapi, Mission E3): negative-path METAMORPHIC relations.
//!
//! Pass 1 pins the error TAXONOMY (which trigger yields which row) and pass 2
//! pins PROPAGATION (backend state decides the envelope). This file pins
//! RELATIONS between calls: properties that must hold when the same fault is
//! replayed across tools, sessions, and sequence positions. No test here pins
//! a single trigger-to-row mapping; every assertion compares two or more
//! responses (or one response against a forbidden-payload predicate).
//!
//! Pinned relations (discriminants are codes, envelope shapes, and raw bytes,
//! never message text):
//!
//! | row | relation                                              | assertion                              |
//! |-----|-------------------------------------------------------|----------------------------------------|
//! | C1  | same mistyped-`root` fault on all 8 root-taking tools | identical tool-error discriminants     |
//! | C2  | same unknown-argument fault on all 8 named tools      | identical tool-error discriminants     |
//! | C3  | same fault class, 4 `limit` spellings, one channel    | identical tool-error discriminants     |
//! | D1  | identical tool-error calls, two fresh sessions        | byte-identical raw response lines      |
//! | D2  | identical JSON-RPC-error calls, two fresh sessions    | byte-identical raw response lines      |
//! | D3  | same bad args twice in one session (ids differ)       | identical modulo `id` normalization    |
//! | P1  | bad call first vs last (same id set, permuted order)  | per-id byte-identical responses        |
//! | P2  | notification before a bad call vs no notification     | byte-identical error line              |
//! | F1  | fault catalog, tool-level and JSON-RPC-level          | no success members / payload keys      |
//! | F2  | backend failures (corrupt index, deleted file)        | tool-error shape, no partial payloads  |
//!
//! Non-duplication: pass 1 pins per-tool parse rows (bounds/types/unknown
//! keys/sandbox escape on 4 tools) and JSON-RPC codes; pass 2 pins corrupt-db
//! uniformity, mid-session flips, atomicity, and mixed transcripts. E3 reuses
//! some triggers but asserts only cross-call relations (equality of
//! discriminants, byte identity across sessions/orders, absence of payload
//! keys in error bytes) that neither pass states.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
            "clientInfo": {"name": "asgrep-mcp-test", "version": "0"}
        }
    })
}

fn initialized_notif() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// Handshake, then strictly sequential send-one/read-one. Returns the RAW
/// response lines (one per payload, post-handshake) so determinism relations
/// compare bytes, not re-serialized values.
fn rpc_session_raw(payloads: Vec<Value>, root: Option<&Path>) -> Vec<String> {
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
    let recv_raw = |stdout: &mut BufReader<std::process::ChildStdout>| -> String {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout");
        line.trim_end_matches(['\r', '\n']).to_owned()
    };
    send(&mut stdin, &init_payload());
    let init = recv_raw(&mut stdout);
    assert_eq!(
        serde_json::from_str::<Value>(&init).expect("init JSON")["id"],
        "__init",
        "{init}"
    );
    send(&mut stdin, &initialized_notif());
    let mut responses = Vec::new();
    for payload in &payloads {
        send(&mut stdin, payload);
        responses.push(recv_raw(&mut stdout));
    }
    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");
    responses
}

fn tool_call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

fn parse(raw: &str) -> Value {
    serde_json::from_str(raw).expect("JSON-RPC")
}

/// Uniform tool-error envelope: `isError: true`, no top-level `error`, one
/// `text` block, no `structuredContent`. Never asserts message text.
fn assert_tool_error_shape(response: &Value) {
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

/// Machine-readable error discriminant for a tool response: everything about
/// the error EXCEPT the human message text. Equal discriminants mean the same
/// error code/shape reached the caller.
fn tool_error_discriminant(response: &Value) -> (bool, usize, String, bool, bool, bool) {
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

/// JSON-RPC error row: numeric `code`, no `result`.
fn assert_jsonrpc_error(response: &Value, code: i64) {
    assert_eq!(response["error"]["code"], code, "{response:#}");
    assert!(response["error"].is_object(), "{response:#}");
    assert!(response.get("result").is_none(), "{response:#}");
}

/// Canonical form of a response with the `id` normalized away, for comparing
/// calls that differ only in id. `serde_json::Value` sorts object keys, so
/// `to_string` is a canonical byte encoding.
fn canonical_modulo_id(raw: &str) -> String {
    let mut value = parse(raw);
    value["id"] = json!(0);
    serde_json::to_string(&value).expect("canonical JSON")
}

fn by_id(raw_lines: &[String]) -> HashMap<i64, &str> {
    let mut map = HashMap::new();
    for line in raw_lines {
        let id = parse(line)["id"].as_i64().expect("numeric id echo");
        assert!(map.insert(id, line.as_str()).is_none(), "duplicate id {id}");
    }
    map
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

/// Single-file tree with one findable symbol, not yet indexed.
fn file_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    temp
}

/// Single-file tree with one findable symbol, indexed.
fn indexed_tree() -> tempfile::TempDir {
    let temp = file_tree();
    index_tree(temp.path());
    temp
}

/// Overwrite the durable index db with deterministic non-SQLite bytes.
fn corrupt_index_db(root: &Path) {
    let db = root.join(".asgrep").join("index.db");
    assert!(db.is_file(), "expected an index db at {}", db.display());
    std::fs::write(&db, "E3-corrupt-index-sentinel;".repeat(128)).unwrap();
}

const SEARCH_CHANNELS: [&str; 5] = [
    "search",
    "keyword_search",
    "ast_search",
    "semantic_search",
    "code_search",
];

/// C1: the SAME mistyped-`root` fault on every tool that takes a `root` (all
/// 5 search channels plus `code_read`, `index_status`, `index_repo`) yields
/// the identical tool-error discriminant. Pass 1 pins the per-tool rows; this
/// pins the cross-tool consistency relation.
#[test]
fn consistency_mistyped_root_is_same_tool_error_on_every_root_tool() {
    let temp = file_tree();
    let mut calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| tool_call(i as u32 + 1, channel, json!({"query": "x", "root": 42})))
        .collect();
    calls.push(tool_call(
        6,
        "code_read",
        json!({"ids": ["src/lib.rs#L1-L1"], "root": 42}),
    ));
    calls.push(tool_call(7, "index_status", json!({"root": 42})));
    calls.push(tool_call(8, "index_repo", json!({"root": 42})));
    let raw = rpc_session_raw(calls, Some(temp.path()));
    assert_eq!(raw.len(), 8);
    let responses: Vec<Value> = raw.iter().map(|line| parse(line)).collect();
    for (response, id) in responses.iter().zip(1..=8) {
        assert_eq!(response["id"], id, "{response:#}");
        assert_tool_error_shape(response);
    }
    let first = tool_error_discriminant(&responses[0]);
    for response in &responses[1..] {
        assert_eq!(
            tool_error_discriminant(response),
            first,
            "same fault diverged across tools: {response:#}"
        );
    }
}

/// C2: the SAME unknown-argument fault on all 8 named tools (valid base args
/// plus one unknown key) yields the identical tool-error discriminant.
#[test]
fn consistency_unknown_argument_is_same_tool_error_on_every_tool() {
    let temp = file_tree();
    let mut calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| tool_call(i as u32 + 1, channel, json!({"query": "x", "bogus": 1})))
        .collect();
    calls.push(tool_call(
        6,
        "code_read",
        json!({"ids": ["src/lib.rs#L1-L1"], "bogus": 1}),
    ));
    calls.push(tool_call(7, "index_status", json!({"bogus": 1})));
    calls.push(tool_call(8, "index_repo", json!({"bogus": 1})));
    let raw = rpc_session_raw(calls, Some(temp.path()));
    assert_eq!(raw.len(), 8);
    let responses: Vec<Value> = raw.iter().map(|line| parse(line)).collect();
    for (response, id) in responses.iter().zip(1..=8) {
        assert_eq!(response["id"], id, "{response:#}");
        assert_tool_error_shape(response);
    }
    let first = tool_error_discriminant(&responses[0]);
    for response in &responses[1..] {
        assert_eq!(
            tool_error_discriminant(response),
            first,
            "same fault diverged across tools: {response:#}"
        );
    }
}

/// C3: the same fault CLASS in four spellings (`limit` 0, -1, mistyped, and
/// above max) on one channel yields the identical tool-error discriminant.
/// Bound, sign, type, and ceiling rejections are one code, not four.
#[test]
fn consistency_limit_fault_spellings_share_one_tool_error_code() {
    let temp = file_tree();
    let raw = rpc_session_raw(
        vec![
            tool_call(1, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(2, "keyword_search", json!({"query": "x", "limit": -1})),
            tool_call(3, "keyword_search", json!({"query": "x", "limit": "many"})),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "x", "limit": 999999999}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(raw.len(), 4);
    let responses: Vec<Value> = raw.iter().map(|line| parse(line)).collect();
    for (response, id) in responses.iter().zip(1..=4) {
        assert_eq!(response["id"], id, "{response:#}");
        assert_tool_error_shape(response);
    }
    let first = tool_error_discriminant(&responses[0]);
    for response in &responses[1..] {
        assert_eq!(
            tool_error_discriminant(response),
            first,
            "same fault class diverged by spelling: {response:#}"
        );
    }
}

/// D1: identical bad tool calls (same ids, same args) in two fresh sessions
/// over the same tree yield BYTE-IDENTICAL raw response lines: unknown tool,
/// invalid args, and read-time failure are all deterministic.
#[test]
fn determinism_identical_tool_errors_are_byte_identical_across_sessions() {
    let temp = file_tree();
    let calls = || {
        vec![
            tool_call(7, "no_such_tool", json!({})),
            tool_call(8, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(9, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
        ]
    };
    let first = rpc_session_raw(calls(), Some(temp.path()));
    let second = rpc_session_raw(calls(), Some(temp.path()));
    assert_eq!(first, second, "tool-error bytes drifted across sessions");
    for line in &first {
        assert_tool_error_shape(&parse(line));
    }
}

/// D2: identical JSON-RPC-error triggers (same ids) in two fresh sessions
/// yield BYTE-IDENTICAL raw response lines.
#[test]
fn determinism_identical_jsonrpc_errors_are_byte_identical_across_sessions() {
    let temp = file_tree();
    let calls = || {
        vec![
            json!({"jsonrpc":"2.0","id":21,"method":"missing"}),
            json!({"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":42,"arguments":{}}}),
        ]
    };
    let first = rpc_session_raw(calls(), Some(temp.path()));
    let second = rpc_session_raw(calls(), Some(temp.path()));
    assert_eq!(first, second, "JSON-RPC-error bytes drifted across sessions");
    for line in &first {
        assert_jsonrpc_error(&parse(line), -32601);
    }
}

/// D3: the same bad args twice within ONE session (ids necessarily differ)
/// yield identical responses modulo `id` normalization -- the error text and
/// envelope carry no per-call randomness or counter state.
#[test]
fn determinism_repeated_bad_call_in_one_session_is_identical_modulo_id() {
    let temp = file_tree();
    let raw = rpc_session_raw(
        vec![
            tool_call(31, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(32, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(33, "no_such_tool", json!({"query": "x"})),
            tool_call(34, "no_such_tool", json!({"query": "x"})),
        ],
        Some(temp.path()),
    );
    assert_eq!(raw.len(), 4);
    let normalized: Vec<String> = raw.iter().map(|line| canonical_modulo_id(line)).collect();
    assert_eq!(
        normalized[0], normalized[1],
        "repeat of invalid-args call drifted"
    );
    assert_eq!(
        normalized[2], normalized[3],
        "repeat of unknown-tool call drifted"
    );
    for line in &raw {
        assert_tool_error_shape(&parse(line));
    }
}

/// P1: the same id set in permuted order -- backend-failure call first vs
/// last among two successes -- yields per-id BYTE-IDENTICAL responses.
/// Position in the transcript changes neither the error nor the successes.
#[test]
fn position_permuted_transcript_yields_same_per_call_responses() {
    let temp = file_tree();
    let bad = || tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]}));
    let good_a = || tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}));
    let good_b = || tool_call(3, "index_status", json!({}));
    let bad_first = rpc_session_raw(vec![bad(), good_a(), good_b()], Some(temp.path()));
    let bad_last = rpc_session_raw(vec![good_a(), good_b(), bad()], Some(temp.path()));
    assert_eq!(
        by_id(&bad_first),
        by_id(&bad_last),
        "per-call responses depend on sequence position"
    );
    let by_first = by_id(&bad_first);
    assert_tool_error_shape(&parse(by_first[&1]));
    assert_eq!(parse(by_first[&2])["result"]["isError"], false);
    assert_eq!(parse(by_first[&3])["result"]["isError"], false);
}

/// P2: a notification (absent `id`, hence silent) ahead of a bad call leaves
/// the bad call's error bytes unchanged versus the bare call, and the session
/// continues. Silence neither perturbs nor annotates the error.
#[test]
fn position_notification_before_bad_call_leaves_error_bytes_unchanged() {
    let temp = file_tree();
    let bad = tool_call(5, "keyword_search", json!({"query": "x", "limit": 0}));
    let bare = rpc_session_raw(vec![bad.clone()], Some(temp.path()));
    assert_eq!(bare.len(), 1);

    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .env("ASGREP_ROOT", temp.path())
        .spawn()
        .expect("spawn MCP");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let send = |stdin: &mut std::process::ChildStdin, payload: &Value| {
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
    };
    let recv_raw = |stdout: &mut BufReader<std::process::ChildStdout>| -> String {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout");
        line.trim_end_matches(['\r', '\n']).to_owned()
    };
    send(&mut stdin, &init_payload());
    assert_eq!(parse(&recv_raw(&mut stdout))["id"], "__init");
    send(&mut stdin, &initialized_notif());
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"index_status","arguments":{}}}),
    );
    send(&mut stdin, &bad);
    let with_notif = recv_raw(&mut stdout);
    drop(stdin);
    let status = child.wait().expect("wait MCP");
    assert!(status.success(), "MCP exited {status}");

    assert_eq!(
        bare[0], with_notif,
        "notification perturbed the error bytes"
    );
    assert_tool_error_shape(&parse(&with_notif));
}

/// F1: across a fault catalog spanning both error levels, every error response
/// fails closed: JSON-RPC errors carry no `result`, tool errors carry no
/// top-level `error` and no `structuredContent`, and NO error's raw bytes
/// contain any success-payload key. Unknown-method spellings additionally
/// share one code with identical error-object keys.
#[test]
fn failclosed_error_responses_carry_no_success_members_or_payload_keys() {
    let temp = file_tree();
    let outside = tempfile::tempdir().unwrap();
    let escaped = outside.path().display().to_string();
    let mut calls = vec![
        tool_call(1, "no_such_tool", json!({})),
        tool_call(2, "keyword_search", json!({"query": "x", "limit": 0})),
        tool_call(3, "keyword_search", json!({"query": "x", "root": escaped})),
        tool_call(4, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
        tool_call(
            5,
            "code_read",
            json!({"ids": ["src/lib.rs#L1-L1"], "root": 42}),
        ),
        tool_call(6, "index_repo", json!({"bogus": 1})),
        json!({"jsonrpc":"2.0","id":7,"method":"missing"}),
        json!({"jsonrpc":"2.0","id":8,"method":"tools/unknown"}),
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{}}),
    ];
    let n = calls.len();
    let raw = rpc_session_raw(std::mem::take(&mut calls), Some(temp.path()));
    assert_eq!(raw.len(), n);
    for line in &raw[..6] {
        assert_tool_error_shape(&parse(line));
    }
    for line in &raw[6..] {
        assert_jsonrpc_error(&parse(line), -32601);
    }
    // Same JSON-RPC fault, two spellings: one code (above) and identical
    // error-object member keys.
    let first_keys: Vec<String> = parse(&raw[6])["error"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    for line in &raw[7..] {
        let keys: Vec<String> = parse(line)["error"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, first_keys, "error-object keys diverged: {line}");
    }
    // Fail-closed byte scan: no success-payload key may appear in any error.
    const FORBIDDEN: [&str; 5] = [
        "structuredContent",
        "\"nodes\":",
        "\"h\":",
        "\"zn\":",
        "\"file_count\":",
    ];
    for line in &raw {
        for key in FORBIDDEN {
            assert!(
                !line.contains(key),
                "error response smuggles success payload {key}: {line}"
            );
        }
    }
}

/// F2: backend failures (corrupt index on all 5 search channels; deleted
/// read target) surface as tool errors whose bytes carry no partial payloads.
/// Positive controls on a healthy tree prove the payload keys WOULD be visible
/// if present, so the absence checks are not vacuous.
#[test]
fn failclosed_backend_failures_carry_no_partial_payloads() {
    const FORBIDDEN: [&str; 5] = [
        "structuredContent",
        "\"nodes\":",
        "\"h\":",
        "\"zn\":",
        "\"file_count\":",
    ];

    // Positive controls: healthy successes DO carry the payload keys.
    let healthy = indexed_tree();
    let ok = rpc_session_raw(
        vec![
            tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(healthy.path()),
    );
    assert_eq!(ok.len(), 2);
    assert_eq!(parse(&ok[0])["result"]["isError"], false, "{}", ok[0]);
    assert_eq!(parse(&ok[1])["result"]["isError"], false, "{}", ok[1]);
    assert!(ok[0].contains("\"nodes\""), "control lost nodes: {}", ok[0]);
    assert!(
        ok[1].contains("\"h\"") && ok[1].contains("\"zn\""),
        "control lost hits: {}",
        ok[1]
    );

    // Corrupt backend: every search channel fails closed, no partial hits.
    let corrupt = indexed_tree();
    corrupt_index_db(corrupt.path());
    let calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| {
            tool_call(i as u32 + 1, channel, json!({"query": "target_symbol", "limit": 4}))
        })
        .collect();
    let raw = rpc_session_raw(calls, Some(corrupt.path()));
    assert_eq!(raw.len(), SEARCH_CHANNELS.len());
    for line in &raw {
        assert_tool_error_shape(&parse(line));
        for key in FORBIDDEN {
            assert!(
                !line.contains(key),
                "backend failure smuggles partial payload {key}: {line}"
            );
        }
    }

    // Deleted read target: read-time failure fails closed, no partial nodes.
    let temp = file_tree();
    std::fs::remove_file(temp.path().join("src").join("lib.rs")).unwrap();
    let gone = rpc_session_raw(
        vec![tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}))],
        Some(temp.path()),
    );
    assert_eq!(gone.len(), 1);
    assert_tool_error_shape(&parse(&gone[0]));
    for key in FORBIDDEN {
        assert!(
            !gone[0].contains(key),
            "deleted-file failure smuggles partial payload {key}: {}",
            gone[0]
        );
    }
}
