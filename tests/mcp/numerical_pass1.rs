//! Pass 1 (numerical, Mission N1): hand-computed numeric oracles for MCP-owned
//! integer arithmetic, driven through the stdio protocol.
//!
//! Non-overlap contract: pass 2 pins accept/reject boundary SIDES
//! (limit 1/100 vs 0/101, budget 1/65536 vs 0, context 0/100 vs 101) and
//! error_api pins reject shapes; protocol.rs pins `chars <= 1` for one split.
//! This file pins EXACT values the earlier suites leave loose:
//!
//! * `code_read` per-ref split `max_chars / n`, remainder to FIRST refs
//!   (`per_ref_chars + usize::from(index < remainder)` in `tool_code_read`);
//! * context-window clamps `max(req - ctx, 1)` / `min(req + ctx, total)`;
//! * `truncate_chars` counts CHARS not bytes, with the exact truncated flag;
//! * `scan_line_window` maps an empty file to exactly one empty line;
//! * `zd` echoes the budget (`zd[0]`) and `zd[1]` equals the rendered snippet
//!   byte sum; `preview=full` without a budget defaults to exactly 8192;
//! * `tools/list` schema bounds and `ttlMs` match the parser literals.
//!
//! Every expectation below is hand-computed from the cited formula. There is
//! no float surface in `crates/ast-sgrep-mcp/src` (score fusion and token
//! costs live in core/plugins); this suite covers the MCP integer surface.

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
fn rpc_session(payloads: Vec<Value>, root: Option<&Path>) -> Vec<Value> {
    rpc_session_versioned(payloads, root, "2025-11-25")
}

fn rpc_session_versioned(
    payloads: Vec<Value>,
    root: Option<&Path>,
    protocol_version: &str,
) -> Vec<Value> {
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
                "protocolVersion": protocol_version,
                "capabilities": {},
                "clientInfo": {"name": "asgrep-mcp-numerical", "version": "0"}
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
fn per_ref_budgets_deal_remainder_to_first_refs() {
    // Hand-computed: max_chars=5 over 3 refs -> per_ref = 5/3 = 1,
    // remainder = 5%3 = 2, so budgets are [1+1, 1+1, 1+0] = [2, 2, 1].
    // Each line holds 3 chars, so every node truncates: "aa", "bb", "c".
    // Kills: remainder dropped ([1,1,1]), remainder to LAST refs ([1,1,2]),
    // and float division mutants.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("tri.rs"), "aaa\nbbb\nccc\n").unwrap();
    let responses = rpc_session(
        vec![read_call(
            1,
            json!({"ids": ["tri.rs#L1-L1", "tri.rs#L2-L2", "tri.rs#L3-L3"], "max_chars": 5}),
        )],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    let body = tool_body(&responses[0]);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3, "{body:#}");
    assert_eq!(nodes[0]["content"], "aa", "{body:#}");
    assert_eq!(nodes[0]["truncated"], true, "{body:#}");
    assert_eq!(nodes[1]["content"], "bb", "{body:#}");
    assert_eq!(nodes[1]["truncated"], true, "{body:#}");
    assert_eq!(nodes[2]["content"], "c", "{body:#}");
    assert_eq!(nodes[2]["truncated"], true, "{body:#}");
    let total: usize = nodes
        .iter()
        .map(|n| n["content"].as_str().unwrap().chars().count())
        .sum();
    assert_eq!(total, 5, "split must spend exactly max_chars: {body:#}");
}

#[test]
fn zero_budget_tail_yields_empty_truncated_node() {
    // Hand-computed: max_chars=1 over 2 refs -> per_ref = 0, remainder = 1,
    // so budgets are [1, 0]. The second node selects a non-empty line but
    // truncates to budget 0 -> ("", true), NOT an error and NOT ("", false).
    // Kills: div-by-zero guards that reject, and `truncated` computed as
    // `budget == 0 -> false`.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("pair.rs"), "one\ntwo\n").unwrap();
    let responses = rpc_session(
        vec![read_call(
            1,
            json!({"ids": ["pair.rs#L1-L1", "pair.rs#L2-L2"], "max_chars": 1}),
        )],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    let body = tool_body(&responses[0]);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes[0]["content"], "o", "{body:#}");
    assert_eq!(nodes[0]["truncated"], true, "{body:#}");
    assert_eq!(nodes[1]["content"], "", "{body:#}");
    assert_eq!(nodes[1]["truncated"], true, "{body:#}");
}

#[test]
fn context_window_clamps_to_file_edges() {
    // Hand-computed on a 5-line file: start = max(req-ctx, 1),
    // end = min(req+ctx, total).
    // L1-L1 ctx=100 -> start=max(1-100,1)=1, end=min(101,5)=5.
    // L5-L5 ctx=2   -> start=5-2=3,          end=min(7,5)=5.
    // Kills: missing top clamp (start 0/underflow), missing bottom clamp
    // (end 101/7), and off-by-one widening.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("five.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["five.rs#L1-L1"], "context_lines": 100})),
            read_call(2, json!({"ids": ["five.rs#L5-L5"], "context_lines": 2})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    let top = tool_body(&responses[0]);
    assert_eq!(top["nodes"][0]["lines"], json!({"start": 1, "end": 5}), "{top:#}");
    assert_eq!(top["nodes"][0]["content"], "l1\nl2\nl3\nl4\nl5", "{top:#}");
    let bottom = tool_body(&responses[1]);
    assert_eq!(bottom["nodes"][0]["lines"], json!({"start": 3, "end": 5}), "{bottom:#}");
    assert_eq!(bottom["nodes"][0]["content"], "l3\nl4\nl5", "{bottom:#}");
}

#[test]
fn context_window_asymmetric_at_edges() {
    // Hand-computed on a 5-line file:
    // L1-L2 ctx=1 -> start=max(0,1)=1, end=min(3,5)=3 -> "l1\nl2\nl3".
    // L4-L5 ctx=1 -> start=3,           end=min(6,5)=5 -> "l3\nl4\nl5".
    // Kills: symmetric-extension mutants that emit start=0 or end=6.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("five.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["five.rs#L1-L2"], "context_lines": 1})),
            read_call(2, json!({"ids": ["five.rs#L4-L5"], "context_lines": 1})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    let first = tool_body(&responses[0]);
    assert_eq!(first["nodes"][0]["lines"], json!({"start": 1, "end": 3}), "{first:#}");
    assert_eq!(first["nodes"][0]["content"], "l1\nl2\nl3", "{first:#}");
    let last = tool_body(&responses[1]);
    assert_eq!(last["nodes"][0]["lines"], json!({"start": 3, "end": 5}), "{last:#}");
    assert_eq!(last["nodes"][0]["content"], "l3\nl4\nl5", "{last:#}");
}

#[test]
fn truncation_counts_chars_not_bytes_with_exact_boundary() {
    // Hand-computed: line "ééé" is 3 chars / 6 bytes. `truncate_chars` walks
    // `char_indices`, so max_chars=2 keeps two CHARS ("éé", 4 bytes) with
    // truncated=true; a byte counter would keep "é" (2 bytes) or split a
    // code point. max_chars=3 is the exact boundary: `nth(3)` on 3 chars
    // is None, so the full line returns with truncated=false.
    // Kills: byte-slice truncation, and `>=` vs `>` flag mutants.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("uni.rs"), "ééé\n").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["uni.rs#L1-L1"], "max_chars": 2})),
            read_call(2, json!({"ids": ["uni.rs#L1-L1"], "max_chars": 3})),
            read_call(3, json!({"ids": ["uni.rs#L1-L1"], "max_chars": 1})),
        ],
        Some(temp.path()),
    );
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    let two = tool_body(&responses[0]);
    assert_eq!(two["nodes"][0]["content"], "éé", "{two:#}");
    assert_eq!(two["nodes"][0]["truncated"], true, "{two:#}");
    assert_eq!(
        two["nodes"][0]["content"].as_str().unwrap().len(),
        4,
        "two chars must occupy 4 bytes, not 2: {two:#}"
    );
    let exact = tool_body(&responses[1]);
    assert_eq!(exact["nodes"][0]["content"], "ééé", "{exact:#}");
    assert_eq!(exact["nodes"][0]["truncated"], false, "{exact:#}");
    let one = tool_body(&responses[2]);
    assert_eq!(one["nodes"][0]["content"], "é", "{one:#}");
    assert_eq!(one["nodes"][0]["truncated"], true, "{one:#}");
}

#[test]
fn empty_file_reads_as_single_empty_line() {
    // Hand-computed: `scan_line_window` maps a 0-byte file to total_lines=1
    // with one empty selected line, so L1-L1 succeeds with ("", false) and
    // lines {1,1}; L1-L2 fails because requested_end=2 > total=1.
    // Kills: total_lines=0 (L1-L1 would error) and total_lines=2 mutants.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("empty.rs"), "").unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["empty.rs#L1-L1"]})),
            read_call(2, json!({"ids": ["empty.rs#L1-L2"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], true, "{:#}", responses[1]);
    let body = tool_body(&responses[0]);
    assert_eq!(body["nodes"][0]["lines"], json!({"start": 1, "end": 1}), "{body:#}");
    assert_eq!(body["nodes"][0]["content"], "", "{body:#}");
    assert_eq!(body["nodes"][0]["truncated"], false, "{body:#}");
}

#[test]
fn zd_echoes_budget_and_spent_matches_snippet_bytes() {
    // Hand-computed wiring: `zd = [max_tokens, plan_cost]` where plan_cost
    // sums rendered body byte lengths, and each budgeted hit row carries its
    // body at index 4. So zd[0] must equal the requested budget literally,
    // and zd[1] must equal the independently re-summed snippet bytes.
    // `resend_seen` disables `~` elision so rows keep their bodies.
    // Kills: zd echo dropped/swapped, and cost-vs-body accounting drift.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 7})),
            search_call(2, json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 65536})),
        ],
        Some(temp.path()),
    );
    for (response, budget) in responses.iter().zip([7_u64, 65536_u64]) {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
        let body = tool_body(response);
        let hits = body["h"].as_array().unwrap();
        assert!(!hits.is_empty(), "{body:#}");
        let zd = body["zd"].as_array().unwrap();
        assert_eq!(zd.len(), 2, "{body:#}");
        assert_eq!(zd[0], budget, "zd[0] must echo the budget: {body:#}");
        let resummed: u64 = hits
            .iter()
            .map(|hit| hit.as_array().unwrap()[4].as_str().unwrap().len() as u64)
            .sum();
        assert_eq!(zd[1], resummed, "zd[1] must equal snippet bytes: {body:#}");
    }
}

#[test]
fn full_preview_defaults_to_8192_while_unbudgeted_short_has_no_zd() {
    // Hand-computed MCP literals: `preview=full` without `budget_tokens`
    // uses `max_tokens = 8192`, so zd[0]==8192 exactly. Unbudgeted short
    // and none previews go through `format_response_with_budget`, which
    // never sets `zd` -- its presence would mean the wrong renderer ran.
    // Kills: default-budget literal mutants and arm-swap mutants.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "preview": "full"})),
            search_call(2, json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "preview": "full", "budget_tokens": 100})),
            search_call(3, json!({"query": "target_symbol", "limit": 4, "resend_seen": true})),
            search_call(4, json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "preview": "none"})),
        ],
        Some(temp.path()),
    );
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    let defaulted = tool_body(&responses[0]);
    assert_eq!(defaulted["zd"][0], 8192, "{defaulted:#}");
    let explicit = tool_body(&responses[1]);
    assert_eq!(explicit["zd"][0], 100, "{explicit:#}");
    let short = tool_body(&responses[2]);
    assert!(short.get("zd").is_none(), "unbudgeted short must omit zd: {short:#}");
    let none = tool_body(&responses[3]);
    assert!(none.get("zd").is_none(), "preview=none must omit zd: {none:#}");
}

#[test]
fn tools_list_schema_bounds_match_parser() {
    // Hand-computed literals shared between `tools_catalog` and the
    // `bounded_usize` call sites: limit 1..=100, budget 1..=65536,
    // context 0..=100, max_chars 1..=1000000, ids 1..=20 items.
    // `ttlMs` is exactly TOOLS_LIST_TTL_MS = 3_600_000 on a 2026-07-28
    // session (protocol.rs only pins `> 0`).
    // Kills: schema/parser bound drift in either direction.
    let listed = rpc_session(
        vec![json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})],
        None,
    );
    let tools = listed[0]["result"]["tools"].as_array().unwrap().clone();
    let find = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} missing: {tools:#?}"))
            .clone()
    };
    let search = find("search");
    let props = &search["inputSchema"]["properties"];
    assert_eq!(props["limit"]["minimum"], 1, "{search:#}");
    assert_eq!(props["limit"]["maximum"], 100, "{search:#}");
    assert_eq!(props["budget_tokens"]["minimum"], 1, "{search:#}");
    assert_eq!(props["budget_tokens"]["maximum"], 65536, "{search:#}");
    let read = find("code_read");
    let read_props = &read["inputSchema"]["properties"];
    assert_eq!(read_props["ids"]["minItems"], 1, "{read:#}");
    assert_eq!(read_props["ids"]["maxItems"], 20, "{read:#}");
    assert_eq!(read_props["context_lines"]["minimum"], 0, "{read:#}");
    assert_eq!(read_props["context_lines"]["maximum"], 100, "{read:#}");
    assert_eq!(read_props["max_chars"]["minimum"], 1, "{read:#}");
    assert_eq!(read_props["max_chars"]["maximum"], 1_000_000, "{read:#}");

    let fresh = rpc_session_versioned(
        vec![json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})],
        None,
        "2026-07-28",
    );
    assert_eq!(fresh[0]["result"]["ttlMs"], 3_600_000, "{:#}", fresh[0]);
}
