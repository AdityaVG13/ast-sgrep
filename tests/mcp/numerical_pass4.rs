//! Pass 4 (numerical, Mission N4): end-to-end numeric drills over FULL stdio
//! sessions for MCP-owned integer arithmetic.
//!
//! Non-overlap contract: pass 1 pins EXACT single-shot values (splits 5/3 and
//! 1/2, edge clamps, 3-char char/byte boundary, `zd` at budgets 7/65536); pass
//! 2 pins ACCEPT/REJECT boundary SIDES and degenerate totality; pass 3 pins
//! RELATIONS between responses (never an exact value alone). This file runs
//! multi-call drills in ONE session per test with fresh hand-computed tables:
//!
//! * per-ref split chains 7/3 -> 8/3 and 11/4 -> 14/4 with exact per-ref
//!   strings, per-ref char/byte counts, and conservation totals;
//! * a center-window sweep (ctx 0..=3) with an exact (start, end, content,
//!   chars, bytes, lines) table;
//! * a limit sweep 1/2/3/100 on a 3-file tree with exact row counts 1/2/3/3,
//!   exact `zn`, and exact 32-byte snippets;
//! * a budget chain 3/50/200 with exact `zd[0]` echoes and exact total
//!   snippet bytes 0/32/96 with `zd[1]` conservation per step;
//! * an elision chain with exact `ze` absence/3/absence and exact 96/3/96
//!   snippet-byte totals;
//! * truncation chains on a 26-char ASCII line and a 5-char Greek line with
//!   exact char/byte tables and exact truncated flags;
//! * a search->read handoff: exact hit count, compact-id read with an exact
//!   32-byte body, then a 47-char two-line window truncated at caps 33/47/48
//!   with exact byte expectations.
//!
//! Assertions are exact ints, chars/bytes, line windows, and content
//! strings -- never message text.

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
                "clientInfo": {"name": "asgrep-mcp-numerical-n4", "version": "0"}
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

/// One live session where later calls depend on earlier responses: search,
/// then read the returned compact hit id, then fixed follow-up reads.
/// Returns every tool-call response in order.
fn rpc_handoff_session(root: &Path, search_args: Value, follow_ups: Vec<Value>) -> Vec<Value> {
    let mut command = Command::new(mcp_bin());
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .env("ASGREP_ROOT", root);
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
                "clientInfo": {"name": "asgrep-mcp-numerical-n4", "version": "0"}
            }
        }),
    );
    let init = recv(&mut stdout);
    assert_eq!(init["id"], "__init", "{init:#}");
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    send(&mut stdin, &search_call(1, search_args));
    let searched = recv(&mut stdout);
    assert!(!is_error(&searched), "{searched:#}");
    let found = tool_body(&searched);
    let hit_id = found["h"].as_array().unwrap()[0].as_array().unwrap()[0]
        .as_str()
        .unwrap()
        .to_owned();
    let mut responses = vec![searched];
    send(&mut stdin, &read_call(2, json!({"ids": [hit_id]})));
    responses.push(recv(&mut stdout));
    for (i, args) in follow_ups.iter().enumerate() {
        send(&mut stdin, &read_call(i as u32 + 3, args.clone()));
        responses.push(recv(&mut stdout));
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

fn snippet_bytes(body: &Value) -> Vec<usize> {
    body["h"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit.as_array().unwrap()[4].as_str().unwrap().len())
        .collect()
}

/// Tiny indexed tree: one file with a findable symbol.
fn indexed_tree_single() -> tempfile::TempDir {
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

/// Indexed tree with the same symbol in exactly three files.
fn indexed_tree_three() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for name in ["f0.rs", "f1.rs", "f2.rs"] {
        std::fs::write(
            source.join(name),
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        )
        .unwrap();
    }
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
fn split_chain_seven_then_eight_over_three_refs() {
    // Hand-computed: 7/3 -> per_ref=2, remainder=1, budgets [3,2,2];
    // 8/3 -> per_ref=2, remainder=2, budgets [3,3,2]. Each line holds 10
    // chars, so every node saturates and the chain spends exactly 7 then 8.
    // Kills: remainder dropped/rotated, per-step state leaking across calls.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("tri.rs"),
        "AAAAAAAAAA\nBBBBBBBBBB\nCCCCCCCCCC\n",
    )
    .unwrap();
    let ids = json!(["tri.rs#L1-L1", "tri.rs#L2-L2", "tri.rs#L3-L3"]);
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ids, "max_chars": 7})),
            read_call(
                2,
                json!({"ids": ["tri.rs#L1-L1", "tri.rs#L2-L2", "tri.rs#L3-L3"], "max_chars": 8}),
            ),
        ],
        Some(temp.path()),
    );
    let want = [
        (7_usize, ["AAA", "BB", "CC"]),
        (8_usize, ["AAA", "BBB", "CC"]),
    ];
    for (response, (cap, contents)) in responses.iter().zip(want) {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "{body:#}");
        let mut total = 0_usize;
        for (node, want_content) in nodes.iter().zip(contents) {
            let content = node["content"].as_str().unwrap();
            assert_eq!(content, want_content, "{body:#}");
            assert_eq!(node["truncated"], true, "{body:#}");
            let chars = content.chars().count();
            assert_eq!(chars, want_content.len(), "{body:#}");
            assert_eq!(content.len(), chars, "{body:#}");
            total += chars;
        }
        assert_eq!(total, cap, "split must spend exactly the cap: {body:#}");
    }
}

#[test]
fn split_chain_eleven_then_fourteen_over_four_refs() {
    // Hand-computed: 11/4 -> per_ref=2, remainder=3, budgets [3,3,3,2];
    // 14/4 -> per_ref=3, remainder=2, budgets [4,4,3,3]. Each line holds 10
    // chars, so every node saturates and the chain spends exactly 11 then 14.
    // Kills: remainder dealt to the wrong end, totals drifting across calls.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("quad.rs"),
        "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\ndddddddddd\n",
    )
    .unwrap();
    let responses = rpc_session(
        vec![
            read_call(
                1,
                json!({"ids": ["quad.rs#L1-L1", "quad.rs#L2-L2", "quad.rs#L3-L3", "quad.rs#L4-L4"], "max_chars": 11}),
            ),
            read_call(
                2,
                json!({"ids": ["quad.rs#L1-L1", "quad.rs#L2-L2", "quad.rs#L3-L3", "quad.rs#L4-L4"], "max_chars": 14}),
            ),
        ],
        Some(temp.path()),
    );
    let want = [
        (11_usize, ["aaa", "bbb", "ccc", "dd"]),
        (14_usize, ["aaaa", "bbbb", "ccc", "ddd"]),
    ];
    for (response, (cap, contents)) in responses.iter().zip(want) {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 4, "{body:#}");
        let mut total = 0_usize;
        for (node, want_content) in nodes.iter().zip(contents) {
            let content = node["content"].as_str().unwrap();
            assert_eq!(content, want_content, "{body:#}");
            assert_eq!(node["truncated"], true, "{body:#}");
            let chars = content.chars().count();
            assert_eq!(chars, want_content.len(), "{body:#}");
            assert_eq!(content.len(), chars, "{body:#}");
            total += chars;
        }
        assert_eq!(total, cap, "split must spend exactly the cap: {body:#}");
    }
}

#[test]
fn window_sweep_exact_table_at_center() {
    // Hand-computed on a 7-line file at L4-L4: start = max(4-ctx,1),
    // end = min(4+ctx,7). ctx0 -> (4,4) "r4" (2 chars); ctx1 -> (3,5), 8
    // chars; ctx2 -> (2,6), 14 chars; ctx3 -> (1,7), 20 chars. ASCII, so
    // bytes == chars; newlines == lines - 1.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("seven.rs"),
        "r1\nr2\nr3\nr4\nr5\nr6\nr7\n",
    )
    .unwrap();
    let calls: Vec<Value> = [0_u64, 1, 2, 3]
        .iter()
        .enumerate()
        .map(|(i, ctx)| {
            read_call(
                i as u32 + 1,
                json!({"ids": ["seven.rs#L4-L4"], "context_lines": ctx}),
            )
        })
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    let want = [
        (1_u64, 4_u64, 4_u64, "r4", 2_usize),
        (3_u64, 3_u64, 5_u64, "r3\nr4\nr5", 8_usize),
        (5_u64, 2_u64, 6_u64, "r2\nr3\nr4\nr5\nr6", 14_usize),
        (7_u64, 1_u64, 7_u64, "r1\nr2\nr3\nr4\nr5\nr6\nr7", 20_usize),
    ];
    assert_eq!(responses.len(), 4);
    for (response, (lines, start, end, content, chars)) in responses.iter().zip(want) {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        assert_eq!(
            body["nodes"][0]["lines"],
            json!({"start": start, "end": end}),
            "{body:#}"
        );
        let got = body["nodes"][0]["content"].as_str().unwrap();
        assert_eq!(got, content, "{body:#}");
        assert_eq!(got.chars().count(), chars, "{body:#}");
        assert_eq!(got.len(), chars, "{body:#}");
        assert_eq!(got.lines().count() as u64, lines, "{body:#}");
        assert_eq!(end - start + 1, lines, "{body:#}");
        assert_eq!(body["nodes"][0]["truncated"], false, "{body:#}");
    }
}

#[test]
fn limit_sweep_exact_counts_on_three_file_tree() {
    // Hand-computed: each of the 3 files defines target_symbol once, so the
    // query matches exactly 3 hits. Limits 1/2/3/100 yield exactly 1/2/3/3
    // rows with zn echoing the count; every snippet is the 32-byte line
    // "fn target_symbol() { helper(); }"; the head row is byte-identical
    // across all four steps. Kills: limit ignored/offset, count/zn drift.
    let temp = indexed_tree_three();
    let limits = [1_u64, 2, 3, 100];
    let calls: Vec<Value> = limits
        .iter()
        .enumerate()
        .map(|(i, limit)| {
            search_call(
                i as u32 + 1,
                json!({"query": "target_symbol", "limit": limit, "resend_seen": true}),
            )
        })
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), 4);
    let want_rows = [1_usize, 2, 3, 3];
    let mut heads = Vec::new();
    for (response, (limit, want)) in responses.iter().zip(limits.iter().zip(want_rows)) {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        let hits = body["h"].as_array().unwrap();
        assert_eq!(hits.len(), want, "limit {limit}: {body:#}");
        assert_eq!(body["zn"], want as u64, "limit {limit}: {body:#}");
        for hit in hits {
            let snippet = hit.as_array().unwrap()[4].as_str().unwrap();
            assert_eq!(snippet, "fn target_symbol() { helper(); }", "limit {limit}: {body:#}");
            assert_eq!(snippet.chars().count(), 32, "limit {limit}: {body:#}");
            assert_eq!(snippet.len(), 32, "limit {limit}: {body:#}");
        }
        heads.push(hits[0].clone());
    }
    for head in &heads[1..] {
        assert_eq!(*head, heads[0], "head row must be stable across limits");
    }
}

#[test]
fn budget_chain_exact_echo_and_byte_totals() {
    // Hand-computed on the 3-file tree (3 x 32-byte snippets): budget 3
    // funds zero snippet bytes (all metadata); budget 50 funds the first
    // 32-byte block only; budget 200 funds all three (96 bytes). Each step
    // echoes its budget in zd[0] exactly and zd[1] equals the re-summed
    // snippet bytes exactly. Kills: echo/budget drift, cost-vs-body drift.
    let temp = indexed_tree_three();
    let budgets = [3_u64, 50, 200];
    let calls: Vec<Value> = budgets
        .iter()
        .enumerate()
        .map(|(i, budget)| {
            search_call(
                i as u32 + 1,
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": budget}),
            )
        })
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), 3);
    let want_totals = [0_usize, 32, 96];
    let want_sorted = [vec![0, 0, 0], vec![0, 0, 32], vec![32, 32, 32]];
    for (response, ((budget, total), mut sorted)) in
        responses.iter().zip(budgets.iter().zip(want_totals).zip(want_sorted))
    {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        assert_eq!(body["h"].as_array().unwrap().len(), 3, "{body:#}");
        assert_eq!(body["zn"], 3, "{body:#}");
        let zd = body["zd"].as_array().unwrap();
        assert_eq!(zd.len(), 2, "{body:#}");
        assert_eq!(zd[0], *budget, "{body:#}");
        let bytes = snippet_bytes(&body);
        let resummed: usize = bytes.iter().sum();
        assert_eq!(resummed, total, "budget {budget}: {body:#}");
        assert_eq!(zd[1], total as u64, "budget {budget}: {body:#}");
        let mut got = bytes.clone();
        got.sort_unstable();
        sorted.sort_unstable();
        assert_eq!(got, sorted, "budget {budget}: {body:#}");
    }
}

#[test]
fn elision_chain_exact_counts_and_bytes() {
    // Hand-computed on the 3-file tree: the first unbudgeted search sends
    // all 3 x 32-byte snippets (96 bytes, no `ze` key); the immediate repeat
    // elides all three (`~`, 1 byte each, 3 bytes total, ze == 3 exactly);
    // a resend_seen repeat sends the 96 bytes again with no `ze` key.
    // Kills: elision off-by-one, ze missing/miscounted, resend leaking `~`.
    let temp = indexed_tree_three();
    let responses = rpc_session(
        vec![
            search_call(1, json!({"query": "target_symbol", "limit": 4})),
            search_call(2, json!({"query": "target_symbol", "limit": 4})),
            search_call(3, json!({"query": "target_symbol", "limit": 4, "resend_seen": true})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert!(!is_error(response), "{response:#}");
    }
    let first = tool_body(&responses[0]);
    assert_eq!(first["h"].as_array().unwrap().len(), 3, "{first:#}");
    assert_eq!(first["zn"], 3, "{first:#}");
    assert!(first.get("ze").is_none(), "first send must omit ze: {first:#}");
    let first_bytes: usize = snippet_bytes(&first).iter().sum();
    assert_eq!(first_bytes, 96, "{first:#}");

    let elided = tool_body(&responses[1]);
    assert_eq!(elided["h"].as_array().unwrap().len(), 3, "{elided:#}");
    assert_eq!(elided["zn"], 3, "{elided:#}");
    assert_eq!(elided["ze"], 3, "{elided:#}");
    for hit in elided["h"].as_array().unwrap() {
        let snippet = hit.as_array().unwrap()[4].as_str().unwrap();
        assert_eq!(snippet, "~", "{elided:#}");
        assert_eq!(snippet.len(), 1, "{elided:#}");
    }
    let elided_bytes: usize = snippet_bytes(&elided).iter().sum();
    assert_eq!(elided_bytes, 3, "{elided:#}");

    let resent = tool_body(&responses[2]);
    assert_eq!(resent["h"].as_array().unwrap().len(), 3, "{resent:#}");
    assert_eq!(resent["zn"], 3, "{resent:#}");
    assert!(resent.get("ze").is_none(), "resend must omit ze: {resent:#}");
    let resent_bytes: usize = snippet_bytes(&resent).iter().sum();
    assert_eq!(resent_bytes, 96, "{resent:#}");
    for hit in resent["h"].as_array().unwrap() {
        assert_eq!(
            hit.as_array().unwrap()[4].as_str().unwrap(),
            "fn target_symbol() { helper(); }",
            "{resent:#}"
        );
    }
}

#[test]
fn truncation_chains_exact_bytes_ascii_and_greek() {
    // Hand-computed: "a..z" is 26 chars / 26 bytes; caps 1/5/25/26/27 give
    // chars 1/5/25/26/26 with truncated true/true/true/false/false.
    // "αβγδε" is 5 chars / 10 bytes; caps 1/3/4/5/6 give chars 1/3/4/5/5 and
    // bytes 2/6/8/10/10 with truncated true/true/true/false/false.
    // Kills: byte-step truncation, >= vs > flag flips, split code points.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("alpha.rs"), "abcdefghijklmnopqrstuvwxyz\n").unwrap();
    std::fs::write(temp.path().join("greek.rs"), "αβγδε\n").unwrap();
    let ascii_caps = [1_u64, 5, 25, 26, 27];
    let greek_caps = [1_u64, 3, 4, 5, 6];
    let mut calls = Vec::new();
    for (i, cap) in ascii_caps.iter().enumerate() {
        calls.push(read_call(
            i as u32 + 1,
            json!({"ids": ["alpha.rs#L1-L1"], "max_chars": cap}),
        ));
    }
    for (i, cap) in greek_caps.iter().enumerate() {
        calls.push(read_call(
            i as u32 + 11,
            json!({"ids": ["greek.rs#L1-L1"], "max_chars": cap}),
        ));
    }
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), 10);
    let ascii_want = [
        ("a", 1_usize, 1_usize, true),
        ("abcde", 5, 5, true),
        ("abcdefghijklmnopqrstuvwxy", 25, 25, true),
        ("abcdefghijklmnopqrstuvwxyz", 26, 26, false),
        ("abcdefghijklmnopqrstuvwxyz", 26, 26, false),
    ];
    for (response, (content, chars, bytes, truncated)) in
        responses[..5].iter().zip(ascii_want)
    {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        let got = body["nodes"][0]["content"].as_str().unwrap();
        assert_eq!(got, content, "{body:#}");
        assert_eq!(got.chars().count(), chars, "{body:#}");
        assert_eq!(got.len(), bytes, "{body:#}");
        assert_eq!(body["nodes"][0]["truncated"], truncated, "{body:#}");
    }
    let greek_want = [
        ("α", 1_usize, 2_usize, true),
        ("αβγ", 3, 6, true),
        ("αβγδ", 4, 8, true),
        ("αβγδε", 5, 10, false),
        ("αβγδε", 5, 10, false),
    ];
    for (response, (content, chars, bytes, truncated)) in
        responses[5..].iter().zip(greek_want)
    {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        let got = body["nodes"][0]["content"].as_str().unwrap();
        assert_eq!(got, content, "{body:#}");
        assert_eq!(got.chars().count(), chars, "{body:#}");
        assert_eq!(got.len(), bytes, "{body:#}");
        assert_eq!(body["nodes"][0]["truncated"], truncated, "{body:#}");
    }
}

#[test]
fn search_to_read_handoff_exact_window_and_bytes() {
    // Hand-computed end-to-end drill on the single-file tree (line 1 is the
    // 32-char "fn target_symbol() { helper(); }", line 2 is the 14-char
    // "fn helper() {}"): search hits exactly 1 row; reading the compact hit
    // id returns lines {1,1} with the exact 32-byte body untruncated;
    // L1-L2 joins to 32+1+14 = 47 chars, so caps 33/47/48 yield 33/47/47
    // bytes with truncated true/false/false. Kills: count/window/byte drift
    // anywhere along the search -> resolve -> window -> truncate pipeline.
    let temp = indexed_tree_single();
    // One live session: the search populates the path registry, the compact
    // hit id resolves against it, then the fixed truncation follow-ups run.
    let responses = rpc_handoff_session(
        temp.path(),
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        vec![
            json!({"ids": ["src/lib.rs#L1-L2"], "max_chars": 33}),
            json!({"ids": ["src/lib.rs#L1-L2"], "max_chars": 47}),
            json!({"ids": ["src/lib.rs#L1-L2"], "max_chars": 48}),
        ],
    );
    assert_eq!(responses.len(), 5);
    for response in &responses {
        assert!(!is_error(response), "{response:#}");
    }
    let found = tool_body(&responses[0]);
    assert_eq!(found["h"].as_array().unwrap().len(), 1, "{found:#}");
    assert_eq!(found["zn"], 1, "{found:#}");
    let via_id = tool_body(&responses[1]);
    assert_eq!(
        via_id["nodes"][0]["lines"],
        json!({"start": 1, "end": 1}),
        "{via_id:#}"
    );
    let id_content = via_id["nodes"][0]["content"].as_str().unwrap();
    assert_eq!(id_content, "fn target_symbol() { helper(); }", "{via_id:#}");
    assert_eq!(id_content.chars().count(), 32, "{via_id:#}");
    assert_eq!(id_content.len(), 32, "{via_id:#}");
    assert_eq!(via_id["nodes"][0]["truncated"], false, "{via_id:#}");

    let full_two = "fn target_symbol() { helper(); }\nfn helper() {}";
    assert_eq!(full_two.chars().count(), 47);
    assert_eq!(full_two.len(), 47);
    let want = [
        ("fn target_symbol() { helper(); }\n", 33_usize, true),
        (full_two, 47_usize, false),
        (full_two, 47_usize, false),
    ];
    for (response, (content, bytes, truncated)) in responses[2..].iter().zip(want) {
        let body = tool_body(response);
        assert_eq!(
            body["nodes"][0]["lines"],
            json!({"start": 1, "end": 2}),
            "{body:#}"
        );
        let got = body["nodes"][0]["content"].as_str().unwrap();
        assert_eq!(got, content, "{body:#}");
        assert_eq!(got.chars().count(), bytes, "{body:#}");
        assert_eq!(got.len(), bytes, "{body:#}");
        assert_eq!(body["nodes"][0]["truncated"], truncated, "{body:#}");
    }
}
