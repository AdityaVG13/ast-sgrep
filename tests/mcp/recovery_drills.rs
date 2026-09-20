//! Recovery drills: full crash→recover→serve cycles over stdio.
//!
//! Consolidates `durable_recovery_pass4`. Each test pins ONE intent with
//! multiple crash facets; catalog map in `tests/catalog/recovery.md` (mcp R4
//! rows). Every drill starts from a WORKING session serving status/search/read,
//! captures the pre-crash baseline bytes, CRASHES, then starts a FRESH session
//! that RECOVERS and SERVES the identical chain.
//!
//! Discriminants are exit codes, `isError` booleans, envelope shapes (key
//! presence, tuple widths, counts, id echo), filesystem facts, and byte
//! (in)equality -- never message text. Every stdio read and process wait
//! carries a timeout.

use ast_sgrep_testkit::{
    assert_tool_success, corrupt_index_db, indexed_tree, tool_body, tool_call, tool_text,
    truncate_file, LiveSession,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

// Total budget for post-fault stdout drains (`drain_until_eof`).
const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

const FIXTURE_SOURCE: &str = "fn target_symbol() { helper(); }\nfn helper() {}\n";

/// Two-symbol indexed tree: one searchable target plus a second symbol and a
/// multi-line file for read-window drills.
fn drill_tree() -> tempfile::TempDir {
    indexed_tree(&[("src/lib.rs", FIXTURE_SOURCE)])
}

fn index_db_path(root: &Path) -> PathBuf {
    root.join(".asgrep").join("index.db")
}

fn status_call(id: u32) -> Value {
    tool_call(id, "index_status", json!({}))
}

fn search_call(id: u32) -> Value {
    tool_call(
        id,
        "keyword_search",
        json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
    )
}

fn read_call(id: u32) -> Value {
    tool_call(id, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}))
}

fn reindex_call(id: u32) -> Value {
    tool_call(id, "index_repo", json!({}))
}

/// Serve the canonical drill chain: status, search, read. Returns the three
/// responses in order; asserts id echo so pipelined order cannot hide.
// WHY file-local: one-use drill comparator for this file's serve proofs.
fn serve_chain(session: &mut LiveSession, base_id: u32) -> (Value, Value, Value) {
    session.send(&status_call(base_id));
    let status = session.recv();
    assert_eq!(status["id"], base_id, "{status:#}");
    session.send(&search_call(base_id + 1));
    let search = session.recv();
    assert_eq!(search["id"], base_id + 1, "{search:#}");
    session.send(&read_call(base_id + 2));
    let read = session.recv();
    assert_eq!(read["id"], base_id + 2, "{read:#}");
    (status, search, read)
}

/// The working-session proof: every leg serves with the expected shape.
// WHY file-local: one-use drill comparator for this file's serve proofs.
fn assert_serve_valid(status: &Value, search: &Value, read: &Value) {
    assert_tool_success(status);
    assert_eq!(tool_body(status)["file_count"], 1, "{status:#}");
    assert_tool_success(search);
    let envelope = tool_body(search);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
    for hit in hits {
        assert_eq!(hit.as_array().unwrap().len(), 5, "{hit:#}");
    }
    assert_tool_success(read);
    let body = tool_body(read);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1, "{body:#}");
    assert_eq!(body["nodes"][0]["id"], "src/lib.rs#L1-L1", "{body:#}");
}

/// Status across a rebuild: `writer_generation` is a fresh unique stamp per
/// index build, so it is compared by shape while every content key must match.
// WHY file-local: one-use comparator for this file's rebuild facets.
fn assert_status_equal_across_rebuild(recovered: &str, baseline: &str) {
    let mut baseline_body: Value = serde_json::from_str(baseline).expect("status JSON");
    let mut recovered_body: Value = serde_json::from_str(recovered).expect("status JSON");
    for body in [&baseline_body, &recovered_body] {
        assert!(body["writer_generation"].is_u64(), "{body:#}");
    }
    baseline_body
        .as_object_mut()
        .unwrap()
        .remove("writer_generation");
    recovered_body
        .as_object_mut()
        .unwrap()
        .remove("writer_generation");
    assert_eq!(
        recovered_body, baseline_body,
        "status drifted across rebuild"
    );
}

/// Capture the pre-crash baseline chain from a working session.
// WHY file-local: one-use drill helper; returns owned baseline bytes.
fn capture_baseline(session: &mut LiveSession) -> (String, String, String) {
    let (status, search, read) = serve_chain(session, 1);
    assert_serve_valid(&status, &search, &read);
    (
        tool_text(&status).to_owned(),
        tool_text(&search).to_owned(),
        tool_text(&read).to_owned(),
    )
}

/// Fresh-session recovery proof with intact durable state: the chain serves
/// byte-identically to the baseline.
// WHY file-local: one-use drill helper for the intact-state facets.
fn assert_fresh_serves_baseline(root: &Path, baseline: &(String, String, String), what: &str) {
    let mut fresh = LiveSession::spawn(Some(root));
    fresh.handshake();
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 1);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_eq!(
        tool_text(&rstatus),
        baseline.0,
        "status drifted across {what}"
    );
    assert_eq!(
        tool_text(&rsearch),
        baseline.1,
        "search drifted across {what}"
    );
    assert_eq!(tool_text(&rread), baseline.2, "read drifted across {what}");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

/// Fresh-session recovery proof after durable loss: heal via `index_repo`,
/// then search/read reproduce byte-identically and status matches across the
/// rebuild stamp.
// WHY file-local: one-use drill helper for the durable-loss facets.
fn assert_heal_then_serves_baseline(root: &Path, baseline: &(String, String, String), what: &str) {
    let mut fresh = LiveSession::spawn(Some(root));
    fresh.handshake();
    fresh.send(&reindex_call(1));
    let rebuilt = fresh.recv();
    assert_eq!(rebuilt["id"], 1, "{rebuilt:#}");
    assert_tool_success(&rebuilt);
    assert_eq!(tool_body(&rebuilt)["files_indexed"], 1, "{rebuilt:#}");
    let (rstatus, rsearch, rread) = serve_chain(&mut fresh, 2);
    assert_serve_valid(&rstatus, &rsearch, &rread);
    assert_status_equal_across_rebuild(tool_text(&rstatus), &baseline.0);
    assert_eq!(
        tool_text(&rsearch),
        baseline.1,
        "search drifted across {what}"
    );
    assert_eq!(tool_text(&rread), baseline.2, "read drifted across {what}");
    fresh.close_stdin();
    assert!(fresh.wait_clean().success());
}

/// INTENT: SIGKILL of a working session -- idle or with pipelined requests in
/// flight -- leaves no durable trace: a fresh session serves the pre-crash
/// chain byte-identically.
/// KILLS: crash-trace mutants (kill residue in durable state or serve path).
/// FACETS: idle kill; kill with 3 pipelined sends unread (pending output
/// asserted nothing about -- it is nondeterministic by design).
#[test]
fn kill_idle_and_in_flight_recovers_identical_serve() {
    // Facet 1 (idle kill): SIGKILL of an idle working session after a healthy
    // serve. Tree and index are intact, so recovery is direct.
    let temp = drill_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    let crash = work.crash_kill();
    assert!(
        !crash.success(),
        "SIGKILL must terminate the server: {crash}"
    );
    assert_fresh_serves_baseline(temp.path(), &baseline, "idle kill");

    // Facet 2 (in-flight kill): SIGKILL with pipelined requests in flight
    // (three sends, no reads) after a healthy baseline. The killed session's
    // pending output is nondeterministic and asserted nothing about; the fresh
    // session serves the baseline chain byte-identically.
    let temp = drill_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    work.send(&status_call(11));
    work.send(&search_call(12));
    work.send(&read_call(13));
    let crash = work.crash_kill();
    assert!(
        !crash.success(),
        "SIGKILL must terminate the server: {crash}"
    );
    assert_fresh_serves_baseline(temp.path(), &baseline, "in-flight kill");
}

/// INTENT: stdin-EOF crashes mid-session -- clean EOF or EOF with a torn
/// request in flight -- exit 0 with no torn-id response and no non-JSON
/// stdout, and a fresh session serves the pre-EOF chain byte-identically.
/// KILLS: eof-trace and torn-id-answered mutants.
/// FACETS: clean EOF serve recovery; aborted-mid-request exit shape + serve
/// recovery.
#[test]
fn stdin_eof_clean_and_aborted_recovers_identical_serve() {
    // Facet 1 (clean EOF): clean stdin EOF mid-session after a healthy serve
    // exits 0, and the fresh session reproduces the chain byte-identically.
    let temp = drill_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    work.close_stdin();
    let exit = work.wait_clean();
    assert!(exit.success(), "clean EOF must exit 0: {exit}");
    assert_fresh_serves_baseline(temp.path(), &baseline, "clean EOF");

    // Facet 2 (aborted EOF): stdin closes with a torn request (bytes, no
    // newline, no closing brace) in flight after a healthy serve. The crashed
    // process exits 0 with no torn-id response and no non-JSON stdout; the
    // fresh session serves the baseline chain byte-identically.
    let temp = drill_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    work.send_partial(r#"{"jsonrpc":"2.0","id":99,"method":"ping""#);
    work.close_stdin();
    let lines = work.drain_until_eof(WAIT_TIMEOUT);
    let exit = work.wait_clean();
    assert!(exit.success(), "torn EOF must exit 0: {exit}");
    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line.trim()).expect("stdout stays JSON");
        assert_ne!(
            value.get("id"),
            Some(&json!(99)),
            "torn id answered: {value:#}"
        );
    }
    assert_fresh_serves_baseline(temp.path(), &baseline, "aborted EOF");
}

/// INTENT: a crash during durable loss heals to the pre-crash baseline --
/// out-of-band filesystem restore plus a fresh session's `index_repo` rebuild
/// -- and chained crashes converge to the ORIGINAL baseline with no residue.
/// KILLS: restore/heal-divergence and chain-residue mutants.
/// FACETS: root deleted + kill → restore + reindex; index corrupted + kill →
/// delete + reindex; index deleted + kill → reindex; chained kill → interim
/// serve → tear + kill → delete + rebuild → original baseline.
#[test]
fn crash_during_durable_loss_heals_to_baseline_and_chained_double_crash() {
    // Facet 1 (root drill): the workspace root is deleted under a working
    // session and the server is SIGKILLed during the durable loss. Recovery
    // restores out of band and heals via a fresh session's `index_repo`.
    let temp = drill_tree();
    let source = temp.path().join("src");
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    std::fs::remove_dir_all(temp.path()).expect("remove root");
    assert!(!temp.path().exists());
    let crash = work.crash_kill();
    assert!(
        !crash.success(),
        "SIGKILL must terminate the server: {crash}"
    );
    std::fs::create_dir_all(&source).expect("restore src");
    std::fs::write(source.join("lib.rs"), FIXTURE_SOURCE).expect("restore source");
    assert!(source.join("lib.rs").is_file());
    assert_heal_then_serves_baseline(temp.path(), &baseline, "root crash");

    // Facet 2 (index-corrupt drill): `index.db` is overwritten with garbage
    // under a working session, then the server is SIGKILLed. Recovery deletes
    // the corrupt inode and a fresh session's `index_repo` rebuilds.
    let temp = drill_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    corrupt_index_db(temp.path());
    assert!(index_db_path(temp.path()).is_file());
    let crash = work.crash_kill();
    assert!(
        !crash.success(),
        "SIGKILL must terminate the server: {crash}"
    );
    std::fs::remove_file(index_db_path(temp.path())).expect("remove db");
    assert!(!index_db_path(temp.path()).exists());
    assert_heal_then_serves_baseline(temp.path(), &baseline, "index-corrupt crash");

    // Facet 3 (index-deleted drill): `index.db` is deleted outright under a
    // working session (no inode, no bytes, only the source tree remains), then
    // the server is SIGKILLed. A fresh session's `index_repo` rebuilds.
    let temp = drill_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    let db = index_db_path(temp.path());
    assert!(db.is_file());
    std::fs::remove_file(&db).expect("remove db");
    assert!(!db.exists());
    let crash = work.crash_kill();
    assert!(
        !crash.success(),
        "SIGKILL must terminate the server: {crash}"
    );
    assert_heal_then_serves_baseline(temp.path(), &baseline, "index-deleted crash");

    // Facet 4 (chained drill): kill #1 with durable state intact, an interim
    // fresh session proves recovery, then the index is torn to a stub and kill
    // #2 crashes during the fault. Recovery deletes the stub and a final fresh
    // session's `index_repo` rebuilds: the final SERVE reproduces the ORIGINAL
    // pre-crash baseline.
    let temp = drill_tree();
    let mut work = LiveSession::spawn(Some(temp.path()));
    work.handshake();
    let baseline = capture_baseline(&mut work);
    let crash_one = work.crash_kill();
    assert!(!crash_one.success(), "kill #1 must terminate: {crash_one}");
    let mut interim = LiveSession::spawn(Some(temp.path()));
    interim.handshake();
    let (istatus, isearch, iread) = serve_chain(&mut interim, 1);
    assert_serve_valid(&istatus, &isearch, &iread);
    assert_eq!(tool_text(&istatus), baseline.0, "interim status drifted");
    assert_eq!(tool_text(&isearch), baseline.1, "interim search drifted");
    assert_eq!(tool_text(&iread), baseline.2, "interim read drifted");
    let db = index_db_path(temp.path());
    assert!(std::fs::metadata(&db).expect("stat db").len() > 4096);
    truncate_file(&db, 7);
    assert_eq!(std::fs::metadata(&db).expect("stat db").len(), 7);
    let crash_two = interim.crash_kill();
    assert!(!crash_two.success(), "kill #2 must terminate: {crash_two}");
    std::fs::remove_file(index_db_path(temp.path())).expect("remove db");
    assert!(!index_db_path(temp.path()).exists());
    assert_heal_then_serves_baseline(temp.path(), &baseline, "chained crash");
}
