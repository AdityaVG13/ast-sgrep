//! Oracle-foundry surface suite (missions L3): relations BETWEEN calls.
//!
//! Absorbs `oracle_foundry_pass3` (10 tests) into 8 intent-grouped tests:
//! differential (alias), metamorphic (repetition, prefix stability,
//! normalization), adversarial (uniform error shape, pipelining) and envelope
//! topology. Discriminants are `isError` booleans, JSON-RPC codes and envelope
//! shapes -- never message text.
//!
//! Sessions run on [`LiveSession`] (timeout-bounded reads/waits); builders,
//! extractors and single-tree fixtures come from `testkit`.

use ast_sgrep_testkit::{
    assert_tool_error_shape, collect_responses, indexed_tree, multi_hit_tree, tool_body, tool_call,
    tool_text, LiveSession,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

/// INTENT: the `code_search` compat alias agrees with `keyword_search`
/// byte-identically (text AND structured bodies) for the same query.
///
/// KILLS: alias-drift/compat mutants.
///
/// ABSORBS: code_search_alias_matches_keyword_search_exactly.
#[test]
fn alias_matches_keyword_search_exactly() {
    // `resend_seen` keeps the second call out of snippet-elision state.
    let temp = multi_hit_tree();
    let args = json!({"query": "shared_token", "limit": 8, "resend_seen": true});
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "keyword_search", args.clone()),
        tool_call(2, "code_search", args),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    assert_eq!(
        tool_text(&responses[0]),
        tool_text(&responses[1]),
        "alias drift between code_search and keyword_search"
    );
    assert_eq!(
        responses[0]["result"]["structuredContent"],
        responses[1]["result"]["structuredContent"]
    );
}

/// INTENT: pure reads (`index_status`, `code_read`) repeat byte-for-byte in
/// one session, with `file_count` 1 and exact file content.
///
/// KILLS: state-leak/nondeterminism mutants.
///
/// ABSORBS: repeated_index_status_and_code_read_are_byte_identical.
#[test]
fn pure_reads_repeat_byte_identical() {
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "index_status", json!({})),
        tool_call(2, "index_status", json!({})),
        tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        tool_call(4, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    assert_eq!(tool_text(&responses[0]), tool_text(&responses[1]));
    assert_eq!(tool_text(&responses[2]), tool_text(&responses[3]));
    assert_eq!(tool_body(&responses[0])["file_count"], 1);
    assert_eq!(
        tool_body(&responses[2])["nodes"][0]["content"],
        "fn target_symbol() {}"
    );
}

/// INTENT: raising the limit extends the hit list without reordering or
/// rewriting the shared prefix; the path table only grows.
///
/// KILLS: ranking-instability mutants.
///
/// ABSORBS: limit_growth_is_prefix_stable.
#[test]
fn limit_growth_is_prefix_stable() {
    // `resend_seen` disables elision so both calls carry full tuples.
    let temp = multi_hit_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(
            1,
            "keyword_search",
            json!({"query": "shared_token", "limit": 2, "resend_seen": true}),
        ),
        tool_call(
            2,
            "keyword_search",
            json!({"query": "shared_token", "limit": 8, "resend_seen": true}),
        ),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses[0]["result"]["isError"], false, "{:#}", responses[0]);
    assert_eq!(responses[1]["result"]["isError"], false, "{:#}", responses[1]);
    let small = tool_body(&responses[0]);
    let large = tool_body(&responses[1]);
    assert_eq!(small["zn"], 2, "{small:#}");
    assert_eq!(small["h"].as_array().unwrap().len(), 2);
    assert!(
        large["h"].as_array().unwrap().len() > 2,
        "need headroom to test prefix stability: {large:#}"
    );
    assert_eq!(&large["h"].as_array().unwrap()[..2], small["h"].as_array().unwrap());
    assert_eq!(small["q"], large["q"]);
    for (id, path) in small["p"].as_object().unwrap() {
        assert_eq!(&large["p"][id], path, "path table shrank for {id}");
    }
}

/// INTENT: query/report normalization is stable across spellings -- surrounding
/// whitespace is trimmed (echoed `q` proves it), preview names are
/// case-insensitive, and `preview=none` blanks snippets while keeping ranking
/// and ids.
///
/// KILLS: trim-removed, case-fold-removed, preview-ignored mutants.
///
/// ABSORBS: query_trim_and_preview_case_are_normalized,
/// preview_none_keeps_ids_drops_snippets.
#[test]
fn query_and_preview_normalization() {
    let temp = multi_hit_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(
            1,
            "keyword_search",
            json!({"query": "shared_token", "limit": 8, "resend_seen": true}),
        ),
        tool_call(
            2,
            "keyword_search",
            json!({"query": "  shared_token  ", "limit": 8, "resend_seen": true}),
        ),
        tool_call(
            3,
            "keyword_search",
            json!({"query": "shared_token", "limit": 8, "resend_seen": true, "preview": "full"}),
        ),
        tool_call(
            4,
            "keyword_search",
            json!({"query": "shared_token", "limit": 8, "resend_seen": true, "preview": "FULL"}),
        ),
        tool_call(
            5,
            "keyword_search",
            json!({"query": "shared_token", "limit": 8, "resend_seen": true}),
        ),
        tool_call(
            6,
            "keyword_search",
            json!({"query": "shared_token", "limit": 8, "resend_seen": true, "preview": "none"}),
        ),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    // Trim facet: padded query collapses to the bare query, byte-identical.
    assert_eq!(tool_text(&responses[0]), tool_text(&responses[1]));
    assert_eq!(tool_body(&responses[1])["q"], "shared_token");
    // Preview-case facet: FULL == full, byte-identical.
    assert_eq!(tool_text(&responses[2]), tool_text(&responses[3]));
    // Preview-none facet: same ranking/ids as the short control, snippets blank.
    let short = tool_body(&responses[4]);
    let none = tool_body(&responses[5]);
    let short_hits = short["h"].as_array().unwrap();
    let none_hits = none["h"].as_array().unwrap();
    assert!(!short_hits.is_empty());
    assert_eq!(none_hits.len(), short_hits.len());
    for (full, blank) in short_hits.iter().zip(none_hits.iter()) {
        assert_eq!(blank[0], full[0], "id drift across preview modes");
        assert_eq!(blank[4], "", "preview=none must blank snippets: {blank:#}");
        assert!(
            !full[4].as_str().unwrap().is_empty(),
            "control call must carry snippets: {full:#}"
        );
    }
}

/// INTENT: fixed invalid inputs -- unknown tools (empty, absent,
/// case-shifted), malformed search args (missing query, float/huge limit, bad
/// preview, empty/overlong filters) and adversarial read ids (empty list,
/// empty/unicode/overlong ids, oversize lists, wrong JSON types, missing
/// fields, null arguments) -- are all uniform tool errors; a unicode query is
/// valid input and runs as a miss.
///
/// KILLS: shape-divergence/dispatch mutants, arg-shape validation mutants.
///
/// ABSORBS: unknown_tool_and_bad_search_args_share_tool_error_shape,
/// adversarial_code_read_ids_rejected_server_survives.
#[test]
fn adversarial_inputs_share_tool_error_shape() {
    // Leg 1: unknown tools + malformed search args on a multi-hit tree.
    let temp = multi_hit_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "", json!({})),
        tool_call(2, "no_such_tool", json!({})),
        tool_call(3, "Keyword_Search", json!({"query": "x", "limit": 4})),
        tool_call(4, "keyword_search", json!({"limit": 4})),
        tool_call(5, "keyword_search", json!({"query": "x", "limit": 1.5})),
        tool_call(6, "keyword_search", json!({"query": "x", "limit": u64::MAX})),
        tool_call(7, "keyword_search", json!({"query": "   ", "limit": 4})),
        tool_call(8, "keyword_search", json!({"query": "x", "limit": 4, "preview": "huge"})),
        tool_call(9, "keyword_search", json!({"query": "x", "limit": 4, "file_filter": ""})),
        tool_call(
            10,
            "keyword_search",
            json!({"query": "x", "limit": 4, "file_filter": "a".repeat(4097)}),
        ),
        tool_call(11, "keyword_search", json!({"query": "x", "limit": 4, "lang": "  "})),
        tool_call(
            12,
            "keyword_search",
            json!({"query": "日本語🔍", "limit": 4, "resend_seen": true}),
        ),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 12);
    for response in &responses[..11] {
        assert_tool_error_shape(response);
    }
    // Unicode is a runnable query: a miss envelope, not a tool error.
    assert_eq!(responses[11]["result"]["isError"], false, "{:#}", responses[11]);
    let body = tool_body(&responses[11]);
    assert_eq!(body["why"], "no_match", "{body:#}");
    assert_eq!(body["h"].as_array().unwrap().len(), 0);

    // Leg 2: adversarial read ids on a single-file tree; all tool errors.
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "code_read", json!({"ids": []})),
        tool_call(2, "code_read", json!({"ids": [""]})),
        tool_call(3, "code_read", json!({"ids": ["日本.rs#L1-L1"]})),
        tool_call(4, "code_read", json!({"ids": [format!("{}#L1-L1", "a".repeat(5000))]})),
        tool_call(5, "code_read", json!({"ids": vec!["src/lib.rs#L1-L1"; 21]})),
        tool_call(6, "code_read", json!({"ids": "src/lib.rs#L1-L1"})),
        tool_call(7, "code_read", json!({"ids": [42]})),
        tool_call(8, "code_read", json!({"ids": Value::Null})),
        tool_call(9, "code_read", json!({})),
        tool_call(10, "code_read", Value::Null),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 10);
    for response in &responses {
        assert_tool_error_shape(response);
    }

    // Leg 3 (recovery probe): a valid read in a FRESH process, not the
    // adversarial session above. Proven: the server binary still serves valid
    // reads after adversarial input. NOT proven: same-session liveness (the
    // poisoned session is never replayed); no test pins read-after-error in
    // one process (catalog gap, out of scope).
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(11, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})));
    let response = session.recv();
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(response["result"]["isError"], false, "{:#}", response);
    assert_eq!(tool_body(&response)["nodes"][0]["content"], "fn target_symbol() {}");
}

/// INTENT: reindexing an unchanged tree succeeds, indexes nothing new, and
/// status file counts agree across the second run.
///
/// KILLS: reindex-duplication mutants.
///
/// ABSORBS: index_repo_twice_keeps_status_file_count_stable.
#[test]
fn reindex_is_idempotent() {
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        tool_call(1, "index_repo", json!({})),
        tool_call(2, "index_status", json!({})),
        tool_call(3, "index_repo", json!({})),
        tool_call(4, "index_status", json!({})),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    for response in &responses {
        assert_eq!(response["result"]["isError"], false, "{response:#}");
    }
    assert_eq!(tool_body(&responses[2])["files_indexed"], 0);
    assert_eq!(
        tool_body(&responses[1])["file_count"],
        tool_body(&responses[3])["file_count"]
    );
    assert_eq!(tool_body(&responses[1])["file_count"], 1);
}

/// INTENT: six mixed requests fired without waiting are each answered exactly
/// once (id multiset, not delivery order), each response carries exactly one
/// of result/error, and per-response error topology still holds.
///
/// KILLS: drop/duplicate/crosstalk mutants.
///
/// ABSORBS: pipelined_batch_returns_every_id_exactly_once.
#[test]
fn pipelined_batch_answers_every_id_once() {
    // The server answers concurrently (order not pinned); id agreement is.
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    for payload in [
        json!({"jsonrpc":"2.0","id":101,"method":"tools/list","params":{}}),
        tool_call(102, "index_status", json!({})),
        tool_call(
            103,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ),
        tool_call(104, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        tool_call(105, "no_such_tool", json!({})),
        json!({"jsonrpc":"2.0","id":106,"method":"missing"}),
    ] {
        session.send(&payload);
    }
    let responses = collect_responses(&session, 6, Duration::from_secs(15));
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 6, "{responses:#?}");
    let mut by_id: HashMap<i64, &Value> = HashMap::new();
    for response in &responses {
        let id = response["id"].as_i64().expect("numeric id echo");
        assert!(by_id.insert(id, response).is_none(), "duplicate id {id}");
        let has_result = response.get("result").is_some();
        let has_error = response.get("error").is_some();
        assert!(
            has_result ^ has_error,
            "response must carry exactly one of result/error: {response:#}"
        );
    }
    assert_eq!(by_id.len(), 6, "id multiset mismatch: {responses:#?}");
    for id in 101..=106 {
        assert!(by_id.contains_key(&id), "missing id {id}");
    }
    assert_eq!(by_id[&106]["error"]["code"], -32601);
    assert_eq!(by_id[&105]["result"]["isError"], true);
    assert!(by_id[&105].get("error").is_none());
    assert_eq!(by_id[&103]["result"]["isError"], false, "{:#}", by_id[&103]);
    assert_eq!(by_id[&104]["result"]["isError"], false, "{:#}", by_id[&104]);
}

/// INTENT: unknown methods and unshaped tools/call envelopes are top-level
/// -32601 with no `result`; calls that reach dispatch (even with absent/null
/// arguments, which default to `{}`) are tool errors.
///
/// KILLS: topology-confusion/default-args mutants.
///
/// ABSORBS: malformed_envelope_topology_method_vs_tool_errors.
#[test]
fn envelope_topology_method_vs_tool_errors() {
    let mut session = LiveSession::spawn(None);
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        json!({"jsonrpc":"2.0","id":1,"method":"missing"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"keyword_search","arguments":[]}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"keyword_search"}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"keyword_search","arguments":null}}),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 5);
    for response in &responses[..3] {
        assert_eq!(response["error"]["code"], -32601, "{response:#}");
        assert!(response.get("result").is_none(), "{response:#}");
    }
    assert_tool_error_shape(&responses[3]);
    assert_tool_error_shape(&responses[4]);
}
