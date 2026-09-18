//! Pass 3 (numerical, Mission N3): metamorphic relations over MCP-owned
//! integer arithmetic, driven through the stdio protocol.
//!
//! Non-overlap contract: pass 1 pins EXACT hand-computed values (per-ref
//! remainder deal, context clamps, char-vs-byte boundaries, `zd` accounting,
//! schema literals); pass 2 pins ACCEPT/REJECT boundary SIDES and degenerate
//! totality. This file asserts RELATIONS between responses -- never an exact
//! value on its own:
//!
//! * clamp idempotence (a saturated window re-clamped is a fixed point);
//! * per-ref split conservation (sum of per-ref chars == min(cap, fs total));
//! * truncation monotonicity (a larger cap never yields shorter output);
//! * window nesting (a smaller window's content nests inside a larger one);
//! * determinism across reruns (byte-identical tool bodies).
//!
//! Every expectation compares two or more MCP responses against each other
//! (or against an independent filesystem oracle), so a mutant that shifts all
//! exact values uniformly still breaks a relation. Assertions are
//! discriminants only (`isError`, ints, chars/bytes, line windows, raw-text
//! equality) -- never message text.

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
                "clientInfo": {"name": "asgrep-mcp-numerical-n3", "version": "0"}
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

/// Raw tool-body bytes: the determinism comparator.
fn tool_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"].as_str().unwrap()
}

fn is_error(response: &Value) -> bool {
    response["result"]["isError"].as_bool().unwrap()
}

fn node_content(body: &Value, index: usize) -> &str {
    body["nodes"][index]["content"].as_str().unwrap()
}

fn window(body: &Value, index: usize) -> (u64, u64) {
    (
        body["nodes"][index]["lines"]["start"].as_u64().unwrap(),
        body["nodes"][index]["lines"]["end"].as_u64().unwrap(),
    )
}

/// Tiny indexed tree with the same symbol in several files (multi-hit ranking).
fn indexed_tree_multi() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for name in ["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"] {
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
fn clamp_is_idempotent_from_top_edge() {
    // Relation: clamp twice == once. A request that saturates the file
    // (L2-L2 ctx=50 on a 7-line file) yields window W; re-requesting W with
    // the same ctx AND with a larger ctx must return the identical window,
    // content, and truncated flag. Kills: clamp applied to the wrong
    // endpoint on re-entry, ctx added twice, off-by-one rewidening.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("seven.rs"), "r1\nr2\nr3\nr4\nr5\nr6\nr7\n").unwrap();
    let first = rpc_session(
        vec![read_call(1, json!({"ids": ["seven.rs#L2-L2"], "context_lines": 50}))],
        Some(temp.path()),
    );
    assert!(!is_error(&first[0]), "{:#}", first[0]);
    let base = tool_body(&first[0]);
    let (start, end) = window(&base, 0);
    let widened = format!("seven.rs#L{start}-L{end}");
    let again = rpc_session(
        vec![
            read_call(1, json!({"ids": [widened.clone()], "context_lines": 50})),
            read_call(2, json!({"ids": [widened], "context_lines": 100})),
            read_call(3, json!({"ids": ["seven.rs#L2-L2"], "context_lines": 100})),
        ],
        Some(temp.path()),
    );
    for response in &again {
        assert!(!is_error(response), "{response:#}");
    }
    for (index, response) in again.iter().enumerate() {
        let body = tool_body(response);
        assert_eq!(window(&body, 0), (start, end), "re-clamp {index} moved the window: base {base:#} vs {body:#}");
        assert_eq!(node_content(&body, 0), node_content(&base, 0), "re-clamp {index} changed content");
        assert_eq!(body["nodes"][0]["truncated"], base["nodes"][0]["truncated"], "re-clamp {index} flipped truncated");
    }
}

#[test]
fn clamp_is_idempotent_from_bottom_edge() {
    // Mirror of the top-edge fixed point, saturating from L6-L6: the bottom
    // clamp binds first. Same relation (re-clamp with same/larger ctx is a
    // fixed point), opposite edge. Kills: asymmetric top/bottom clamp logic.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("seven.rs"), "r1\nr2\nr3\nr4\nr5\nr6\nr7\n").unwrap();
    let first = rpc_session(
        vec![read_call(1, json!({"ids": ["seven.rs#L6-L6"], "context_lines": 50}))],
        Some(temp.path()),
    );
    assert!(!is_error(&first[0]), "{:#}", first[0]);
    let base = tool_body(&first[0]);
    let (start, end) = window(&base, 0);
    let widened = format!("seven.rs#L{start}-L{end}");
    let again = rpc_session(
        vec![
            read_call(1, json!({"ids": [widened.clone()], "context_lines": 50})),
            read_call(2, json!({"ids": [widened], "context_lines": 100})),
            read_call(3, json!({"ids": ["seven.rs#L6-L6"], "context_lines": 100})),
        ],
        Some(temp.path()),
    );
    for response in &again {
        assert!(!is_error(response), "{response:#}");
    }
    for (index, response) in again.iter().enumerate() {
        let body = tool_body(response);
        assert_eq!(window(&body, 0), (start, end), "re-clamp {index} moved the window: base {base:#} vs {body:#}");
        assert_eq!(node_content(&body, 0), node_content(&base, 0), "re-clamp {index} changed content");
        assert_eq!(body["nodes"][0]["truncated"], base["nodes"][0]["truncated"], "re-clamp {index} flipped truncated");
    }
}

#[test]
fn per_ref_split_conserves_saturated_budget() {
    // Relation: sum of per-ref chars == max_chars when every line exceeds its
    // budget (4 refs x 12-char lines; caps 10 and 15 both under the 48-char
    // total, with nonzero remainders 2 and 3). Kills: remainder dropped,
    // remainder double-counted, per-ref rounding up.
    let temp = tempfile::tempdir().unwrap();
    let line = "a".repeat(12);
    let text = (0..4).map(|_| line.as_str()).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(temp.path().join("quad.rs"), text).unwrap();
    let ids: Vec<Value> = (1..=4).map(|n| json!(format!("quad.rs#L{n}-L{n}"))).collect();
    for cap in [10_u64, 15_u64] {
        let responses = rpc_session(
            vec![read_call(1, json!({"ids": ids, "max_chars": cap}))],
            Some(temp.path()),
        );
        assert!(!is_error(&responses[0]), "{:#}", responses[0]);
        let body = tool_body(&responses[0]);
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 4, "{body:#}");
        let total: usize = nodes
            .iter()
            .map(|n| n["content"].as_str().unwrap().chars().count())
            .sum();
        assert_eq!(total as u64, cap, "saturated split must spend exactly the cap: {body:#}");
        for (index, node) in nodes.iter().enumerate() {
            assert_eq!(node["truncated"], true, "node {index} at cap {cap} must truncate: {body:#}");
        }
    }
}

#[test]
fn per_ref_split_conserves_unsaturated_total() {
    // Relation: sum of per-ref chars == min(cap, total) when the cap exceeds
    // the file content. Total comes from an independent filesystem oracle
    // (not MCP), and two different caps must agree with each other and with
    // it. Kills: padding to the cap, truncation when budget suffices.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("trio.rs"), "ab\ncde\nf\n").unwrap();
    let fs_total: usize = std::fs::read_to_string(temp.path().join("trio.rs"))
        .unwrap()
        .lines()
        .map(|line| line.chars().count())
        .sum();
    let ids = json!(["trio.rs#L1-L1", "trio.rs#L2-L2", "trio.rs#L3-L3"]);
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ids, "max_chars": 100})),
            read_call(2, json!({"ids": ["trio.rs#L1-L1", "trio.rs#L2-L2", "trio.rs#L3-L3"], "max_chars": 1000})),
        ],
        Some(temp.path()),
    );
    let mut sums = Vec::new();
    for response in &responses {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        let nodes = body["nodes"].as_array().unwrap();
        let total: usize = nodes
            .iter()
            .map(|n| n["content"].as_str().unwrap().chars().count())
            .sum();
        assert_eq!(total, fs_total, "unsaturated split must equal the fs total: {body:#}");
        for node in nodes {
            assert_eq!(node["truncated"], false, "unsaturated node must not truncate: {body:#}");
        }
        sums.push(total);
    }
    assert_eq!(sums[0], sums[1], "unsaturated sum must be cap-independent");
}

#[test]
fn truncation_is_monotone_in_cap_single_ref() {
    // Relation: on one 20-char line, char length and byte length are
    // non-decreasing in the cap; each shorter content is a prefix of the next
    // (truncate-then-extend consistency); the truncated flag flips true->false
    // at most once; caps at/above saturation agree exactly. Kills: reversed
    // comparisons, byte/char confusion across caps, flag flapping.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("alpha.rs"), "abcdefghijklmnopqrst\n").unwrap();
    let caps = [1_u64, 3, 8, 19, 20, 21];
    let calls: Vec<Value> = caps
        .iter()
        .enumerate()
        .map(|(i, cap)| read_call(i as u32 + 1, json!({"ids": ["alpha.rs#L1-L1"], "max_chars": cap})))
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    let bodies: Vec<Value> = responses
        .iter()
        .map(|response| {
            assert!(!is_error(response), "{response:#}");
            tool_body(response)
        })
        .collect();
    let contents: Vec<&str> = bodies.iter().map(|body| node_content(body, 0)).collect();
    for pair in contents.windows(2) {
        assert!(pair[1].len() >= pair[0].len(), "bytes must not shrink: {:?} vs {:?}", pair[0], pair[1]);
        assert!(
            pair[1].chars().count() >= pair[0].chars().count(),
            "chars must not shrink: {:?} vs {:?}",
            pair[0],
            pair[1]
        );
        assert!(pair[1].starts_with(pair[0]), "shorter content must prefix the longer: {:?} vs {:?}", pair[0], pair[1]);
    }
    let flags: Vec<bool> = bodies.iter().map(|body| body["nodes"][0]["truncated"].as_bool().unwrap()).collect();
    let first_false = flags.iter().position(|flag| !flag);
    if let Some(cut) = first_false {
        assert!(flags[cut..].iter().all(|flag| !flag), "truncated must not flip back to true: {flags:?}");
        assert!(flags[..cut].iter().all(|flag| *flag), "truncated must be true before saturation: {flags:?}");
    }
    assert_eq!(contents[4], contents[5], "caps at/above saturation must agree");
    assert!(!flags[4] && !flags[5], "saturated reads must not truncate: {flags:?}");
}

#[test]
fn truncation_total_is_monotone_in_cap_multi_ref() {
    // Relation: over 3 refs x 10-char lines, each node's char length and the
    // total are non-decreasing in the cap (per-node budgets q+[i<r] gain at
    // most one char per cap step and never shrink); totals at/above
    // saturation agree. Kills: per-node budget shrinking as the cap grows,
    // remainder rotation that steals from early refs.
    let temp = tempfile::tempdir().unwrap();
    let text = (0..3).map(|_| "0123456789").collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(temp.path().join("digits.rs"), text).unwrap();
    let fs_total: usize = std::fs::read_to_string(temp.path().join("digits.rs"))
        .unwrap()
        .lines()
        .map(|line| line.chars().count())
        .sum();
    let caps = [2_u64, 5, 9, 30, 31];
    let calls: Vec<Value> = caps
        .iter()
        .enumerate()
        .map(|(i, cap)| {
            read_call(
                i as u32 + 1,
                json!({"ids": ["digits.rs#L1-L1", "digits.rs#L2-L2", "digits.rs#L3-L3"], "max_chars": cap}),
            )
        })
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    let mut per_node: Vec<Vec<usize>> = Vec::new();
    let mut totals: Vec<usize> = Vec::new();
    for response in &responses {
        assert!(!is_error(response), "{response:#}");
        let body = tool_body(response);
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "{body:#}");
        let lens: Vec<usize> = nodes.iter().map(|n| n["content"].as_str().unwrap().chars().count()).collect();
        totals.push(lens.iter().sum());
        per_node.push(lens);
    }
    for pair in totals.windows(2) {
        assert!(pair[1] >= pair[0], "total chars must not shrink in cap: {totals:?}");
    }
    for node in 0..3 {
        for step in 1..per_node.len() {
            assert!(
                per_node[step][node] >= per_node[step - 1][node],
                "node {node} shrank from cap {} to {}: {per_node:?}",
                caps[step - 1],
                caps[step]
            );
        }
    }
    assert_eq!(totals[3], totals[4], "saturated totals must agree: {totals:?}");
    assert_eq!(totals[4], fs_total, "saturated total must equal the fs total: {totals:?} vs {fs_total}");
}

#[test]
fn window_nesting_contains_smaller_context() {
    // Relation: at L5-L5 with ctx in {0,1,2,4}, starts are non-increasing,
    // ends non-decreasing, and each smaller content is a substring of the
    // next (line-set nesting). Kills: ctx applied to the wrong side,
    // content/window mismatch, non-monotone widening.
    let temp = tempfile::tempdir().unwrap();
    let text = (1..=9).map(|n| format!("L{n}")).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(temp.path().join("nine.rs"), text).unwrap();
    let ctxs = [0_u64, 1, 2, 4];
    let calls: Vec<Value> = ctxs
        .iter()
        .enumerate()
        .map(|(i, ctx)| read_call(i as u32 + 1, json!({"ids": ["nine.rs#L5-L5"], "context_lines": ctx})))
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    let bodies: Vec<Value> = responses
        .iter()
        .map(|response| {
            assert!(!is_error(response), "{response:#}");
            tool_body(response)
        })
        .collect();
    let windows: Vec<(u64, u64)> = bodies.iter().map(|body| window(body, 0)).collect();
    for pair in windows.windows(2) {
        assert!(pair[1].0 <= pair[0].0, "start must not move right as ctx grows: {windows:?}");
        assert!(pair[1].1 >= pair[0].1, "end must not move left as ctx grows: {windows:?}");
    }
    let contents: Vec<&str> = bodies.iter().map(|body| node_content(body, 0)).collect();
    for pair in contents.windows(2) {
        assert!(pair[1].contains(pair[0]), "smaller window must nest in larger: {:?} vs {:?}", pair[0], pair[1]);
    }
}

#[test]
fn window_nesting_contains_subrange_and_equivalent_spellings() {
    // Relations: (a) with ctx=0, L5-L5 nests in L4-L6 nests in L3-L7, both in
    // line ranges and in content substrings; (b) the same effective window
    // spelled two ways agrees exactly -- (L5-L5,ctx=1) == (L4-L6,ctx=0) and
    // (L4-L6,ctx=1) == (L3-L7,ctx=0). Kills: range/context conflation, window
    // computed from the wrong endpoint, spelling-dependent output.
    let temp = tempfile::tempdir().unwrap();
    let text = (1..=9).map(|n| format!("L{n}")).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(temp.path().join("nine.rs"), text).unwrap();
    let responses = rpc_session(
        vec![
            read_call(1, json!({"ids": ["nine.rs#L5-L5"], "context_lines": 0})),
            read_call(2, json!({"ids": ["nine.rs#L4-L6"], "context_lines": 0})),
            read_call(3, json!({"ids": ["nine.rs#L3-L7"], "context_lines": 0})),
            read_call(4, json!({"ids": ["nine.rs#L5-L5"], "context_lines": 1})),
            read_call(5, json!({"ids": ["nine.rs#L4-L6"], "context_lines": 1})),
        ],
        Some(temp.path()),
    );
    let bodies: Vec<Value> = responses
        .iter()
        .map(|response| {
            assert!(!is_error(response), "{response:#}");
            tool_body(response)
        })
        .collect();
    let windows: Vec<(u64, u64)> = bodies[..3].iter().map(|body| window(body, 0)).collect();
    assert!(windows[1].0 <= windows[0].0 && windows[1].1 >= windows[0].1, "L4-L6 must contain L5-L5: {windows:?}");
    assert!(windows[2].0 <= windows[1].0 && windows[2].1 >= windows[1].1, "L3-L7 must contain L4-L6: {windows:?}");
    let contents: Vec<&str> = bodies.iter().map(|body| node_content(body, 0)).collect();
    assert!(contents[1].contains(contents[0]), "L4-L6 must nest L5-L5 content: {:?} vs {:?}", contents[0], contents[1]);
    assert!(contents[2].contains(contents[1]), "L3-L7 must nest L4-L6 content: {:?} vs {:?}", contents[1], contents[2]);
    assert_eq!(window(&bodies[3], 0), windows[1], "ctx-spelling must match range-spelling: {:#} vs {:#}", bodies[3], bodies[1]);
    assert_eq!(contents[3], contents[1], "ctx-spelling content must match range-spelling");
    assert_eq!(window(&bodies[4], 0), windows[2], "ctx-spelling must match range-spelling: {:#} vs {:#}", bodies[4], bodies[2]);
    assert_eq!(contents[4], contents[2], "ctx-spelling content must match range-spelling");
}

#[test]
fn rerun_is_byte_identical_within_session() {
    // Relation: repeating a call in one session returns byte-identical tool
    // bodies, for both code_read and (resend_seen, budgeted) keyword_search.
    // Kills: per-call counters, nondeterministic ordering, elision state
    // leaking into resend_seen output.
    let temp = indexed_tree_multi();
    std::fs::write(temp.path().join("pin.rs"), "aaa\nbbb\n").unwrap();
    let search_args = json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 64});
    let read_args = json!({"ids": ["pin.rs#L1-L1", "pin.rs#L2-L2"], "max_chars": 4});
    let responses = rpc_session(
        vec![
            read_call(1, read_args.clone()),
            read_call(2, read_args),
            search_call(3, search_args.clone()),
            search_call(4, search_args),
        ],
        Some(temp.path()),
    );
    for response in &responses {
        assert!(!is_error(response), "{response:#}");
    }
    assert_eq!(tool_text(&responses[0]), tool_text(&responses[1]), "repeated code_read must be byte-identical");
    assert_eq!(tool_text(&responses[2]), tool_text(&responses[3]), "repeated keyword_search must be byte-identical");
}

#[test]
fn rerun_is_byte_identical_across_sessions() {
    // Relation: the same calls in FRESH processes return byte-identical tool
    // bodies (no pid/timestamp/counter entropy, stable ranking). Kills:
    // time-seeded ids, hash-order iteration, cross-run ranking flips.
    let temp = indexed_tree_multi();
    std::fs::write(temp.path().join("pin.rs"), "aaa\nbbb\n").unwrap();
    let search_args = json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 64});
    let read_args = json!({"ids": ["pin.rs#L1-L1", "pin.rs#L2-L2"], "max_chars": 4});
    let first = rpc_session(vec![read_call(1, read_args.clone()), search_call(2, search_args.clone())], Some(temp.path()));
    let second = rpc_session(vec![read_call(1, read_args), search_call(2, search_args)], Some(temp.path()));
    for response in first.iter().chain(second.iter()) {
        assert!(!is_error(response), "{response:#}");
    }
    assert_eq!(tool_text(&first[0]), tool_text(&second[0]), "code_read must be byte-identical across processes");
    assert_eq!(tool_text(&first[1]), tool_text(&second[1]), "keyword_search must be byte-identical across processes");
}

#[test]
fn limit_is_monotone_in_rows_with_stable_head() {
    // Relation: hit-row counts are non-decreasing in limit, each count fits
    // its limit, and the head row is identical across limits (limit truncates
    // a stable ranking rather than re-ranking). Kills: limit ignored,
    // limit-as-offset, ranking that depends on the limit.
    let temp = indexed_tree_multi();
    let limits = [1_u64, 2, 4, 100];
    // `resend_seen` disables cross-call `~` elision so head rows compare
    // snippet-to-snippet rather than snippet-to-elision-marker.
    let calls: Vec<Value> = limits
        .iter()
        .enumerate()
        .map(|(i, limit)| search_call(i as u32 + 1, json!({"query": "target_symbol", "limit": limit, "resend_seen": true})))
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    let bodies: Vec<Value> = responses
        .iter()
        .map(|response| {
            assert!(!is_error(response), "{response:#}");
            tool_body(response)
        })
        .collect();
    let counts: Vec<usize> = bodies.iter().map(|body| body["h"].as_array().unwrap().len()).collect();
    assert!(counts.iter().all(|count| *count > 0), "every limit must hit: {counts:?}");
    for (index, pair) in counts.windows(2).enumerate() {
        assert!(pair[1] >= pair[0], "row counts must not shrink from limit {} to {}: {counts:?}", limits[index], limits[index + 1]);
    }
    for (count, limit) in counts.iter().zip(limits) {
        assert!((*count as u64) <= limit, "count {count} exceeds its limit {limit}: {counts:?}");
    }
    let heads: Vec<&Value> = bodies.iter().map(|body| &body["h"].as_array().unwrap()[0]).collect();
    for head in &heads[1..] {
        assert_eq!(*head, heads[0], "head row must be stable across limits");
    }
}

#[test]
fn multibyte_truncation_scales_chars_not_bytes() {
    // Relation: on a 24xU+00E9 line, char counts are non-decreasing in the
    // cap, every byte length is exactly 2x its char count (char-step
    // truncation at every cap, not just pass 1's 3-char boundary), shorter
    // contents prefix longer ones, and caps at/above saturation agree with
    // truncated=false. Kills: byte-step truncation, mixed char/byte regimes
    // across caps, split code points.
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("wide.rs"), format!("{}\n", "é".repeat(24))).unwrap();
    let caps = [1_u64, 5, 10, 23, 24, 25];
    let calls: Vec<Value> = caps
        .iter()
        .enumerate()
        .map(|(i, cap)| read_call(i as u32 + 1, json!({"ids": ["wide.rs#L1-L1"], "max_chars": cap})))
        .collect();
    let responses = rpc_session(calls, Some(temp.path()));
    let bodies: Vec<Value> = responses
        .iter()
        .map(|response| {
            assert!(!is_error(response), "{response:#}");
            tool_body(response)
        })
        .collect();
    let contents: Vec<&str> = bodies.iter().map(|body| node_content(body, 0)).collect();
    for (content, cap) in contents.iter().zip(caps) {
        assert_eq!(content.len(), 2 * content.chars().count(), "cap {cap}: bytes must be 2x chars: {content:?}");
    }
    for pair in contents.windows(2) {
        assert!(
            pair[1].chars().count() >= pair[0].chars().count(),
            "chars must not shrink: {:?} vs {:?}",
            pair[0],
            pair[1]
        );
        assert!(pair[1].starts_with(pair[0]), "shorter content must prefix the longer: {:?} vs {:?}", pair[0], pair[1]);
    }
    assert_eq!(contents[4], contents[5], "caps at/above saturation must agree");
    assert_eq!(bodies[4]["nodes"][0]["truncated"], false, "{:#}", bodies[4]);
    assert_eq!(bodies[5]["nodes"][0]["truncated"], false, "{:#}", bodies[5]);
}
