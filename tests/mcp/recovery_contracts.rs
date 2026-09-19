//! Recovery contracts: fail-closed boundaries and active mid-session faults.
//!
//! Consolidates `durable_recovery_pass1` (static contracts) + `pass2` (fault
//! injection). Each test pins ONE intent with multiple fault facets; catalog
//! map in `tests/catalog/recovery.md` (mcp R1/R2 rows).
//!
//! Discriminants are exit codes, `isError` booleans, envelope shapes (key
//! presence, tuple widths, counts, id echo), and byte equality -- never
//! message text. Every live-session read and process wait carries a timeout
//! so a regressed server fails the test instead of hanging the suite.

use ast_sgrep_testkit::{
    assert_ping_ok, assert_tool_error_shape, assert_tool_success, assert_tools_list_ok,
    corrupt_index_db, index_tree, indexed_tree, init_payload, ping, rpc_session, rpc_session_env,
    spawn_raw_no_handshake, tool_body, tool_call, tool_text, tools_list, truncate_file,
    LiveSession, TESTKIT_CLIENT_NAME,
};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

// Total budget for post-fault stdout drains (`drain_until_eof`).
const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

fn index_db_len(root: &Path) -> u64 {
    let db = root.join(".asgrep").join("index.db");
    assert!(db.is_file(), "expected an index db at {}", db.display());
    std::fs::metadata(&db).expect("stat index db").len()
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
    baseline_body.as_object_mut().unwrap().remove("writer_generation");
    recovered_body.as_object_mut().unwrap().remove("writer_generation");
    assert_eq!(recovered_body, baseline_body, "status drifted across rebuild");
}

/// INTENT: a missing `ASGREP_ROOT` fails the process closed with zero
/// JSON-RPC on stdout -- there is no session to serve.
/// KILLS: half-initialized-server mutants (startup answers tools against nothing).
#[test]
fn startup_with_missing_root_exits_without_json() {
    // No handshake at all: the canned drivers assert a clean exit, so the
    // raw spawn pins the nonzero startup failure instead.
    let temp = tempfile::tempdir().expect("tempdir");
    let missing = temp.path().join("does_not_exist");
    assert!(!missing.exists());
    let output = spawn_raw_no_handshake(
        Some(&missing),
        &[],
        &[init_payload(TESTKIT_CLIENT_NAME)],
    );
    assert!(
        !output.status.success(),
        "missing ASGREP_ROOT must fail the process, got {}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    let rpc_lines = stdout
        .lines()
        .filter(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v.get("jsonrpc").cloned())
                .is_some()
        })
        .count();
    assert_eq!(rpc_lines, 0, "no JSON-RPC may escape a failed startup: {stdout:?}");
}

/// INTENT: workspace-root disappearance is a per-call fail-closed fault, the
/// server itself survives, and healing (live recreation or restore+restart)
/// leaves no durable trace.
/// KILLS: hang-on-missing-root, no-live-heal, and fault-trace mutants.
/// FACETS: pipelined batch refusal; sequential next-call refusal; ping alive;
/// live heal in the same session; fault+restore+restart chain reproduction.
#[test]
fn root_loss_mid_session_fails_closed_ping_survives_live_heal_and_restart_reproduces() {
    // Facet 1 (pipelined): every tool in a pipelined batch fails closed with
    // the uniform shape while `ping` still answers and the process exits 0.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_status", json!({})));
    let before = session.recv();
    assert_tool_success(&before);
    std::fs::remove_dir_all(temp.path()).expect("remove root");
    assert!(!temp.path().exists());
    session.send(&tool_call(2, "index_status", json!({})));
    session.send(&tool_call(3, "keyword_search", json!({"query": "hey", "limit": 4})));
    session.send(&tool_call(4, "code_read", json!({"ids": ["a.rs#L1-L1"]})));
    let mut pipelined = vec![session.recv(), session.recv(), session.recv()];
    pipelined.sort_by_key(|r| r["id"].as_u64().unwrap());
    assert_eq!(pipelined[0]["id"], 2);
    for response in &pipelined {
        assert_tool_error_shape(response);
    }
    session.send(&ping(5));
    assert_ping_ok(&session.recv(), 5);
    session.close_stdin();
    assert!(session.wait_clean().success());

    // Facet 2 (sequential + live heal): the NEXT sequential call after deletion
    // fails closed, `ping` answers, and recreating the root heals the SAME
    // live session -- no restart required.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_status", json!({})));
    assert_tool_success(&session.recv());
    std::fs::remove_dir_all(temp.path()).expect("remove root");
    session.send(&tool_call(2, "index_status", json!({})));
    let failed = session.recv();
    assert_eq!(failed["id"], 2, "{failed:#}");
    assert_tool_error_shape(&failed);
    session.send(&ping(3));
    assert_ping_ok(&session.recv(), 3);
    std::fs::create_dir_all(temp.path()).expect("recreate root");
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    session.send(&tool_call(4, "index_status", json!({})));
    let healed = session.recv();
    assert_eq!(healed["id"], 4, "{healed:#}");
    assert_tool_success(&healed);
    assert_eq!(tool_body(&healed)["file_count"], 0);
    session.close_stdin();
    assert!(session.wait_clean().success());

    // Facet 3 (fault+restore+restart): capture the indexed chain bytes, delete
    // the root under a live session (next call fails closed), restore the
    // identical tree, restart. The fresh process reproduces the pre-fault
    // chain -- the fault leaves no durable trace.
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let source = temp.path().join("src");
    let chain = || {
        let mut session = LiveSession::spawn(Some(temp.path()));
        session.handshake();
        session.send(&tool_call(1, "index_status", json!({})));
        let status = session.recv();
        session.send(&tool_call(
            2,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ));
        let search = session.recv();
        session.close_stdin();
        assert!(session.wait_clean().success());
        (tool_text(&status).to_owned(), tool_text(&search).to_owned())
    };
    let (baseline_status, baseline_search) = chain();
    let mut faulty = LiveSession::spawn(Some(temp.path()));
    faulty.handshake();
    faulty.send(&tool_call(1, "index_status", json!({})));
    assert_tool_success(&faulty.recv());
    std::fs::remove_dir_all(temp.path()).expect("remove root");
    faulty.send(&tool_call(2, "index_status", json!({})));
    let failed = faulty.recv();
    assert_eq!(failed["id"], 2, "{failed:#}");
    assert_tool_error_shape(&failed);
    faulty.close_stdin();
    assert!(faulty.wait_clean().success());
    std::fs::create_dir_all(&source).expect("restore src");
    std::fs::write(source.join("lib.rs"), "fn target_symbol() {}\n").expect("restore source");
    index_tree(temp.path());
    let (restored_status, restored_search) = chain();
    assert_status_equal_across_rebuild(&restored_status, &baseline_status);
    assert_eq!(restored_search, baseline_search, "search drifted across fault");
    let search: Value = serde_json::from_str(&restored_search).expect("search JSON");
    let hits = search["h"].as_array().expect("hits");
    assert!(!hits.is_empty(), "{search:#}");
    assert_eq!(search["zn"].as_u64().unwrap() as usize, hits.len());
}

/// INTENT: misconfigured index/root locations fail closed without poisoning
/// the healthy default: a per-call root pointing at a regular file, and a
/// garbage `ASGREP_INDEX_PATH` pin.
/// KILLS: root-type-confusion and pin-poison mutants.
/// FACETS: file-root refusal across all tools; pinned-garbage refusal; unpin
/// serves the healthy default again.
#[test]
fn misconfigured_root_and_pinned_index_refused_default_unaffected() {
    // Facet 1: per-call root that exists but is a regular file -- neither the
    // missing-root nor the escaping-root case: every tool still fails closed.
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    let file_root = temp.path().join("src").join("lib.rs");
    assert!(file_root.is_file());
    let file_root = file_root.display().to_string();
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({"root": file_root})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "root": file_root}),
            ),
            tool_call(
                3,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": file_root}),
            ),
            tool_call(4, "index_repo", json!({"root": file_root})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 4);
    for response in &responses {
        assert_tool_error_shape(response);
    }

    // Facet 2: a garbage pinned db is refused loudly, and dropping the pin
    // serves the healthy default index again -- the pin never poisons it.
    let pin_dir = tempfile::tempdir().expect("tempdir");
    let pinned = pin_dir.path().join("pinned.db");
    std::fs::write(&pinned, "R1-pinned-garbage;".repeat(256)).expect("write pin");
    let pinned = pinned.display().to_string();
    let responses = rpc_session_env(
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4}),
            ),
        ],
        Some(temp.path()),
        &[("ASGREP_INDEX_PATH", pinned.as_str())],
    );
    assert_eq!(responses.len(), 2);
    for response in &responses {
        assert_tool_error_shape(response);
    }
    let responses = rpc_session(
        vec![
            tool_call(3, "index_status", json!({})),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_tool_success(&responses[0]);
    assert_eq!(tool_body(&responses[0])["file_count"], 1);
    assert_tool_success(&responses[1]);
    assert!(!tool_body(&responses[1])["h"].as_array().unwrap().is_empty());
}

/// INTENT: the corrupt-`index.db` boundary -- garbage bytes are refused loudly
/// by every index-dependent tool while `code_read` keeps serving files, and
/// deleting the corrupt inode plus `index_repo` heals search and status.
/// KILLS: silent-empty/fabricated-hit and heal-failure mutants.
/// FACETS: status/search/reindex+/-force refusal; read survival; delete+reindex
/// heal with hit-count shape.
#[test]
fn corrupt_index_refused_loudly_reads_survive_delete_and_reindex_heals() {
    // Facet 1 (refuse): garbage `index.db` under a healthy root is a tool
    // error on every index-dependent path -- never silent empty, never
    // fabricated hits -- while `code_read` serves files directly.
    let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
    corrupt_index_db(temp.path());
    let responses = rpc_session(
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(
                2,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4}),
            ),
            tool_call(3, "index_repo", json!({})),
            tool_call(4, "index_repo", json!({"force": true})),
            tool_call(5, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 5);
    for response in &responses[..4] {
        assert_tool_error_shape(response);
    }
    assert_tool_success(&responses[4]);
    assert_eq!(tool_body(&responses[4])["nodes"].as_array().unwrap().len(), 1);

    // Facet 2 (heal): remove the corrupt inode, `index_repo` rebuilds from
    // source, and status plus search serve the healed index.
    std::fs::remove_file(temp.path().join(".asgrep").join("index.db")).expect("remove db");
    let responses = rpc_session(
        vec![
            tool_call(1, "index_repo", json!({})),
            tool_call(2, "index_status", json!({})),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
            ),
        ],
        Some(temp.path()),
    );
    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_tool_success(response);
    }
    assert_eq!(tool_body(&responses[0])["files_indexed"], 1);
    assert_eq!(tool_body(&responses[1])["file_count"], 1);
    let envelope = tool_body(&responses[2]);
    let hits = envelope["h"].as_array().unwrap();
    assert!(!hits.is_empty(), "{envelope:#}");
    assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
}

/// INTENT: a live tear of `index.db` mid-session is refused loudly -- never a
/// silent empty success, never fabricated hits -- while `code_read` survives.
/// KILLS: warm-cache-masking and silent-empty mutants.
/// FACETS: 7-byte stub tear; half-length tear (header intact, pages missing);
/// post-tear searches use a fresh limit so they cannot ride the warm cache.
#[test]
fn index_torn_mid_session_stub_and_half_refused_reads_survive() {
    for (tear_len, arm) in [(7u64, "stub"), (0u64, "half")] {
        let temp = indexed_tree(&[("src/lib.rs", "fn target_symbol() {}\n")]);
        let mut session = LiveSession::spawn(Some(temp.path()));
        session.handshake();
        session.send(&tool_call(
            1,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 4, "resend_seen": true}),
        ));
        let before = session.recv();
        assert_tool_success(&before);
        assert!(!tool_body(&before)["h"].as_array().unwrap().is_empty());

        let full = index_db_len(temp.path());
        assert!(full > 4096, "fixture too small to tear: {full}");
        let torn = if arm == "stub" { tear_len } else { full / 2 };
        truncate_file(&temp.path().join(".asgrep").join("index.db"), torn);

        session.send(&tool_call(2, "index_status", json!({})));
        let status = session.recv();
        assert_eq!(status["id"], 2, "{status:#}");
        assert_tool_error_shape(&status);

        session.send(&tool_call(
            3,
            "keyword_search",
            json!({"query": "target_symbol", "limit": 8, "resend_seen": true}),
        ));
        let search = session.recv();
        assert_eq!(search["id"], 3, "{search:#}");
        assert_tool_error_shape(&search);

        // `code_read` serves files from the healthy tree, not the torn index.
        // Probed on the stub arm; the half arm pins the refusal delta only.
        if arm == "stub" {
            session.send(&tool_call(4, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})));
            let read = session.recv();
            assert_eq!(read["id"], 4, "{read:#}");
            assert_tool_success(&read);
            assert_eq!(tool_body(&read)["nodes"].as_array().unwrap().len(), 1);
        }
        session.close_stdin();
        let exit = session.wait_clean();
        assert!(exit.success(), "MCP exited {exit} on the {arm} arm");
    }
}

/// INTENT: stdin EOF mid-session -- clean, or with a torn request in flight --
/// terminates the process cleanly with no response for the torn id and no
/// non-JSON garbage on stdout.
/// KILLS: hang-on-eof and torn-id-answered mutants.
/// FACETS: clean EOF exits 0 in budget; partial-line EOF exits 0, emits no
/// torn-id response, stdout stays JSON.
#[test]
fn stdin_eof_clean_and_partial_line_exits_cleanly_without_torn_response() {
    // Facet 1 (clean EOF): EOF right after a healthy call terminates with
    // exit 0 inside the wait budget -- never a hang on a half-open session.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_status", json!({})));
    assert_tool_success(&session.recv());
    session.close_stdin();
    assert!(session.wait_clean().success());

    // Facet 2 (torn EOF): stdin closes with a torn request (bytes, no newline,
    // no closing brace) in flight. Exit 0, no torn-id response, JSON stdout.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send_partial(r#"{"jsonrpc":"2.0","id":9,"method":"ping""#);
    session.close_stdin();
    let lines = session.drain_until_eof(WAIT_TIMEOUT);
    assert!(session.wait_clean().success());
    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line.trim()).expect("stdout stays JSON");
        assert_ne!(value.get("id"), Some(&json!(9)), "torn id answered: {value:#}");
    }
}

/// INTENT: mid-stream faults are contained -- an unparsable line is ignored
/// without echo or hang, an invalid envelope yields the JSON-RPC error shape,
/// and the session serves the next valid call after each.
/// KILLS: hang/echo and error-shape mutants.
/// FACETS: garbage line draws no response and no late echo; unknown-method
/// envelope echoes its id with a numeric code and no `result`.
#[test]
fn stream_faults_garbage_ignored_invalid_envelope_errors_session_survives() {
    // Facet 1 (garbage line): ignored mid-stream; the next calls answer with
    // their own ids inside the read budget; no late echo after close.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_status", json!({})));
    assert_eq!(session.recv()["id"], 1);
    session.send_raw_line("{{not json");
    session.send(&ping(2));
    assert_ping_ok(&session.recv(), 2);
    session.send(&tool_call(3, "index_status", json!({})));
    let after = session.recv();
    assert_eq!(after["id"], 3, "{after:#}");
    assert_tool_success(&after);
    session.close_stdin();
    let rest = session.drain_until_eof(WAIT_TIMEOUT);
    assert!(
        rest.iter().all(|line| line.trim().is_empty()),
        "server echoed mid-stream garbage: {rest:?}"
    );
    assert!(session.wait_clean().success());

    // Facet 2 (invalid envelope): a well-formed unknown-method envelope yields
    // the JSON-RPC error shape -- echoed id, top-level `error` with a numeric
    // code, no `result` -- and the session serves the next valid call.
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_status", json!({})));
    assert_eq!(session.recv()["id"], 1);
    session.send(&json!({"jsonrpc": "2.0", "id": 77, "method": "missing"}));
    let error = session.recv();
    assert_eq!(error["id"], 77, "{error:#}");
    assert!(error["error"].is_object(), "{error:#}");
    assert!(error["error"]["code"].is_i64(), "{error:#}");
    assert!(error.get("result").is_none(), "{error:#}");
    session.send(&ping(78));
    assert_ping_ok(&session.recv(), 78);
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// INTENT: a fresh process serves deterministically with no inherited session
/// state -- the empty-root chain reproduces byte-identically across restarts
/// and snippet-elision memory does not survive a restart.
/// KILLS: restart-drift and durable-elision mutants.
/// FACETS: empty tools/list+status+miss bytes; elided-then-restart full-snippet
/// bytes with no elision markers.
#[test]
fn restart_reproduces_empty_chain_and_clears_elision_state() {
    // Facet 1 (empty restart): an empty root (no files, no index) serves
    // deterministic zero-hit responses across fresh processes.
    let temp = tempfile::tempdir().expect("tempdir");
    let chain = || {
        vec![
            tools_list(1),
            tool_call(2, "index_status", json!({})),
            tool_call(
                3,
                "keyword_search",
                json!({"query": "anything", "limit": 4, "resend_seen": true}),
            ),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 3);
    assert_eq!(second.len(), 3);
    assert_tools_list_ok(&first[0], 1);
    assert_eq!(
        serde_json::to_string(&first[0]["result"]).unwrap(),
        serde_json::to_string(&second[0]["result"]).unwrap(),
        "tools/list drifted across restarts"
    );
    assert_eq!(tool_text(&first[1]), tool_text(&second[1]), "status drifted");
    assert_tool_success(&first[1]);
    assert_eq!(tool_body(&first[1])["file_count"], 0);
    assert_eq!(tool_text(&first[2]), tool_text(&second[2]), "miss drifted");
    assert_tool_success(&first[2]);
    let miss = tool_body(&first[2]);
    assert_eq!(miss["why"], "empty_index", "{miss:#}");
    assert_eq!(miss["zn"], 0);
    assert_eq!(miss["h"].as_array().unwrap().len(), 0);
    assert!(miss.get("p").is_none(), "miss carries no path table: {miss:#}");

    // Facet 2 (elision reset): snippet elision is session memory, not durable
    // state -- a fresh process re-sends full snippets byte-identical to the
    // first session's first response.
    let temp = indexed_tree(&[(
        "src/lib.rs",
        "fn target_symbol() { helper(); }\nfn helper() {}\n",
    )]);
    let search = || tool_call(1, "keyword_search", json!({"query": "target_symbol", "limit": 4}));
    let first_session = rpc_session(vec![search(), search()], Some(temp.path()));
    assert_eq!(first_session.len(), 2);
    let first_bytes = tool_text(&first_session[0]).to_owned();
    let elided_bytes = tool_text(&first_session[1]).to_owned();
    let elided = tool_body(&first_session[1]);
    assert!(
        elided["h"].as_array().unwrap().iter().all(|hit| hit[4] == "~"),
        "expected every snippet elided: {elided:#}"
    );
    assert!(elided["ze"].as_u64().unwrap() > 0, "{elided:#}");
    assert!(elided_bytes.len() < first_bytes.len(), "elided response must be smaller");
    let second_session = rpc_session(vec![search()], Some(temp.path()));
    assert_eq!(
        tool_text(&second_session[0]),
        first_bytes,
        "restart must restore full snippets"
    );
    assert!(
        tool_body(&second_session[0]).get("ze").is_none(),
        "fresh session must not elide: {:#}",
        second_session[0]
    );
}
