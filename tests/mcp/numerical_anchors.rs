//! MCP numerical anchors: the 6 KEEP tests from `tests/catalog/numerical-mcp.md`
//! plus the folded rerun contract.
//!
//! Non-overlap contract: `numerical_code_read.rs` pins the `code_read`
//! arithmetic surfaces (split / window / truncation / emptyfile);
//! `numerical_keyword_search.rs` pins the `keyword_search` surfaces (limit /
//! budget). This file pins the endpoints that must stay standalone:
//! schema literals, ids arity, preview defaults, determinism, elision, and
//! the search→read handoff — plus the rerun contract that folds the
//! within-session leg into the across-sessions anchor with a cross-leg
//! byte-identity check (stronger than either leg alone).
//!
//! Transport: `ast-sgrep-testkit` (`rpc_session` / `CallSession` /
//! `LiveSession`); every read and wait is timeout-bounded (15s) so a
//! regressed server fails the test instead of hanging the suite.

use ast_sgrep_testkit::{
    assert_tool_error_shape, assert_tool_success_shape, indexed_tree, initialized_notif,
    rpc_session, snippet_bytes, tool_body, tool_call, tool_text, tools_list, CallSession,
    LiveSession,
};
use serde_json::{json, Value};
use std::path::Path;

/// WHY: `LiveSession::handshake` hardcodes protocolVersion 2025-11-25, but the
/// ttlMs pin needs a 2026-07-28 session. Keep file-local: no second suite
/// needs a versioned pin. Every read rides the 15s `recv` bound; the exit
/// rides `wait_clean`.
fn rpc_session_versioned(
    payloads: Vec<Value>,
    root: Option<&Path>,
    protocol_version: &str,
) -> Vec<Value> {
    let mut session = LiveSession::spawn(root);
    session.send(&json!({
        "jsonrpc": "2.0",
        "id": "__init",
        "method": "initialize",
        "params": {
            "protocolVersion": protocol_version,
            "capabilities": {},
            "clientInfo": {"name": "asgrep-testkit", "version": "0"}
        }
    }));
    let init = session.recv();
    assert_eq!(init["id"], "__init", "{init:#}");
    session.send(&initialized_notif());
    let mut responses = Vec::new();
    for payload in &payloads {
        session.send(payload);
        if payload.get("id").is_some() {
            responses.push(session.recv());
        }
    }
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
    responses
}

/// INTENT=tools/list schema bounds equal parser literals; ttlMs is exactly
/// 3_600_000 on a 2026-07-28 session.
/// KILLS=schema/parser bound drift in either direction.
/// ABSORBS=none (KEEP anchor).
#[test]
fn tools_list_schema_bounds_match_parser() {
    let listed = rpc_session(vec![tools_list(1)], None);
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

    let fresh = rpc_session_versioned(vec![tools_list(2)], None, "2026-07-28");
    assert_eq!(fresh[0]["result"]["ttlMs"], 3_600_000, "{:#}", fresh[0]);
}

/// INTENT=ids rejects [] (no div-by-zero) and 21 items, accepts 1.
/// KILLS=div-by-zero / arity-fencepost mutant.
/// ABSORBS=none (KEEP anchor).
#[test]
fn ids_arity_edges_reject_empty_and_21_accept_single() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("ok.rs"), "hi\n").unwrap();
    let twenty_one: Vec<Value> = (0..21).map(|_| json!("ok.rs#L1-L1")).collect();
    let responses = rpc_session(
        vec![
            tool_call(1, "code_read", json!({"ids": []})),
            tool_call(2, "code_read", json!({"ids": twenty_one})),
            tool_call(3, "code_read", json!({"ids": ["ok.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    assert_tool_error_shape(&responses[0]);
    assert_tool_error_shape(&responses[1]);
    assert_tool_success_shape(&responses[2]);
    let body = tool_body(&responses[2]);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1, "{body:#}");
    assert_eq!(body["nodes"][0]["content"], "hi", "{body:#}");
}

/// INTENT=preview=full defaults budget to 8192; unbudgeted short/none omit zd.
/// KILLS=default-literal / renderer-arm-swap mutant.
/// ABSORBS=none (KEEP anchor).
#[test]
fn full_preview_defaults_to_8192_while_unbudgeted_short_has_no_zd() {
    let temp = indexed_tree(&[(
        "src/lib.rs",
        "fn target_symbol() { helper(); }\nfn helper() {}\n",
    )]);
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "preview": "full"}),
            ),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "preview": "full", "budget_tokens": 100}),
            ),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "preview": "none"}),
            ),
        ],
        Some(temp.path()),
    );
    for response in &responses {
        assert_tool_success_shape(response);
    }
    let defaulted = tool_body(&responses[0]);
    assert_eq!(defaulted["zd"][0], 8192, "{defaulted:#}");
    let explicit = tool_body(&responses[1]);
    assert_eq!(explicit["zd"][0], 100, "{explicit:#}");
    let short = tool_body(&responses[2]);
    assert!(
        short.get("zd").is_none(),
        "unbudgeted short must omit zd: {short:#}"
    );
    let none = tool_body(&responses[3]);
    assert!(
        none.get("zd").is_none(),
        "preview=none must omit zd: {none:#}"
    );
}

/// INTENT=same calls in fresh processes are byte-identical.
/// KILLS=time-seed / hash-order / ranking-flip mutant.
/// ABSORBS=none (KEEP anchor; the folded rerun contract below absorbs the
/// within-session MERGE with a stronger cross-leg check).
#[test]
fn rerun_is_byte_identical_across_sessions() {
    let temp = indexed_tree(&[
        (
            "src/a.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/b.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/c.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/d.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/e.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
    ]);
    std::fs::write(temp.path().join("pin.rs"), "aaa\nbbb\n").unwrap();
    let search_args =
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 64});
    let read_args = json!({"ids": ["pin.rs#L1-L1", "pin.rs#L2-L2"], "max_chars": 4});
    let first = rpc_session(
        vec![
            tool_call(1, "code_read", read_args.clone()),
            tool_call(2, "keyword_search", search_args.clone()),
        ],
        Some(temp.path()),
    );
    let second = rpc_session(
        vec![
            tool_call(1, "code_read", read_args),
            tool_call(2, "keyword_search", search_args),
        ],
        Some(temp.path()),
    );
    for response in first.iter().chain(second.iter()) {
        assert_tool_success_shape(response);
    }
    assert_eq!(
        tool_text(&first[0]),
        tool_text(&second[0]),
        "code_read must be byte-identical across processes"
    );
    assert_eq!(
        tool_text(&first[1]),
        tool_text(&second[1]),
        "keyword_search must be byte-identical across processes"
    );
}

/// INTENT=first send 96B/no-ze, repeat 3B/ze=3 with "~", resend 96B/no-ze.
/// KILLS=elision off-by-one / ze-missing / resend-`~`-leak mutant.
/// ABSORBS=none (KEEP anchor).
#[test]
fn elision_chain_exact_counts_and_bytes() {
    let temp = indexed_tree(&[
        (
            "src/f0.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/f1.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/f2.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
    ]);
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4}),
            ),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4}),
            ),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_tool_success_shape(response);
    }
    let first = tool_body(&responses[0]);
    assert_eq!(first["h"].as_array().unwrap().len(), 3, "{first:#}");
    assert_eq!(first["zn"], 3, "{first:#}");
    assert!(
        first.get("ze").is_none(),
        "first send must omit ze: {first:#}"
    );
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
    assert!(
        resent.get("ze").is_none(),
        "resend must omit ze: {resent:#}"
    );
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

/// INTENT=search→compact-id read→L1-L2 truncation (33/47/48) pipeline with
/// exact bytes.
/// KILLS=count/window/byte pipeline-drift mutant.
/// ABSORBS=none (KEEP anchor).
#[test]
fn search_to_read_handoff_exact_window_and_bytes() {
    let temp = indexed_tree(&[(
        "src/lib.rs",
        "fn target_symbol() { helper(); }\nfn helper() {}\n",
    )]);
    // One live session: the search populates the path registry, the compact
    // hit id resolves against it, then the fixed truncation follow-ups run.
    // `CallSession` reads ride the 15s `recv` bound; `finish` rides `wait_clean`.
    let mut session = CallSession::spawn(temp.path());
    let searched = session.call(
        "keyword_search",
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
    );
    assert_tool_success_shape(&searched);
    let found = tool_body(&searched);
    assert_eq!(found["h"].as_array().unwrap().len(), 1, "{found:#}");
    assert_eq!(found["zn"], 1, "{found:#}");
    let hit_id = found["h"].as_array().unwrap()[0].as_array().unwrap()[0]
        .as_str()
        .unwrap()
        .to_owned();
    let via_id_response = session.call("code_read", json!({"ids": [hit_id]}));
    assert_tool_success_shape(&via_id_response);
    let via_id = tool_body(&via_id_response);
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
    let follow_ups = [
        json!({"ids": ["src/lib.rs#L1-L2"], "max_chars": 33}),
        json!({"ids": ["src/lib.rs#L1-L2"], "max_chars": 47}),
        json!({"ids": ["src/lib.rs#L1-L2"], "max_chars": 48}),
    ];
    for (args, (content, bytes, truncated)) in follow_ups.into_iter().zip(want) {
        let response = session.call("code_read", args);
        assert_tool_success_shape(&response);
        let body = tool_body(&response);
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
    session.finish();
}

/// INTENT=folded determinism: within-session repeats AND fresh-process repeats
/// are ALL byte-identical (within==within, across==across, and within==across).
/// KILLS=per-call-counter / elision-state-leak / time-seed / hash-order /
/// ranking-flip mutant.
/// ABSORBS=rerun_is_byte_identical_within_session (pass3; the across KEEP above
/// stays standalone and this contract re-pins it via cross-leg equality).
#[test]
fn rerun_folded_within_and_across_contract() {
    let temp = indexed_tree(&[
        (
            "src/a.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/b.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/c.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/d.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        (
            "src/e.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
    ]);
    std::fs::write(temp.path().join("pin.rs"), "aaa\nbbb\n").unwrap();
    let search_args =
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 64});
    let read_args = json!({"ids": ["pin.rs#L1-L1", "pin.rs#L2-L2"], "max_chars": 4});
    // Within-session leg: repeated read+search in ONE session.
    let within = rpc_session(
        vec![
            tool_call(1, "code_read", read_args.clone()),
            tool_call(2, "code_read", read_args.clone()),
            tool_call(3, "keyword_search", search_args.clone()),
            tool_call(4, "keyword_search", search_args.clone()),
        ],
        Some(temp.path()),
    );
    for response in &within {
        assert_tool_success_shape(response);
    }
    assert_eq!(
        tool_text(&within[0]),
        tool_text(&within[1]),
        "repeated code_read must be byte-identical within one session"
    );
    assert_eq!(
        tool_text(&within[2]),
        tool_text(&within[3]),
        "repeated keyword_search must be byte-identical within one session"
    );
    // Across-sessions legs: same calls in FRESH processes.
    let first = rpc_session(
        vec![
            tool_call(1, "code_read", read_args.clone()),
            tool_call(2, "keyword_search", search_args.clone()),
        ],
        Some(temp.path()),
    );
    let second = rpc_session(
        vec![
            tool_call(1, "code_read", read_args),
            tool_call(2, "keyword_search", search_args),
        ],
        Some(temp.path()),
    );
    for response in first.iter().chain(second.iter()) {
        assert_tool_success_shape(response);
    }
    // Cross-leg equality: every read byte-identical to every other read, and
    // likewise for searches. This subsumes both the within leg and the across
    // KEEP (which stays standalone above for a minimal isolated pin).
    let reads = [
        tool_text(&within[0]).to_owned(),
        tool_text(&within[1]).to_owned(),
        tool_text(&first[0]).to_owned(),
        tool_text(&second[0]).to_owned(),
    ];
    for other in &reads[1..] {
        assert_eq!(
            *other, reads[0],
            "all code_read legs must be byte-identical"
        );
    }
    let searches = [
        tool_text(&within[2]).to_owned(),
        tool_text(&within[3]).to_owned(),
        tool_text(&first[1]).to_owned(),
        tool_text(&second[1]).to_owned(),
    ];
    for other in &searches[1..] {
        assert_eq!(
            *other, searches[0],
            "all keyword_search legs must be byte-identical"
        );
    }
}
