//! I3 rebuild-parity metamorphic oracles for ast-sgrep-mcp.
//!
//! Non-overlap contract: pass1 pins the broad invalidation contract
//! (status discriminants, serves-stale-silently, in-session / external /
//! restart reindex agreement); pass2 pins each tree delta class (add, modify,
//! delete, rename, mixed) with exact path sets and refresh stats. This file
//! pins NONE of those again. Instead it asserts RELATIONS over stdio
//! sessions on tempfile fixtures:
//!
//! * REFRESH-VS-FRESH PARITY: an incrementally refreshed index and a freshly
//!   built index over the same final tree answer byte-identical search
//!   responses with equal status counts; a `force` rebuild preserves answers;
//! * DELTA ORDER INDEPENDENCE: the same final tree reached via different
//!   delta orders converges to byte-identical search responses and equal
//!   counts after one refresh each;
//! * REFRESH IDEMPOTENCE: a second refresh with no tree change is a content
//!   no-op (identical search bytes, identical counts, zero-mutation stats);
//! * INDEX_STATUS CONVERGENCE: repeated status calls are byte-stable, and
//!   counts stay converged across repeated refreshes.
//!
//! Discriminants are `isError` booleans, envelope codes (`why`), key
//! presence, counts, and byte equality -- never message text.
//!
//! `writer_generation` is asserted nonzero where relevant but never compared
//! for equality across refreshes: every `index_all` advertises a new stamp
//! even when no rows change, so the stamp is a liveness signal, not a
//! content digest. Content equality is carried by search bytes and counts.

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
            "clientInfo": {"name": "asgrep-mcp-i3", "version": "0"}
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

/// Successful lexical search response body bytes for one query.
fn search_text(root: &Path, query: &str) -> String {
    let response = rpc_at(search_call(1, query), root);
    assert_tool_success(&response);
    tool_text(&response).to_owned()
}

/// Incremental `index_repo` refresh; returns the stats body.
fn refresh(root: &Path) -> Value {
    let response = rpc_at(tool_call(1, "index_repo", json!({})), root);
    assert_tool_success(&response);
    tool_body(&response)
}

/// Full `index_repo` rebuild (`force: true`); returns the stats body.
fn force_refresh(root: &Path) -> Value {
    let response = rpc_at(tool_call(1, "index_repo", json!({"force": true})), root);
    assert_tool_success(&response);
    tool_body(&response)
}

fn status_body(root: &Path) -> Value {
    let response = rpc_at(tool_call(1, "index_status", json!({})), root);
    assert_tool_success(&response);
    tool_body(&response)
}

fn status_text(root: &Path) -> String {
    let response = rpc_at(tool_call(1, "index_status", json!({})), root);
    assert_tool_success(&response);
    tool_text(&response).to_owned()
}

/// Content counts that must converge for the same final tree. `root` and
/// `index_path` are absolute and excluded; `writer_generation` is a liveness
/// stamp (bumped on every refresh) and compared only for nonzeroness.
fn assert_same_counts(a: &Value, b: &Value) {
    for key in ["file_count", "line_count", "symbol_count"] {
        assert_eq!(a[key], b[key], "status {key} must converge: {a:#} vs {b:#}");
    }
}

fn assert_nonzero_generation(status: &Value) {
    assert_ne!(status["writer_generation"], 0, "{status:#}");
}

#[test]
fn incremental_refresh_matches_fresh_build_after_modify() {
    let inc = tempfile::tempdir().unwrap();
    std::fs::write(inc.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(inc.path());
    std::fs::write(inc.path().join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    refresh(inc.path());

    // Same final tree, indexed once from scratch: no delta history at all.
    let fresh = tempfile::tempdir().unwrap();
    std::fs::write(fresh.path().join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    index_tree(fresh.path());

    let hit_inc = search_text(inc.path(), "zebroid_quixotic");
    let hit_fresh = search_text(fresh.path(), "zebroid_quixotic");
    assert_hit_envelope(&serde_json::from_str(&hit_inc).unwrap());
    assert_eq!(hit_inc, hit_fresh, "incremental vs fresh hit bytes");

    let miss_inc = rpc_at(search_call(1, "alpha_marker"), inc.path());
    let miss_fresh = rpc_at(search_call(1, "alpha_marker"), fresh.path());
    assert_miss_envelope(&tool_body(&miss_inc), "no_match");
    assert_eq!(
        tool_text(&miss_inc),
        tool_text(&miss_fresh),
        "incremental vs fresh miss bytes"
    );
    assert_same_counts(&status_body(inc.path()), &status_body(fresh.path()));
}

#[test]
fn incremental_refresh_matches_fresh_build_after_mixed_deltas() {
    let inc = tempfile::tempdir().unwrap();
    std::fs::write(inc.path().join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
    std::fs::write(inc.path().join("b.rs"), "fn joltik_nimbus() {}\n").unwrap();
    index_tree(inc.path());
    // Mixed delta: one modify, one delete, one add.
    std::fs::write(
        inc.path().join("a.rs"),
        "fn quarry_sphinx() {}\nfn vortex_elm() {}\n",
    )
    .unwrap();
    std::fs::remove_file(inc.path().join("b.rs")).unwrap();
    std::fs::write(inc.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
    refresh(inc.path());

    // Same final tree, indexed once from scratch.
    let fresh = tempfile::tempdir().unwrap();
    std::fs::write(
        fresh.path().join("a.rs"),
        "fn quarry_sphinx() {}\nfn vortex_elm() {}\n",
    )
    .unwrap();
    std::fs::write(fresh.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
    index_tree(fresh.path());

    assert_eq!(
        search_text(inc.path(), "vortex_elm"),
        search_text(fresh.path(), "vortex_elm"),
        "modified-symbol hit bytes"
    );
    assert_eq!(
        search_text(inc.path(), "blip_candle"),
        search_text(fresh.path(), "blip_candle"),
        "added-symbol hit bytes"
    );
    let miss_inc = rpc_at(search_call(1, "joltik_nimbus"), inc.path());
    let miss_fresh = rpc_at(search_call(1, "joltik_nimbus"), fresh.path());
    assert_miss_envelope(&tool_body(&miss_inc), "no_match");
    assert_eq!(tool_text(&miss_inc), tool_text(&miss_fresh));
    let status_inc = status_body(inc.path());
    let status_fresh = status_body(fresh.path());
    assert_eq!(status_inc["file_count"], 2, "{status_inc:#}");
    assert_same_counts(&status_inc, &status_fresh);
}

#[test]
fn force_rebuild_preserves_search_bytes_and_counts() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn froth_gazebo() {}\n").unwrap();
    index_tree(temp.path());
    std::fs::write(temp.path().join("a.rs"), "fn plumb_kiosk() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn snipe_tundra() {}\n").unwrap();

    refresh(temp.path());
    let hit_before = search_text(temp.path(), "plumb_kiosk");
    assert_hit_envelope(&serde_json::from_str(&hit_before).unwrap());
    let counts_before = status_body(temp.path());
    assert_nonzero_generation(&counts_before);

    // A full rebuild over the same final tree must preserve answers exactly.
    let stats = force_refresh(temp.path());
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    let hit_after = search_text(temp.path(), "plumb_kiosk");
    assert_eq!(hit_before, hit_after, "force rebuild must preserve hit bytes");
    let miss_after = rpc_at(search_call(1, "froth_gazebo"), temp.path());
    assert_miss_envelope(&tool_body(&miss_after), "no_match");
    let counts_after = status_body(temp.path());
    assert_nonzero_generation(&counts_after);
    assert_same_counts(&counts_before, &counts_after);
}

#[test]
fn force_refresh_from_stale_matches_incremental_refresh() {
    let make_v1 = |dir: &Path| {
        std::fs::write(dir.join("a.rs"), "fn alpha_marker() {}\n").unwrap();
        std::fs::write(dir.join("b.rs"), "fn womble_frascati() {}\n").unwrap();
        index_tree(dir);
        // Identical stale delta on both trees: modify + delete + add.
        std::fs::write(dir.join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
        std::fs::remove_file(dir.join("b.rs")).unwrap();
        std::fs::write(dir.join("c.rs"), "fn joltik_nimbus() {}\n").unwrap();
    };
    let inc = tempfile::tempdir().unwrap();
    make_v1(inc.path());
    let forced = tempfile::tempdir().unwrap();
    make_v1(forced.path());

    refresh(inc.path());
    force_refresh(forced.path());

    assert_eq!(
        search_text(inc.path(), "zebroid_quixotic"),
        search_text(forced.path(), "zebroid_quixotic"),
        "incremental vs force hit bytes"
    );
    assert_eq!(
        search_text(inc.path(), "joltik_nimbus"),
        search_text(forced.path(), "joltik_nimbus"),
        "incremental vs force added-symbol hit bytes"
    );
    let miss_inc = rpc_at(search_call(1, "womble_frascati"), inc.path());
    assert_miss_envelope(&tool_body(&miss_inc), "no_match");
    let miss_forced = rpc_at(search_call(1, "womble_frascati"), forced.path());
    assert_eq!(tool_text(&miss_inc), tool_text(&miss_forced));
    assert_same_counts(&status_body(inc.path()), &status_body(forced.path()));
}

#[test]
fn add_order_independent_under_single_refresh() {
    let first = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("base.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(first.path());
    let second = tempfile::tempdir().unwrap();
    std::fs::write(second.path().join("base.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(second.path());

    // Same two adds in opposite order.
    std::fs::write(first.path().join("c.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    std::fs::write(first.path().join("d.rs"), "fn womble_frascati() {}\n").unwrap();
    std::fs::write(second.path().join("d.rs"), "fn womble_frascati() {}\n").unwrap();
    std::fs::write(second.path().join("c.rs"), "fn zebroid_quixotic() {}\n").unwrap();

    refresh(first.path());
    refresh(second.path());

    assert_eq!(
        search_text(first.path(), "zebroid_quixotic"),
        search_text(second.path(), "zebroid_quixotic"),
        "add order must not affect hit bytes"
    );
    assert_eq!(
        search_text(first.path(), "womble_frascati"),
        search_text(second.path(), "womble_frascati"),
        "add order must not affect hit bytes"
    );
    let a = status_body(first.path());
    let b = status_body(second.path());
    assert_eq!(a["file_count"], 3, "{a:#}");
    assert_same_counts(&a, &b);
}

#[test]
fn mixed_add_delete_order_independent() {
    let setup = |dir: &Path| {
        std::fs::write(dir.join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
        std::fs::write(dir.join("b.rs"), "fn vortex_elm() {}\n").unwrap();
        index_tree(dir);
    };
    let first = tempfile::tempdir().unwrap();
    setup(first.path());
    let second = tempfile::tempdir().unwrap();
    setup(second.path());

    // Same mixed delta in opposite order: delete-then-add vs add-then-delete.
    std::fs::remove_file(first.path().join("a.rs")).unwrap();
    std::fs::write(first.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
    std::fs::write(second.path().join("c.rs"), "fn blip_candle() {}\n").unwrap();
    std::fs::remove_file(second.path().join("a.rs")).unwrap();

    refresh(first.path());
    refresh(second.path());

    assert_eq!(
        search_text(first.path(), "blip_candle"),
        search_text(second.path(), "blip_candle"),
        "mixed order must not affect hit bytes"
    );
    assert_eq!(
        search_text(first.path(), "vortex_elm"),
        search_text(second.path(), "vortex_elm"),
        "kept symbol must be order-independent"
    );
    let miss_first = rpc_at(search_call(1, "quarry_sphinx"), first.path());
    assert_miss_envelope(&tool_body(&miss_first), "no_match");
    let miss_second = rpc_at(search_call(1, "quarry_sphinx"), second.path());
    assert_eq!(tool_text(&miss_first), tool_text(&miss_second));
    let a = status_body(first.path());
    let b = status_body(second.path());
    assert_eq!(a["file_count"], 2, "{a:#}");
    assert_same_counts(&a, &b);
}

#[test]
fn modify_add_interleave_order_independent() {
    let setup = |dir: &Path| {
        std::fs::write(dir.join("a.rs"), "fn froth_gazebo() {}\n").unwrap();
        index_tree(dir);
    };
    let first = tempfile::tempdir().unwrap();
    setup(first.path());
    let second = tempfile::tempdir().unwrap();
    setup(second.path());

    // Same modify + add in opposite interleave order.
    std::fs::write(first.path().join("a.rs"), "fn plumb_kiosk() {}\n").unwrap();
    std::fs::write(first.path().join("b.rs"), "fn snipe_tundra() {}\n").unwrap();
    std::fs::write(second.path().join("b.rs"), "fn snipe_tundra() {}\n").unwrap();
    std::fs::write(second.path().join("a.rs"), "fn plumb_kiosk() {}\n").unwrap();

    refresh(first.path());
    refresh(second.path());

    assert_eq!(
        search_text(first.path(), "plumb_kiosk"),
        search_text(second.path(), "plumb_kiosk"),
        "interleave order must not affect hit bytes"
    );
    assert_eq!(
        search_text(first.path(), "snipe_tundra"),
        search_text(second.path(), "snipe_tundra"),
        "interleave order must not affect hit bytes"
    );
    let miss_first = rpc_at(search_call(1, "froth_gazebo"), first.path());
    assert_miss_envelope(&tool_body(&miss_first), "no_match");
    let miss_second = rpc_at(search_call(1, "froth_gazebo"), second.path());
    assert_eq!(tool_text(&miss_first), tool_text(&miss_second));
    assert_same_counts(&status_body(first.path()), &status_body(second.path()));
}

#[test]
fn refresh_twice_search_bytes_identical() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());
    std::fs::write(temp.path().join("a.rs"), "fn zebroid_quixotic() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn womble_frascati() {}\n").unwrap();

    refresh(temp.path());
    let hit_once = search_text(temp.path(), "zebroid_quixotic");
    assert_hit_envelope(&serde_json::from_str(&hit_once).unwrap());
    let miss_once = search_text(temp.path(), "alpha_marker");
    assert_miss_envelope(&serde_json::from_str(&miss_once).unwrap(), "no_match");

    // Second refresh with no tree change: answers must be identical.
    refresh(temp.path());
    assert_eq!(
        search_text(temp.path(), "zebroid_quixotic"),
        hit_once,
        "refresh twice must equal refresh once"
    );
    assert_eq!(
        search_text(temp.path(), "alpha_marker"),
        miss_once,
        "miss bytes must survive a redundant refresh"
    );
    assert_eq!(
        search_text(temp.path(), "womble_frascati"),
        search_text(temp.path(), "womble_frascati"),
        "repeated fresh-process search must be self-consistent"
    );
}

#[test]
fn second_refresh_without_changes_is_content_noop() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn i3_noop_old() {}\n").unwrap();
    index_tree(temp.path());
    std::fs::write(temp.path().join("a.rs"), "fn i3_noop_new() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn i3_noop_add() {}\n").unwrap();

    // First refresh absorbs the delta: exactly the two touched files.
    let first = refresh(temp.path());
    assert_eq!(first["files_indexed"], 2, "{first:#}");
    assert_eq!(first["files_removed"], 0, "{first:#}");
    assert_eq!(first["files_failed"], 0, "{first:#}");
    let counts_before = status_body(temp.path());
    assert_nonzero_generation(&counts_before);

    // Second refresh with no tree change mutates nothing.
    let second = refresh(temp.path());
    assert_eq!(second["files_indexed"], 0, "{second:#}");
    assert_eq!(second["files_removed"], 0, "{second:#}");
    assert_eq!(second["files_failed"], 0, "{second:#}");
    let counts_after = status_body(temp.path());
    assert_nonzero_generation(&counts_after);
    assert_same_counts(&counts_before, &counts_after);
}

#[test]
fn index_status_byte_stable_across_repeated_calls() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn i3_stable_one() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn i3_stable_two() {}\n").unwrap();
    index_tree(temp.path());

    // No refresh between calls: the status body must be byte-identical.
    let first = status_text(temp.path());
    let second = status_text(temp.path());
    let third = status_text(temp.path());
    assert_eq!(first, second, "repeated status must be byte-stable");
    assert_eq!(first, third, "repeated status must be byte-stable");
    let body: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(body["file_count"], 2, "{body:#}");
    assert_nonzero_generation(&body);
}

#[test]
fn index_status_counts_converge_across_repeated_refresh() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn i3_conv_old() {}\n").unwrap();
    index_tree(temp.path());
    std::fs::write(temp.path().join("a.rs"), "fn i3_conv_new() {}\n").unwrap();

    // Three refreshes over one delta: counts and answers converge at once
    // and stay converged; only the liveness stamp may advance.
    refresh(temp.path());
    let counts_first = status_body(temp.path());
    let hit_first = search_text(temp.path(), "i3_conv_new");
    refresh(temp.path());
    let counts_second = status_body(temp.path());
    let hit_second = search_text(temp.path(), "i3_conv_new");
    refresh(temp.path());
    let counts_third = status_body(temp.path());
    let hit_third = search_text(temp.path(), "i3_conv_new");

    assert_nonzero_generation(&counts_first);
    assert_nonzero_generation(&counts_second);
    assert_nonzero_generation(&counts_third);
    assert_same_counts(&counts_first, &counts_second);
    assert_same_counts(&counts_first, &counts_third);
    assert_eq!(hit_first, hit_second, "hits must converge across refreshes");
    assert_eq!(hit_first, hit_third, "hits must converge across refreshes");
    assert_hit_envelope(&serde_json::from_str(&hit_first).unwrap());
}
