//! Pass 2 (errorapi, Mission E2): backend-failure propagation oracles.
//!
//! Pass 1 pins the error TAXONOMY (which trigger yields which row). This file
//! pins PROPAGATION: every backend failure must surface as the documented
//! envelope, never as a success result and never as a different level's
//! error. Method: hold the call arguments constant and vary only backend
//! state (index absent/corrupt/pinned-at-dir, env-gated refusal, filesystem
//! mutation, session registry state). Outcome differences are then
//! backend-attributed by construction.
//!
//! Pinned propagation map (discriminants are codes and shapes, never message
//! text; `success` = `isError: false` + `structuredContent`, `toolerr` = the
//! uniform tool-error envelope with no top-level `error`):
//!
//! | row | backend state (args held valid)            | mapping                                  |
//! |-----|--------------------------------------------|------------------------------------------|
//! | P1  | no index, all 5 search channels            | success all; miss x4, native hits x1       |
//! | P2  | corrupt index db, all 5 search channels    | toolerr; `code_read` still success         |
//! | P3  | `ASGREP_NEURAL_EMBED=1`, neural unshipped  | `semantic_search` toolerr; unset: success  |
//! | P4  | read-target file deleted mid-session       | same id flips success->toolerr->success    |
//! | P5  | symlink escapes workspace (unix)           | `code_read` toolerr; session survives      |
//! | P6  | multi-id read with one bad id              | whole-call toolerr; no partial `nodes`     |
//! | P7  | compact id before/after search registration| toolerr, then success for the same shape   |
//! | P8  | sequential mixed-outcome transcript         | per-call envelopes, ids echo 1..=7 in order |
//! | P9  | pipelined mixed batch incl. backend failure| every id exactly once; good results intact |
//! | P10 | empty vs corrupt index, identical query    | FOLDED into P2 as closing contrast lines     |
//!
//! Non-duplication: pass 1 pins parse-level rows (arg bounds/types, unknown
//! keys, sandbox escape, JSON-RPC codes); recovery pins corrupt/pinned-garbage
//! index faults and heal flows; protocol pins single-channel miss/binary/EOF
//! rows and `isError` bits. E2 adds cross-channel uniformity, env-gated and
//! path-state backend refusals, read atomicity, registry-dependent resolution,
//! and mixed-outcome transcripts with a backend-failure row (prior mixed
//! transcripts carry only unknown-tool/method rows).
//!
//! Transport comes from [`error_testkit`](self::error_testkit): every
//! live-session read and process wait is timeout-bounded.

#[path = "error_testkit.rs"]
mod error_testkit;

use error_testkit::*;
use serde_json::{json, Value};
use std::collections::HashMap;

/// P1: no index is a success on EVERY search channel, never a tool error --
/// but the shape follows each channel's contract. Index-backed channels
/// (`search`, `keyword_search`, `semantic_search`, `code_search`) return the
/// `empty_index` miss (`why`, `zn: 0`, empty `h`); native `ast_search` needs
/// no index and returns hits. Args are valid; backend state alone decides.
/// INTENT: no index: success on all 5 channels, empty_index miss ×4 + native hits ×1.
/// KILLS: miss-as-tool-error, channel-divergence.
/// ABSORBS: none.
#[test]
fn backend_empty_index_is_success_miss_on_every_channel() {
    let temp = file_tree();
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
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), SEARCH_CHANNELS.len());
    for (response, channel) in responses.iter().zip(SEARCH_CHANNELS) {
        assert_tool_success_shape(response);
        let body = tool_body(response);
        if channel == "ast_search" {
            assert!(body["zn"].as_u64().unwrap_or(0) >= 1, "{channel}: {body:#}");
            assert!(
                !body["h"].as_array().unwrap().is_empty(),
                "{channel}: {body:#}"
            );
        } else {
            assert_eq!(body["why"], "empty_index", "{channel}: {body:#}");
            assert_eq!(body["zn"], 0, "{channel}: {body:#}");
            assert_eq!(body["h"], json!([]), "{channel}: {body:#}");
        }
    }
}

/// P2: a corrupt index db is a tool error on EVERY search channel -- never a
/// silent empty success, never fabricated hits -- while `code_read` serves
/// files directly and stays a success. Closing lines hold the P10 contrast:
/// the same valid query against an empty index is a success miss, so backend
/// state alone flips miss<->toolerr and neither mode is confused.
/// INTENT: corrupt db: tool error ×5 channels, code_read still success.
/// KILLS: silent-empty-success, fabricated-hits; state-confusion(miss↔error) via the closing contrast.
/// ABSORBS: backend_empty_vs_corrupt_index_map_to_miss_vs_tool_error (P10 contrast as closing lines; P10's extra session pair deleted).
/// OVERLAP: recovery corrupt pins (adds cross-channel uniformity + read contrast).
#[test]
fn backend_corrupt_index_is_tool_error_on_every_channel() {
    let temp = indexed_tree();
    corrupt_index_db(temp.path());
    let mut calls: Vec<Value> = SEARCH_CHANNELS
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
    calls.push(tool_call(
        6,
        "code_read",
        json!({"ids": ["src/lib.rs#L1-L1"]}),
    ));
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), 6);
    for (response, id) in responses[..5].iter().zip(1..=5) {
        assert_tool_error_shape(response);
        assert_eq!(response["id"], id, "{response:#}");
    }
    assert_tool_success_shape(&responses[5]);
    assert_eq!(
        tool_body(&responses[5])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[5]
    );

    // P10 contrast, folded: identical query, empty backend — a success miss.
    let empty = file_tree();
    let miss = rpc_session(
        vec![tool_call(
            1,
            "search",
            json!({"query": "target_symbol", "limit": 4}),
        )],
        Some(empty.path()),
    );
    assert_tool_success_shape(&miss[0]);
    let body = tool_body(&miss[0]);
    assert_eq!(body["why"], "empty_index", "{body:#}");
    assert_eq!(body["zn"], 0, "{body:#}");
}

/// P3: identical valid `semantic_search` args succeed by default but are a
/// tool error (never a JSON-RPC error, never a success) when the backend is
/// configured for neural embed this build cannot serve. The refusal is
/// backend-attributed: only the env differs.
/// INTENT: same semantic_search args: success default, tool error under ASGREP_NEURAL_EMBED=1.
/// KILLS: gate-drop, refusal-as-success.
/// ABSORBS: none.
#[test]
fn backend_neural_gate_refusal_is_tool_error_not_success() {
    let temp = indexed_tree();
    let args = json!({"query": "target_symbol", "limit": 4});
    let ok = rpc_session(
        vec![tool_call(1, "semantic_search", args.clone())],
        Some(temp.path()),
    );
    assert_tool_success_shape(&ok[0]);
    let refused = rpc_session_env(
        vec![tool_call(1, "semantic_search", args)],
        Some(temp.path()),
        &[("ASGREP_NEURAL_EMBED", "1")],
    );
    assert_eq!(refused.len(), 1);
    assert_tool_error_shape(&refused[0]);
    assert_eq!(refused[0]["id"], 1, "{:#}", refused[0]);
}

/// P4: the SAME well-formed node id flips success -> tool error -> success
/// within one session as the backend file is deleted and restored. The id
/// parses identically every time, so the middle failure is purely
/// backend-attributed (read-time, not parse-time), and it never surfaces as
/// a success with empty nodes.
/// INTENT: same node id flips success→toolerr→success as file deleted/restored.
/// KILLS: parse-vs-read-confusion, stale-success.
/// ABSORBS: none.
#[test]
fn backend_file_deleted_mid_session_flips_read_to_tool_error() {
    let temp = file_tree();
    let victim = temp.path().join("src").join("lib.rs");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&read_call(1), 1);
    assert_tool_success_shape(&before);
    assert_eq!(
        tool_body(&before)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{before:#}"
    );

    std::fs::remove_file(&victim).unwrap();
    let during = session.call(&read_call(2), 2);
    assert_tool_error_shape(&during);

    std::fs::write(&victim, FIXTURE_SOURCE).unwrap();
    let after = session.call(&read_call(3), 3);
    assert_tool_success_shape(&after);
    assert_eq!(
        tool_body(&after)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{after:#}"
    );

    session.finish_clean();
}

/// P5: a symlink inside the workspace pointing outside passes id-shape parse
/// and fails at filesystem containment -- a backend failure, hence a tool
/// error -- and the session serves a valid read right after.
/// INTENT: symlink escape fails containment as tool error (unix), session serves after.
/// KILLS: jail-drop, session-poison.
/// ABSORBS: none.
#[cfg(unix)]
#[test]
fn backend_symlink_escape_read_is_tool_error_session_survives() {
    let workspace = file_tree();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.rs"), "fn secret() {}\n").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.rs"),
        workspace.path().join("link.rs"),
    )
    .unwrap();
    let responses = rpc_session(
        vec![
            tool_call(1, "code_read", json!({"ids": ["link.rs#L1-L1"]})),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(workspace.path()),
    );
    assert_eq!(responses.len(), 2);
    assert_tool_error_shape(&responses[0]);
    assert_eq!(responses[0]["id"], 1, "{:#}", responses[0]);
    assert_tool_success_shape(&responses[1]);
    assert_eq!(
        tool_body(&responses[1])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[1]
    );
}

/// P6: a multi-id `code_read` with one unreadable id fails the WHOLE call as
/// a tool error in either position -- no partial `nodes` leak through any
/// channel -- and a lone good read succeeds in the same session.
/// INTENT: multi-id read with one bad id fails whole call either position, lone good succeeds.
/// KILLS: partial-nodes-leak.
/// ABSORBS: none.
#[test]
fn backend_multi_id_read_fails_atomically_no_partial_nodes() {
    let temp = file_tree();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1", "src/lib.rs#L1-L99"]}),
            ),
            tool_call(
                2,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L99", "src/lib.rs#L1-L1"]}),
            ),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for (response, id) in responses[..2].iter().zip(1..=2) {
        assert_tool_error_shape(response);
        assert_eq!(response["id"], id, "{response:#}");
    }
    assert_tool_success_shape(&responses[2]);
    assert_eq!(
        tool_body(&responses[2])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[2]
    );
}

/// P7: a well-shaped compact id with an empty session registry is a tool
/// error; after a search registers that registry, the hit's own compact id
/// resolves to a success. Same id shape, opposite outcomes -- the failure is
/// session-backend state, not syntax.
/// INTENT: compact id toolerr before registry, success after search registers.
/// KILLS: registry-bypass, unregistered-success.
/// ABSORBS: none.
#[test]
fn backend_compact_id_resolves_only_after_search_registration() {
    let temp = indexed_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let unregistered = session.call(&tool_call(1, "code_read", json!({"ids": ["0:1-1"]})), 1);
    assert_tool_error_shape(&unregistered);

    let search = session.call(&search_call(2, "keyword_search", 4), 2);
    assert_tool_success_shape(&search);
    let hit_id = tool_body(&search)["h"][0][0]
        .as_str()
        .expect("compact hit id")
        .to_owned();
    assert!(
        !hit_id.contains("#L"),
        "expected a compact id, got {hit_id}"
    );

    let resolved = session.call(&tool_call(3, "code_read", json!({"ids": [hit_id]})), 3);
    assert_tool_success_shape(&resolved);
    assert_eq!(
        tool_body(&resolved)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{resolved:#}"
    );

    session.finish_clean();
}

/// P8: one sequential transcript, five outcome classes. Unknown-tool,
/// invalid-args, and backend-failure rows are uniform tool errors (never
/// top-level `error`, never success); the unknown method is -32601 with no
/// `result`; successes bracket the failures with ids echoing 1..=7 in order.
/// No bad call drops or reorders any other result.
/// INTENT: 7-call mixed transcript: 3 tool-error classes uniform, -32601 rpc row, successes intact, ids 1..=7.
/// KILLS: result-drop, reorder, level-confusion.
/// ABSORBS: none.
#[test]
fn propagation_sequence_reports_per_call_errors_without_dropping_results() {
    let temp = indexed_tree();
    let responses = rpc_session(
        vec![
            tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
            tool_call(3, "no_such_tool", json!({})),
            tool_call(4, "keyword_search", json!({"query": "x", "limit": 0})),
            json!({"jsonrpc":"2.0","id":5,"method":"missing"}),
            tool_call(
                6,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(7, "index_status", json!({})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 7);
    for (response, id) in responses.iter().zip(1..=7) {
        assert_eq!(response["id"], id, "{response:#}");
    }
    // Successes: full success shape, real payloads.
    assert_tool_success_shape(&responses[0]);
    assert_eq!(
        tool_body(&responses[0])["nodes"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        responses[0]
    );
    assert_tool_success_shape(&responses[5]);
    assert!(
        !tool_body(&responses[5])["h"].as_array().unwrap().is_empty(),
        "{:#}",
        responses[5]
    );
    assert_tool_success_shape(&responses[6]);
    assert_eq!(
        tool_body(&responses[6])["file_count"],
        1,
        "{:#}",
        responses[6]
    );
    // The three tool-level failure classes share one envelope, mutually
    // indistinguishable by shape -- and none leaks to another level.
    for response in &responses[1..4] {
        assert_tool_error_shape(response);
    }
    // Unknown method is the only JSON-RPC-level error.
    assert_jsonrpc_error(&responses[4], -32601);
}

/// P9: a pipelined batch with a backend failure plus an invalid-args row:
/// every id returns exactly once, each response carries exactly one of
/// `result`/`error`, per-id classes hold, and the good results are intact.
/// Order is not pinned (concurrent service).
/// INTENT: pipelined 6-call batch: every id exactly once, exactly one of result/error, good intact.
/// KILLS: id-drop, id-dup, result+error-coexist.
/// ABSORBS: none.
#[test]
fn propagation_pipelined_batch_with_backend_failure_keeps_every_id() {
    let temp = indexed_tree();
    let responses = rpc_pipeline(
        vec![
            tool_call(11, "index_status", json!({})),
            tool_call(
                12,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
            tool_call(13, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
            tool_call(14, "keyword_search", json!({"query": "x", "limit": 0})),
            tool_call(15, "no_such_tool", json!({})),
            json!({"jsonrpc":"2.0","id":16,"method":"missing"}),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 6, "{responses:#?}");
    let mut by_id: HashMap<i64, &Value> = HashMap::new();
    for response in &responses {
        let id = response["id"].as_i64().expect("numeric id echo");
        assert!(by_id.insert(id, response).is_none(), "duplicate id {id}");
        let has_result = response.get("result").is_some();
        let has_error = response.get("error").is_some();
        assert!(
            has_result ^ has_error,
            "exactly one of result/error: {response:#}"
        );
    }
    for id in 11..=16 {
        assert!(by_id.contains_key(&id), "missing id {id}");
    }
    assert_tool_success_shape(by_id[&11]);
    assert_eq!(tool_body(by_id[&11])["file_count"], 1);
    assert_tool_success_shape(by_id[&12]);
    assert!(!tool_body(by_id[&12])["h"].as_array().unwrap().is_empty());
    assert_tool_error_shape(by_id[&13]);
    assert_tool_error_shape(by_id[&14]);
    assert_tool_error_shape(by_id[&15]);
    assert_jsonrpc_error(by_id[&16], -32601);
}
