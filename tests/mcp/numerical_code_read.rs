//! MCP numerical `code_read` contracts: per-ref split, context window,
//! truncation, and empty-file arithmetic in one contract test each.
//!
//! Non-overlap contract: `numerical_anchors.rs` keeps the 6 KEEP endpoints
//! standalone; `numerical_keyword_search.rs` pins limit/budget. This file
//! absorbs the 25 `code_read` MERGEs (split 8, window 8, truncation 6,
//! emptyfile 3) into 4 contract tests. Each contract runs its absorbed legs
//! as scoped blocks in one `#[test]` fn so the surface stays green as a unit.
//!
//! Transport: `ast-sgrep-testkit::rpc_session`; every read and wait is
//! timeout-bounded (15s) so a regressed server fails the test instead of
//! hanging the suite.

use ast_sgrep_testkit::{
    assert_tool_error_shape, assert_tool_success_shape, rpc_session, tool_body, tool_call,
};
use serde_json::{json, Value};

/// WHY: window assertions need the `(start, end)` line pair; testkit exposes
/// tool envelopes but no line-window projection. Keep file-local: no second
/// suite needs window pairs.
fn window(body: &Value, index: usize) -> (u64, u64) {
    (
        body["nodes"][index]["lines"]["start"].as_u64().unwrap(),
        body["nodes"][index]["lines"]["end"].as_u64().unwrap(),
    )
}

/// WHY: content assertions repeat the `nodes[i].content` path; testkit has no
/// node-content accessor. Keep file-local: no second suite needs it.
fn node_content(body: &Value, index: usize) -> &str {
    body["nodes"][index]["content"].as_str().unwrap()
}

/// INTENT=`max_chars / n` remainder-to-FIRST deal, conservation
/// (saturated spend == cap, unsaturated == fs total), multi-ref monotonicity,
/// and exact split chains 7/3→8/3 and 11/4→14/4.
/// KILLS=remainder-dropped/to-last/rotated/double-counted, float-div,
/// zero-budget-guard, `truncated`-on-empty-budget, round-up, n=20-fencepost,
/// pad-to-cap, truncate-when-sufficient, per-node-shrink, cross-call-state-leak.
/// ABSORBS=per_ref_budgets_deal_remainder_to_first_refs,
/// zero_budget_tail_yields_empty_truncated_node,
/// twenty_ref_split_spends_max_chars_exactly,
/// per_ref_split_conserves_saturated_budget,
/// per_ref_split_conserves_unsaturated_total,
/// truncation_total_is_monotone_in_cap_multi_ref,
/// split_chain_seven_then_eight_over_three_refs,
/// split_chain_eleven_then_fourteen_over_four_refs.
#[test]
fn code_read_split_contract() {
    // Leg 1 (pass1 exact): 5/3 -> [2,2,1] with "aa"/"bb"/"c".
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tri.rs"), "aaa\nbbb\nccc\n").unwrap();
        let responses = rpc_session(
            vec![tool_call(
                1,
                "code_read",
                json!({"ids": ["tri.rs#L1-L1", "tri.rs#L2-L2", "tri.rs#L3-L3"], "max_chars": 5}),
            )],
            Some(temp.path()),
        );
        assert_tool_success_shape(&responses[0]);
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
    // Leg 2 (pass1 exact): 1/2 -> [1,0]; tail is ("", true).
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("pair.rs"), "one\ntwo\n").unwrap();
        let responses = rpc_session(
            vec![tool_call(
                1,
                "code_read",
                json!({"ids": ["pair.rs#L1-L1", "pair.rs#L2-L2"], "max_chars": 1}),
            )],
            Some(temp.path()),
        );
        assert_tool_success_shape(&responses[0]);
        let body = tool_body(&responses[0]);
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes[0]["content"], "o", "{body:#}");
        assert_eq!(nodes[0]["truncated"], true, "{body:#}");
        assert_eq!(nodes[1]["content"], "", "{body:#}");
        assert_eq!(nodes[1]["truncated"], true, "{body:#}");
    }
    // Leg 3 (pass2 totality): 25/20 -> [2;5]+[1;15], total exactly 25.
    {
        let temp = tempfile::tempdir().unwrap();
        let lines = (0..20).map(|_| "aaaa").collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(temp.path().join("grid.rs"), lines).unwrap();
        let ids: Vec<Value> = (1..=20).map(|n| json!(format!("grid.rs#L{n}-L{n}"))).collect();
        let responses = rpc_session(
            vec![tool_call(1, "code_read", json!({"ids": ids, "max_chars": 25}))],
            Some(temp.path()),
        );
        assert_tool_success_shape(&responses[0]);
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
    // Leg 4 (pass3 metamorphic): saturated 4-ref sums equal the cap (10, 15).
    {
        let temp = tempfile::tempdir().unwrap();
        let line = "a".repeat(12);
        let text = (0..4).map(|_| line.as_str()).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(temp.path().join("quad.rs"), text).unwrap();
        let ids: Vec<Value> = (1..=4).map(|n| json!(format!("quad.rs#L{n}-L{n}"))).collect();
        for cap in [10_u64, 15_u64] {
            let responses = rpc_session(
                vec![tool_call(1, "code_read", json!({"ids": ids, "max_chars": cap}))],
                Some(temp.path()),
            );
            assert_tool_success_shape(&responses[0]);
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
    // Leg 5 (pass3 metamorphic): unsaturated sums equal the fs total, caps agree.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("trio.rs"), "ab\ncde\nf\n").unwrap();
        let fs_total: usize = std::fs::read_to_string(temp.path().join("trio.rs"))
            .unwrap()
            .lines()
            .map(|line| line.chars().count())
            .sum();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["trio.rs#L1-L1", "trio.rs#L2-L2", "trio.rs#L3-L3"], "max_chars": 100})),
                tool_call(2, "code_read", json!({"ids": ["trio.rs#L1-L1", "trio.rs#L2-L2", "trio.rs#L3-L3"], "max_chars": 1000})),
            ],
            Some(temp.path()),
        );
        let mut sums = Vec::new();
        for response in &responses {
            assert_tool_success_shape(response);
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
    // Leg 6 (pass3 metamorphic): multi-ref totals + per-node monotone in cap.
    {
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
                tool_call(
                    i as u32 + 1,
                    "code_read",
                    json!({"ids": ["digits.rs#L1-L1", "digits.rs#L2-L2", "digits.rs#L3-L3"], "max_chars": cap}),
                )
            })
            .collect();
        let responses = rpc_session(calls, Some(temp.path()));
        let mut per_node: Vec<Vec<usize>> = Vec::new();
        let mut totals: Vec<usize> = Vec::new();
        for response in &responses {
            assert_tool_success_shape(response);
            let body = tool_body(response);
            let nodes = body["nodes"].as_array().unwrap();
            assert_eq!(nodes.len(), 3, "{body:#}");
            let lens: Vec<usize> = nodes
                .iter()
                .map(|n| n["content"].as_str().unwrap().chars().count())
                .collect();
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
    // Leg 7 (pass4 drill): 7/3 -> [3,2,2] then 8/3 -> [3,3,2].
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("tri.rs"),
            "AAAAAAAAAA\nBBBBBBBBBB\nCCCCCCCCCC\n",
        )
        .unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["tri.rs#L1-L1", "tri.rs#L2-L2", "tri.rs#L3-L3"], "max_chars": 7})),
                tool_call(2, "code_read", json!({"ids": ["tri.rs#L1-L1", "tri.rs#L2-L2", "tri.rs#L3-L3"], "max_chars": 8})),
            ],
            Some(temp.path()),
        );
        let want = [(7_usize, ["AAA", "BB", "CC"]), (8_usize, ["AAA", "BBB", "CC"])];
        for (response, (cap, contents)) in responses.iter().zip(want) {
            assert_tool_success_shape(response);
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
    // Leg 8 (pass4 drill): 11/4 -> [3,3,3,2] then 14/4 -> [4,4,3,3].
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("quad.rs"),
            "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\ndddddddddd\n",
        )
        .unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["quad.rs#L1-L1", "quad.rs#L2-L2", "quad.rs#L3-L3", "quad.rs#L4-L4"], "max_chars": 11})),
                tool_call(2, "code_read", json!({"ids": ["quad.rs#L1-L1", "quad.rs#L2-L2", "quad.rs#L3-L3", "quad.rs#L4-L4"], "max_chars": 14})),
            ],
            Some(temp.path()),
        );
        let want = [
            (11_usize, ["aaa", "bbb", "ccc", "dd"]),
            (14_usize, ["aaaa", "bbbb", "ccc", "ddd"]),
        ];
        for (response, (cap, contents)) in responses.iter().zip(want) {
            assert_tool_success_shape(response);
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
}

/// INTENT=context clamp `max(req-ctx,1)`/`min(req+ctx,total)` at edges,
/// boundary sides (0/100 accept, 101+degenerate reject), clamp idempotence,
/// window nesting, spelling equivalence, and the exact center sweep.
/// KILLS=missing-clamp / off-by-one-widen / symmetric-extension /
/// bound-off-by-one / wrong-endpoint / double-ctx / rewiden /
/// wrong-side / content-mismatch / range-context-conflation mutant.
/// ABSORBS=context_window_clamps_to_file_edges,
/// context_window_asymmetric_at_edges,
/// context_boundary_sides_accept_0_and_100_reject_101_and_degenerate,
/// clamp_is_idempotent_from_top_edge, clamp_is_idempotent_from_bottom_edge,
/// window_nesting_contains_smaller_context,
/// window_nesting_contains_subrange_and_equivalent_spellings,
/// window_sweep_exact_table_at_center.
#[test]
fn code_read_window_contract() {
    // Leg 1 (pass1 exact): L1 ctx100 -> (1,5); L5 ctx2 -> (3,5).
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("five.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["five.rs#L1-L1"], "context_lines": 100})),
                tool_call(2, "code_read", json!({"ids": ["five.rs#L5-L5"], "context_lines": 2})),
            ],
            Some(temp.path()),
        );
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
        let top = tool_body(&responses[0]);
        assert_eq!(top["nodes"][0]["lines"], json!({"start": 1, "end": 5}), "{top:#}");
        assert_eq!(top["nodes"][0]["content"], "l1\nl2\nl3\nl4\nl5", "{top:#}");
        let bottom = tool_body(&responses[1]);
        assert_eq!(bottom["nodes"][0]["lines"], json!({"start": 3, "end": 5}), "{bottom:#}");
        assert_eq!(bottom["nodes"][0]["content"], "l3\nl4\nl5", "{bottom:#}");
    }
    // Leg 2 (pass1 exact): one-sided clamps L1-L2 ctx1 -> (1,3), L4-L5 -> (3,5).
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("five.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["five.rs#L1-L2"], "context_lines": 1})),
                tool_call(2, "code_read", json!({"ids": ["five.rs#L4-L5"], "context_lines": 1})),
            ],
            Some(temp.path()),
        );
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
        let first = tool_body(&responses[0]);
        assert_eq!(first["nodes"][0]["lines"], json!({"start": 1, "end": 3}), "{first:#}");
        assert_eq!(first["nodes"][0]["content"], "l1\nl2\nl3", "{first:#}");
        let last = tool_body(&responses[1]);
        assert_eq!(last["nodes"][0]["lines"], json!({"start": 3, "end": 5}), "{last:#}");
        assert_eq!(last["nodes"][0]["content"], "l3\nl4\nl5", "{last:#}");
    }
    // Leg 3 (pass2 totality): ctx 0/100 accept, 101/-1/huge/float reject.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("five.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["five.rs#L3-L3"], "context_lines": 0})),
                tool_call(2, "code_read", json!({"ids": ["five.rs#L3-L3"], "context_lines": 100})),
                tool_call(3, "code_read", json!({"ids": ["five.rs#L3-L3"], "context_lines": 101})),
                tool_call(4, "code_read", json!({"ids": ["five.rs#L3-L3"], "context_lines": -1})),
                tool_call(5, "code_read", json!({"ids": ["five.rs#L3-L3"], "context_lines": u64::MAX})),
                tool_call(6, "code_read", json!({"ids": ["five.rs#L3-L3"], "context_lines": 1.5})),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 6);
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
        for response in &responses[2..] {
            assert_tool_error_shape(response);
        }
        let exact = tool_body(&responses[0]);
        assert_eq!(exact["nodes"][0]["lines"], json!({"start": 3, "end": 3}), "{exact:#}");
        assert_eq!(exact["nodes"][0]["content"], "l3", "{exact:#}");
        assert_eq!(exact["nodes"][0]["truncated"], false, "{exact:#}");
        let clamped = tool_body(&responses[1]);
        assert_eq!(clamped["nodes"][0]["lines"], json!({"start": 1, "end": 5}), "{clamped:#}");
        assert_eq!(clamped["nodes"][0]["content"], "l1\nl2\nl3\nl4\nl5", "{clamped:#}");
    }
    // Leg 4 (pass3 metamorphic): top-edge re-clamp is a fixed point.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("seven.rs"), "r1\nr2\nr3\nr4\nr5\nr6\nr7\n").unwrap();
        let first = rpc_session(
            vec![tool_call(1, "code_read", json!({"ids": ["seven.rs#L2-L2"], "context_lines": 50}))],
            Some(temp.path()),
        );
        assert_tool_success_shape(&first[0]);
        let base = tool_body(&first[0]);
        let (start, end) = window(&base, 0);
        let widened = format!("seven.rs#L{start}-L{end}");
        let again = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": [widened.clone()], "context_lines": 50})),
                tool_call(2, "code_read", json!({"ids": [widened], "context_lines": 100})),
                tool_call(3, "code_read", json!({"ids": ["seven.rs#L2-L2"], "context_lines": 100})),
            ],
            Some(temp.path()),
        );
        for response in &again {
            assert_tool_success_shape(response);
        }
        for (index, response) in again.iter().enumerate() {
            let body = tool_body(response);
            assert_eq!(window(&body, 0), (start, end), "re-clamp {index} moved the window: base {base:#} vs {body:#}");
            assert_eq!(node_content(&body, 0), node_content(&base, 0), "re-clamp {index} changed content");
            assert_eq!(body["nodes"][0]["truncated"], base["nodes"][0]["truncated"], "re-clamp {index} flipped truncated");
        }
    }
    // Leg 5 (pass3 metamorphic): bottom-edge re-clamp is a fixed point.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("seven.rs"), "r1\nr2\nr3\nr4\nr5\nr6\nr7\n").unwrap();
        let first = rpc_session(
            vec![tool_call(1, "code_read", json!({"ids": ["seven.rs#L6-L6"], "context_lines": 50}))],
            Some(temp.path()),
        );
        assert_tool_success_shape(&first[0]);
        let base = tool_body(&first[0]);
        let (start, end) = window(&base, 0);
        let widened = format!("seven.rs#L{start}-L{end}");
        let again = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": [widened.clone()], "context_lines": 50})),
                tool_call(2, "code_read", json!({"ids": [widened], "context_lines": 100})),
                tool_call(3, "code_read", json!({"ids": ["seven.rs#L6-L6"], "context_lines": 100})),
            ],
            Some(temp.path()),
        );
        for response in &again {
            assert_tool_success_shape(response);
        }
        for (index, response) in again.iter().enumerate() {
            let body = tool_body(response);
            assert_eq!(window(&body, 0), (start, end), "re-clamp {index} moved the window: base {base:#} vs {body:#}");
            assert_eq!(node_content(&body, 0), node_content(&base, 0), "re-clamp {index} changed content");
            assert_eq!(body["nodes"][0]["truncated"], base["nodes"][0]["truncated"], "re-clamp {index} flipped truncated");
        }
    }
    // Leg 6 (pass3 metamorphic): ctx 0,1,2,4 nest (starts fall, ends rise).
    {
        let temp = tempfile::tempdir().unwrap();
        let text = (1..=9).map(|n| format!("L{n}")).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(temp.path().join("nine.rs"), text).unwrap();
        let ctxs = [0_u64, 1, 2, 4];
        let calls: Vec<Value> = ctxs
            .iter()
            .enumerate()
            .map(|(i, ctx)| tool_call(i as u32 + 1, "code_read", json!({"ids": ["nine.rs#L5-L5"], "context_lines": ctx})))
            .collect();
        let responses = rpc_session(calls, Some(temp.path()));
        let bodies: Vec<Value> = responses
            .iter()
            .map(|response| {
                assert_tool_success_shape(response);
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
    // Leg 7 (pass3 metamorphic): subrange nesting + spelling equivalence.
    {
        let temp = tempfile::tempdir().unwrap();
        let text = (1..=9).map(|n| format!("L{n}")).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(temp.path().join("nine.rs"), text).unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["nine.rs#L5-L5"], "context_lines": 0})),
                tool_call(2, "code_read", json!({"ids": ["nine.rs#L4-L6"], "context_lines": 0})),
                tool_call(3, "code_read", json!({"ids": ["nine.rs#L3-L7"], "context_lines": 0})),
                tool_call(4, "code_read", json!({"ids": ["nine.rs#L5-L5"], "context_lines": 1})),
                tool_call(5, "code_read", json!({"ids": ["nine.rs#L4-L6"], "context_lines": 1})),
            ],
            Some(temp.path()),
        );
        let bodies: Vec<Value> = responses
            .iter()
            .map(|response| {
                assert_tool_success_shape(response);
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
    // Leg 8 (pass4 drill): exact (start,end,content,chars,bytes,lines) sweep.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("seven.rs"), "r1\nr2\nr3\nr4\nr5\nr6\nr7\n").unwrap();
        let calls: Vec<Value> = [0_u64, 1, 2, 3]
            .iter()
            .enumerate()
            .map(|(i, ctx)| tool_call(i as u32 + 1, "code_read", json!({"ids": ["seven.rs#L4-L4"], "context_lines": ctx})))
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
            assert_tool_success_shape(response);
            let body = tool_body(response);
            assert_eq!(body["nodes"][0]["lines"], json!({"start": start, "end": end}), "{body:#}");
            let got = body["nodes"][0]["content"].as_str().unwrap();
            assert_eq!(got, content, "{body:#}");
            assert_eq!(got.chars().count(), chars, "{body:#}");
            assert_eq!(got.len(), chars, "{body:#}");
            assert_eq!(got.lines().count() as u64, lines, "{body:#}");
            assert_eq!(end - start + 1, lines, "{body:#}");
            assert_eq!(body["nodes"][0]["truncated"], false, "{body:#}");
        }
    }
}

/// INTENT=char-step truncation (never byte slices), exact `truncated` boundary,
/// max_chars sides (1/1M accept, 0/1M+1+degenerate reject), scale to huge
/// lines, single/multi-byte monotonicity, and exact ASCII+Greek chains.
/// KILLS=byte-slice / split-code-point / `>=`-vs-`>` flag / bound-off-by-one /
/// scale-miscount / reversed-comparison / flag-flap mutant.
/// ABSORBS=truncation_counts_chars_not_bytes_with_exact_boundary,
/// max_chars_boundary_sides_accept_1_and_1000000_reject_0_and_1000001,
/// huge_line_truncates_to_budget_in_chars_not_bytes,
/// truncation_is_monotone_in_cap_single_ref,
/// multibyte_truncation_scales_chars_not_bytes,
/// truncation_chains_exact_bytes_ascii_and_greek.
#[test]
fn code_read_truncation_contract() {
    // Leg 1 (pass1 exact): "ééé" truncates by chars with exact flag boundary.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("uni.rs"), "ééé\n").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["uni.rs#L1-L1"], "max_chars": 2})),
                tool_call(2, "code_read", json!({"ids": ["uni.rs#L1-L1"], "max_chars": 3})),
                tool_call(3, "code_read", json!({"ids": ["uni.rs#L1-L1"], "max_chars": 1})),
            ],
            Some(temp.path()),
        );
        for response in &responses {
            assert_tool_success_shape(response);
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
    // Leg 2 (pass2 totality): max_chars 1/1M accept, 0/1M+1/degenerate reject.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("abc.rs"), "abc\n").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["abc.rs#L1-L1"], "max_chars": 1})),
                tool_call(2, "code_read", json!({"ids": ["abc.rs#L1-L1"], "max_chars": 1_000_000})),
                tool_call(3, "code_read", json!({"ids": ["abc.rs#L1-L1"], "max_chars": 0})),
                tool_call(4, "code_read", json!({"ids": ["abc.rs#L1-L1"], "max_chars": 1_000_001})),
                tool_call(5, "code_read", json!({"ids": ["abc.rs#L1-L1"], "max_chars": -1})),
                tool_call(6, "code_read", json!({"ids": ["abc.rs#L1-L1"], "max_chars": u64::MAX})),
                tool_call(7, "code_read", json!({"ids": ["abc.rs#L1-L1"], "max_chars": "many"})),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 7);
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
        for response in &responses[2..] {
            assert_tool_error_shape(response);
        }
        let min = tool_body(&responses[0]);
        assert_eq!(min["nodes"][0]["content"], "a", "{min:#}");
        assert_eq!(min["nodes"][0]["truncated"], true, "{min:#}");
        let max = tool_body(&responses[1]);
        assert_eq!(max["nodes"][0]["content"], "abc", "{max:#}");
        assert_eq!(max["nodes"][0]["truncated"], false, "{max:#}");
    }
    // Leg 3 (pass2 totality): huge ASCII + wide lines keep exactly 10 chars.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("huge.rs"), format!("{}\n", "z".repeat(5000))).unwrap();
        std::fs::write(temp.path().join("wide.rs"), format!("{}\n", "é".repeat(100))).unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["huge.rs#L1-L1"], "max_chars": 10})),
                tool_call(2, "code_read", json!({"ids": ["wide.rs#L1-L1"], "max_chars": 10})),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 2);
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
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
    // Leg 4 (pass3 metamorphic): single-ref monotone + prefix + one flag flip.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("alpha.rs"), "abcdefghijklmnopqrst\n").unwrap();
        let caps = [1_u64, 3, 8, 19, 20, 21];
        let calls: Vec<Value> = caps
            .iter()
            .enumerate()
            .map(|(i, cap)| tool_call(i as u32 + 1, "code_read", json!({"ids": ["alpha.rs#L1-L1"], "max_chars": cap})))
            .collect();
        let responses = rpc_session(calls, Some(temp.path()));
        let bodies: Vec<Value> = responses
            .iter()
            .map(|response| {
                assert_tool_success_shape(response);
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
    // Leg 5 (pass3 metamorphic): 24x"é" bytes always 2x chars, prefix-chained.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("wide.rs"), format!("{}\n", "é".repeat(24))).unwrap();
        let caps = [1_u64, 5, 10, 23, 24, 25];
        let calls: Vec<Value> = caps
            .iter()
            .enumerate()
            .map(|(i, cap)| tool_call(i as u32 + 1, "code_read", json!({"ids": ["wide.rs#L1-L1"], "max_chars": cap})))
            .collect();
        let responses = rpc_session(calls, Some(temp.path()));
        let bodies: Vec<Value> = responses
            .iter()
            .map(|response| {
                assert_tool_success_shape(response);
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
    // Leg 6 (pass4 drill): exact ASCII + Greek char/byte/flag tables.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("alpha.rs"), "abcdefghijklmnopqrstuvwxyz\n").unwrap();
        std::fs::write(temp.path().join("greek.rs"), "αβγδε\n").unwrap();
        let ascii_caps = [1_u64, 5, 25, 26, 27];
        let greek_caps = [1_u64, 3, 4, 5, 6];
        let mut calls = Vec::new();
        for (i, cap) in ascii_caps.iter().enumerate() {
            calls.push(tool_call(i as u32 + 1, "code_read", json!({"ids": ["alpha.rs#L1-L1"], "max_chars": cap})));
        }
        for (i, cap) in greek_caps.iter().enumerate() {
            calls.push(tool_call(i as u32 + 11, "code_read", json!({"ids": ["greek.rs#L1-L1"], "max_chars": cap})));
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
        for (response, (content, chars, bytes, truncated)) in responses[..5].iter().zip(ascii_want) {
            assert_tool_success_shape(response);
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
        for (response, (content, chars, bytes, truncated)) in responses[5..].iter().zip(greek_want) {
            assert_tool_success_shape(response);
            let body = tool_body(response);
            let got = body["nodes"][0]["content"].as_str().unwrap();
            assert_eq!(got, content, "{body:#}");
            assert_eq!(got.chars().count(), chars, "{body:#}");
            assert_eq!(got.len(), bytes, "{body:#}");
            assert_eq!(body["nodes"][0]["truncated"], truncated, "{body:#}");
        }
    }
}

/// INTENT=0-byte file is one virtual line (L1-L1 ("",false), over-end rejects),
/// empty truncation never flags at min/max budgets, and bare/missing newlines
/// each total exactly 1 line.
/// KILLS=total_lines 0-or-2 / empty-truncation-flag / line-count mutant.
/// ABSORBS=empty_file_reads_as_single_empty_line,
/// empty_file_totality_under_min_and_max_budgets,
/// line_window_edges_for_missing_and_bare_newlines.
#[test]
fn code_read_emptyfile_contract() {
    // Leg 1 (pass1 exact): L1-L1 ("",false); L1-L2 rejects.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("empty.rs"), "").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["empty.rs#L1-L1"]})),
                tool_call(2, "code_read", json!({"ids": ["empty.rs#L1-L2"]})),
            ],
            Some(temp.path()),
        );
        assert_tool_success_shape(&responses[0]);
        assert_tool_error_shape(&responses[1]);
        let body = tool_body(&responses[0]);
        assert_eq!(body["nodes"][0]["lines"], json!({"start": 1, "end": 1}), "{body:#}");
        assert_eq!(body["nodes"][0]["content"], "", "{body:#}");
        assert_eq!(body["nodes"][0]["truncated"], false, "{body:#}");
    }
    // Leg 2 (pass2 totality): ("",false) at budgets 1 and 1M; L2-L2 rejects.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("empty.rs"), "").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["empty.rs#L1-L1"], "max_chars": 1})),
                tool_call(2, "code_read", json!({"ids": ["empty.rs#L1-L1"], "max_chars": 1_000_000})),
                tool_call(3, "code_read", json!({"ids": ["empty.rs#L2-L2"]})),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 3);
        assert_tool_success_shape(&responses[0]);
        assert_tool_success_shape(&responses[1]);
        assert_tool_error_shape(&responses[2]);
        for response in &responses[..2] {
            let body = tool_body(response);
            assert_eq!(body["nodes"][0]["lines"], json!({"start": 1, "end": 1}), "{body:#}");
            assert_eq!(body["nodes"][0]["content"], "", "{body:#}");
            assert_eq!(body["nodes"][0]["truncated"], false, "{body:#}");
        }
    }
    // Leg 3 (pass2 totality): "solo"/"x\n"/"\n" each total 1; past-total rejects.
    {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("solo.rs"), "solo").unwrap();
        std::fs::write(temp.path().join("nl.rs"), "x\n").unwrap();
        std::fs::write(temp.path().join("bare.rs"), "\n").unwrap();
        let responses = rpc_session(
            vec![
                tool_call(1, "code_read", json!({"ids": ["solo.rs#L1-L1"]})),
                tool_call(2, "code_read", json!({"ids": ["solo.rs#L1-L2"]})),
                tool_call(3, "code_read", json!({"ids": ["nl.rs#L1-L1"]})),
                tool_call(4, "code_read", json!({"ids": ["bare.rs#L1-L1"]})),
            ],
            Some(temp.path()),
        );
        assert_eq!(responses.len(), 4);
        assert_tool_success_shape(&responses[0]);
        assert_tool_error_shape(&responses[1]);
        assert_tool_success_shape(&responses[2]);
        assert_tool_success_shape(&responses[3]);
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
}
