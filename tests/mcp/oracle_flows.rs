//! Oracle-foundry flows suite (missions L4): end-to-end flows over sessions.
//!
//! Absorbs `oracle_foundry_pass4` (11 tests) into 7 intent-grouped tests:
//! dispatch-by-name, search-then-read consistency, the structural channel, the
//! index lifecycle, the error taxonomy, session recovery, determinism and
//! fail-closed trees. Discriminants are `isError` booleans, JSON-RPC codes,
//! envelope shapes and counts -- never message text.
//!
//! Provenance note: every "session" here is one server spawn with per-test
//! process isolation; multi-step flows replay earlier calls in each new spawn
//! because the compact path registry is a per-process map. Comments state the
//! actual spawn count per test.
//!
//! Sessions run on [`LiveSession`] (timeout-bounded reads/waits); builders,
//! extractors and fixtures come from `testkit`.

use ast_sgrep_testkit::{
    assert_tool_error_shape, distinct_hit_paths, file_tree, indexed_tree, tool_body, tool_call,
    tool_text, tools_list, LiveSession,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// INTENT: a realistic client dispatches by the names advertised in
/// tools/list (never hardcoded) and drives search-to-read off them; the
/// advertised tool count is pinned as a tripwire.
///
/// KILLS: tool-rename/dispatch mutants; BEHAVIOR-ONLY chain coverage.
///
/// ABSORBS: tool_names_consumed_from_list_response_drive_search_and_read,
/// full_chain_list_search_read_in_one_session (MERGE: flow duplicated by the
/// dispatch and fan-out tests; only the tools.len()==8 tripwire is unique and
/// is carried here).
#[test]
fn dispatch_by_advertised_names_with_tool_count_tripwire() {
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);

    // Spawn 1: discovery. Proven: exactly 8 advertised tools including the
    // search/read pair the legs below dispatch by.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tools_list(1));
    let listed = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 8, "{listed:#}");
    let names: Vec<String> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    let search_name = names
        .iter()
        .find(|name| *name == "keyword_search")
        .unwrap()
        .clone();
    let read_name = names
        .iter()
        .find(|name| *name == "code_read")
        .unwrap()
        .clone();

    // Spawn 2: search by advertised name; hits are non-empty, zn agrees.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(
        2,
        &search_name,
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
    ));
    let searched = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(searched["result"]["isError"], false, "{searched:#}");
    let envelope = tool_body(&searched);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    let compact_id = hits[0][0].as_str().expect("compact id").to_owned();

    // Spawn 3: search replayed (per-process path registry) then read by
    // advertised name; the read resolves to exactly one well-formed node.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(
            2,
            &search_name,
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ),
        tool_call(3, &read_name, json!({"ids": [compact_id]})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:#}",
        responses[1]
    );
    let body = tool_body(&responses[1]);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "{body:#}");
    assert!(nodes[0]["id"].as_str().unwrap().contains("#L"), "{body:#}");
    assert!(nodes[0]["lines"]["start"].as_u64().unwrap() >= 1);
    assert!(
        nodes[0]["lines"]["end"].as_u64().unwrap() >= nodes[0]["lines"]["start"].as_u64().unwrap()
    );
    assert!(
        !nodes[0]["content"].as_str().unwrap().is_empty(),
        "{body:#}"
    );
}

/// INTENT: search-then-read is consistent -- one fan-out read over every hit
/// id returns exactly one well-formed non-empty node per hit, and every
/// read-back file is a member of the search `p` table with all tree files
/// round-tripping.
///
/// KILLS: fan-out drop/extra mutants; BEHAVIOR-ONLY cross-tool consistency.
///
/// ABSORBS: search_then_read_every_hit_resolves_with_matching_count,
/// search_path_table_agrees_with_read_node_files.
///
/// Provenance: the `p` table is decoded by the production resolver
/// (`resolve_compact_paths`), so a shared-encoder bug would pass, and no
/// ground-truth filename is asserted. Proven is cross-tool consistency
/// (search and read agree), not absolute filename correctness.
#[test]
fn search_then_read_fan_out_matches_counts_and_path_table() {
    let temp = indexed_tree(&[
        ("src/probe1.rs", "fn shared_probe_1() {}\n"),
        ("src/probe2.rs", "fn shared_probe_2() {}\n"),
        ("src/probe3.rs", "fn shared_probe_3() {}\n"),
    ]);

    // Spawn 1: search; three distinct hit paths, three table entries.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(
        1,
        "keyword_search",
        json!({"query": "shared_probe", "limit": 8, "resend_seen": true}),
    ));
    let searched = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(searched["result"]["isError"], false, "{searched:#}");
    let envelope = tool_body(&searched);
    let hits = envelope["h"].as_array().unwrap().clone();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    assert_eq!(distinct_hit_paths(&hits).len(), 3, "{envelope:#}");
    let table: HashMap<String, String> = ast_sgrep_plugins::resolve_compact_paths(&envelope)
        .into_iter()
        .collect();
    assert_eq!(table.len(), 3, "{envelope:#}");
    let table_paths: HashSet<&String> = table.values().collect();
    let ids: Vec<Value> = hits.iter().map(|hit| hit[0].clone()).collect();

    // Spawn 2: search replayed (per-process path registry) then fan-out read.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(
            1,
            "keyword_search",
            json!({"query": "shared_probe", "limit": 8, "resend_seen": true}),
        ),
        tool_call(2, "code_read", json!({"ids": ids})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:#}",
        responses[1]
    );
    let body = tool_body(&responses[1]);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), hits.len(), "{body:#}");
    let mut read_files = HashSet::new();
    for node in nodes {
        assert!(node["id"].as_str().unwrap().contains("#L"), "{node:#}");
        let start = node["lines"]["start"].as_u64().unwrap();
        let end = node["lines"]["end"].as_u64().unwrap();
        assert!(start >= 1 && end >= start, "{node:#}");
        assert!(!node["content"].as_str().unwrap().is_empty(), "{node:#}");
        let file = node["id"]
            .as_str()
            .unwrap()
            .split("#L")
            .next()
            .unwrap()
            .to_owned();
        assert!(
            table_paths.contains(&file),
            "{node:#} not in {table_paths:?}"
        );
        read_files.insert(file);
    }
    assert_eq!(read_files.len(), 3, "every tree file must round-trip");
}

/// INTENT: the structural channel feeds code_read too -- every ast_search hit
/// carries kind `p` and its compact id expands to a non-empty node.
///
/// KILLS: channel/kind mutants (only ast_search-to-read coverage).
///
/// ABSORBS: ast_search_chain_reads_pattern_hits.
#[test]
fn ast_search_chain_reads_pattern_hits() {
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let query = json!({"query": "fn $NAME() { $$$BODY }", "limit": 4, "resend_seen": true});

    // Spawn 1: structural search; non-empty hits, all kind `p`.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "ast_search", query.clone()));
    let searched = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(searched["result"]["isError"], false, "{searched:#}");
    let envelope = tool_body(&searched);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    for hit in hits {
        assert_eq!(hit[1], "p", "{hit:#}");
    }
    let compact_id = hits[0][0].as_str().unwrap().to_owned();

    // Spawn 2: search replayed (per-process path registry) then read.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "ast_search", query),
        tool_call(2, "code_read", json!({"ids": [compact_id]})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:#}",
        responses[1]
    );
    let body = tool_body(&responses[1]);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1);
    assert!(
        !body["nodes"][0]["content"].as_str().unwrap().is_empty(),
        "{body:#}"
    );
}

/// INTENT: the index lifecycle runs miss (empty_index) -> index_repo indexes
/// exactly the tree files -> the same query hits every file -> read back; and
/// an empty tree fails closed (status zero, empty_index miss, read is a tool
/// error, never fabricated content).
///
/// KILLS: lifecycle mutants, fabrication/fail-open mutants.
///
/// ABSORBS: index_lifecycle_miss_then_index_then_hit_then_read,
/// empty_tree_fails_closed_status_search_read.
///
/// Provenance: the lifecycle runs as 4 separate spawns (per-test process
/// isolation), not "one session". Proven is the state-machine sequence across
/// a shared on-disk tree, not single-process continuity.
#[test]
fn index_lifecycle_and_empty_tree_fail_closed() {
    let temp = file_tree(&[
        ("src/cycle1.rs", "fn lifecycle_probe_1() {}\n"),
        ("src/cycle2.rs", "fn lifecycle_probe_2() {}\n"),
    ]);
    let query = json!({"query": "lifecycle_probe", "limit": 8, "resend_seen": true});

    // Spawn 1: unindexed tree misses with the empty_index envelope.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "keyword_search", query.clone()));
    let miss = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(miss["result"]["isError"], false, "{miss:#}");
    let miss_body = tool_body(&miss);
    assert_eq!(miss_body["why"], "empty_index", "{miss_body:#}");
    assert_eq!(miss_body["zn"], 0);
    assert_eq!(miss_body["h"].as_array().unwrap().len(), 0);

    // Spawn 2: indexing plants exactly the 2 tree files.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(2, "index_repo", json!({})));
    let indexed = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(indexed["result"]["isError"], false, "{indexed:#}");
    assert_eq!(tool_body(&indexed)["files_indexed"], 2);

    // Spawn 3: the same query now hits both files.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(3, "keyword_search", query.clone()));
    let searched = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(searched["result"]["isError"], false, "{searched:#}");
    let envelope = tool_body(&searched);
    let hits = envelope["h"].as_array().unwrap();
    assert!(envelope["zn"].as_u64().unwrap() >= 2, "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    assert_eq!(distinct_hit_paths(hits).len(), 2, "{envelope:#}");
    let compact_id = hits[0][0].as_str().unwrap().to_owned();

    // Spawn 4: the chained read replays search first so the compact path
    // registry (a per-process session map) holds the id, like a real client.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(3, "keyword_search", query),
        tool_call(4, "code_read", json!({"ids": [compact_id]})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:#}",
        responses[1]
    );
    assert_eq!(
        tool_body(&responses[1])["nodes"].as_array().unwrap().len(),
        1
    );

    // Empty-tree leg (fresh tree, one spawn): status zero, empty_index miss
    // (never a bare empty hit list), read is a tool error.
    let temp = tempfile::tempdir().unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "index_status", json!({})),
        tool_call(
            2,
            "keyword_search",
            json!({"query": "anything", "limit": 4, "resend_seen": true}),
        ),
        tool_call(3, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 3);
    assert_eq!(
        responses[0]["result"]["isError"], false,
        "{:#}",
        responses[0]
    );
    assert_eq!(tool_body(&responses[0])["file_count"], 0);
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:#}",
        responses[1]
    );
    let miss = tool_body(&responses[1]);
    assert_eq!(miss["why"], "empty_index", "{miss:#}");
    assert_eq!(miss["zn"], 0);
    assert_eq!(miss["h"].as_array().unwrap().len(), 0);
    assert_tool_error_shape(&responses[2]);
}

/// INTENT: the failure taxonomy side by side in one session -- unknown
/// JSON-RPC methods are top-level -32601 with no `result`; unknown tools,
/// mistyped args and missing roots are uniform tool errors with no top-level
/// `error`.
///
/// KILLS: taxonomy-confusion mutants (root-arg cases are new vs the surface
/// topology test).
///
/// ABSORBS: error_taxonomy_end_to_end_method_vs_tool_vs_args_vs_root.
#[test]
fn error_taxonomy_method_vs_tool_vs_args_vs_root() {
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let missing_root = temp.path().join("does_not_exist").display().to_string();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        json!({"jsonrpc":"2.0","id":1,"method":"missing"}),
        tool_call(2, "no_such_tool", json!({})),
        tool_call(3, "index_status", json!({"root": 42})),
        tool_call(
            4,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "root": missing_root}),
        ),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["error"]["code"], -32601, "{:#}", responses[0]);
    assert!(responses[0].get("result").is_none(), "{:#}", responses[0]);
    for response in &responses[1..] {
        assert_tool_error_shape(response);
    }
}

/// INTENT: errors do not poison the session -- a nonexistent per-call root
/// fails every tool uniformly and the same session serves a valid call right
/// after; likewise an unknown tool plus a bad read leave the search path
/// unpoisoned in the same process.
///
/// KILLS: fail-open/session-poison mutants.
///
/// ABSORBS: missing_root_fails_closed_across_tools_then_recovers,
/// session_recovers_after_errors_without_restart.
///
/// Provenance: the missing-root leg is genuinely one spawn (failure was the
/// argument, not session state). In the poison leg only the SEARCH replay is
/// proven in the poisoned process; the read half runs in a FRESH process (the
/// per-process path registry forces the replay), so read-after-error in one
/// process is NOT proven (catalog gap, out of scope).
#[test]
fn session_recovers_after_errors_without_restart() {
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);

    // Leg 1 (one spawn): missing per-call root fails all tools uniformly,
    // then a valid call succeeds in the SAME session.
    let missing_root = temp.path().join("does_not_exist").display().to_string();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
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
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 4);
    for response in &responses[..3] {
        assert_tool_error_shape(response);
    }
    assert_eq!(
        responses[3]["result"]["isError"], false,
        "{:#}",
        responses[3]
    );
    assert!(!tool_body(&responses[3])["h"].as_array().unwrap().is_empty());

    // Leg 2 (poisoned process): unknown tool + bad read, then a genuine
    // search still succeeds WITHOUT restart.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "no_such_tool", json!({})),
        tool_call(2, "code_read", json!({"ids": ["missing.rs#L1-L1"]})),
        tool_call(
            3,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_tool_error_shape(&responses[0]);
    assert_tool_error_shape(&responses[1]);
    assert_eq!(
        responses[2]["result"]["isError"], false,
        "{:#}",
        responses[2]
    );
    let compact_id = tool_body(&responses[2])["h"][0][0]
        .as_str()
        .unwrap()
        .to_owned();

    // Leg 3 (FRESH process, not the poisoned one above): a client chain
    // succeeds after errors. Proven: post-error chains work; NOT proven:
    // read-after-error in the poisoned process itself.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "no_such_tool", json!({})),
        tool_call(
            3,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ),
        tool_call(4, "code_read", json!({"ids": [compact_id]})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_tool_error_shape(&responses[0]);
    assert_eq!(
        responses[1]["result"]["isError"], false,
        "{:#}",
        responses[1]
    );
    assert_eq!(
        responses[2]["result"]["isError"], false,
        "{:#}",
        responses[2]
    );
    assert_eq!(
        tool_body(&responses[2])["nodes"].as_array().unwrap().len(),
        1
    );
}

/// INTENT: the same chain (tools/list, search with resend_seen, long-id read)
/// run in two fresh processes returns identical bytes.
///
/// KILLS: per-process-state-leak mutants.
///
/// ABSORBS: session_rerun_is_deterministic_across_processes.
#[test]
fn session_rerun_is_deterministic_across_processes() {
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let chain = || {
        vec![
            tools_list(1),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ]
    };
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut session = LiveSession::spawn(Some(temp.path()));
        session.handshake();
        let mut responses = Vec::new();
        for payload in chain() {
            session.send(&payload);
            responses.push(session.recv());
        }
        session.close_stdin();
        assert!(session.wait_clean().success());
        runs.push(responses);
    }
    assert_eq!(runs[0].len(), 3);
    assert_eq!(runs[1].len(), 3);
    assert_eq!(
        serde_json::to_string(&runs[0][0]["result"]).unwrap(),
        serde_json::to_string(&runs[1][0]["result"]).unwrap(),
        "tools/list drifted across processes"
    );
    assert_eq!(
        tool_text(&runs[0][1]),
        tool_text(&runs[1][1]),
        "search drifted"
    );
    assert_eq!(
        tool_text(&runs[0][2]),
        tool_text(&runs[1][2]),
        "read drifted"
    );
    assert_eq!(runs[0][1]["result"]["isError"], false);
    assert_eq!(runs[0][2]["result"]["isError"], false);
}
