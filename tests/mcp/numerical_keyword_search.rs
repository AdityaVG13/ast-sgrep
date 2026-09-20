//! MCP numerical `keyword_search` contracts: limit and budget arithmetic in
//! one contract test each.
//!
//! Non-overlap contract: `numerical_anchors.rs` keeps the 6 KEEP endpoints
//! standalone (including preview-default and elision zd/ze pins);
//! `numerical_code_read.rs` pins the `code_read` surfaces. This file absorbs
//! the 8 `keyword_search` MERGEs (limit 4, budget 4) into 2 contract tests.
//!
//! Transport: `ast-sgrep-testkit::rpc_session`; every read and wait is
//! timeout-bounded (15s) so a regressed server fails the test instead of
//! hanging the suite.

use ast_sgrep_testkit::{
    assert_tool_error_shape, assert_tool_success_shape, indexed_tree, rpc_session, snippet_bytes,
    tool_body, tool_call,
};
use serde_json::{json, Value};

/// INTENT=limit sides (1/100 accept, 0/101 reject), degenerate totality
/// (-1/huge/float/string/2^64 reject, session survives), row-count
/// monotonicity with stable head, and the exact 1/2/3/3 sweep on 3 files.
/// KILLS=bound off-by-one / panic / silent-clamp / wraparound-accept /
/// limit-ignored / limit-as-offset / limit-dependent-ranking /
/// count-zn-drift mutant.
/// ABSORBS=limit_boundary_sides_accept_1_and_100_reject_0_and_101,
/// limit_degenerate_rejects_negative_huge_float_string,
/// limit_is_monotone_in_rows_with_stable_head,
/// limit_sweep_exact_counts_on_three_file_tree.
#[test]
fn keyword_search_limit_contract() {
    // Leg 1 (pass2 totality): 1/100 accept (1 caps to one row), 0/101 reject.
    {
        let temp = indexed_tree(&[(
            "src/lib.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        )]);
        let responses = rpc_session(
            vec![
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
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 4);
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
        assert_tool_error_shape(&responses[2]);
        assert_tool_error_shape(&responses[3]);
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
    // Leg 2 (pass2 totality): -1/u64MAX/1.5/"4"/2^64 reject, session survives.
    {
        let over_u64: Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"keyword_search","arguments":{"query":"target_symbol","limit":18446744073709551616}}}"#,
        )
        .unwrap();
        let temp = indexed_tree(&[(
            "src/lib.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        )]);
        let responses = rpc_session(
            vec![
                tool_call(
                    1,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": -1}),
                ),
                tool_call(
                    2,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": u64::MAX}),
                ),
                tool_call(
                    3,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 1.5}),
                ),
                tool_call(
                    4,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": "4"}),
                ),
                over_u64,
                tool_call(
                    6,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4}),
                ),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 6);
        for response in &responses[..5] {
            assert_tool_error_shape(response);
        }
        assert_tool_success_shape(&responses[5]);
    }
    // Leg 3 (pass3 metamorphic): counts non-decreasing, within limit, stable head.
    {
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
        let limits = [1_u64, 2, 4, 100];
        let calls: Vec<Value> = limits
            .iter()
            .enumerate()
            .map(|(i, limit)| {
                tool_call(
                    i as u32 + 1,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": limit, "resend_seen": true}),
                )
            })
            .collect();
        let responses = rpc_session(calls, Some(temp.path()));
        let bodies: Vec<Value> = responses
            .iter()
            .map(|response| {
                assert_tool_success_shape(response);
                tool_body(response)
            })
            .collect();
        let counts: Vec<usize> = bodies
            .iter()
            .map(|body| body["h"].as_array().unwrap().len())
            .collect();
        assert!(
            counts.iter().all(|count| *count > 0),
            "every limit must hit: {counts:?}"
        );
        for (index, pair) in counts.windows(2).enumerate() {
            assert!(
                pair[1] >= pair[0],
                "row counts must not shrink from limit {} to {}: {counts:?}",
                limits[index],
                limits[index + 1]
            );
        }
        for (count, limit) in counts.iter().zip(limits) {
            assert!(
                (*count as u64) <= limit,
                "count {count} exceeds its limit {limit}: {counts:?}"
            );
        }
        let heads: Vec<&Value> = bodies
            .iter()
            .map(|body| &body["h"].as_array().unwrap()[0])
            .collect();
        for head in &heads[1..] {
            assert_eq!(*head, heads[0], "head row must be stable across limits");
        }
    }
    // Leg 4 (pass4 drill): limits 1/2/3/100 yield 1/2/3/3 rows, 32B snippets.
    {
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
        let limits = [1_u64, 2, 3, 100];
        let calls: Vec<Value> = limits
            .iter()
            .enumerate()
            .map(|(i, limit)| {
                tool_call(
                    i as u32 + 1,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": limit, "resend_seen": true}),
                )
            })
            .collect();
        let responses = rpc_session(calls, Some(temp.path()));
        assert_eq!(responses.len(), 4);
        let want_rows = [1_usize, 2, 3, 3];
        let mut heads = Vec::new();
        for (response, (limit, want)) in responses.iter().zip(limits.iter().zip(want_rows)) {
            assert_tool_success_shape(response);
            let body = tool_body(response);
            let hits = body["h"].as_array().unwrap();
            assert_eq!(hits.len(), want, "limit {limit}: {body:#}");
            assert_eq!(body["zn"], want as u64, "limit {limit}: {body:#}");
            for hit in hits {
                let snippet = hit.as_array().unwrap()[4].as_str().unwrap();
                assert_eq!(
                    snippet, "fn target_symbol() { helper(); }",
                    "limit {limit}: {body:#}"
                );
                assert_eq!(snippet.chars().count(), 32, "limit {limit}: {body:#}");
                assert_eq!(snippet.len(), 32, "limit {limit}: {body:#}");
            }
            heads.push(hits[0].clone());
        }
        for head in &heads[1..] {
            assert_eq!(*head, heads[0], "head row must be stable across limits");
        }
    }
}

/// INTENT=zd[0] echoes the budget and zd[1] equals re-summed snippet bytes,
/// budget sides (1/65536 accept, 0/65537 reject), degenerate totality, and the
/// exact 3/50/200 chain funding 0/32/96 bytes.
/// KILLS=zd echo dropped/swapped, cost-vs-body drift, bound off-by-one,
/// wire-type accept / panic mutant.
/// ABSORBS=zd_echoes_budget_and_spent_matches_snippet_bytes,
/// budget_boundary_sides_accept_1_and_65536_reject_0_and_65537,
/// budget_degenerate_rejects_negative_huge_float_string_bool,
/// budget_chain_exact_echo_and_byte_totals.
#[test]
fn keyword_search_budget_contract() {
    // Leg 1 (pass1 exact): zd[0] echoes (7, 65536); zd[1] == snippet bytes.
    {
        let temp = indexed_tree(&[(
            "src/lib.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        )]);
        let responses = rpc_session(
            vec![
                tool_call(
                    1,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 7}),
                ),
                tool_call(
                    2,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 65536}),
                ),
            ],
            Some(temp.path()),
        );
        for (response, budget) in responses.iter().zip([7_u64, 65536_u64]) {
            assert_tool_success_shape(response);
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
    // Leg 2 (pass2 totality): 1/65536 accept with echo, 0/65537 reject.
    {
        let temp = indexed_tree(&[(
            "src/lib.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        )]);
        let responses = rpc_session(
            vec![
                tool_call(
                    1,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 1}),
                ),
                tool_call(
                    2,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": 65536}),
                ),
                tool_call(
                    3,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "budget_tokens": 0}),
                ),
                tool_call(
                    4,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "budget_tokens": 65537}),
                ),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 4);
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
        assert_tool_error_shape(&responses[2]);
        assert_tool_error_shape(&responses[3]);
        let min = tool_body(&responses[0]);
        assert_eq!(min["zd"][0], 1, "{min:#}");
        let max = tool_body(&responses[1]);
        assert_eq!(max["zd"][0], 65536, "{max:#}");
    }
    // Leg 3 (pass2 totality): -1/huge/float/string/bool reject, session survives.
    {
        let temp = indexed_tree(&[(
            "src/lib.rs",
            "fn target_symbol() { helper(); }\nfn helper() {}\n",
        )]);
        let responses = rpc_session(
            vec![
                tool_call(
                    1,
                    "keyword_search",
                    json!({"query": "target_symbol", "budget_tokens": -1}),
                ),
                tool_call(
                    2,
                    "keyword_search",
                    json!({"query": "target_symbol", "budget_tokens": u64::MAX}),
                ),
                tool_call(
                    3,
                    "keyword_search",
                    json!({"query": "target_symbol", "budget_tokens": 1.5}),
                ),
                tool_call(
                    4,
                    "keyword_search",
                    json!({"query": "target_symbol", "budget_tokens": "many"}),
                ),
                tool_call(
                    5,
                    "keyword_search",
                    json!({"query": "target_symbol", "budget_tokens": true}),
                ),
                tool_call(
                    6,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "budget_tokens": 8}),
                ),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 6);
        for response in &responses[..5] {
            assert_tool_error_shape(response);
        }
        assert_tool_success_shape(&responses[5]);
        assert_eq!(tool_body(&responses[5])["zd"][0], 8, "{:#}", responses[5]);
    }
    // Leg 4 (pass4 drill): budgets 3/50/200 fund 0/32/96 bytes with echoes.
    {
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
        let budgets = [3_u64, 50, 200];
        let calls: Vec<Value> = budgets
            .iter()
            .enumerate()
            .map(|(i, budget)| {
                tool_call(
                    i as u32 + 1,
                    "keyword_search",
                    json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "budget_tokens": budget}),
                )
            })
            .collect();
        let responses = rpc_session(calls, Some(temp.path()));
        assert_eq!(responses.len(), 3);
        let want_totals = [0_usize, 32, 96];
        let want_sorted = [vec![0, 0, 0], vec![0, 0, 32], vec![32, 32, 32]];
        for (response, ((budget, total), mut sorted)) in responses
            .iter()
            .zip(budgets.iter().zip(want_totals).zip(want_sorted))
        {
            assert_tool_success_shape(response);
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
}
