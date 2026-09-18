//! I4 end-to-end freshness drills for ast-sgrep-mcp.
//!
//! Non-overlap contract: pass1 pins the broad invalidation contract (status
//! discriminants, serves-stale-silently, in-session / external / restart
//! agreement); pass2 pins each tree delta class with exact path sets and
//! refresh stats, mostly via fresh one-shot processes; pass3 pins rebuild
//! relations (incremental-vs-fresh parity, order independence, idempotence).
//! This file pins NONE of those again. Instead each test runs a FULL
//! change -> detect -> refresh -> serve drill inside ONE live stdio session
//! on a tempfile fixture:
//!
//! * SERVE (baseline): the session searches a freshly indexed root and the
//!   test captures exact baseline responses (hit path sets, miss codes,
//!   status bytes, writer generation);
//! * CHANGE: the test applies a real tree mutation out of band;
//! * DETECT: the same session proves staleness behaviorally -- search still
//!   serves pre-change bytes (or misses new symbols) while `index_status`
//!   stays byte-identical (no silent mutation; the pass1 contract holds);
//! * REFRESH: the same session calls `index_repo` and the test pins the
//!   advertised generation advance;
//! * SERVE (fresh): the same session proves exact new responses -- hit path
//!   sets, miss codes, counts, and byte-stable repeated serve.
//!
//! Drills cover ADD, MODIFY, DELETE, RENAME, two multi-edit single-refresh
//! arcs, a net-zero change-cancel arc, and one chained drill that runs all
//! four change kinds back to back in a single session.
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
            "clientInfo": {"name": "asgrep-mcp-i4", "version": "0"}
        }
    })
}

/// A live session that stays open across test-thread tree mutations between
/// tool calls, so one session spans the whole change -> detect -> refresh
/// -> serve arc.
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

    /// Successful lexical search; returns the raw body bytes.
    fn search_text(&mut self, query: &str) -> String {
        let response = self.call(
            "keyword_search",
            json!({"query": query, "limit": 8, "resend_seen": true}),
        );
        assert_tool_success(&response);
        tool_text(&response).to_owned()
    }

    /// Successful status call; returns the raw body bytes.
    fn status_text(&mut self) -> String {
        let response = self.call("index_status", json!({}));
        assert_tool_success(&response);
        tool_text(&response).to_owned()
    }

    /// Successful in-session refresh; returns the stats body.
    fn refresh(&mut self) -> Value {
        let response = self.call("index_repo", json!({}));
        assert_tool_success(&response);
        tool_body(&response)
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

/// Project paths carrying hits, resolved through the compact `p` table
/// exactly the way the server resolves them for `code_read`.
fn hit_path_set(body: &Value) -> BTreeSet<String> {
    let table: std::collections::HashMap<String, String> =
        ast_sgrep_plugins::resolve_compact_paths(body)
            .into_iter()
            .collect();
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

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("tool body JSON")
}

#[test]
fn drill_add_change_detect_refresh_serve() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn alpha_marker() {}\n").unwrap();
    index_tree(temp.path());

    // SERVE (baseline): session searches the indexed root.
    let mut session = LiveSession::spawn(temp.path());
    let keep_before = session.search_text("alpha_marker");
    assert_hit_path_set(&parse(&keep_before), &["a.rs"]);
    let status_before = session.status_text();
    let gen_before = parse(&status_before)["writer_generation"]
        .as_u64()
        .unwrap();
    assert_ne!(gen_before, 0);
    assert_eq!(parse(&status_before)["file_count"], 1);

    // CHANGE: a real tree add out of band.
    std::fs::write(temp.path().join("b.rs"), "fn zebroid_quixotic() {}\n").unwrap();

    // DETECT: the new symbol misses (stale serve) and status is byte-stable.
    let unseen = session.search_text("zebroid_quixotic");
    assert_miss_envelope(&parse(&unseen), "no_match");
    assert_eq!(
        session.status_text(),
        status_before,
        "bare add must move no status byte"
    );

    // REFRESH in session: exact add stats, generation advertised.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    let status_after = parse(&session.status_text());
    assert_eq!(status_after["file_count"], 2, "{status_after:#}");
    assert_ne!(
        status_after["writer_generation"].as_u64().unwrap(),
        gen_before,
        "refresh must advertise"
    );

    // SERVE (fresh): exact new responses, byte-stable on repeat.
    let found = session.search_text("zebroid_quixotic");
    assert_hit_path_set(&parse(&found), &["b.rs"]);
    assert_eq!(
        session.search_text("zebroid_quixotic"),
        found,
        "fresh serve must be byte-stable"
    );
    assert_eq!(
        session.search_text("alpha_marker"),
        keep_before,
        "untouched symbol serve must be unchanged"
    );
    session.finish();
}

#[test]
fn drill_modify_change_detect_refresh_serve() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn quarry_sphinx() {}\n").unwrap();
    index_tree(temp.path());

    // SERVE (baseline).
    let mut session = LiveSession::spawn(temp.path());
    let old_before = session.search_text("quarry_sphinx");
    assert_hit_path_set(&parse(&old_before), &["a.rs"]);
    let status_before = session.status_text();
    let gen_before = parse(&status_before)["writer_generation"]
        .as_u64()
        .unwrap();

    // CHANGE: rewrite the symbol in place.
    std::fs::write(temp.path().join("a.rs"), "fn vortex_elm() {}\n").unwrap();

    // DETECT: stale old bytes still serve, the new symbol misses, status
    // byte-stable.
    assert_eq!(
        session.search_text("quarry_sphinx"),
        old_before,
        "stale modify must serve pre-change bytes"
    );
    let unseen = session.search_text("vortex_elm");
    assert_miss_envelope(&parse(&unseen), "no_match");
    assert_eq!(
        session.status_text(),
        status_before,
        "bare modify must move no status byte"
    );

    // REFRESH in session.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    let status_after = parse(&session.status_text());
    assert_eq!(status_after["file_count"], 1, "{status_after:#}");
    assert_ne!(
        status_after["writer_generation"].as_u64().unwrap(),
        gen_before,
        "refresh must advertise"
    );

    // SERVE (fresh): old misses, new hits exactly the edited path.
    let gone = session.search_text("quarry_sphinx");
    assert_miss_envelope(&parse(&gone), "no_match");
    let found = session.search_text("vortex_elm");
    assert_hit_path_set(&parse(&found), &["a.rs"]);
    assert_ne!(found, old_before, "fresh bytes must differ from stale bytes");
    assert_eq!(
        session.search_text("vortex_elm"),
        found,
        "fresh serve must be byte-stable"
    );
    session.finish();
}

#[test]
fn drill_delete_change_detect_refresh_serve() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.rs"), "fn blip_candle() {}\n").unwrap();
    std::fs::write(temp.path().join("drop.rs"), "fn froth_gazebo() {}\n").unwrap();
    index_tree(temp.path());

    // SERVE (baseline).
    let mut session = LiveSession::spawn(temp.path());
    let gone_before = session.search_text("froth_gazebo");
    assert_hit_path_set(&parse(&gone_before), &["drop.rs"]);
    let keep_before = session.search_text("blip_candle");
    assert_hit_path_set(&parse(&keep_before), &["keep.rs"]);
    let status_before = session.status_text();
    assert_eq!(parse(&status_before)["file_count"], 2);
    let gen_before = parse(&status_before)["writer_generation"]
        .as_u64()
        .unwrap();

    // CHANGE: remove the file out of band.
    std::fs::remove_file(temp.path().join("drop.rs")).unwrap();

    // DETECT: the deleted row still serves stale bytes, status byte-stable.
    assert_eq!(
        session.search_text("froth_gazebo"),
        gone_before,
        "stale delete must serve pre-change bytes"
    );
    assert_eq!(
        session.status_text(),
        status_before,
        "bare delete must move no status byte"
    );

    // REFRESH in session: exact delete stats, count drops by the delta.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 0, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    let status_after = parse(&session.status_text());
    assert_eq!(status_after["file_count"], 1, "{status_after:#}");
    assert_ne!(
        status_after["writer_generation"].as_u64().unwrap(),
        gen_before,
        "refresh must advertise"
    );

    // SERVE (fresh): deleted symbol misses, kept symbol byte-identical.
    let gone = session.search_text("froth_gazebo");
    assert_miss_envelope(&parse(&gone), "no_match");
    assert_eq!(
        session.search_text("blip_candle"),
        keep_before,
        "kept symbol serve must be unchanged"
    );
    session.finish();
}

#[test]
fn drill_rename_change_detect_refresh_serve() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("old.rs"), "fn plumb_kiosk() {}\n").unwrap();
    index_tree(temp.path());

    // SERVE (baseline): the symbol hits under the old path.
    let mut session = LiveSession::spawn(temp.path());
    let stale_before = session.search_text("plumb_kiosk");
    assert_hit_path_set(&parse(&stale_before), &["old.rs"]);
    let status_before = session.status_text();
    let gen_before = parse(&status_before)["writer_generation"]
        .as_u64()
        .unwrap();

    // CHANGE: move the file out of band.
    std::fs::rename(temp.path().join("old.rs"), temp.path().join("new.rs")).unwrap();

    // DETECT: stale serve still carries the old path, status byte-stable.
    let stale = session.search_text("plumb_kiosk");
    assert_eq!(stale, stale_before, "stale rename must serve old bytes");
    assert_hit_path_set(&parse(&stale), &["old.rs"]);
    assert_eq!(
        session.status_text(),
        status_before,
        "bare rename must move no status byte"
    );

    // REFRESH in session: rename is remove + add, count stable.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    let status_after = parse(&session.status_text());
    assert_eq!(status_after["file_count"], 1, "{status_after:#}");
    assert_ne!(
        status_after["writer_generation"].as_u64().unwrap(),
        gen_before,
        "refresh must advertise"
    );

    // SERVE (fresh): the symbol hits under exactly the new path.
    let moved = session.search_text("plumb_kiosk");
    assert_hit_path_set(&parse(&moved), &["new.rs"]);
    assert_ne!(moved, stale_before, "fresh bytes must differ from stale bytes");
    assert_eq!(
        session.search_text("plumb_kiosk"),
        moved,
        "fresh serve must be byte-stable"
    );
    session.finish();
}

#[test]
fn drill_modify_twice_single_refresh_serves_final_only() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn snipe_tundra() {}\n").unwrap();
    index_tree(temp.path());

    // SERVE (baseline).
    let mut session = LiveSession::spawn(temp.path());
    let v1_before = session.search_text("snipe_tundra");
    assert_hit_path_set(&parse(&v1_before), &["a.rs"]);
    let status_before = session.status_text();

    // CHANGE: two successive modifies, no refresh between them.
    std::fs::write(temp.path().join("a.rs"), "fn womble_frascati() {}\n").unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn joltik_nimbus() {}\n").unwrap();

    // DETECT: stale v1 bytes still serve, neither later revision visible,
    // status byte-stable.
    assert_eq!(session.search_text("snipe_tundra"), v1_before);
    assert_miss_envelope(&parse(&session.search_text("womble_frascati")), "no_match");
    assert_miss_envelope(&parse(&session.search_text("joltik_nimbus")), "no_match");
    assert_eq!(
        session.status_text(),
        status_before,
        "bare double modify must move no status byte"
    );

    // REFRESH once absorbs both edits: exactly one file touched.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    // SERVE (fresh): only the final revision hits; both older ones miss.
    let found = session.search_text("joltik_nimbus");
    assert_hit_path_set(&parse(&found), &["a.rs"]);
    assert_miss_envelope(&parse(&session.search_text("snipe_tundra")), "no_match");
    assert_miss_envelope(&parse(&session.search_text("womble_frascati")), "no_match");
    assert_eq!(
        session.search_text("joltik_nimbus"),
        found,
        "fresh serve must be byte-stable"
    );
    session.finish();
}

#[test]
fn drill_add_then_delete_before_refresh_is_net_zero() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn gazebo_froth() {}\n").unwrap();
    index_tree(temp.path());

    // SERVE (baseline).
    let mut session = LiveSession::spawn(temp.path());
    let keep_before = session.search_text("gazebo_froth");
    assert_hit_path_set(&parse(&keep_before), &["a.rs"]);
    let status_before = parse(&session.status_text());
    assert_eq!(status_before["file_count"], 1, "{status_before:#}");

    // CHANGE then anti-change: add a file, then delete it before refresh.
    std::fs::write(
        temp.path().join("tmp.rs"),
        "fn quixotic_zebroid() {}\n",
    )
    .unwrap();
    std::fs::remove_file(temp.path().join("tmp.rs")).unwrap();

    // DETECT: the never-indexed symbol misses and counts are untouched.
    assert_miss_envelope(&parse(&session.search_text("quixotic_zebroid")), "no_match");
    assert_eq!(parse(&session.status_text())["file_count"], 1);

    // REFRESH: zero-mutation stats for a cancelled delta.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 0, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");

    // SERVE (fresh): the tree serves exactly as at baseline.
    assert_eq!(
        session.search_text("gazebo_froth"),
        keep_before,
        "cancelled delta must leave serve bytes unchanged"
    );
    assert_miss_envelope(&parse(&session.search_text("quixotic_zebroid")), "no_match");
    let status_after = parse(&session.status_text());
    assert_eq!(status_after["file_count"], 1, "{status_after:#}");
    assert_eq!(
        status_after["symbol_count"], status_before["symbol_count"],
        "cancelled delta must leave counts unchanged"
    );
    session.finish();
}

#[test]
fn drill_rename_chain_single_refresh_serves_final_path() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn tundra_snipe() {}\n").unwrap();
    index_tree(temp.path());

    // SERVE (baseline): the symbol hits under the original path.
    let mut session = LiveSession::spawn(temp.path());
    let first_before = session.search_text("tundra_snipe");
    assert_hit_path_set(&parse(&first_before), &["a.rs"]);
    let status_before = session.status_text();

    // CHANGE: rename twice (a -> b -> c) with no refresh between.
    std::fs::rename(temp.path().join("a.rs"), temp.path().join("b.rs")).unwrap();
    std::fs::rename(temp.path().join("b.rs"), temp.path().join("c.rs")).unwrap();

    // DETECT: stale serve still carries the original path, status byte-stable.
    let stale = session.search_text("tundra_snipe");
    assert_eq!(stale, first_before, "stale chain must serve original bytes");
    assert_hit_path_set(&parse(&stale), &["a.rs"]);
    assert_eq!(
        session.status_text(),
        status_before,
        "bare rename chain must move no status byte"
    );

    // REFRESH once absorbs the chain: remove + add, count stable.
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    assert_eq!(stats["files_failed"], 0, "{stats:#}");
    assert_eq!(parse(&session.status_text())["file_count"], 1);

    // SERVE (fresh): exactly the final path; both older paths are gone.
    let moved = session.search_text("tundra_snipe");
    assert_hit_path_set(&parse(&moved), &["c.rs"]);
    assert_eq!(
        session.search_text("tundra_snipe"),
        moved,
        "fresh serve must be byte-stable"
    );
    session.finish();
}

#[test]
fn drill_chained_add_modify_delete_rename_in_one_session() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a.rs"), "fn marble_alpha() {}\n").unwrap();
    std::fs::write(temp.path().join("b.rs"), "fn candle_blip() {}\n").unwrap();
    index_tree(temp.path());

    let mut session = LiveSession::spawn(temp.path());

    // Baseline serve: both symbols hit under their own paths.
    assert_hit_path_set(&parse(&session.search_text("marble_alpha")), &["a.rs"]);
    assert_hit_path_set(&parse(&session.search_text("candle_blip")), &["b.rs"]);
    assert_eq!(parse(&session.status_text())["file_count"], 2);
    let mut gen = parse(&session.status_text())["writer_generation"]
        .as_u64()
        .unwrap();

    // Link 1 -- ADD c.rs: detect (miss), refresh, serve (exact new path).
    std::fs::write(temp.path().join("c.rs"), "fn vortex_nimbus() {}\n").unwrap();
    assert_miss_envelope(&parse(&session.search_text("vortex_nimbus")), "no_match");
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 3, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    gen = status["writer_generation"].as_u64().unwrap();
    assert_hit_path_set(&parse(&session.search_text("vortex_nimbus")), &["c.rs"]);

    // Link 2 -- MODIFY a.rs (marble_alpha -> quixotic_elm): detect (stale old
    // bytes, new misses), refresh, serve (old misses, new hits a.rs).
    let a_before = session.search_text("marble_alpha");
    assert_hit_path_set(&parse(&a_before), &["a.rs"]);
    std::fs::write(temp.path().join("a.rs"), "fn quixotic_elm() {}\n").unwrap();
    assert_eq!(session.search_text("marble_alpha"), a_before);
    assert_miss_envelope(&parse(&session.search_text("quixotic_elm")), "no_match");
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 0, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 3, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    gen = status["writer_generation"].as_u64().unwrap();
    assert_miss_envelope(&parse(&session.search_text("marble_alpha")), "no_match");
    assert_hit_path_set(&parse(&session.search_text("quixotic_elm")), &["a.rs"]);

    // Link 3 -- DELETE b.rs: detect (stale bytes), refresh, serve (miss).
    let b_before = session.search_text("candle_blip");
    assert_hit_path_set(&parse(&b_before), &["b.rs"]);
    std::fs::remove_file(temp.path().join("b.rs")).unwrap();
    assert_eq!(session.search_text("candle_blip"), b_before);
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 0, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 2, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    gen = status["writer_generation"].as_u64().unwrap();
    assert_miss_envelope(&parse(&session.search_text("candle_blip")), "no_match");

    // Link 4 -- RENAME c.rs -> d.rs: detect (stale old path), refresh,
    // serve (exact new path).
    let c_before = session.search_text("vortex_nimbus");
    assert_hit_path_set(&parse(&c_before), &["c.rs"]);
    std::fs::rename(temp.path().join("c.rs"), temp.path().join("d.rs")).unwrap();
    let stale = session.search_text("vortex_nimbus");
    assert_eq!(stale, c_before);
    assert_hit_path_set(&parse(&stale), &["c.rs"]);
    let stats = session.refresh();
    assert_eq!(stats["files_indexed"], 1, "{stats:#}");
    assert_eq!(stats["files_removed"], 1, "{stats:#}");
    let status = parse(&session.status_text());
    assert_eq!(status["file_count"], 2, "{status:#}");
    assert_ne!(status["writer_generation"].as_u64().unwrap(), gen);
    assert_hit_path_set(&parse(&session.search_text("vortex_nimbus")), &["d.rs"]);

    // Final serve: the whole chained tree answers exactly, byte-stable.
    let a2 = session.search_text("quixotic_elm");
    assert_hit_path_set(&parse(&a2), &["a.rs"]);
    assert_eq!(session.search_text("quixotic_elm"), a2);
    let c = session.search_text("vortex_nimbus");
    assert_hit_path_set(&parse(&c), &["d.rs"]);
    assert_eq!(session.search_text("vortex_nimbus"), c);
    assert_miss_envelope(&parse(&session.search_text("marble_alpha")), "no_match");
    assert_miss_envelope(&parse(&session.search_text("candle_blip")), "no_match");
    session.finish();
}
