//! Pass 4 (oracle-foundry, Mission 3): L4 end-to-end oracles for
//! ast-sgrep-mcp, driven as FULL stdio sessions over indexed tempdir trees.
//!
//! Non-overlap contract: pass 2 pins argument boundaries, pass 3 pins
//! relations between calls (alias, prefix stability, normalization, elision
//! opt-outs, adversarial shapes, pipelining, envelope topology), and
//! `protocol.rs` pins handshake/negotiation, discovery, structured content,
//! tool names, per-channel kinds, single-id expansion, read windows, schema
//! rejection, sandbox escapes, byte-stability, elision, miss envelopes, and
//! cancellation. This file pins NONE of those again. Instead it pins
//! end-to-end FLOWS:
//!
//! * full chains: tools/list -> search -> code_read in ONE session, with the
//!   tool names and the read ids consumed from earlier responses in the chain;
//! * search-then-read consistency: every hit id resolves, node count matches
//!   hit count, and read-back file paths agree with the search `p` table;
//! * error taxonomy end-to-end: unknown method vs unknown tool vs bad args vs
//!   missing root in one session, plus same-session recovery without restart;
//! * session determinism: an identical chain rerun in a fresh process returns
//!   identical bytes;
//! * fail-closed: empty trees and missing roots yield diagnostic envelopes or
//!   uniform tool errors, never silent success with fabricated content.
//!
//! Discriminants are `isError` booleans, JSON-RPC codes, envelope shapes (key
//! presence, tuple widths, id multisets), and counts -- never message text.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
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

/// Drive several requests through ONE server process, strictly sequential
/// (send one, read one), so response order matches request order.
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
                "clientInfo": {"name": "asgrep-mcp-pass4", "version": "0"}
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

fn tool_call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
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

fn index_tree(path: &Path) {
    ast_sgrep_core::Indexer::new(ast_sgrep_core::IndexOptions {
        root: path.to_path_buf(),
        ..ast_sgrep_core::IndexOptions::default()
    })
    .unwrap()
    .index_all()
    .unwrap();
}

/// Single-file indexed tree with one findable symbol.
fn indexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").unwrap();
    index_tree(temp.path());
    temp
}

/// Three files sharing one lexical token, indexed.
fn multi_file_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for index in 1..=3 {
        std::fs::write(
            source.join(format!("probe{index}.rs")),
            format!("fn shared_probe_{index}() {{}}\n"),
        )
        .unwrap();
    }
    index_tree(temp.path());
    temp
}

/// Two files sharing one lexical token, NOT indexed (for the index lifecycle).
fn unindexed_tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for index in 1..=2 {
        std::fs::write(
            source.join(format!("cycle{index}.rs")),
            format!("fn lifecycle_probe_{index}() {{}}\n"),
        )
        .unwrap();
    }
    temp
}

/// Distinct `<path_id>` prefixes across compact hit ids.
fn distinct_hit_paths(hits: &[Value]) -> HashSet<String> {
    hits.iter()
        .map(|hit| {
            hit[0]
                .as_str()
                .expect("hit id is a string")
                .rsplit_once(':')
                .expect("compact id carries a path id")
                .0
                .to_owned()
        })
        .collect()
}

#[test]
fn full_chain_list_search_read_in_one_session() {
    // L4 chain: tools/list -> keyword_search -> code_read, each step consuming
    // the previous step's output inside a single session.
    let temp = indexed_tree();
    let listed = rpc_session(
        vec![json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})],
        Some(temp.path()),
    );
    let tools = listed[0]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 8, "{:#}", listed[0]);
    let names: HashSet<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains("keyword_search"), "{names:?}");
    assert!(names.contains("code_read"), "{names:?}");

    let searched = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(searched[1]["result"]["isError"], false, "{:#}", searched[1]);
    let envelope = tool_body(&searched[1]);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    let compact_id = hits[0][0].as_str().expect("compact id").to_owned();

    let read = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(3, "code_read", json!({"ids": [compact_id]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(read.len(), 3);
    assert_eq!(read[2]["result"]["isError"], false, "{:#}", read[2]);
    let body = tool_body(&read[2]);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "{body:#}");
    assert!(nodes[0]["id"].as_str().unwrap().contains("#L"), "{body:#}");
    assert!(nodes[0]["lines"]["start"].as_u64().unwrap() >= 1);
    assert!(
        nodes[0]["lines"]["end"].as_u64().unwrap()
            >= nodes[0]["lines"]["start"].as_u64().unwrap()
    );
    assert!(!nodes[0]["content"].as_str().unwrap().is_empty(), "{body:#}");
}

#[test]
fn tool_names_consumed_from_list_response_drive_search_and_read() {
    // A realistic client never hardcodes tool names: it reads them from
    // tools/list and dispatches by the advertised strings.
    let temp = indexed_tree();
    let listed = rpc_session(
        vec![json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})],
        Some(temp.path()),
    );
    let names: Vec<String> = listed[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    let search_name = names.iter().find(|name| *name == "keyword_search").unwrap().clone();
    let read_name = names.iter().find(|name| *name == "code_read").unwrap().clone();

    let searched = rpc_session(
        vec![tool_call(
            2,
            &search_name,
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        )],
        Some(temp.path()),
    );
    assert_eq!(searched[0]["result"]["isError"], false, "{:#}", searched[0]);
    let envelope = tool_body(&searched[0]);
    let compact_id = envelope["h"][0][0].as_str().unwrap().to_owned();

    let read = rpc_session(
        vec![
            tool_call(
                2,
                &search_name,
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(3, &read_name, json!({"ids": [compact_id]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(read[1]["result"]["isError"], false, "{:#}", read[1]);
    assert_eq!(tool_body(&read[1])["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn search_then_read_every_hit_resolves_with_matching_count() {
    // Consistency: one code_read fan-out over every hit id returns exactly one
    // node per hit, each with a well-formed window and a non-empty body.
    let temp = multi_file_tree();
    let searched = rpc_session(
        vec![tool_call(
            1,
            "keyword_search",
            json!({"query": "shared_probe", "limit": 8, "resend_seen": true}),
        )],
        Some(temp.path()),
    );
    assert_eq!(searched[0]["result"]["isError"], false, "{:#}", searched[0]);
    let envelope = tool_body(&searched[0]);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    let ids: Vec<Value> = hits.iter().map(|hit| hit[0].clone()).collect();

    let read = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "shared_probe", "limit": 8, "resend_seen": true}),
            ),
            tool_call(2, "code_read", json!({"ids": ids})),
        ],
        Some(temp.path()),
    );
    assert_eq!(read[1]["result"]["isError"], false, "{:#}", read[1]);
    let body = tool_body(&read[1]);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), hits.len(), "{body:#}");
    for node in nodes {
        assert!(node["id"].as_str().unwrap().contains("#L"), "{node:#}");
        let start = node["lines"]["start"].as_u64().unwrap();
        let end = node["lines"]["end"].as_u64().unwrap();
        assert!(start >= 1 && end >= start, "{node:#}");
        assert!(!node["content"].as_str().unwrap().is_empty(), "{node:#}");
    }
}

#[test]
fn search_path_table_agrees_with_read_node_files() {
    // Consistency across the wire boundary: the file portion of every
    // read-back node id is a member of the search `p` table (resolved exactly
    // the way the server resolves it, including folded roots), and all three
    // files of the tree are represented.
    let temp = multi_file_tree();
    let searched = rpc_session(
        vec![tool_call(
            1,
            "keyword_search",
            json!({"query": "shared_probe", "limit": 8, "resend_seen": true}),
        )],
        Some(temp.path()),
    );
    let envelope = tool_body(&searched[0]);
    let hits = envelope["h"].as_array().unwrap();
    assert_eq!(distinct_hit_paths(hits).len(), 3, "{envelope:#}");
    let table: HashMap<String, String> = ast_sgrep_plugins::resolve_compact_paths(&envelope)
        .into_iter()
        .collect();
    assert_eq!(table.len(), 3, "{envelope:#}");
    let table_paths: HashSet<&String> = table.values().collect();
    let ids: Vec<Value> = hits.iter().map(|hit| hit[0].clone()).collect();

    let read = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "shared_probe", "limit": 8, "resend_seen": true}),
            ),
            tool_call(2, "code_read", json!({"ids": ids})),
        ],
        Some(temp.path()),
    );
    assert_eq!(read[1]["result"]["isError"], false, "{:#}", read[1]);
    let nodes = tool_body(&read[1])["nodes"].clone();
    let mut read_files = HashSet::new();
    for node in nodes.as_array().unwrap() {
        let file = node["id"]
            .as_str()
            .unwrap()
            .split("#L")
            .next()
            .unwrap()
            .to_owned();
        assert!(table_paths.contains(&file), "{node:#} not in {table_paths:?}");
        read_files.insert(file);
    }
    assert_eq!(read_files.len(), 3, "every tree file must round-trip");
}

#[test]
fn ast_search_chain_reads_pattern_hits() {
    // The structural channel feeds code_read too: every ast_search hit carries
    // kind `p`, and its compact id expands to a non-empty node.
    let temp = indexed_tree();
    let searched = rpc_session(
        vec![tool_call(
            1,
            "ast_search",
            json!({"query": "fn $NAME() { $$$BODY }", "limit": 4, "resend_seen": true}),
        )],
        Some(temp.path()),
    );
    assert_eq!(searched[0]["result"]["isError"], false, "{:#}", searched[0]);
    let envelope = tool_body(&searched[0]);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    for hit in hits {
        assert_eq!(hit[1], "p", "{hit:#}");
    }
    let compact_id = hits[0][0].as_str().unwrap().to_owned();

    let read = rpc_session(
        vec![
            tool_call(
                1,
                "ast_search",
                json!({"query": "fn $NAME() { $$$BODY }", "limit": 4, "resend_seen": true}),
            ),
            tool_call(2, "code_read", json!({"ids": [compact_id]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(read[1]["result"]["isError"], false, "{:#}", read[1]);
    let body = tool_body(&read[1]);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1);
    assert!(
        !body["nodes"][0]["content"].as_str().unwrap().is_empty(),
        "{body:#}"
    );
}

#[test]
fn index_lifecycle_miss_then_index_then_hit_then_read() {
    // Full lifecycle in one session: miss (empty_index) -> index_repo indexes
    // exactly the 2 tree files -> the same query hits both files -> read back.
    let temp = unindexed_tree();
    let miss = rpc_session(
        vec![tool_call(
            1,
            "keyword_search",
            json!({"query": "lifecycle_probe", "limit": 8, "resend_seen": true}),
        )],
        Some(temp.path()),
    );
    assert_eq!(miss[0]["result"]["isError"], false, "{:#}", miss[0]);
    let miss_body = tool_body(&miss[0]);
    assert_eq!(miss_body["why"], "empty_index", "{miss_body:#}");
    assert_eq!(miss_body["zn"], 0);
    assert_eq!(miss_body["h"].as_array().unwrap().len(), 0);

    let indexed = rpc_session(vec![tool_call(2, "index_repo", json!({}))], Some(temp.path()));
    assert_eq!(indexed[0]["result"]["isError"], false, "{:#}", indexed[0]);
    assert_eq!(tool_body(&indexed[0])["files_indexed"], 2);

    let searched = rpc_session(
        vec![tool_call(
            3,
            "keyword_search",
            json!({"query": "lifecycle_probe", "limit": 8, "resend_seen": true}),
        )],
        Some(temp.path()),
    );
    assert_eq!(searched[0]["result"]["isError"], false, "{:#}", searched[0]);
    let envelope = tool_body(&searched[0]);
    let hits = envelope["h"].as_array().unwrap();
    assert!(envelope["zn"].as_u64().unwrap() >= 2, "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    assert_eq!(distinct_hit_paths(hits).len(), 2, "{envelope:#}");
    let compact_id = hits[0][0].as_str().unwrap().to_owned();

    // The chained read replays search first so the compact path registry (a
    // per-process session map) holds the id, exactly like a real client chain.
    let read = rpc_session(
        vec![
            tool_call(
                3,
                "keyword_search",
                json!({"query": "lifecycle_probe", "limit": 8, "resend_seen": true}),
            ),
            tool_call(4, "code_read", json!({"ids": [compact_id]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(read[1]["result"]["isError"], false, "{:#}", read[1]);
    assert_eq!(tool_body(&read[1])["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn error_taxonomy_end_to_end_method_vs_tool_vs_args_vs_root() {
    // One session, four failure kinds: unknown JSON-RPC methods are top-level
    // -32601 with no `result`; unknown tools, mistyped args, and missing roots
    // are uniform tool errors with no top-level `error`.
    let temp = indexed_tree();
    let missing_root = temp.path().join("does_not_exist").display().to_string();
    let responses = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"missing"}),
            tool_call(2, "no_such_tool", json!({})),
            tool_call(3, "index_status", json!({"root": 42})),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "root": missing_root}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["error"]["code"], -32601, "{:#}", responses[0]);
    assert!(responses[0].get("result").is_none(), "{:#}", responses[0]);
    for response in &responses[1..] {
        assert_tool_error_shape(response);
    }
}

#[test]
fn missing_root_fails_closed_across_tools_then_recovers() {
    // A nonexistent per-call root fails every tool uniformly, and the session
    // serves a valid call immediately after: the failure was the argument,
    // not poisoned session state.
    let temp = indexed_tree();
    let missing_root = temp.path().join("does_not_exist").display().to_string();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "root": missing_root.clone()}),
            ),
            tool_call(
                2,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": missing_root.clone()}),
            ),
            tool_call(3, "index_status", json!({"root": missing_root})),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4);
    for response in &responses[..3] {
        assert_tool_error_shape(response);
    }
    assert_eq!(responses[3]["result"]["isError"], false, "{:#}", responses[3]);
    assert!(!tool_body(&responses[3])["h"].as_array().unwrap().is_empty());
}

#[test]
fn session_recovers_after_errors_without_restart() {
    // Errors mid-session do not poison it: after an unknown tool and a read of
    // a nonexistent file, a genuine search-then-read chain still succeeds in
    // the SAME process.
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            tool_call(1, "no_such_tool", json!({})),
            tool_call(2, "code_read", json!({"ids": ["missing.rs#L1-L1"]})),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_tool_error_shape(&responses[0]);
    assert_tool_error_shape(&responses[1]);
    assert_eq!(responses[2]["result"]["isError"], false, "{:#}", responses[2]);
    let compact_id = tool_body(&responses[2])["h"][0][0]
        .as_str()
        .unwrap()
        .to_owned();

    let chained = rpc_session(
        vec![
            tool_call(1, "no_such_tool", json!({})),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(4, "code_read", json!({"ids": [compact_id]})),
        ],
        Some(temp.path()),
    );
    assert_tool_error_shape(&chained[0]);
    assert_eq!(chained[1]["result"]["isError"], false, "{:#}", chained[1]);
    assert_eq!(chained[2]["result"]["isError"], false, "{:#}", chained[2]);
    assert_eq!(tool_body(&chained[2])["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn session_rerun_is_deterministic_across_processes() {
    // The same chain (tools/list, search with resend_seen, long-id read) run
    // in two fresh processes returns identical bytes: no per-process state
    // leaks into results.
    let temp = indexed_tree();
    let chain = || {
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 3);
    assert_eq!(second.len(), 3);
    assert_eq!(
        serde_json::to_string(&first[0]["result"]).unwrap(),
        serde_json::to_string(&second[0]["result"]).unwrap(),
        "tools/list drifted across processes"
    );
    assert_eq!(tool_text(&first[1]), tool_text(&second[1]), "search drifted");
    assert_eq!(tool_text(&first[2]), tool_text(&second[2]), "read drifted");
    assert_eq!(first[1]["result"]["isError"], false);
    assert_eq!(first[2]["result"]["isError"], false);
}

#[test]
fn empty_tree_fails_closed_status_search_read() {
    // A tree with no files and no index: status reports zero files, search
    // returns the empty_index miss (never a bare empty hit list), and any
    // read is a tool error. Nothing fabricates content.
    let temp = tempfile::tempdir().unwrap();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "anything", "limit": 4, "resend_seen": true}),
            ),
            tool_call(3, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(tool_body(&responses[0])["file_count"], 0);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    let miss = tool_body(&responses[1]);
    assert_eq!(miss["why"], "empty_index", "{miss:#}");
    assert_eq!(miss["zn"], 0);
    assert_eq!(miss["h"].as_array().unwrap().len(), 0);
    assert_tool_error_shape(&responses[2]);
}
