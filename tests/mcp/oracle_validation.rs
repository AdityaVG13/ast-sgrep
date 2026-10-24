//! Oracle-foundry validation suite (missions L2): tool-argument boundary matrix.
//!
//! Absorbs `oracle_foundry_pass2` (5 tests) into ONE matrix test with per-arg
//! legs. Every boundary pins accept AND reject: reject-only corpora cannot kill
//! `>`-vs-`>=` mutants on the accept side. Discriminants are `isError`
//! booleans, never message text.
//!
//! Sessions run on [`LiveSession`] (timeout-bounded reads/waits); fixtures and
//! builders come from `testkit`. This file needs no file-local
//! helpers.

use ast_sgrep_testkit::{
    assert_jsonrpc_error, assert_tool_error_shape, indexed_tree, is_error, tool_body, tool_call,
    LiveSession,
};
use serde_json::json;

/// INTENT: every tool-arg boundary accepts its edges and rejects its outliers
/// in one matrix (limit 1..=100, budget 1..=65536, context 0..=100,
/// max_chars >= 1, node ranges 1-based with end >= start, query 1..=4096
/// chars), with shape/content spot-checks on the accept legs.
///
/// KILLS: limit/budget/context/query bound off-by-one both sides,
/// bound-check-removed, budget-ignored (tuples stay 5-wide), window-math
/// off-by-one, `start > 0` dropped, `end >= start` dropped.
/// NOT killed (catalog gaps, kept as-is): budget ceiling-reject (no
/// 65537-reject leg, only ceiling-accept), max_chars accept-side
/// (reject-only here). Query empty-string reject is closed since the
/// fastmcp facade: admission enforces minLength 1 (-32602) pre-dispatch.
///
/// ABSORBS: search_limit_accepts_1_and_100_rejects_0_and_101,
/// budget_tokens_accepts_1_and_max_rejects_0,
/// code_read_context_zero_is_exact_window,
/// code_read_rejects_zero_start_and_reversed_range,
/// search_query_single_char_ok_overlong_rejected.
#[test]
fn arg_boundaries_accept_edges_reject_outliers() {
    // One indexed tree serves every leg: src/lib.rs carries the searchable
    // symbol; five.rs/two.rs carry the read windows (code_read needs no index
    // but tolerates one).
    let temp = indexed_tree(&[
        (
            "src/lib.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        ),
        ("five.rs", "l1\nl2\nl3\nl4\nl5\n"),
        ("two.rs", "one\ntwo\n"),
    ]);
    // One spawn, strictly sequential (send one, read one) so response order
    // matches request order. Reads carry the default 15s bound; the exit wait
    // is bounded by `wait_clean`.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    let mut responses = Vec::new();
    for payload in [
        // Limit leg: 1 and 100 accepted, 0 and 101 rejected.
        tool_call(
            1,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 1}),
        ),
        tool_call(
            2,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 100}),
        ),
        tool_call(
            3,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 0}),
        ),
        tool_call(
            4,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 101}),
        ),
        // Budget leg: 1 and 65536 accepted with 6-wide detail tuples,
        // 0 rejected. (Ceiling-accept only; no 65537-reject leg exists.)
        tool_call(
            5,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "budget_tokens": 1}),
        ),
        tool_call(
            6,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "budget_tokens": 65536}),
        ),
        tool_call(
            7,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "budget_tokens": 0}),
        ),
        // Context leg: 0 and 100 accepted with exact L3-L3 window,
        // 101 and max_chars 0 rejected.
        tool_call(
            8,
            "code_read",
            json!({"ids": ["five.rs#L3-L3"], "context_lines": 0}),
        ),
        tool_call(
            9,
            "code_read",
            json!({"ids": ["five.rs#L3-L3"], "context_lines": 100}),
        ),
        tool_call(
            10,
            "code_read",
            json!({"ids": ["five.rs#L3-L3"], "context_lines": 101}),
        ),
        tool_call(
            11,
            "code_read",
            json!({"ids": ["five.rs#L3-L3"], "max_chars": 0}),
        ),
        // Range leg: L1-L1 reads, L0-L1 and L2-L1 rejected.
        tool_call(12, "code_read", json!({"ids": ["two.rs#L1-L1"]})),
        tool_call(13, "code_read", json!({"ids": ["two.rs#L0-L1"]})),
        tool_call(14, "code_read", json!({"ids": ["two.rs#L2-L1"]})),
        // Query leg: len 1 and 4096 accepted, 4097 rejected.
        tool_call(15, "keyword_search", json!({"query": "x", "limit": 4})),
        tool_call(
            16,
            "keyword_search",
            json!({"query": "a".repeat(4096), "limit": 4}),
        ),
        tool_call(
            17,
            "keyword_search",
            json!({"query": "a".repeat(4097), "limit": 4}),
        ),
    ] {
        session.send(&payload);
        responses.push(session.recv());
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_eq!(responses.len(), 17);

    // Limit leg discriminants + non-empty hits on the accept edge.
    assert!(!is_error(&responses[0]), "{:#}", responses[0]);
    assert!(!is_error(&responses[1]), "{:#}", responses[1]);
    assert_jsonrpc_error(&responses[2], -32602);
    assert_jsonrpc_error(&responses[3], -32602);
    let body = tool_body(&responses[0]);
    assert!(!body["h"].as_array().unwrap().is_empty(), "{body:#}");

    // Budget leg discriminants + 6-wide detail tuples on the accept edge.
    assert!(!is_error(&responses[4]), "{:#}", responses[4]);
    assert!(!is_error(&responses[5]), "{:#}", responses[5]);
    assert_jsonrpc_error(&responses[6], -32602);
    let body = tool_body(&responses[4]);
    for hit in body["h"].as_array().unwrap() {
        let tuple = hit.as_array().expect("hit is a positional tuple");
        assert_eq!(tuple.len(), 6, "{hit:#}");
        assert!(
            ["metadata", "signature", "block", "full"].contains(&tuple[5].as_str().unwrap()),
            "{hit:#}"
        );
    }

    // Context leg discriminants + exact L3-L3 window on the accept edge.
    assert!(!is_error(&responses[7]), "{:#}", responses[7]);
    assert!(!is_error(&responses[8]), "{:#}", responses[8]);
    assert_jsonrpc_error(&responses[9], -32602);
    assert_jsonrpc_error(&responses[10], -32602);
    let body = tool_body(&responses[7]);
    assert_eq!(body["nodes"][0]["lines"], json!({"start": 3, "end": 3}));
    assert_eq!(body["nodes"][0]["content"], "l3");

    // Range leg discriminants.
    assert!(!is_error(&responses[11]), "{:#}", responses[11]);
    // 0-based and reversed ranges are well-typed strings: the HANDLER
    // rejects them semantically (tool error), not schema admission.
    assert_tool_error_shape(&responses[12]);
    assert_tool_error_shape(&responses[13]);

    // Query leg discriminants.
    assert!(!is_error(&responses[14]), "{:#}", responses[14]);
    assert!(!is_error(&responses[15]), "{:#}", responses[15]);
    assert_jsonrpc_error(&responses[16], -32602);
}
