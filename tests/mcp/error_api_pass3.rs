//! Pass 3 (errorapi, Mission E3): negative-path METAMORPHIC relations.
//!
//! Pass 1 pins the error TAXONOMY (which trigger yields which row) and pass 2
//! pins PROPAGATION (backend state decides the envelope). This file pins
//! RELATIONS between calls: properties that must hold when the same fault is
//! replayed across tools, sessions, and sequence positions. No test here pins
//! a single trigger-to-row mapping; every assertion compares two or more
//! responses (or one response against a forbidden-payload predicate).
//!
//! Pinned relations (discriminants are codes, envelope shapes, and raw bytes,
//! never message text):
//!
//! | row | relation                                              | assertion                              |
//! |-----|-------------------------------------------------------|----------------------------------------|
//! | C1  | same mistyped-`root` fault on all 8 root-taking tools | identical tool-error discriminants     |
//! | C2  | same unknown-argument fault on all 8 named tools      | identical tool-error discriminants     |
//! | C3  | same fault class, 4 `limit` spellings, one channel    | identical tool-error discriminants     |
//! | D1  | identical error calls, two fresh sessions, both levels| byte-identical raw response lines      |
//! | D3  | same bad args twice in one session (ids differ)       | identical modulo `id` normalization    |
//! | P1  | bad call first vs last (same id set, permuted order)  | per-id byte-identical responses        |
//! | P2  | notification before a bad call vs no notification     | byte-identical error line              |
//! | F1  | fault catalog, tool-level and JSON-RPC-level          | no success members / payload keys      |
//! | F2  | backend failures (corrupt index, deleted file)        | tool-error shape, no partial payloads  |
//!
//! D1 absorbs the former D2 (the same byte-identity relation at the JSON-RPC
//! level): one test, two call sets, two levels.
//!
//! Non-duplication: pass 1 pins per-tool parse rows (bounds/types/unknown
//! keys/sandbox escape on 4 tools) and JSON-RPC codes; pass 2 pins corrupt-db
//! uniformity, mid-session flips, atomicity, and mixed transcripts. E3 reuses
//! some triggers but asserts only cross-call relations (equality of
//! discriminants, byte identity across sessions/orders, absence of payload
//! keys in error bytes) that neither pass states.
//!
//! Transport comes from [`error_testkit`](self::error_testkit): every
//! live-session read and process wait is timeout-bounded.

#[path = "error_testkit.rs"]
mod error_testkit;

use error_testkit::*;
use serde_json::{json, Value};

/// C1: the SAME mistyped-`root` fault on every tool that takes a `root` (all
/// 5 search channels plus `code_read`, `index_status`, `index_repo`) yields
/// the identical tool-error discriminant. Pass 1 pins the per-tool rows; this
/// pins the cross-tool consistency relation.
/// INTENT: same mistyped-root fault: identical discriminant on all 8 root tools.
/// KILLS: cross-tool-divergence.
/// ABSORBS: none.
#[test]
fn consistency_mistyped_root_is_same_tool_error_on_every_root_tool() {
    let temp = file_tree();
    let mut calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| tool_call(i as u32 + 1, channel, json!({"query": "x", "root": 42})))
        .collect();
    calls.push(tool_call(
        6,
        "code_read",
        json!({"ids": ["src/lib.rs#L1-L1"], "root": 42}),
    ));
    calls.push(tool_call(7, "index_status", json!({"root": 42})));
    calls.push(tool_call(8, "index_repo", json!({"root": 42})));
    let raw = rpc_session_raw(calls, Some(temp.path()));
    assert_eq!(raw.len(), 8);
    let responses: Vec<Value> = raw.iter().map(|line| parse_line(line)).collect();
    for (response, id) in responses.iter().zip(1..=8) {
        assert_eq!(response["id"], id, "{response:#}");
        assert_tool_error_shape(response);
    }
    let first = tool_error_discriminant(&responses[0]);
    for response in &responses[1..] {
        assert_eq!(
            tool_error_discriminant(response),
            first,
            "same fault diverged across tools: {response:#}"
        );
    }
}

/// C2: the SAME unknown-argument fault on all 8 named tools (valid base args
/// plus one unknown key) yields the identical tool-error discriminant.
/// INTENT: same unknown-arg fault: identical discriminant on all 8 tools.
/// KILLS: cross-tool-divergence.
/// ABSORBS: none.
#[test]
fn consistency_unknown_argument_is_same_tool_error_on_every_tool() {
    let temp = file_tree();
    let mut calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| tool_call(i as u32 + 1, channel, json!({"query": "x", "bogus": 1})))
        .collect();
    calls.push(tool_call(
        6,
        "code_read",
        json!({"ids": ["src/lib.rs#L1-L1"], "bogus": 1}),
    ));
    calls.push(tool_call(7, "index_status", json!({"bogus": 1})));
    calls.push(tool_call(8, "index_repo", json!({"bogus": 1})));
    let raw = rpc_session_raw(calls, Some(temp.path()));
    assert_eq!(raw.len(), 8);
    let responses: Vec<Value> = raw.iter().map(|line| parse_line(line)).collect();
    for (response, id) in responses.iter().zip(1..=8) {
        assert_eq!(response["id"], id, "{response:#}");
        assert_tool_error_shape(response);
    }
    let first = tool_error_discriminant(&responses[0]);
    for response in &responses[1..] {
        assert_eq!(
            tool_error_discriminant(response),
            first,
            "same fault diverged across tools: {response:#}"
        );
    }
}

/// C3: the same fault CLASS in four spellings (`limit` 0, -1, mistyped, and
/// above max) on one channel yields the identical tool-error discriminant.
/// Bound, sign, type, and ceiling rejections are one code, not four.
/// INTENT: limit 0/-1/mistyped/huge share one discriminant.
/// KILLS: spelling-divergence.
/// ABSORBS: none.
#[test]
fn consistency_limit_fault_spellings_share_one_tool_error_code() {
    let temp = file_tree();
    let raw = rpc_session_raw(
        vec![
            tool_call(1, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(2, "keyword_search", json!({"query": "x", "limit": -1})),
            tool_call(3, "keyword_search", json!({"query": "x", "limit": "many"})),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "x", "limit": 999999999}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(raw.len(), 4);
    let responses: Vec<Value> = raw.iter().map(|line| parse_line(line)).collect();
    for (response, id) in responses.iter().zip(1..=4) {
        assert_eq!(response["id"], id, "{response:#}");
        assert_tool_error_shape(response);
    }
    let first = tool_error_discriminant(&responses[0]);
    for response in &responses[1..] {
        assert_eq!(
            tool_error_discriminant(response),
            first,
            "same fault class diverged by spelling: {response:#}"
        );
    }
}

/// D1: identical bad calls (same ids, same args) in two fresh sessions over
/// the same tree yield BYTE-IDENTICAL raw response lines — at BOTH levels.
/// Tool leg: unknown tool, invalid args, and read-time failure. JSON-RPC leg:
/// unknown method and unshaped `tools/call` params.
/// INTENT: same bad tool calls × 2 sessions: byte-identical raw lines; same rpc-error triggers × 2 sessions: byte-identical raw lines.
/// KILLS: nondeterministic-error-bytes (both levels).
/// ABSORBS: determinism_identical_tool_errors_are_byte_identical_across_sessions + determinism_identical_jsonrpc_errors_are_byte_identical_across_sessions (same relation at the second level; one test, two call sets).
#[test]
fn determinism_identical_errors_are_byte_identical_across_sessions() {
    let temp = file_tree();
    // Tool-error leg: same ids, same args, two fresh sessions.
    let tool_calls = || {
        vec![
            tool_call(7, "no_such_tool", json!({})),
            tool_call(8, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(9, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
        ]
    };
    let first = rpc_session_raw(tool_calls(), Some(temp.path()));
    let second = rpc_session_raw(tool_calls(), Some(temp.path()));
    assert_eq!(first, second, "tool-error bytes drifted across sessions");
    for line in &first {
        assert_tool_error_shape(&parse_line(line));
    }
    // JSON-RPC-error leg: same ids, two fresh sessions.
    let rpc_calls = || {
        vec![
            json!({"jsonrpc":"2.0","id":21,"method":"missing"}),
            json!({"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":42,"arguments":{}}}),
        ]
    };
    let first = rpc_session_raw(rpc_calls(), Some(temp.path()));
    let second = rpc_session_raw(rpc_calls(), Some(temp.path()));
    assert_eq!(
        first, second,
        "JSON-RPC-error bytes drifted across sessions"
    );
    for line in &first {
        assert_jsonrpc_error(&parse_line(line), -32601);
    }
}

/// D3: the same bad args twice within ONE session (ids necessarily differ)
/// yield identical responses modulo `id` normalization -- the error text and
/// envelope carry no per-call randomness or counter state.
/// INTENT: same bad args twice in one session identical modulo id.
/// KILLS: per-call-randomness, counter-state-leak.
/// ABSORBS: none.
#[test]
fn determinism_repeated_bad_call_in_one_session_is_identical_modulo_id() {
    let temp = file_tree();
    let raw = rpc_session_raw(
        vec![
            tool_call(31, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(32, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(33, "no_such_tool", json!({"query": "x"})),
            tool_call(34, "no_such_tool", json!({"query": "x"})),
        ],
        Some(temp.path()),
    );
    assert_eq!(raw.len(), 4);
    let normalized: Vec<String> = raw.iter().map(|line| canonical_modulo_id(line)).collect();
    assert_eq!(
        normalized[0], normalized[1],
        "repeat of invalid-args call drifted"
    );
    assert_eq!(
        normalized[2], normalized[3],
        "repeat of unknown-tool call drifted"
    );
    for line in &raw {
        assert_tool_error_shape(&parse_line(line));
    }
}

/// P1: the same id set in permuted order -- backend-failure call first vs
/// last among two successes -- yields per-id BYTE-IDENTICAL responses.
/// Position in the transcript changes neither the error nor the successes.
/// INTENT: bad call first vs last: per-id byte-identical responses.
/// KILLS: position-dependence.
/// ABSORBS: none.
#[test]
fn position_permuted_transcript_yields_same_per_call_responses() {
    let temp = file_tree();
    let bad = || tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]}));
    let good_a = || tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}));
    let good_b = || tool_call(3, "index_status", json!({}));
    let bad_first = rpc_session_raw(vec![bad(), good_a(), good_b()], Some(temp.path()));
    let bad_last = rpc_session_raw(vec![good_a(), good_b(), bad()], Some(temp.path()));
    assert_eq!(
        by_id(&bad_first),
        by_id(&bad_last),
        "per-call responses depend on sequence position"
    );
    let by_first = by_id(&bad_first);
    assert_tool_error_shape(&parse_line(by_first[&1]));
    assert_eq!(parse_line(by_first[&2])["result"]["isError"], false);
    assert_eq!(parse_line(by_first[&3])["result"]["isError"], false);
}

/// P2: a notification (absent `id`, hence silent) ahead of a bad call leaves
/// the bad call's error bytes unchanged versus the bare call, and the session
/// continues. Silence neither perturbs nor annotates the error.
/// INTENT: notification ahead of bad call leaves error bytes unchanged.
/// KILLS: notification-perturbs-error.
/// ABSORBS: none.
#[test]
fn position_notification_before_bad_call_leaves_error_bytes_unchanged() {
    let temp = file_tree();
    let bad = tool_call(5, "keyword_search", json!({"query": "x", "limit": 0}));
    let bare = rpc_session_raw(vec![bad.clone()], Some(temp.path()));
    assert_eq!(bare.len(), 1);

    let mut session = LiveSession::spawn_env(Some(temp.path()), &[]);
    session.handshake();
    session.send(
        &json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"index_status","arguments":{}}}),
    );
    session.send(&bad);
    let with_notif = session.recv_raw();
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");

    assert_eq!(
        bare[0], with_notif,
        "notification perturbed the error bytes"
    );
    assert_tool_error_shape(&parse_line(&with_notif));
}

/// F1: across a fault catalog spanning both error levels, every error response
/// fails closed: JSON-RPC errors carry no `result`, tool errors carry no
/// top-level `error` and no `structuredContent`, and NO error's raw bytes
/// contain any success-payload key. Unknown-method spellings additionally
/// share one code with identical error-object keys.
/// INTENT: 9-call fault catalog fails closed: no result/structuredContent/payload keys; rpc keys uniform.
/// KILLS: payload-smuggle-in-error, key-divergence.
/// ABSORBS: none.
#[test]
fn failclosed_error_responses_carry_no_success_members_or_payload_keys() {
    let temp = file_tree();
    let outside = tempfile::tempdir().unwrap();
    let escaped = outside.path().display().to_string();
    let mut calls = vec![
        tool_call(1, "no_such_tool", json!({})),
        tool_call(2, "keyword_search", json!({"query": "x", "limit": 0})),
        tool_call(3, "keyword_search", json!({"query": "x", "root": escaped})),
        tool_call(4, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
        tool_call(
            5,
            "code_read",
            json!({"ids": ["src/lib.rs#L1-L1"], "root": 42}),
        ),
        tool_call(6, "index_repo", json!({"bogus": 1})),
        json!({"jsonrpc":"2.0","id":7,"method":"missing"}),
        json!({"jsonrpc":"2.0","id":8,"method":"tools/unknown"}),
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{}}),
    ];
    let n = calls.len();
    let raw = rpc_session_raw(std::mem::take(&mut calls), Some(temp.path()));
    assert_eq!(raw.len(), n);
    for line in &raw[..6] {
        assert_tool_error_shape(&parse_line(line));
    }
    for line in &raw[6..] {
        assert_jsonrpc_error(&parse_line(line), -32601);
    }
    // Same JSON-RPC fault, two spellings: one code (above) and identical
    // error-object member keys.
    let first_keys: Vec<String> = parse_line(&raw[6])["error"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    for line in &raw[7..] {
        let keys: Vec<String> = parse_line(line)["error"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, first_keys, "error-object keys diverged: {line}");
    }
    // Fail-closed byte scan: no success-payload key may appear in any error.
    const FORBIDDEN: [&str; 5] = [
        "structuredContent",
        "\"nodes\":",
        "\"h\":",
        "\"zn\":",
        "\"file_count\":",
    ];
    for line in &raw {
        for key in FORBIDDEN {
            assert!(
                !line.contains(key),
                "error response smuggles success payload {key}: {line}"
            );
        }
    }
}

/// F2: backend failures (corrupt index on all 5 search channels; deleted
/// read target) surface as tool errors whose bytes carry no partial payloads.
/// Positive controls on a healthy tree prove the payload keys WOULD be visible
/// if present, so the absence checks are not vacuous.
/// INTENT: corrupt ×5 channels + deleted target: tool errors carry no partial payloads, positive controls first.
/// KILLS: partial-payload-smuggle.
/// ABSORBS: none.
/// OVERLAP: P2 corrupt-channels trigger (adds byte-absence + controls).
#[test]
fn failclosed_backend_failures_carry_no_partial_payloads() {
    const FORBIDDEN: [&str; 5] = [
        "structuredContent",
        "\"nodes\":",
        "\"h\":",
        "\"zn\":",
        "\"file_count\":",
    ];

    // Positive controls: healthy successes DO carry the payload keys.
    let healthy = indexed_tree();
    let ok = rpc_session_raw(
        vec![
            tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(healthy.path()),
    );
    assert_eq!(ok.len(), 2);
    assert_eq!(parse_line(&ok[0])["result"]["isError"], false, "{}", ok[0]);
    assert_eq!(parse_line(&ok[1])["result"]["isError"], false, "{}", ok[1]);
    assert!(ok[0].contains("\"nodes\""), "control lost nodes: {}", ok[0]);
    assert!(
        ok[1].contains("\"h\"") && ok[1].contains("\"zn\""),
        "control lost hits: {}",
        ok[1]
    );

    // Corrupt backend: every search channel fails closed, no partial hits.
    let corrupt = indexed_tree();
    corrupt_index_db(corrupt.path());
    let calls: Vec<Value> = SEARCH_CHANNELS
        .iter()
        .enumerate()
        .map(|(i, channel)| {
            tool_call(
                i as u32 + 1,
                channel,
                json!({"query": "target_symbol", "limit": 4}),
            )
        })
        .collect();
    let raw = rpc_session_raw(calls, Some(corrupt.path()));
    assert_eq!(raw.len(), SEARCH_CHANNELS.len());
    for line in &raw {
        assert_tool_error_shape(&parse_line(line));
        for key in FORBIDDEN {
            assert!(
                !line.contains(key),
                "backend failure smuggles partial payload {key}: {line}"
            );
        }
    }

    // Deleted read target: read-time failure fails closed, no partial nodes.
    let temp = file_tree();
    std::fs::remove_file(temp.path().join("src").join("lib.rs")).unwrap();
    let gone = rpc_session_raw(
        vec![tool_call(
            1,
            "code_read",
            json!({"ids": ["src/lib.rs#L1-L1"]}),
        )],
        Some(temp.path()),
    );
    assert_eq!(gone.len(), 1);
    assert_tool_error_shape(&parse_line(&gone[0]));
    for key in FORBIDDEN {
        assert!(
            !gone[0].contains(key),
            "deleted-file failure smuggles partial payload {key}: {}",
            gone[0]
        );
    }
}
