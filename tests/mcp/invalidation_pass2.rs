//! I2 delta-discriminating oracles for ast-sgrep-mcp index freshness.
//!
//! Non-overlap contract: pass1 pins the broad invalidation contract
//! (status discriminants, serves-stale-silently, in-session/external/restart
//! reindex agreement, deletion pruning). This file pins NONE of those again.
//! Instead it proves EACH TREE DELTA CLASS over stdio sessions on tempfile
//! fixtures:
//!
//! * ADD: a new file under root is invisible until refresh, then its symbols
//!   hit under exactly the new path;
//! * MODIFY: an edited file serves stale rows until refresh, then the old
//!   symbol misses and the new symbol hits under exactly the edited path;
//! * DELETE: a removed file serves stale rows until refresh, then its symbols
//!   miss and counts drop by exactly the delta;
//! * RENAME: a moved file serves the old path until refresh, then hits carry
//!   exactly the new path and the old path leaves the envelope;
//! * MULTI / MIXED: one refresh absorbs several deltas with exact per-path
//!   hit sets and exact refresh stats.
//!
//! Fresh -> stale -> fresh transitions follow the pass1 contract: a bare
//! tree delta moves NO `index_status` discriminant (stale is silent); the
//! refresh advertises a new `writer_generation` and exact counts. Hit sets
//! are asserted per path via the compact `p` table, never via message text.
//!
//! Discriminants are `isError` booleans, envelope codes (`why`), key
//! presence, exact path sets, counts, and byte equality -- never message
//! text.

use serde_json::{json, Value};
use std::collections::BTreeSet;
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
            "clientInfo": {"name": "asgrep-mcp-i2", "version": "0"}
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

fn status_body(root: &Path) -> Value {
    tool_body(&rpc_at(tool_call(1, "index_status", json!({})), root))
}

/// Project paths carrying hits, resolved through the compact `p` table
/// exactly the way the server resolves them for `code_read`.
fn hit_path_set(body: &Value) -> BTreeSet<String> {
    let table: std::collections::HashMap<String, String> =
        ast_sgrep_plugins::resolve_compact_paths(body)
            .into_iter()
            .collect();
    // The `p` table must name exactly the hit paths: no stale extras.
    let table_paths: BTreeSet<String> = table.values().cloned().collect();
    let hits = body["h"].as_array().expect("hit h array");
    assert_eq!(
        body["zn"].as_u64().unwrap_or(u64::MAX),
        hits.len() as u64,
        "zn must equal hit rows: {body:#}"
    );
    let mut paths = BTreeSet::new();
    for row in hits {
        let id = row[0].as_str().expect("hit id string");
        let (path_id, _) = id.rsplit_once(':').expect("compact id carries range");
        let path = table
            .get(path_id)
            .unwrap_or_else(|| panic!("path id {path_id} must resolve in p table: {body:#}"))
            .clone();
        paths.insert(path);
    }
    assert_eq!(
        table_paths, paths,
        "p table must name exactly the hit paths: {body:#}"
    );
    paths
}

/// Hit envelope whose every hit resolves under exactly `expected` paths.
fn assert_hit_path_set(body: &Value, expected: &[&str]) {
    assert_hit_envelope(body);
    let want: BTreeSet<String> = expected.iter().map(ToString::to_string).collect();
    assert_eq!(hit_path_set(body), want, "{body:#}");
}

#[test]
fn add_delta_new_file_hits_appear_only_after_refresh() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = LiveSession::spawn(temp.path());
    let keep = session.call(
        "keyword_search",
        json!({"query": "alpha_marker", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&keep);
    assert_hit_path_set(&tool_body(&keep), &["a.rs"]);

    // ADD delta, no refresh yet: the new file is invisible to the same session.
    std::fs::write(temp.path().join("b.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    let unseen = session.call(
        "keyword_search",
        json!({"query": "zebroid_quixotic", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&unseen);
    assert_miss_envelope(&tool_body(&unseen), "no_match");

    let reindex = session.call("index_repo", json!({}));
    assert_tool_success(&reindex);

    // After refresh the new symbol hits under exactly the new path, and the
    // old symbol still hits under exactly the old path: no stale, no bleed.
    let found = session.call(
        "keyword_search",
        json!({"query": "zebroid_quixotic", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&found);
    assert_hit_path_set(&tool_body(&found), &["b.rs"]);
    let still = session.call(
        "keyword_search",
        json!({"query": "alpha_marker", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&still);
    assert_hit_path_set(&tool_body(&still), &["a.rs"]);
    session.finish();
}

#[test]
fn add_delta_status_counts_and_refresh_stats() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn womble_frascati() {}\n").unwrap();
    index_tree(temp.path());

    // Fresh: one file, nonzero epoch.
    let fresh = status_body(temp.path());
    assert_eq!(fresh["file_count"], 1, "{fresh:#}");
    let gen_fresh = fresh["writer_generation"].as_u64().unwrap();
    assert_ne!(gen_fresh, 0);

    // ADD delta without refresh: fresh -> stale moves no discriminant.
    std::fs::write(temp.path().join("b.rs"), "fn joltik_nimbus() {}\n").unwrap();
    let stale = status_body(temp.path());
    assert_eq!(stale["file_count"], 1, "{stale:#}");
    assert_eq!(
        stale["writer_generation"].as_u64().unwrap(),
        gen_fresh,
        "bare add moves no epoch"
    );

    // Refresh: exact stats shape for a pure add.
    let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    // Stale -> fresh: count grows by exactly the delta, epoch advertised.
    let after = status_body(temp.path());
    assert_eq!(after["file_count"], 2, "{after:#}");
    let gen_after = after["writer_generation"].as_u64().unwrap();
    assert_ne!(gen_after, 0);
    assert_ne!(gen_after, gen_fresh, "refresh must advertise");

    let found = rpc_at(search_call(3, "joltik_nimbus"), temp.path());
    assert_hit_path_set(&tool_body(&found), &["b.rs"]);
}

#[test]
fn modify_delta_symbol_swap_exact_path_sets() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn blip_candle() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = LiveSession::spawn(temp.path());
    let warm = session.call(
        "keyword_search",
        json!({"query": "quarry_sphinx", "limit": 8, "resend_seen": true}),
    );
    assert_hit_path_set(&tool_body(&warm), &["a.rs"]);

    // MODIFY delta: rewrite the symbol in place, then refresh in session.
    std::fs::write(temp.path().join("a.rs"), "fn vortex_elm() {}\n").unwrap();
    let reindex = session.call("index_repo", json!({}));
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    // Old symbol misses (no stale hits); new symbol hits exactly the edited
    // path; the untouched file's symbol still hits exactly its own path.
    let gone = session.call(
        "keyword_search",
        json!({"query": "quarry_sphinx", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&gone);
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let found = session.call(
        "keyword_search",
        json!({"query": "vortex_elm", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&found);
    assert_hit_path_set(&tool_body(&found), &["a.rs"]);
    let kept = session.call(
        "keyword_search",
        json!({"query": "blip_candle", "limit": 8, "resend_seen": true}),
    );
    assert_tool_success(&kept);
    assert_hit_path_set(&tool_body(&kept), &["b.rs"]);
    session.finish();
}

#[test]
fn modify_delta_symbol_move_stale_path_set_then_fresh() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn froth_gazebo() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn plumb_kiosk() {}\n").unwrap();
    index_tree(temp.path());

    let fresh = status_body(temp.path());
    let gen_fresh = fresh["writer_generation"].as_u64().unwrap();

    // MODIFY delta: move the symbol from a.rs to b.rs without refreshing.
    std::fs::write(temp.path().join("a.rs"), "fn snipe_tundra() {}\n").unwrap();
    std::fs::write(
        temp.path().join("b.rs"),
        "fn plumb_kiosk() {}\nfn froth_gazebo() {}\n",
    )
    .unwrap();

    // Stale serves the pre-move path set, and moves no discriminant.
    let stale = status_body(temp.path());
    assert_eq!(stale["file_count"], fresh["file_count"]);
    assert_eq!(
        stale["writer_generation"].as_u64().unwrap(),
        gen_fresh,
        "bare modify moves no epoch"
    );
    let served = rpc_at(search_call(1, "froth_gazebo"), temp.path());
    assert_tool_success(&served);
    assert_hit_path_set(&tool_body(&served), &["a.rs"]);

    let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);

    // Fresh serves exactly the post-move path set: b.rs only, a.rs gone.
    let moved = rpc_at(search_call(3, "froth_gazebo"), temp.path());
    assert_tool_success(&moved);
    assert_hit_path_set(&tool_body(&moved), &["b.rs"]);
    let after = status_body(temp.path());
    assert_eq!(after["file_count"], 2, "{after:#}");
    assert_ne!(
        after["writer_generation"].as_u64().unwrap(),
        gen_fresh,
        "refresh must advertise"
    );
}

#[test]
fn delete_delta_pruned_hits_exact_paths() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.rs"), "fn alpha_marker() {}\n").unwrap();
    std::fs::write(temp.path().join("drop.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    index_tree(temp.path());

    let before = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
    assert_hit_path_set(&tool_body(&before), &["drop.rs"]);

    // DELETE delta without refresh: the stale row still serves its old path.
    std::fs::remove_file(temp.path().join("drop.rs")).unwrap();
    let stale = rpc_at(search_call(2, "zebroid_quixotic"), temp.path());
    assert_tool_success(&stale);
    assert_hit_path_set(&tool_body(&stale), &["drop.rs"]);

    let reindex = rpc_at(tool_call(3, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);

    // After refresh the deleted symbol misses and the kept symbol hits
    // exactly its own path: no stale hits survive.
    let gone = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
    assert_tool_success(&gone);
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let kept = rpc_at(search_call(5, "alpha_marker"), temp.path());
    assert_tool_success(&kept);
    assert_hit_path_set(&tool_body(&kept), &["keep.rs"]);
    assert_eq!(status_body(temp.path())["file_count"], 1);
}

#[test]
fn delete_delta_status_transition_and_refresh_stats() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.rs"), "fn i2_delstat_keep() {}\n").unwrap();
    std::fs::write(temp.path().join("drop.rs"), "fn i2_delstat_gone() {}\n").unwrap();
    index_tree(temp.path());

    let fresh = status_body(temp.path());
    assert_eq!(fresh["file_count"], 2, "{fresh:#}");
    let gen_fresh = fresh["writer_generation"].as_u64().unwrap();
    assert_ne!(gen_fresh, 0);

    // DELETE delta without refresh: fresh -> stale moves no discriminant.
    std::fs::remove_file(temp.path().join("drop.rs")).unwrap();
    let stale = status_body(temp.path());
    assert_eq!(stale["file_count"], 2, "{stale:#}");
    assert_eq!(
        stale["writer_generation"].as_u64().unwrap(),
        gen_fresh,
        "bare delete moves no epoch"
    );

    // Refresh: exact stats shape for a pure delete.
    let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_indexed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    // Stale -> fresh: count drops by exactly the delta, epoch advertised.
    let after = status_body(temp.path());
    assert_eq!(after["file_count"], 1, "{after:#}");
    let gen_after = after["writer_generation"].as_u64().unwrap();
    assert_ne!(gen_after, 0);
    assert_ne!(gen_after, gen_fresh, "refresh must advertise");
}

#[test]
fn rename_delta_old_path_gone_new_path_hit() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("old.rs"), "fn i2_rename_sym() {}\n").unwrap();
    index_tree(temp.path());

    let before = rpc_at(search_call(1, "i2_rename_sym"), temp.path());
    assert_hit_path_set(&tool_body(&before), &["old.rs"]);

    // RENAME delta without refresh: stale hits still carry the old path.
    std::fs::rename(temp.path().join("old.rs"), temp.path().join("new.rs")).unwrap();
    let stale = rpc_at(search_call(2, "i2_rename_sym"), temp.path());
    assert_tool_success(&stale);
    assert_hit_path_set(&tool_body(&stale), &["old.rs"]);

    let reindex = rpc_at(tool_call(3, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);

    // After refresh the symbol hits under exactly the new path; the old
    // path leaves both the hit set and the `p` table (pinned by helper).
    let moved = rpc_at(search_call(4, "i2_rename_sym"), temp.path());
    assert_tool_success(&moved);
    assert_hit_path_set(&tool_body(&moved), &["new.rs"]);
}

#[test]
fn rename_delta_status_file_count_stable() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("old.rs"), "fn i2_renstat_sym() {}\n").unwrap();
    index_tree(temp.path());

    let fresh = status_body(temp.path());
    assert_eq!(fresh["file_count"], 1, "{fresh:#}");
    let symbols = fresh["symbol_count"].as_u64().unwrap();
    assert!(symbols >= 1, "{fresh:#}");
    let gen_fresh = fresh["writer_generation"].as_u64().unwrap();
    assert_ne!(gen_fresh, 0);

    // RENAME delta without refresh: fresh -> stale moves no discriminant.
    std::fs::rename(temp.path().join("old.rs"), temp.path().join("new.rs")).unwrap();
    let stale = status_body(temp.path());
    assert_eq!(stale["file_count"], 1, "{stale:#}");
    assert_eq!(
        stale["writer_generation"].as_u64().unwrap(),
        gen_fresh,
        "bare rename moves no epoch"
    );

    // Refresh: rename is remove + add, count stable, epoch advertised.
    let reindex = rpc_at(tool_call(2, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    let after = status_body(temp.path());
    assert_eq!(after["file_count"], 1, "{after:#}");
    assert_eq!(after["symbol_count"], symbols, "{after:#}");
    assert_ne!(
        after["writer_generation"].as_u64().unwrap(),
        gen_fresh,
        "refresh must advertise"
    );
}

#[test]
fn add_multiple_delta_single_refresh_exact_hit_sets() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("base.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    // Two ADD deltas absorbed by a single refresh.
    std::fs::write(temp.path().join("c.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    std::fs::write(temp.path().join("d.rs"), "fn womble_frascati() {}\n").unwrap();
    let unseen_c = rpc_at(search_call(1, "zebroid_quixotic"), temp.path());
    assert_miss_envelope(&tool_body(&unseen_c), "no_match");
    let unseen_d = rpc_at(search_call(2, "womble_frascati"), temp.path());
    assert_miss_envelope(&tool_body(&unseen_d), "no_match");

    let reindex = rpc_at(tool_call(3, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert_eq!(stats["files_indexed"], 2, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    assert_eq!(status_body(temp.path())["file_count"], 3);

    // Each added symbol hits exactly its own path; the base symbol is
    // untouched: no cross-contamination between the two deltas.
    let hit_c = rpc_at(search_call(4, "zebroid_quixotic"), temp.path());
    assert_hit_path_set(&tool_body(&hit_c), &["c.rs"]);
    let hit_d = rpc_at(search_call(5, "womble_frascati"), temp.path());
    assert_hit_path_set(&tool_body(&hit_d), &["d.rs"]);
    let hit_base = rpc_at(search_call(6, "alpha_marker"), temp.path());
    assert_hit_path_set(&tool_body(&hit_base), &["base.rs"]);
}

#[test]
fn mixed_add_delete_delta_single_refresh_exact_hit_sets() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn joltik_nimbus() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn quarry_sphinx() {}\n").unwrap();
    index_tree(temp.path());
    assert_eq!(status_body(temp.path())["file_count"], 2);

    // Mixed delta: one delete plus one add, absorbed by a single refresh.
    std::fs::remove_file(temp.path().join("a.rs")).unwrap();
    std::fs::write(temp.path().join("c.rs"), "fn vortex_elm() {}\n").unwrap();

    let reindex = rpc_at(tool_call(1, "index_repo", json!({})), temp.path());
    assert_tool_success(&reindex);
    let stats = tool_body(&reindex);
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    assert_eq!(status_body(temp.path())["file_count"], 2);

    // Deleted symbol misses; added symbol hits exactly its path; kept
    // symbol hits exactly its path.
    let gone = rpc_at(search_call(2, "joltik_nimbus"), temp.path());
    assert_tool_success(&gone);
    assert_miss_envelope(&tool_body(&gone), "no_match");
    let added = rpc_at(search_call(3, "vortex_elm"), temp.path());
    assert_tool_success(&added);
    assert_hit_path_set(&tool_body(&added), &["c.rs"]);
    let kept = rpc_at(search_call(4, "quarry_sphinx"), temp.path());
    assert_tool_success(&kept);
    assert_hit_path_set(&tool_body(&kept), &["b.rs"]);
}
