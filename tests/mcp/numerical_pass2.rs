//! Pass 2 (numerical, Mission N2): totality over degenerate integer inputs.
//!
//! Non-overlap contract: pass 1 pins EXACT hand-computed values (per-ref
//! remainder deal, context clamps, char-vs-byte truncation, `zd` accounting,
//! schema literals). This file pins ACCEPT/REJECT boundary SIDES and proves
//! every degenerate integer is TOTAL: 0 / negative / huge / mistyped numerics
//! map to the DOCUMENTED outcome (structured success or `isError`), never a
//! panic (stdio EOF / nonzero exit), never silent garbage.
//!
//! Documented bounds under test (`crates/ast-sgrep-mcp/src/lib.rs`):
//! limit 1..=100, budget 1..=65536, context 0..=100, max_chars 1..=1000000,
//! ids 1..=20. Wire numerics are `Option<u64>`, so negatives, floats, strings,
//! bools, and >u64 values fail deserialization into the same `isError` row.
//!
//! Assertions are discriminants only (`isError`, ints, content/truncated,
//! line windows, node counts) -- never message text, which error_api owns.

use serde_json::{json, Value};
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

/// Drive several requests through ONE server process (one spawn per test).
/// Asserts clean exit: a panic on any degenerate input fails the test here.
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
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": "__init",
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "asgrep-mcp-numerical-n2", "version": "0"}
            }
        }),
    );
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

fn search_call(id: u32, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"keyword_search","arguments":arguments}})
}

fn read_call(id: u32, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"code_read","arguments":arguments}})
}

fn tool_body(response: &Value) -> Value {
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
        .expect("tool body JSON")
}

fn is_error(response: &Value) -> bool {
    response["result"]["isError"].as_bool().unwrap()
}

/// Tiny indexed tree: one file with a findable symbol.
fn indexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("lib.rs"),
        "fn target_symbol() { helper(); }\nfn helper() {}\n",
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

#[test]
fn limit_boundary_sides_accept_1_and_100_reject_0_and_101() {
    // Documented: limit in 1..=100. Both edges accept with live hits;
    // both off-by-one neighbors reject. limit=1 additionally caps rows.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": 1})),
            search_call(2, json!({"query": "target_symbol", "limit": 100})),
            search_call(3, json!({"query": "target_symbol", "limit": 0})),
            search_call(4, json!({"query": "target_symbol", "limit": 101})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4);
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(!is_error(&responses[1]), "{:#}", responses[1]);
    assert!(is_error(&responses[2]), "{:#}", responses[2]);
    assert!(is_error(&responses[3]), "{:#}", responses[3]);
    let one = tool_body(&responses[0]);
    let rows = one["h"].as_array().unwrap();
    assert!(!rows.is_empty(), "limit=1 must still hit: {one:#}");
    assert_eq!(rows.len(), 1, "limit=1 caps to one row: {one:#}");
    let hundred = tool_body(&responses[1]);
    assert!(
        !hundred["h"].as_array().unwrap().is_empty(),
        "limit=100 must hit: {hundred:#}"
    );
}

#[test]
fn limit_degenerate_rejects_negative_huge_float_string() {
    // Totality: every non-1..=100 numeric (and non-numeric) limit rejects
    // with isError -- no panic, no silent clamp, no wraparound accept.
    // 2^64 overflows u64 wire parsing; u64::MAX parses then fails the bound.
    let over_u64: Value = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"target_symbol","limit":18446744073709551616}}}"#,
    )
    .unwrap();
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": -1})),
            search_call(2, json!({"query": "target_symbol", "limit": u64::MAX})),
            search_call(3, json!({"query": "target_symbol", "limit": 1.5})),
            search_call(4, json!({"query": "target_symbol", "limit": "4"})),
            over_u64,
            search_call(6, json!({"query": "target_symbol", "limit": 4})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 6);
    for response in &responses[..5] {
        assert!(is_error(response), "degenerate limit must reject: {response:#}");
    }
    // Session survives the storm: a valid call still succeeds.
    assert!(!is_error(&responses[5]), "{:#}", responses[5]);
}

#[test]
fn budget_boundary_sides_accept_1_and_65536_reject_0_and_65537() {
    // Documented: budget_tokens in 1..=65536. Accept sides echo the budget
    // in zd[0] exactly; 0 and 65537 reject. (Pass 1 pins zd[1] accounting
    // at interior values; here only the boundary sides + echo literal.)
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 1})),
            search_call(2, json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 65536})),
            search_call(3, json!({"query": "target_symbol", "limit": 4, "budget_tokens": 0})),
            search_call(4, json!({"query": "target_symbol", "limit": 4, "budget_tokens": 65537})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4);
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(!is_error(&responses[1]), "{:#}", responses[1]);
    assert!(is_error(&responses[2]), "{:#}", responses[2]);
    assert!(is_error(&responses[3]), "{:#}", responses[3]);
    let min = tool_body(&responses[0]);
    assert_eq!(min["zd"][0], 1, "{min:#}");
    let max = tool_body(&responses[1]);
    assert_eq!(max["zd"][0], 65536, "{max:#}");
}

#[test]
fn budget_degenerate_rejects_negative_huge_float_string_bool() {
    // Totality: non-u64 budgets (negative, float, string, bool) and
    // over-max u64 all reject; the session stays usable afterwards.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "budget_tokens": -1})),
            search_call(2, json!({"query": "target_symbol", "budget_tokens": u64::MAX})),
            search_call(3, json!({"query": "target_symbol", "budget_tokens": 1.5})),
            search_call(4, json!({"query": "target_symbol", "budget_tokens": "many"})),
            search_call(5, json!({"query": "target_symbol", "budget_tokens": true})),
            search_call(6, json!({"query": "target_symbol", "limit": 4, "budget_tokens": 8})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 6);
    for response in &responses[..5] {
        assert!(is_error(response), "degenerate budget must reject: {response:#}");
    }
    assert!(!is_error(&responses[5]), "{:#}", responses[5]);
    assert_eq!(tool_body(&responses[5])["zd"][0], 8, "{:#}", responses[5]);
}

#[test]
fn context_boundary_sides_accept_0_and_100_reject_101_and_degenerate() {
    // Documented: context_lines in 0..=100. ctx=0 is the exact window
    // (start=end=req); ctx=100 clamps to file edges; 101/negative/huge/
    // float reject. Hand-computed on a 5-line file at L3-L3.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("five.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["five.rs#L3-L3"], "context_lines": 0})),
            read_call(2, json!({"ids": ["five.rs#L3-L3"], "context_lines": 100})),
            read_call(3, json!({"ids": ["five.rs#L3-L3"], "context_lines": 101})),
            read_call(4, json!({"ids": ["five.rs#L3-L3"], "context_lines": -1})),
            read_call(5, json!({"ids": ["five.rs#L3-L3"], "context_lines": u64::MAX})),
            read_call(6, json!({"ids": ["five.rs#L3-L3"], "context_lines": 1.5})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 6);
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(!is_error(&responses[1]), "{:#}", responses[1]);
    for response in &responses[2..] {
        assert!(is_error(response), "degenerate context must reject: {response:#}");
    }
    let exact = tool_body(&responses[0]);
    assert_eq!(exact["nodes"][0]["lines"], json!({"start": 3, "end": 3}), "{exact:#}");
    assert_eq!(exact["nodes"][0]["content"], "l3", "{exact:#}");
    assert_eq!(exact["nodes"][0]["truncated"], false, "{exact:#}");
    let clamped = tool_body(&responses[1]);
    assert_eq!(clamped["nodes"][0]["lines"], json!({"start": 1, "end": 5}), "{clamped:#}");
    assert_eq!(clamped["nodes"][0]["content"], "l1\nl2\nl3\nl4\nl5", "{clamped:#}");
}

#[test]
fn max_chars_boundary_sides_accept_1_and_1000000_reject_0_and_1000001() {
    // Documented: max_chars in 1..=1000000. n=1 so per_ref=max_chars,
    // remainder=0: budget 1 truncates "abc" to ("a", true); budget 10^6
    // returns the whole line untruncated. 0 / 10^6+1 / negative / huge /
    // mistyped reject.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("abc.rs"), "abc\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["abc.rs#L1-L1"], "max_chars": 1})),
            read_call(2, json!({"ids": ["abc.rs#L1-L1"], "max_chars": 1_000_000})),
            read_call(3, json!({"ids": ["abc.rs#L1-L1"], "max_chars": 0})),
            read_call(4, json!({"ids": ["abc.rs#L1-L1"], "max_chars": 1_000_001})),
            read_call(5, json!({"ids": ["abc.rs#L1-L1"], "max_chars": -1})),
            read_call(6, json!({"ids": ["abc.rs#L1-L1"], "max_chars": u64::MAX})),
            read_call(7, json!({"ids": ["abc.rs#L1-L1"], "max_chars": "many"})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 7);
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(!is_error(&responses[1]), "{:#}", responses[1]);
    for response in &responses[2..] {
        assert!(is_error(response), "degenerate max_chars must reject: {response:#}");
    }
    let min = tool_body(&responses[0]);
    assert_eq!(min["nodes"][0]["content"], "a", "{min:#}");
    assert_eq!(min["nodes"][0]["truncated"], true, "{min:#}");
    let max = tool_body(&responses[1]);
    assert_eq!(max["nodes"][0]["content"], "abc", "{max:#}");
    assert_eq!(max["nodes"][0]["truncated"], false, "{max:#}");
}

#[test]
fn ids_arity_edges_reject_empty_and_21_accept_single() {
    // Documented: ids in 1..=20. n=0 must reject BEFORE `max_chars / n`
    // divides (no div-by-zero panic); 21 rejects; 1 accepts. One session
    // proves the rejects leave no poison behind.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ok.rs"), "hi\n").unwrap();
    let twenty_one: Vec<Value> = (0..21).map(|_| json!("ok.rs#L1-L1")).collect();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": []})),
            read_call(2, json!({"ids": twenty_one})),
            read_call(3, json!({"ids": ["ok.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    assert!(is_error(&responses[0]), "empty ids must reject: {:#}", responses[0]);
    assert!(is_error(&responses[1]), "21 ids must reject: {:#}", responses[1]);
    assert!(!is_error(&responses[2]), "{:#}", responses[2]);
    let body = tool_body(&responses[2]);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1, "{body:#}");
    assert_eq!(body["nodes"][0]["content"], "hi", "{body:#}");
}

#[test]
fn twenty_ref_split_spends_max_chars_exactly() {
    // Division edge at max arity: max_chars=25 over n=20 gives per_ref=1,
    // remainder=5, so budgets are [2; 5] then [1; 15]. Each "aaaa" line
    // truncates; the split spends exactly 25 chars. Kills: remainder
    // dropped (total 20), per_ref rounding up (total > 25), n=20 fencepost.
    let temp = tempfile::tempdir().unwrap();
    let lines = (0..20).map(|_| "aaaa").collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(temp.path().join("grid.rs"), lines).unwrap();
    let ids: Vec<Value> = (1..=20).map(|n| json!(format!("grid.rs#L{n}-L{n}"))).collect();
    let responses = rpc_session(
        vec![read_call(1, json!({"ids": ids, "max_chars": 25}))],
        Some(temp.path()),
    );
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    let body = tool_body(&responses[0]);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 20, "{body:#}");
    for (index, node) in nodes.iter().enumerate() {
        let want = if index < 5 { "aa" } else { "a" };
        assert_eq!(node["content"], want, "node {index}: {body:#}");
        assert_eq!(node["truncated"], true, "node {index}: {body:#}");
    }
    let total: usize = nodes
        .iter()
        .map(|n| n["content"].as_str().unwrap().chars().count())
        .sum();
    assert_eq!(total, 25, "split must spend exactly max_chars: {body:#}");
}

#[test]
fn empty_file_totality_under_min_and_max_budgets() {
    // Truncation of EMPTY input: ("", false) at both max_chars edges --
    // a zero-length selection never reports truncated. L2-L2 starts past
    // the single virtual line (total=1) and rejects. (Pass 1 pins the
    // default-budget L1-L1 success and the L1-L2 over-end reject.)
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("empty.rs"), "").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["empty.rs#L1-L1"], "max_chars": 1})),
            read_call(2, json!({"ids": ["empty.rs#L1-L1"], "max_chars": 1_000_000})),
            read_call(3, json!({"ids": ["empty.rs#L2-L2"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(!is_error(&responses[1]), "{:#}", responses[1]);
    assert!(is_error(&responses[2]), "{:#}", responses[2]);
    for response in &responses[..2] {
        let body = tool_body(response);
        assert_eq!(body["nodes"][0]["lines"], json!({"start": 1, "end": 1}), "{body:#}");
        assert_eq!(body["nodes"][0]["content"], "", "{body:#}");
        assert_eq!(body["nodes"][0]["truncated"], false, "{body:#}");
    }
}

#[test]
fn line_window_edges_for_missing_and_bare_newlines() {
    // scan_line_window totality: a last line without trailing "\n" still
    // counts (total=1); a lone "\n" is one empty line, like the 0-byte
    // file; requesting past total rejects. Hand-computed totals: "solo"
    // -> 1 line; "x\n" -> 1 line; "\n" -> 1 empty line.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("solo.rs"), "solo").unwrap();
    std::fs::write(temp.path().join("nl.rs"), "x\n").unwrap();
    std::fs::write(temp.path().join("bare.rs"), "\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["solo.rs#L1-L1"]})),
            read_call(2, json!({"ids": ["solo.rs#L1-L2"]})),
            read_call(3, json!({"ids": ["nl.rs#L1-L1"]})),
            read_call(4, json!({"ids": ["bare.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4);
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(is_error(&responses[1]), "{:#}", responses[1]);
    assert!(!is_error(&responses[2]), "{:#}", responses[2]);
    assert!(!is_error(&responses[3]), "{:#}", responses[3]);
    let solo = tool_body(&responses[0]);
    assert_eq!(solo["nodes"][0]["lines"], json!({"start": 1, "end": 1}), "{solo:#}");
    assert_eq!(solo["nodes"][0]["content"], "solo", "{solo:#}");
    assert_eq!(solo["nodes"][0]["truncated"], false, "{solo:#}");
    let nl = tool_body(&responses[2]);
    assert_eq!(nl["nodes"][0]["content"], "x", "{nl:#}");
    assert_eq!(nl["nodes"][0]["truncated"], false, "{nl:#}");
    let bare = tool_body(&responses[3]);
    assert_eq!(bare["nodes"][0]["lines"], json!({"start": 1, "end": 1}), "{bare:#}");
    assert_eq!(bare["nodes"][0]["content"], "", "{bare:#}");
    assert_eq!(bare["nodes"][0]["truncated"], false, "{bare:#}");
}

#[test]
fn huge_line_truncates_to_budget_in_chars_not_bytes() {
    // Truncation of HUGE input: a 5000-char line at budget 10 keeps
    // exactly 10 chars with truncated=true; a 100-char multibyte line
    // ("é" x 100 = 200 bytes) at budget 10 keeps 10 CHARS (20 bytes),
    // never a byte slice. (Pass 1 pins the 3-char boundary; these pin
    // scale: char_indices must not degrade or miscount on long lines.)
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("huge.rs"), format!("{}\n", "z".repeat(5000))).unwrap();
    std::fs::write(temp.path().join("wide.rs"), format!("{}\n", "é".repeat(100))).unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["huge.rs#L1-L1"], "max_chars": 10})),
            read_call(2, json!({"ids": ["wide.rs#L1-L1"], "max_chars": 10})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 2);
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(!is_error(&responses[1]), "{:#}", responses[1]);
    let huge = tool_body(&responses[0]);
    let content = huge["nodes"][0]["content"].as_str().unwrap();
    assert_eq!(content, "z".repeat(10), "{huge:#}");
    assert_eq!(content.chars().count(), 10, "{huge:#}");
    assert_eq!(huge["nodes"][0]["truncated"], true, "{huge:#}");
    let wide = tool_body(&responses[1]);
    let wcontent = wide["nodes"][0]["content"].as_str().unwrap();
    assert_eq!(wcontent, "é".repeat(10), "{wide:#}");
    assert_eq!(wcontent.chars().count(), 10, "{wide:#}");
    assert_eq!(wcontent.len(), 20, "10 chars must occupy 20 bytes: {wide:#}");
    assert_eq!(wide["nodes"][0]["truncated"], true, "{wide:#}");
}
