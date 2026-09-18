//! I1 invalidation-contract oracles for ast-sgrep-mcp.
//!
//! Non-overlap contract: `protocol.rs` pins handshake, tool names, per-channel
//! kinds, elision-until-reindex, and miss envelopes; durable-recovery passes pin
//! corrupt-db refusal and restart determinism of empty roots; error-api passes
//! pin argument boundaries. This file pins NONE of those again. Instead it pins
//! INDEX INVALIDATION over stdio sessions on tempfile fixtures:
//!
//! * `index_status` discriminants: missing root (tool error), unindexed root
//!   (`file_count` 0, `writer_generation` 0), fresh root (`file_count` >= 1,
//!   `writer_generation` nonzero);
//! * search on a stale index serves pre-change hits with success shape and no
//!   staleness discriminant in the envelope (serves-stale-silently is the real
//!   contract: pinned, not wished otherwise);
//! * in-session `index_repo` then search reflects edits and deletions;
//! * an external reindex invalidates a warm same-session Searcher;
//! * a restarted process picks up fresh state, and agrees byte-identically
//!   with the invalidated session.
//!
//! Discriminants are `isError` booleans, envelope codes (`why`), key presence,
//! counts, and byte equality -- never message text.

use serde_json::{json, Value};
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

fn init_payload() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "__init",
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "asgrep-mcp-i1", "version": "0"}
        }
    })
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
    send(&mut stdin, &init_payload());
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

fn rpc_at(payload: Value, root: &Path) -> Value {
    let mut responses = rpc_session(vec![payload], Some(root));
    responses.pop().expect("one response")
}

/// A live session that stays open across test-thread actions (file edits,
/// out-of-band reindexes) between tool calls.
struct LiveSession {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u32,
}

impl LiveSession {
    fn spawn(root: &Path) -> Self {
        let mut child = Command::new(mcp_bin())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .env("ASGREP_ROOT", root)
            .spawn()
            .expect("spawn MCP");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        writeln!(stdin, "{}", init_payload()).unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).expect("read init");
        let init: Value = serde_json::from_str(line.trim()).expect("init JSON-RPC");
        assert_eq!(init["id"], "__init", "{init:#}");
        let noted = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        writeln!(stdin, "{noted}").unwrap();
        stdin.flush().unwrap();
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 1,
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let payload = tool_call(id, name, arguments);
        let stdin = self.stdin.as_mut().expect("session open");
        writeln!(stdin, "{payload}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).expect("read MCP line");
        assert!(n > 0, "MCP closed stdout");
        let response: Value = serde_json::from_str(line.trim()).expect("JSON-RPC");
        assert_eq!(response["id"], id, "{response:#}");
        response
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().expect("wait MCP");
        assert!(status.success(), "MCP exited {status}");
    }
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

fn assert_tool_success(response: &Value) {
    assert_eq!(response["result"]["isError"], false, "{response:#}");
    assert!(response.get("error").is_none(), "{response:#}");
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

fn assert_hit_envelope(body: &Value) {
    assert!(body.get("why").is_none(), "hit must carry no why: {body:#}");
    assert!(
        body["zn"].as_u64().unwrap_or(0) >= 1,
        "hit must count >= 1: {body:#}"
    );
    assert!(
        !body["h"].as_array().expect("hit h array").is_empty(),
        "hit h must be nonempty: {body:#}"
    );
}

fn assert_miss_envelope(body: &Value, why: &str) {
    assert_eq!(body["why"], why, "{body:#}");
    assert_eq!(body["zn"], 0, "{body:#}");
    assert_eq!(body["h"], json!([]), "{body:#}");
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

fn search_call(id: u32, query: &str) -> Value {
    tool_call(
        id,
        "keyword_search",
        json!({"query": query, "limit": 8, "resend_seen": true}),
    )
}

#[test]
fn index_status_discriminates_missing_unindexed_and_fresh() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn target_symbol() {}\n").unwrap();

    // Missing root: uniform tool error inside a live session.
    let missing_arg = temp.path().join("does_not_exist").display().to_string();
    let missing = rpc_at(
        tool_call(1, "index_status", json!({"root": missing_arg})),
        temp.path(),
    );
    assert_tool_error_shape(&missing);

    // Existing but never indexed: success with zero counts and epoch 0.
    let unindexed = rpc_at(tool_call(2, "index_status", json!({})), temp.path());
    assert_tool_success(&unindexed);
    let body = tool_body(&unindexed);
    assert_eq!(body["file_count"], 0, "{body:#}");
    assert_eq!(body["writer_generation"], 0, "{body:#}");

    // Fresh index: positive counts and a nonzero writer epoch.
    index_tree(temp.path());
    let fresh = rpc_at(tool_call(3, "index_status", json!({})), temp.path());
    assert_tool_success(&fresh);
    let body = tool_body(&fresh);
    assert_eq!(body["file_count"], 1, "{body:#}");
    assert!(
        body["symbol_count"].as_u64().unwrap_or(0) >= 1,
        "{body:#}"
    );
    assert_ne!(body["writer_generation"], 0, "{body:#}");
}

#[test]
fn search_on_stale_index_serves_pre_change_hits_without_flag() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    let before = rpc_at(search_call(1, "alpha_marker"), temp.path());
    assert_tool_success(&before);
    let before_text = tool_text(&before).to_owned();
    assert_hit_envelope(&tool_body(&before));
    let status_before = tool_body(&rpc_at(tool_call(2, "index_status", json!({})), temp.path()));

    // Edit without reindexing: the file now names beta, the index still alpha.
    std::fs::write(
        temp.path().join("lib.rs"),
        "fn zebroid_quixotic() {}\n",
    )
    .unwrap();

    // Contract: search serves the stale index with success shape, byte-identical
    // to before the edit, and the envelope carries no staleness discriminant.
    let stale = rpc_at(search_call(3, "alpha_marker"), temp.path());
    assert_tool_success(&stale);
    assert_eq!(tool_text(&stale), before_text, "stale search must serve old rows");
    let body = tool_body(&stale);
    assert_hit_envelope(&body);
    for key in ["stale", "fresh", "dirty", "generation", "writer_generation"] {
        assert!(body.get(key).is_none(), "no {key} discriminant: {body:#}");
    }
    // The new symbol is invisible until a reindex.
    let unseen = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
    assert_tool_success(&unseen);
    assert_miss_envelope(&tool_body(&unseen), "no_match");

    // A bare file edit moves no index discriminant.
    let status_after = tool_body(&rpc_at(tool_call(5, "index_status", json!({})), temp.path()));
    assert_eq!(status_after["file_count"], status_before["file_count"]);
    assert_eq!(
        status_after["writer_generation"], status_before["writer_generation"],
        "writer epoch moves only on index mutation"
    );
}

#[test]
fn in_session_reindex_then_search_reflects_changes() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = LiveSession::spawn(temp.path());
    let first = session.call("keyword_search", json!({"query": "alpha_marker", "limit": 8}));
    assert_tool_success(&first);
    assert_hit_envelope(&tool_body(&first));

    std::fs::write(
        temp.path().join("lib.rs"),
        "fn zebroid_quixotic() {}\n",
    )
    .unwrap();
    let reindex = session.call("index_repo", json!({}));
    assert_tool_success(&reindex);
    assert_eq!(reindex["id"], 2);

    let gone = session.call("keyword_search", json!({"query": "alpha_marker", "limit": 8}));
    assert_tool_success(&gone);
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let found = session.call(
        "keyword_search",
        json!({"query": "zebroid_quixotic", "limit": 8}),
    );
    assert_tool_success(&found);
    assert_hit_envelope(&tool_body(&found));
    session.finish();
}

#[test]
fn external_reindex_invalidates_warm_session_searcher() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    // Warm the session Searcher, then mutate the index out of band.
    let mut session = LiveSession::spawn(temp.path());
    let first = session.call("keyword_search", json!({"query": "alpha_marker", "limit": 8}));
    assert_tool_success(&first);
    assert_hit_envelope(&tool_body(&first));

    std::fs::write(
        temp.path().join("lib.rs"),
        "fn zebroid_quixotic() {}\n",
    )
    .unwrap();
    index_tree(temp.path());

    // The same session must serve fresh rows, not its warm snapshot.
    let gone = session.call("keyword_search", json!({"query": "alpha_marker", "limit": 8}));
    assert_tool_success(&gone);
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let found = session.call(
        "keyword_search",
        json!({"query": "zebroid_quixotic", "limit": 8}),
    );
    assert_tool_success(&found);
    assert_hit_envelope(&tool_body(&found));
    session.finish();
}

#[test]
fn restart_picks_up_fresh_state() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    let first = rpc_at(search_call(1, "alpha_marker"), temp.path());
    assert_hit_envelope(&tool_body(&first));
    let gen_before =
        tool_body(&rpc_at(tool_call(2, "index_status", json!({})), temp.path()))["writer_generation"]
            .as_u64()
            .unwrap();

    std::fs::write(
        temp.path().join("lib.rs"),
        "fn zebroid_quixotic() {}\n",
    )
    .unwrap();
    index_tree(temp.path());

    // A restarted process observes the new epoch and the new rows.
    let gone = rpc_at(search_call(3, "alpha_marker"), temp.path());
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let found = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
    assert_tool_success(&found);
    assert_hit_envelope(&tool_body(&found));
    let status = tool_body(&rpc_at(tool_call(5, "index_status", json!({})), temp.path()));
    assert_eq!(status["file_count"], 1);
    assert_ne!(status["writer_generation"].as_u64().unwrap(), 0);
    assert_ne!(
        status["writer_generation"].as_u64().unwrap(),
        gen_before,
        "restart must observe the post-mutation epoch"
    );

    // Fresh state is deterministic across restarts.
    let again = rpc_at(search_call(6, "zebroid_quixotic"), temp.path());
    assert_eq!(tool_text(&again), tool_text(&found));
}

#[test]
fn session_and_fresh_process_agree_after_reindex() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = LiveSession::spawn(temp.path());
    let warm = session.call(
        "keyword_search",
        json!({"query": "alpha_marker", "limit": 8, "resend_seen": true}),
    );
    assert_hit_envelope(&tool_body(&warm));
    std::fs::write(
        temp.path().join("lib.rs"),
        "fn zebroid_quixotic() {}\n",
    )
    .unwrap();
    let reindex = session.call("index_repo", json!({}));
    assert_tool_success(&reindex);
    let in_session = session.call(
        "keyword_search",
        json!({"query": "zebroid_quixotic", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&in_session);
    let in_session_text = tool_text(&in_session).to_owned();
    session.finish();

    // A fresh process over the same generation must answer byte-identically.
    let fresh = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
    assert_tool_success(&fresh);
    assert_eq!(tool_text(&fresh), in_session_text);
}

#[test]
fn index_repo_heals_empty_index_miss_within_session() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn target_symbol() {}\n").unwrap();

    let mut session = LiveSession::spawn(temp.path());
    let miss = session.call("keyword_search", json!({"query": "target_symbol", "limit": 8}));
    assert_tool_success(&miss);
    assert_miss_envelope(&tool_body(&miss), "empty_index");

    let reindex = session.call("index_repo", json!({}));
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert!(
        stats["files_indexed"].as_u64().unwrap_or(0) >= 1,
        "{stats:#}"
    );
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    let found = session.call("keyword_search", json!({"query": "target_symbol", "limit": 8}));
    assert_tool_success(&found);
    assert_hit_envelope(&tool_body(&found));
    let status = session.call("index_status", json!({}));
    assert_tool_success(&status);
    assert_eq!(tool_body(&status)["file_count"], 1);
    session.finish();
}

#[test]
fn reindex_after_deletion_prunes_counts_and_hits() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.rs"), "fn keep_symbol() {}\n").unwrap();
    std::fs::write(temp.path().join("drop.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    index_tree(temp.path());

    let before = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
    assert_hit_envelope(&tool_body(&before));

    std::fs::remove_file(temp.path().join("drop.rs")).unwrap();
    let responses = rpc_session(
        vec![
            tool_call(2, "index_repo", json!({})),
            search_call(3, "zebroid_quixotic"),
            search_call(4, "keep_symbol"),
            tool_call(5, "index_status", json!({})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4, "{responses:#?}");

    let stats = tool_body(&responses[0]);
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_indexed"], 0, "{stats:#}");
    assert_miss_envelope(&tool_body(&responses[1]), "no_match");
    assert_hit_envelope(&tool_body(&responses[2]));
    assert_eq!(tool_body(&responses[3])["file_count"], 1);
}
