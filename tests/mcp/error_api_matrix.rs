//! Error-API matrix for `ast-sgrep-mcp`.
//!
//! F35 collapse of `error_api_pass{1,2,3,4}.rs` into one parametric target.
//! Pure collapse: every test below is unchanged from its pass-file original —
//! same triggers, same assertions, same INTENT/KILLS/ABSORBS audit lines. Only
//! the file layout changed (one import block, E1-E4 sections). Per-pass module
//! docs are kept as section banners; shared setup lives in
//! `tests/mcp/error_testkit.rs` (canonical beats promoted to `ast-sgrep-testkit`).
//!
//! Matrix (mission -> test):
//! - E1 taxonomy: `taxonomy_unknown_method_is_32601_with_id_echo`, `taxonomy_unshaped_envelope_is_32602`, `taxonomy_invalid_request_is_32600`, `taxonomy_pre_initialize_request_is_32600`, `taxonomy_unknown_tool_is_32601_jsonrpc_error`, `taxonomy_search_arg_rejections_share_32602_admission`, `taxonomy_code_read_arg_rejections_share_tool_error_shape`, `taxonomy_code_read_node_id_rows_share_tool_error_shape`, `taxonomy_index_tool_arg_rejections_share_32602_admission`, `taxonomy_sandbox_escape_is_tool_error_across_all_tools`, `taxonomy_notification_produces_no_response_session_continues`
//! - E2 propagation: `backend_empty_index_is_success_miss_on_every_channel`, `backend_corrupt_index_is_tool_error_on_every_channel`, `backend_neural_gate_refusal_is_tool_error_not_success`, `backend_file_deleted_mid_session_flips_read_to_tool_error`, `backend_symlink_escape_read_is_tool_error_session_survives`, `backend_multi_id_read_fails_atomically_no_partial_nodes`, `backend_compact_id_resolves_only_after_search_registration`, `propagation_sequence_reports_per_call_errors_without_dropping_results`, `propagation_pipelined_batch_with_backend_failure_keeps_every_id`
//! - E3 relations: `consistency_mistyped_root_is_same_tool_error_on_every_root_tool`, `consistency_unknown_argument_is_same_tool_error_on_every_tool`, `consistency_limit_fault_spellings_share_one_tool_error_code`, `determinism_identical_errors_are_byte_identical_across_sessions`, `determinism_repeated_bad_call_in_one_session_is_identical_modulo_id`, `position_permuted_transcript_yields_same_per_call_responses`, `position_notification_before_bad_call_leaves_error_bytes_unchanged`, `failclosed_error_responses_carry_no_success_members_or_payload_keys`, `failclosed_backend_failures_carry_no_partial_payloads`
//! - E4 drills: `drill_index_db_deleted_mid_session_miss_then_live_reindex_heals`, `drill_index_db_overwritten_mid_session_refused_then_live_heal`, `drill_malformed_envelopes_mid_session_codes_then_session_usable`, `drill_bad_argument_storm_mid_session_uniform_then_usable`, `drill_sandbox_escape_mid_session_refused_session_unpoisoned`, `drill_chained_double_fault_staged_heal_restores_live_session`
//!
//! No asserted behavior changed in this collapse (see F35: keep every distinct
//! behavior assertion, drop pure duplication — no pure duplication remained;
//! net test count is unchanged).
#[path = "error_testkit.rs"]
mod error_testkit;

use error_testkit::*;
use serde_json::{json, Value};
use std::collections::HashMap;

// ── E1 taxonomy inventory (from error_api_pass1.rs) ──
// Pass 1 (errorapi, Mission E1): error-taxonomy inventory for ast-sgrep-mcp.
//
// This file pins the FULL MCP error surface as a table: one test per
// JSON-RPC code / result shape, each with a hand-built trigger. Earlier
// suites pin scattered rows (`protocol.rs`: -32601 unknown method,
// unknown-tool result, parse-error fail-closed; recovery: fault shapes and
// startup exit). This file pins the TABLE and the GAPS: -32600 rows,
// pre-init admission, notification silence, index-tool arg rejections, and
// the remaining per-tool bound/type rows.
//
// Pinned taxonomy under fastmcp-rust 0.13 (auto facade, legacy era;
// discriminants are codes and shapes, never message text):
//
// | row | trigger                                              | discriminant                          |
// |-----|------------------------------------------------------|---------------------------------------|
// | J1  | unknown method (`missing`, `tools/unknown`)          | -32601, id echo, no `result`            |
// | J2  | unshaped `tools/call`/`initialize` params            | -32602, id echo, no `result`            |
// | J3  | invalid request (no method, no jsonrpc, batch, ...) | -32600, id echoed when sent, no `result`|
// | J4  | any request before `initialize` (fatal)           | -32600, id echo, no `result`, exit != 0 |
// | T0  | inputSchema violation (bounds/types/unknown keys)    | -32602 (framework admission), no `result` |
// | T1  | unknown tool name                                    | -32601 JSON-RPC, no `result`            |
// | T4  | code_read node-id shape rejection                    | tool error, no top-level `error`        |
// | T5b | schema-valid but semantically bad root (file root)   | tool error, no top-level `error`        |
// | T6  | per-call root escaping the workspace                 | tool error, no top-level `error`        |
// | S1  | notification (absent/null id)                        | silence; session continues              |
//
// The admission line moved with the port: rmcp let every `tools/call`
// through to our handler (arg rejections were tool errors); fastmcp
// validates arguments against the declared inputSchema BEFORE dispatch and
// admits violations as -32602 "Input validation failed". Everything the
// schema cannot see (node-id shapes, file roots, sandbox escapes) still
// fails inside dispatch as a tool error.
//
// Enumerated but wire-unreachable (no row): -32603 internal error
// (`handler.rs` join/catalog failures need a task fault, not an input).
// -32700 parse error is pinned in `protocol.rs` (fastmcp fails closed with
// the spec-mandated code and a nonzero exit, where rmcp ignored the line).
//
// Transport comes from [`error_testkit`](self::error_testkit): every
// live-session read and process wait is timeout-bounded.
//

/// J1: unknown methods are -32601 with the id echoed and no `result`.
/// INTENT: unknown methods -32601, id echo, no result.
/// KILLS: code-swap, id-drop.
/// ABSORBS: none.
/// OVERLAP: protocol.rs -32601 (repin in table context).
#[test]
fn taxonomy_unknown_method_is_32601_with_id_echo() {
    let responses = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"missing"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/unknown"}),
        ],
        None,
    );
    assert_eq!(responses.len(), 2);
    for (response, id) in responses.iter().zip([1, 2]) {
        assert_jsonrpc_error(response, -32601);
        assert_eq!(response["id"], id, "{response:#}");
    }
}

/// J2: fastmcp maps unshaped `tools/call` params to -32602 ("invalid exact
/// MCP 2024-11-05 parameters"): missing `name`, mistyped `name`,
/// non-object/null `arguments`, and null params. A mid-session
/// `initialize` with empty params is envelope-class -32600 instead.
/// rmcp answered -32601 here — the pin moved with the serve layer.
/// INTENT: unshaped tools/call params map -32602 incl. non-object arguments; initialize params map -32600.
/// KILLS: code-swap(-32601), dispatch-on-unshaped.
/// ABSORBS: none.
#[test]
fn taxonomy_unshaped_envelope_is_32602() {
    let responses = rpc_session(
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":null}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":42,"arguments":{}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"keyword_search","arguments":[]}}),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"keyword_search","arguments":"x"}}),
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"keyword_search","arguments":42}}),
            json!({"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}),
        ],
        None,
    );
    assert_eq!(responses.len(), 7);
    for (response, id) in responses.iter().zip(1..=6) {
        assert_jsonrpc_error(response, -32602);
        assert_eq!(response["id"], id, "{response:#}");
    }
    // initialize params are envelope-class, not parameters-class.
    assert_jsonrpc_error(&responses[6], -32600);
    assert_eq!(responses[6]["id"], 7, "{:#}", responses[6]);
}

/// J3: invalid requests are -32600 with no `result`: missing method,
/// missing `jsonrpc`, batch arrays, and mistyped `params`. fastmcp ECHOES
/// the request id on -32600 (rmcp dropped it) — the pin is the code and the
/// absent `result`, not the id handling.
/// INTENT: invalid requests -32600, no result member.
/// KILLS: code-swap, result-on-invalid.
/// ABSORBS: none.
#[test]
fn taxonomy_invalid_request_is_32600() {
    let mut session = LiveSession::spawn(None);
    session.handshake();
    for payload in [
        json!({ "jsonrpc": "2.0", "id": 2 }),
        json!({ "id": 3, "method": "ping" }),
        json!([{ "jsonrpc": "2.0", "id": 4, "method": "ping" }]),
        json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": "x" }),
    ] {
        session.send(&payload);
    }
    let mut responses = Vec::new();
    for _ in 0..4 {
        responses.push(session.recv());
    }
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
    assert_eq!(responses.len(), 4);
    for response in &responses {
        assert_jsonrpc_error(response, -32600);
    }
}

/// J4: any request before `initialize` lacking modern `_meta` is refused by
/// lifecycle admission as -32600 ("initialize is the only request allowed
/// before lifecycle admission"), with the id echoed and no `result`. rmcp
/// answered -32602 here — the pin moved with the serve layer. The error is
/// answered in-band and the server terminates CLEANLY on EOF — the failure
/// lives on the wire, not in the process exit status. One process per
/// trigger.
/// INTENT: pre-init requests -32600 with id echo, in-band, clean exit.
/// KILLS: code-swap, silent-drop.
/// ABSORBS: none.
#[test]
fn taxonomy_pre_initialize_request_is_32600() {
    for (payload, id) in [
        (
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            1,
        ),
        (tool_call(2, "index_status", json!({})), 2),
    ] {
        let mut session = LiveSession::spawn(None);
        session.send(&payload);
        let response = session.recv();
        assert_jsonrpc_error(&response, -32600);
        assert_eq!(response["id"], id, "{response:#}");
        session.close_stdin();
        let status = session.wait_clean();
        assert!(
            status.success(),
            "in-band admission error must exit cleanly: {status}"
        );
    }
}

/// T1: unknown tool names are JSON-RPC -32601 method-not-found on the
/// legacy era (rmcp answered a tool error; fastmcp's auto facade owns tool
/// routing). Case, whitespace, and near-miss variants. No `result`.
/// INTENT: unknown/case/whitespace tool names are -32601 JSON-RPC, never a fabricated result.
/// KILLS: tool-error-shape, fuzzy-name-match.
/// ABSORBS: none.
/// OVERLAP: protocol.rs unknown-tool (adds case/whitespace variants).
#[test]
fn taxonomy_unknown_tool_is_32601_jsonrpc_error() {
    let temp = file_tree();
    let responses = rpc_session(
        vec![
            tool_call(1, "no_such_tool", json!({})),
            tool_call(2, "Search", json!({"query": "x"})),
            tool_call(3, "keyword_search ", json!({"query": "x"})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for (response, id) in responses.iter().zip(1..=3) {
        assert_jsonrpc_error(response, -32601);
        assert_eq!(response["id"], id, "{response:#}");
    }
}

/// T2 (now T0): search-channel arg rejections (bound ceilings, mistyped
/// scalars, unknown keys) are admitted by fastmcp's inputSchema validation
/// BEFORE dispatch as -32602 — the arguments never reach our handler. Gap
/// rows: `budget_tokens` above max / negative / mistyped, mistyped
/// `resend_seen` / `preview` / `file_filter` / `lang`, negative `limit`,
/// mistyped `query` / `root`, unknown key.
/// INTENT: 11 search bound/type/unknown-key rejections admit as -32602.
/// KILLS: arg-accepted, dispatch-on-invalid.
/// ABSORBS: none.
#[test]
fn taxonomy_search_arg_rejections_share_32602_admission() {
    let temp = file_tree();
    let bad_args = vec![
        json!({"query": "x", "budget_tokens": 65537}),
        json!({"query": "x", "budget_tokens": -5}),
        json!({"query": "x", "budget_tokens": "many"}),
        json!({"query": "x", "resend_seen": "yes"}),
        json!({"query": "x", "preview": 42}),
        json!({"query": "x", "file_filter": 42}),
        json!({"query": "x", "lang": 42}),
        json!({"query": "x", "limit": -1}),
        json!({"query": 42}),
        json!({"query": "x", "root": 42}),
        json!({"query": "x", "budgettokens": 8}),
    ];
    let calls: Vec<Value> = bad_args
        .into_iter()
        .enumerate()
        .map(|(i, args)| tool_call(i as u32 + 1, "keyword_search", args))
        .collect();
    let n = calls.len();
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), n);
    for response in &responses {
        assert_jsonrpc_error(response, -32602);
    }
}

/// T3: code_read arg rejections share the tool-error shape, and exactly 20
/// ids still succeed (pass 3 pins 21 rejected; this pins the ceiling).
/// INTENT: 6 code_read bound/type rejections share shape + 20-id ceiling still succeeds.
/// KILLS: arg-accepted, ceiling-off-by-one.
/// ABSORBS: none.
/// OVERLAP: pass 3 pins 21 rejected (this pins the ceiling).
#[test]
fn taxonomy_code_read_arg_rejections_share_tool_error_shape() {
    let temp = file_tree();
    let twenty: Vec<Value> = (0..20).map(|_| json!("src/lib.rs#L1-L1")).collect();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "max_chars": 1000001}),
            ),
            tool_call(
                2,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "max_chars": -1}),
            ),
            tool_call(
                3,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "max_chars": "many"}),
            ),
            tool_call(
                4,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "context_lines": -1}),
            ),
            tool_call(
                5,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "context_lines": 1.5}),
            ),
            tool_call(
                6,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": 42}),
            ),
            tool_call(7, "code_read", json!({"ids": twenty})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 7);
    // Rows 1-6: inputSchema violations — fastmcp admits them as -32602
    // before dispatch.
    for response in &responses[..6] {
        assert_jsonrpc_error(response, -32602);
    }
    // Row 7: exactly 20 ids still succeeds (pass 3 pins 21 rejected; this
    // pins the ceiling).
    assert_tool_success_shape(&responses[6]);
    assert_eq!(
        tool_body(&responses[6])["nodes"].as_array().map(Vec::len),
        Some(20),
        "{:#}",
        responses[6]
    );
}

/// T4: node-id shape rejections share the tool-error shape: missing range,
/// wrong separator, absolute path, directory target, unknown file.
/// INTENT: 6 node-id shape rejections share tool-error shape.
/// KILLS: shape-accepted, level-escalation.
/// ABSORBS: none.
#[test]
fn taxonomy_code_read_node_id_rows_share_tool_error_shape() {
    let temp = file_tree();
    // Round-12 ref parity: single-line `#Lstart` and LSP `path:start` are now
    // VALID, so they left the bad set; an unresolved compact-id shape
    // (`:start-end` with no registry entry) keeps a row.
    let bad_ids = [
        "src/lib.rs",
        "src/lib.rs#1-2",
        "src/lib.rs:1-2",
        "/etc/hosts#L1-L1",
        "src#L1-L1",
        "src/nope.rs#L1-L1",
    ];
    let calls: Vec<Value> = bad_ids
        .iter()
        .enumerate()
        .map(|(i, id)| tool_call(i as u32 + 1, "code_read", json!({"ids": [id]})))
        .collect();
    let n = calls.len();
    let responses = rpc_session(calls, Some(temp.path()));
    assert_eq!(responses.len(), n);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

/// T5: index-tool arg rejections. Schema violations (unknown keys on both
/// tools, mistyped `force`, mistyped `root`) admit as -32602 before
/// dispatch; the schema-valid-but-semantic row (a `root` pointing at a
/// FILE) still fails inside dispatch as a tool error. No indexing runs.
/// INTENT: 6 index-tool schema violations admit as -32602; file-root stays a tool error.
/// KILLS: arg-accepted, index-before-validate.
/// ABSORBS: none.
#[test]
fn taxonomy_index_tool_arg_rejections_share_32602_admission() {
    let temp = file_tree();
    // The file-root row needs a second file; the canonical tree stays
    // single-file so `file_count` pins elsewhere stay exact.
    std::fs::write(temp.path().join("plain.txt"), "x\n").unwrap();
    let file_root = temp.path().join("plain.txt").display().to_string();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({"bogus": 1})),
            tool_call(2, "index_status", json!({"root": true})),
            tool_call(3, "index_repo", json!({"bogus": 1})),
            tool_call(4, "index_repo", json!({"force": "yes"})),
            tool_call(5, "index_repo", json!({"force": 1})),
            tool_call(6, "index_repo", json!({"root": 42})),
            tool_call(7, "index_repo", json!({"root": file_root})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 7);
    for response in &responses[..6] {
        assert_jsonrpc_error(response, -32602);
    }
    assert_tool_error_shape(&responses[6]);
}

/// T6: a per-call root outside the configured workspace is a tool error on
/// every tool that takes one (`protocol.rs` pins `index_status` only).
/// INTENT: per-call root outside workspace is tool error on all 4 root-taking tools.
/// KILLS: jail-drop, per-tool-divergence.
/// ABSORBS: none.
/// OVERLAP: protocol.rs index_status-only (extends to all tools).
#[test]
fn taxonomy_sandbox_escape_is_tool_error_across_all_tools() {
    let workspace = file_tree();
    let outside = tempfile::tempdir().unwrap();
    let escaped = outside.path().display().to_string();
    let responses = rpc_session(
        vec![
            tool_call(
                1,
                "keyword_search",
                json!({"query": "x", "limit": 4, "root": escaped}),
            ),
            tool_call(
                2,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": escaped}),
            ),
            tool_call(3, "index_status", json!({"root": escaped})),
            tool_call(4, "index_repo", json!({"root": escaped})),
        ],
        Some(workspace.path()),
    );
    assert_eq!(responses.len(), 4);
    for response in &responses {
        assert_tool_error_shape(response);
    }
}

/// S1: notifications produce no response. rmcp 3.7.0 splits the two shapes:
/// an id-OMITTED message stays silent (a true notification), while `"id":
/// null` is rejected in-band as -32600 with no id echo (JSON-RPC: the id
/// member, when present, must be a string or number — null is reserved for
/// server parse-error replies). The ping (id 77) reply still arrives, and
/// the session exits cleanly.
/// INTENT: id-omitted notification silent; id:null draws -32600 (no echo); ping replies; clean exit.
/// KILLS: notification-response-leak, session-stall, null-id tolerated as notification.
/// ABSORBS: none.
#[test]
fn taxonomy_notification_produces_no_response_session_continues() {
    let mut session = LiveSession::spawn(None);
    session.handshake();
    session.send(
        &json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"index_status","arguments":{}}}),
    );
    session.send(&json!({"jsonrpc":"2.0","id":null,"method":"ping"}));
    session.send(&json!({"jsonrpc":"2.0","id":77,"method":"ping"}));
    let first = session.recv();
    assert_jsonrpc_error(&first, -32600);
    assert!(
        first.get("id").map(|v| v.is_null()).unwrap_or(true),
        "-32600 must not echo an id: {first:#}"
    );
    let second = session.recv();
    assert_eq!(second["id"], 77, "ping reply displaced: {second:#}");
    assert!(second.get("error").is_none(), "{second:#}");
    assert!(second.get("result").is_some(), "{second:#}");
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "MCP exited {status}");
}

// ── E2 propagation oracles (from error_api_pass2.rs) ──
// Pass 2 (errorapi, Mission E2): backend-failure propagation oracles.
//
// Pass 1 pins the error TAXONOMY (which trigger yields which row). This file
// pins PROPAGATION: every backend failure must surface as the documented
// envelope, never as a success result and never as a different level's
// error. Method: hold the call arguments constant and vary only backend
// state (index absent/corrupt/pinned-at-dir, env-gated refusal, filesystem
// mutation, session registry state). Outcome differences are then
// backend-attributed by construction.
//
// Pinned propagation map (discriminants are codes and shapes, never message
// text; `success` = `isError: false` + `structuredContent`, `toolerr` = the
// uniform tool-error envelope with no top-level `error`):
//
// | row | backend state (args held valid)            | mapping                                  |
// |-----|--------------------------------------------|------------------------------------------|
// | P1  | no index, all 5 search channels            | success all; miss x4, native hits x1       |
// | P2  | corrupt index db, all 5 search channels    | toolerr; `code_read` still success         |
// | P3  | `ASGREP_NEURAL_EMBED=1`, neural unshipped  | `semantic_search` toolerr; unset: success  |
// | P4  | read-target file deleted mid-session       | same id flips success->toolerr->success    |
// | P5  | symlink escapes workspace (unix)           | `code_read` toolerr; session survives      |
// | P6  | multi-id read with one bad id              | whole-call toolerr; no partial `nodes`     |
// | P7  | compact id before/after search registration| toolerr, then success for the same shape   |
// | P8  | sequential mixed-outcome transcript         | per-call envelopes, ids echo 1..=7 in order |
// | P9  | pipelined mixed batch incl. backend failure| every id exactly once; good results intact |
// | P10 | empty vs corrupt index, identical query    | FOLDED into P2 as closing contrast lines     |
//
// Non-duplication: pass 1 pins parse-level rows (arg bounds/types, unknown
// keys, sandbox escape, JSON-RPC codes); recovery pins corrupt/pinned-garbage
// index faults and heal flows; protocol pins single-channel miss/binary/EOF
// rows and `isError` bits. E2 adds cross-channel uniformity, env-gated and
// path-state backend refusals, read atomicity, registry-dependent resolution,
// and mixed-outcome transcripts with a backend-failure row (prior mixed
// transcripts carry only unknown-tool/method rows).
//
// Transport comes from [`error_testkit`](self::error_testkit): every
// live-session read and process wait is timeout-bounded.
//

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
    // The stale-range fault stays a tool error; unknown tool (-32601) and the
    // bound-violating limit (-32602) are framework-owned JSON-RPC failures.
    // None leaks to another level.
    assert_tool_error_shape(&responses[1]);
    assert_jsonrpc_error(&responses[2], -32601);
    assert_jsonrpc_error(&responses[3], -32602);
    // Unknown method shares the unknown-tool routing code.
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
    assert_jsonrpc_error(by_id[&14], -32602);
    assert_jsonrpc_error(by_id[&15], -32601);
    assert_jsonrpc_error(by_id[&16], -32601);
}

// ── E3 metamorphic relations (from error_api_pass3.rs) ──
// Pass 3 (errorapi, Mission E3): negative-path METAMORPHIC relations.
//
// Pass 1 pins the error TAXONOMY (which trigger yields which row) and pass 2
// pins PROPAGATION (backend state decides the envelope). This file pins
// RELATIONS between calls: properties that must hold when the same fault is
// replayed across tools, sessions, and sequence positions. No test here pins
// a single trigger-to-row mapping; every assertion compares two or more
// responses (or one response against a forbidden-payload predicate).
//
// Pinned relations (discriminants are codes, envelope shapes, and raw bytes,
// never message text):
//
// | row | relation                                              | assertion                              |
// |-----|-------------------------------------------------------|----------------------------------------|
// | C1  | same mistyped-`root` fault on all 8 root-taking tools | identical tool-error discriminants     |
// | C2  | same unknown-argument fault on all 8 named tools      | identical tool-error discriminants     |
// | C3  | same fault class, 4 `limit` spellings, one channel    | identical tool-error discriminants     |
// | D1  | identical error calls, two fresh sessions, both levels| byte-identical raw response lines      |
// | D3  | same bad args twice in one session (ids differ)       | identical modulo `id` normalization    |
// | P1  | bad call first vs last (same id set, permuted order)  | per-id byte-identical responses        |
// | P2  | notification before a bad call vs no notification     | byte-identical error line              |
// | F1  | fault catalog, tool-level and JSON-RPC-level          | no success members / payload keys      |
// | F2  | backend failures (corrupt index, deleted file)        | tool-error shape, no partial payloads  |
//
// D1 absorbs the former D2 (the same byte-identity relation at the JSON-RPC
// level): one test, two call sets, two levels.
//
// Non-duplication: pass 1 pins per-tool parse rows (bounds/types/unknown
// keys/sandbox escape on 4 tools) and JSON-RPC codes; pass 2 pins corrupt-db
// uniformity, mid-session flips, atomicity, and mixed transcripts. E3 reuses
// some triggers but asserts only cross-call relations (equality of
// discriminants, byte identity across sessions/orders, absence of payload
// keys in error bytes) that neither pass states.
//
// Transport comes from [`error_testkit`](self::error_testkit): every
// live-session read and process wait is timeout-bounded.
//

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
        // fastmcp admits the typed `root` argument before dispatch: the
        // mistyped root is a uniform -32602 on every tool.
        assert_jsonrpc_error(response, -32602);
    }
    let first = canonical_modulo_id(&raw[0]);
    for line in &raw[1..] {
        assert_eq!(
            canonical_modulo_id(line),
            first,
            "same fault diverged across tools: {line}"
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
        // Unknown keys are admission faults: uniform -32602 on every tool.
        assert_jsonrpc_error(response, -32602);
    }
    let first = canonical_modulo_id(&raw[0]);
    for line in &raw[1..] {
        assert_eq!(
            canonical_modulo_id(line),
            first,
            "same fault diverged across tools: {line}"
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
        // Admission owns every bound/type spelling: one code, not four tool
        // errors. The message names the violated bound and legitimately
        // differs by spelling; the code is the discriminant.
        assert_jsonrpc_error(response, -32602);
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
    assert_eq!(first, second, "error bytes drifted across sessions");
    // id 7 unknown tool -> -32601, id 8 bound violation -> -32602, id 9
    // stale range -> semantic tool error; each still byte-deterministic.
    assert_jsonrpc_error(&parse_line(&first[0]), -32601);
    assert_jsonrpc_error(&parse_line(&first[1]), -32602);
    assert_tool_error_shape(&parse_line(&first[2]));
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
    // id 21 unknown method -> -32601; id 22 non-string name is an
    // admission-level shape fault -> -32602.
    assert_jsonrpc_error(&parse_line(&first[0]), -32601);
    assert_jsonrpc_error(&parse_line(&first[1]), -32602);
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
    // Both repeats carry no per-call state; their classes are admission
    // (-32602) and routing (-32601) respectively.
    assert_jsonrpc_error(&parse_line(&raw[0]), -32602);
    assert_jsonrpc_error(&parse_line(&raw[1]), -32602);
    assert_jsonrpc_error(&parse_line(&raw[2]), -32601);
    assert_jsonrpc_error(&parse_line(&raw[3]), -32601);
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
    assert!(!is_error(&parse_line(by_first[&2])), "{:#}", by_first[&2]);
    assert!(!is_error(&parse_line(by_first[&3])), "{:#}", by_first[&3]);
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
    // limit 0 is admission-owned: -32602 either way.
    assert_jsonrpc_error(&parse_line(&with_notif), -32602);
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
    // Per-row classes: semantic faults (escaped root STRING, stale range)
    // stay tool errors; admission/routing faults are JSON-RPC errors.
    assert_jsonrpc_error(&parse_line(&raw[0]), -32601); // unknown tool
    assert_jsonrpc_error(&parse_line(&raw[1]), -32602); // limit 0
    assert_tool_error_shape(&parse_line(&raw[2])); // escaped root (string)
    assert_tool_error_shape(&parse_line(&raw[3])); // stale range
    assert_jsonrpc_error(&parse_line(&raw[4]), -32602); // root: 42
    assert_jsonrpc_error(&parse_line(&raw[5]), -32602); // unknown key
    assert_jsonrpc_error(&parse_line(&raw[6]), -32601); // missing method
    assert_jsonrpc_error(&parse_line(&raw[7]), -32601); // tools/unknown
    assert_jsonrpc_error(&parse_line(&raw[8]), -32602); // unshaped tools/call

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
    // Fail-closed byte scan: no success-payload key may appear in any error,
    // in raw form (structuredContent era) or escaped form (text-block era).
    const FORBIDDEN: [&str; 10] = [
        "structuredContent",
        "\"nodes\":",
        "\"h\":",
        "\"zn\":",
        "\"file_count\":",
        "\\\"nodes\\\":",
        "\\\"h\\\":",
        "\\\"zn\\\":",
        "\\\"file_count\\\":",
        "file_count",
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
    // Escaped forms included: the body rides content[0].text on the legacy
    // era, so a smuggled payload would appear as `\"nodes\":` bytes.
    const FORBIDDEN: [&str; 10] = [
        "structuredContent",
        "\"nodes\":",
        "\"h\":",
        "\"zn\":",
        "\"file_count\":",
        "\\\"nodes\\\":",
        "\\\"h\\\":",
        "\\\"zn\\\":",
        "\\\"file_count\\\":",
        "file_count",
    ];

    // Positive controls: healthy successes DO carry the payload members.
    // The body rides content[0].text on the legacy era, so assert on the
    // PARSED body rather than raw-byte key forms.
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
    assert!(!is_error(&parse_line(&ok[0])), "{}", ok[0]);
    assert!(!is_error(&parse_line(&ok[1])), "{}", ok[1]);
    assert!(
        !tool_body(&parse_line(&ok[0]))["nodes"]
            .as_array()
            .unwrap()
            .is_empty(),
        "control lost nodes: {}",
        ok[0]
    );
    let hits = tool_body(&parse_line(&ok[1]));
    assert!(
        !hits["h"].as_array().unwrap().is_empty() && hits["zn"].as_u64().unwrap() >= 1,
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

// ── E4 end-to-end drills (from error_api_pass4.rs) ──
// Pass 4 (errorapi, Mission E4): end-to-end error DRILLS on live sessions.
//
// Pass 1 pins the error TAXONOMY, pass 2 pins backend PROPAGATION, pass 3
// pins cross-call RELATIONS, and recovery pins crash/restart durability.
// This file pins live-session DRILLS: a working stdio session takes a fault
// injection mid-stream (no kill, no restart), each failing call reports its
// documented per-call code/shape, and the SAME session keeps serving good
// calls -- during the fault where the contract allows it, and after a live
// heal otherwise.
//
// Pinned drills (discriminants are codes, envelope shapes, and payload
// discriminants such as `why`/`zn`/`h`, never message text; every drill
// ends with good calls succeeding and a clean exit):
//
// | row | live fault injection                        | per-call codes/shapes + usability |
// |-----|---------------------------------------------|-----------------------------------|
// | D1  | `index.db` file, then `.asgrep/` dir, deleted live | miss mapping, live rebuild heals  |
// | D2  | `index.db` overwritten with garbage live    | toolerr x3, read survives, live heal |
// | D4  | malformed envelopes mid-session             | -32601/-32600 codes, session usable |
// | D5  | bad-argument storm mid-session              | uniform toolerr, session usable     |
// | D6  | sandbox-escape roots mid-session            | toolerr x3, session unpoisoned      |
// | D7  | CHAINED double fault (garbage db + deleted source) | staged heal, faults independent |
//
// D1 absorbs the former D3 (whole-`.asgrep/` deletion): the same miss-mapping
// + live-rebuild shape at dir granularity, run as D1's second fault leg on
//   the `search` channel (leg 1 uses `keyword_search`).
//
// Non-duplication: recovery pins mid-session stub/half index tears, root
// deletion + live heal, EOF, garbage lines, and shape-only invalid envelopes,
// plus kill+restart crash drills over corrupt/deleted indexes. E2 pins
// pre-session corrupt/empty mappings and a mid-session file flip. E4 pins
// only live-session injections with per-call CODE assertions and usability
// afterward that no earlier suite states: deletion/miss mapping live (D1),
// overwrite + live heal without restart (D2), mid-session -32600 and
// code values (D4), storm uniformity + usability (D5), escape + usability
// (D6), and a staged-heal double fault (D7).
//
// Transport comes from [`error_testkit`](self::error_testkit): every
// live-session read and process wait is timeout-bounded.
//

/// D1: deletion under a live session maps to the documented empty-miss
/// SUCCESS (never a tool error, never stale hits), reads keep serving, and a
/// live `index_repo` rebuild heals search in the SAME session — at BOTH fault
/// granularities. Leg 1 deletes the `index.db` file; leg 2 (absorbed D3)
/// deletes the whole `.asgrep/` directory. Recovery pins deletion only under
/// kill+restart crash drills, never the live per-call mapping.
/// INTENT: live db delete → miss success + zero status, reads serve, live index_repo heals; live .asgrep/ delete → miss on search channel, reads serve, live rebuild recreates dir.
/// KILLS: delete-as-tool-error, dir-delete-as-tool-error, stale-hits, heal-regression.
/// ABSORBS: drill_asgrep_dir_deleted_mid_session_miss_then_live_rebuild (same miss-mapping + live-rebuild shape; differs only in fault granularity file-vs-dir and channel — folded as second fault leg in D1).
/// OVERLAP: recovery deletion (kill+restart only; this is live mapping).
#[test]
fn drill_index_db_deleted_mid_session_miss_then_live_reindex_heals() {
    // Leg 1 (file granularity): `index.db` deleted under a live session.
    {
        let temp = indexed_tree();
        assert!(index_db_path(temp.path()).is_file());
        let mut session = LiveSession::spawn(Some(temp.path()));
        session.handshake();

        let before = session.call(&search_call(1, "keyword_search", 4), 1);
        assert_tool_success_shape(&before);
        assert!(
            !tool_body(&before)["h"].as_array().unwrap().is_empty(),
            "{before:#}"
        );

        std::fs::remove_file(index_db_path(temp.path())).unwrap();
        assert!(!index_db_path(temp.path()).exists());
        // Bundle-era total poison: the read bundle would otherwise keep
        // serving correct rows past the deleted SQLite file.
        remove_read_bundle(&index_db_path(temp.path()));

        // Fresh limit so the call cannot ride the warm searcher cache.
        let miss = session.call(&search_call(2, "keyword_search", 8), 2);
        assert_tool_success_shape(&miss);
        let body = tool_body(&miss);
        assert_eq!(body["why"], "empty_index", "{body:#}");
        assert_eq!(body["zn"], 0, "{body:#}");
        assert_eq!(body["h"], json!([]), "{body:#}");

        let status = session.call(&tool_call(3, "index_status", json!({})), 3);
        assert_tool_success_shape(&status);
        assert_eq!(tool_body(&status)["file_count"], 0, "{status:#}");

        let read = session.call(&read_call(4), 4);
        assert_tool_success_shape(&read);
        assert_eq!(
            tool_body(&read)["nodes"].as_array().map(Vec::len),
            Some(1),
            "{read:#}"
        );

        let rebuilt = session.call(&tool_call(5, "index_repo", json!({})), 5);
        assert_tool_success_shape(&rebuilt);
        assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");

        let after = session.call(&search_call(6, "keyword_search", 12), 6);
        assert_tool_success_shape(&after);
        assert!(
            !tool_body(&after)["h"].as_array().unwrap().is_empty(),
            "{after:#}"
        );

        session.finish_clean();
    }

    // Leg 2 (dir granularity, absorbed D3): the whole `.asgrep/` directory
    // deleted under a live session, drilled on the `search` channel.
    {
        let temp = indexed_tree();
        assert!(temp.path().join(".asgrep").is_dir());
        let mut session = LiveSession::spawn(Some(temp.path()));
        session.handshake();

        let before = session.call(&search_call(1, "search", 4), 1);
        assert_tool_success_shape(&before);
        assert!(
            !tool_body(&before)["h"].as_array().unwrap().is_empty(),
            "{before:#}"
        );

        std::fs::remove_dir_all(temp.path().join(".asgrep")).unwrap();
        assert!(!temp.path().join(".asgrep").exists());

        let miss = session.call(&search_call(2, "search", 8), 2);
        assert_tool_success_shape(&miss);
        let body = tool_body(&miss);
        assert_eq!(body["why"], "empty_index", "{body:#}");
        assert_eq!(body["zn"], 0, "{body:#}");
        assert_eq!(body["h"], json!([]), "{body:#}");

        let status = session.call(&tool_call(3, "index_status", json!({})), 3);
        assert_tool_success_shape(&status);
        assert_eq!(tool_body(&status)["file_count"], 0, "{status:#}");

        let read = session.call(&read_call(4), 4);
        assert_tool_success_shape(&read);
        assert_eq!(
            tool_body(&read)["nodes"].as_array().map(Vec::len),
            Some(1),
            "{read:#}"
        );

        let rebuilt = session.call(&tool_call(5, "index_repo", json!({})), 5);
        assert_tool_success_shape(&rebuilt);
        assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
        assert!(index_db_path(temp.path()).is_file());

        let after = session.call(&search_call(6, "search", 12), 6);
        assert_tool_success_shape(&after);
        assert!(
            !tool_body(&after)["h"].as_array().unwrap().is_empty(),
            "{after:#}"
        );

        session.finish_clean();
    }
}

/// D2: `index.db` is overwritten with garbage under a live session. Search,
/// status, and even reindex-on-garbage are tool errors (never silent empty
/// successes), reads keep serving files, and deleting the corrupt inode plus
/// a live `index_repo` heals the SAME session. Recovery pins stub/half tears
/// live without healin-session, and overwrite only under kill+restart.
/// INTENT: live garbage overwrite → search/status/reindex toolerr, reads serve, delete+rebuild heals.
/// KILLS: silent-empty-on-garbage, heal-regression.
/// ABSORBS: none.
#[test]
fn drill_index_db_overwritten_mid_session_refused_then_live_heal() {
    let temp = indexed_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&search_call(1, "keyword_search", 4), 1);
    assert_tool_success_shape(&before);
    assert!(
        !tool_body(&before)["h"].as_array().unwrap().is_empty(),
        "{before:#}"
    );

    corrupt_index_db(temp.path());
    // Bundle-era total poison: garbage alone no longer refuses while the
    // intact read bundle keeps serving; defeat both materializations.
    remove_read_bundle(&index_db_path(temp.path()));

    // Fresh limit so the call cannot ride the warm searcher cache.
    let search = session.call(&search_call(2, "keyword_search", 8), 2);
    assert_tool_error_shape(&search);
    let status = session.call(&tool_call(3, "index_status", json!({})), 3);
    assert_tool_error_shape(&status);
    let reindex_refused = session.call(&tool_call(4, "index_repo", json!({})), 4);
    assert_tool_error_shape(&reindex_refused);

    let read = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    std::fs::remove_file(index_db_path(temp.path())).unwrap();
    let rebuilt = session.call(&tool_call(6, "index_repo", json!({})), 6);
    assert_tool_success_shape(&rebuilt);
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");

    let after = session.call(&search_call(7, "keyword_search", 12), 7);
    assert_tool_success_shape(&after);
    assert!(
        !tool_body(&after)["h"].as_array().unwrap().is_empty(),
        "{after:#}"
    );

    session.finish_clean();
}

/// D4: malformed envelopes arrive mid-session between good calls. Each one
/// reports its documented JSON-RPC CODE (-32601 with id echo for unknown
/// method and unshaped `tools/call` params; -32600 with NO id for a request
/// with no method), and the session serves a read and a ping right after.
/// Recovery pins one unknown-method envelope mid-stream shape-only; E4 pins
/// the code values and the -32600 row live.
/// INTENT: live malformed envelopes report -32601/-32600/-32601, read+ping serve after.
/// KILLS: code-swap-live, session-poison.
/// ABSORBS: none.
/// OVERLAP: recovery mid-stream shape-only (this pins code values + -32600 live).
#[test]
fn drill_malformed_envelopes_mid_session_codes_then_session_usable() {
    let temp = file_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&read_call(1), 1);
    assert_tool_success_shape(&before);
    assert_eq!(
        tool_body(&before)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{before:#}"
    );

    let unknown = session.call(&json!({"jsonrpc":"2.0","id":2,"method":"missing"}), 2);
    assert_jsonrpc_error(&unknown, -32601);

    session.send(&json!({ "jsonrpc": "2.0", "id": 3 }));
    let invalid = session.recv();
    assert_jsonrpc_error(&invalid, -32600);
    // fastmcp echoes the offending id on -32600.
    assert_eq!(invalid["id"], 3, "{invalid:#}");

    let unshaped = session.call(
        &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{}}),
        4,
    );
    assert_jsonrpc_error(&unshaped, -32602);

    let read = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    let ping = session.call(&json!({"jsonrpc":"2.0","id":6,"method":"ping"}), 6);
    assert!(ping.get("error").is_none(), "{ping:#}");
    assert!(ping.get("result").is_some(), "{ping:#}");

    session.finish_clean();
}

/// D5: a storm of bad-argument calls hits a live session: bound, sign, type,
/// unknown-key, shape, overflow, and unknown-tool faults. fastmcp splits
/// them into three framework-owned classes -- admission -32602 (bound, sign,
/// type, unknown-key, maxItems), routing -32601 (unknown tool), and the
/// handler-semantic tool-error envelope (malformed id STRING) -- each with
/// its id echoed, and a search plus a read succeed immediately after. Pass 2
/// P8 pins one static mixed sequence; this pins a parse-level storm with
/// per-class uniformity plus usability-after on a live session.
/// INTENT: 9-call parse-level storm uniform toolerr with ids, search+read serve after.
/// KILLS: storm-divergence, session-poison.
/// ABSORBS: none.
/// OVERLAP: P8 static mix (this is parse-storm + live usability).
#[test]
fn drill_bad_argument_storm_mid_session_uniform_then_usable() {
    let temp = indexed_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let before = session.call(&search_call(1, "keyword_search", 4), 1);
    assert_tool_success_shape(&before);
    assert!(
        !tool_body(&before)["h"].as_array().unwrap().is_empty(),
        "{before:#}"
    );

    let twenty_one: Vec<Value> = (0..21).map(|_| json!("src/lib.rs#L1-L1")).collect();
    let storm = vec![
        tool_call(10, "keyword_search", json!({"query": "x", "limit": 0})),
        tool_call(11, "keyword_search", json!({"query": "x", "limit": -1})),
        tool_call(12, "keyword_search", json!({"query": "x", "limit": "many"})),
        tool_call(13, "keyword_search", json!({"query": "x", "bogus": 1})),
        tool_call(14, "keyword_search", json!({"query": "x", "root": 42})),
        tool_call(15, "code_read", json!({"ids": ["src/lib.rs#1-2"]})),
        tool_call(16, "code_read", json!({"ids": twenty_one})),
        tool_call(17, "index_repo", json!({"force": "yes"})),
        tool_call(18, "no_such_tool", json!({})),
    ];
    for (i, payload) in storm.iter().enumerate() {
        let id = 10 + i as u32;
        let response = session.call(payload, id);
        match id {
            // Malformed id STRING ("src/lib.rs#1-2") is handler-semantic.
            15 => assert_tool_error_shape(&response),
            // Unknown tool is routing.
            18 => assert_jsonrpc_error(&response, -32601),
            // Every admission fault (bound/sign/type/unknown-key/maxItems)
            // shares one framework code.
            _ => assert_jsonrpc_error(&response, -32602),
        }
    }

    let search = session.call(&search_call(19, "keyword_search", 8), 19);
    assert_tool_success_shape(&search);
    assert!(
        !tool_body(&search)["h"].as_array().unwrap().is_empty(),
        "{search:#}"
    );
    let read = session.call(&read_call(20), 20);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );

    session.finish_clean();
}

/// D6: per-call roots escaping the workspace hit a live session on three
/// tools. Each is a tool error with its id echoed, and the session is
/// unpoisoned: a read, a status, and a ping succeed right after. Pass 1 T6
/// pins the escape rows statically; this pins live injection plus usability.
/// INTENT: live escape roots ×3 tools toolerr, read+status+ping serve after.
/// KILLS: jail-drop-live, session-poison.
/// ABSORBS: none.
/// OVERLAP: T6 static rows (this is live + usability).
#[test]
fn drill_sandbox_escape_mid_session_refused_session_unpoisoned() {
    let workspace = file_tree();
    let outside = tempfile::tempdir().unwrap();
    let escaped = outside.path().display().to_string();
    let mut session = LiveSession::spawn(Some(workspace.path()));
    session.handshake();

    let before = session.call(&read_call(1), 1);
    assert_tool_success_shape(&before);
    assert_eq!(
        tool_body(&before)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{before:#}"
    );

    let attempts = [
        tool_call(
            2,
            "keyword_search",
            json!({"query": "x", "limit": 4, "root": escaped}),
        ),
        tool_call(
            3,
            "code_read",
            json!({"ids": ["src/lib.rs#L1-L1"], "root": escaped}),
        ),
        tool_call(4, "index_repo", json!({"root": escaped})),
    ];
    for (i, payload) in attempts.iter().enumerate() {
        let response = session.call(payload, 2 + i as u32);
        assert_tool_error_shape(&response);
    }

    let read = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read);
    assert_eq!(
        tool_body(&read)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read:#}"
    );
    let status = session.call(&tool_call(6, "index_status", json!({})), 6);
    assert_tool_success_shape(&status);
    let ping = session.call(&json!({"jsonrpc":"2.0","id":7,"method":"ping"}), 7);
    assert!(ping.get("error").is_none(), "{ping:#}");
    assert!(ping.get("result").is_some(), "{ping:#}");

    session.finish_clean();
}

/// D7 (chained double fault): garbage overwrites `index.db` AND the read
/// target is deleted under one live session. Both faults report tool errors
/// on the same session; a staged heal (restore the file first) revives reads
/// while search still fails -- the faults are independent, neither heal
/// masks the other -- and deleting the corrupt inode plus a live `index_repo`
/// heals search. Recovery pins a kill+kill double crash with restarts; this
/// pins a live double fault with a staged live heal and per-call codes.
/// INTENT: garbage db + deleted source live: staged heal revives reads while search still fails, then full heal.
/// KILLS: fault-masking, heal-coupling.
/// ABSORBS: none.
/// OVERLAP: recovery double crash (kill+restart; this is live staged heal).
#[test]
fn drill_chained_double_fault_staged_heal_restores_live_session() {
    let temp = indexed_tree();
    let victim = temp.path().join("src").join("lib.rs");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    let search0 = session.call(&search_call(1, "keyword_search", 4), 1);
    assert_tool_success_shape(&search0);
    assert!(
        !tool_body(&search0)["h"].as_array().unwrap().is_empty(),
        "{search0:#}"
    );
    let read0 = session.call(&read_call(2), 2);
    assert_tool_success_shape(&read0);
    assert_eq!(
        tool_body(&read0)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read0:#}"
    );

    // Inject both faults at once.
    corrupt_index_db(temp.path());
    // Bundle-era total poison (see overwritten drill): the intact read
    // bundle would otherwise keep answering past the garbage SQLite file.
    remove_read_bundle(&index_db_path(temp.path()));
    std::fs::remove_file(&victim).unwrap();

    let search1 = session.call(&search_call(3, "keyword_search", 8), 3);
    assert_tool_error_shape(&search1);
    let read1 = session.call(&read_call(4), 4);
    assert_tool_error_shape(&read1);

    // Staged heal, step 1: restore the file only.
    std::fs::write(&victim, FIXTURE_SOURCE).unwrap();
    let read2 = session.call(&read_call(5), 5);
    assert_tool_success_shape(&read2);
    assert_eq!(
        tool_body(&read2)["nodes"].as_array().map(Vec::len),
        Some(1),
        "{read2:#}"
    );
    let search2 = session.call(&search_call(6, "keyword_search", 9), 6);
    assert_tool_error_shape(&search2);

    // Staged heal, step 2: drop the corrupt inode and rebuild live.
    std::fs::remove_file(index_db_path(temp.path())).unwrap();
    let rebuilt = session.call(&tool_call(7, "index_repo", json!({})), 7);
    assert_tool_success_shape(&rebuilt);
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    let search3 = session.call(&search_call(8, "keyword_search", 12), 8);
    assert_tool_success_shape(&search3);
    assert!(
        !tool_body(&search3)["h"].as_array().unwrap().is_empty(),
        "{search3:#}"
    );

    session.finish_clean();
}
