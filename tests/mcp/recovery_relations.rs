//! Recovery relations: equalities between calls, sessions, and restarts.
//!
//! Consolidates `durable_recovery_pass3` (metamorphic relations). Each test
//! pins ONE intent with multiple relational facets; catalog map in
//! `tests/catalog/recovery.md` (mcp R3 rows). Pure testkit: every flow below
//! is expressible with `LiveSession` + canned `rpc_session` (no byte-level
//! stream faults -- those live in `recovery_contracts`).
//!
//! Discriminants are `isError` booleans, envelope shapes (key presence, tuple
//! widths, counts, id echo), and byte (in)equality -- never message text.
//! Every live-session read and process wait carries a timeout.

use ast_sgrep_testkit::{
    assert_ping_ok, assert_tool_error_shape, assert_tool_success, assert_tools_list_ok, index_tree,
    indexed_tree, ping, rpc_session, tool_body, tool_call, tool_text, tools_list, truncate_file,
    LiveSession,
};
use serde_json::{json, Value};
use std::path::Path;

const FIXTURE_SOURCE: &str = "fn target_symbol() { helper(); }\nfn helper() {}\n";
const FAULT_SOURCE: &str =
    "fn mutated_symbol() { changed(); }\nfn changed() {}\n// R3-fault-sentinel\n";

/// Two-symbol indexed tree: two distinct searchable queries plus a
/// multi-line file for read-window relations.
fn two_symbol_tree() -> tempfile::TempDir {
    indexed_tree(&[("src/lib.rs", FIXTURE_SOURCE)])
}

fn search_call(id: u32, query: &str) -> Value {
    tool_call(id, "keyword_search", json!({"query": query, "limit": 4, "resend_seen": true}))
}

fn index_db(root: &Path) -> std::path::PathBuf {
    root.join(".asgrep").join("index.db")
}

/// tools/list catalog bytes: the discovery region must be restart-stable.
// WHY file-local: one-use comparator for this file's catalog facets.
fn catalog_bytes(response: &Value) -> String {
    serde_json::to_string(&response["result"]).expect("catalog JSON")
}

/// INTENT: discovery, transcripts, and error envelopes are invariant under
/// restarts and fault cycles -- recovery leaks no state into what the server
/// advertises or how it fails.
/// KILLS: session-memory, catalog-drift, and error-drift mutants.
/// FACETS: 3-session full-transcript identity (+ indexed-chain hit-shape spot
/// checks); tools/list bytes under invalid-envelope/index-tear/root-delete
/// cycles and the restarts after them; invalid-call error bytes.
#[test]
fn transcripts_catalog_and_errors_reproduce_across_restarts_and_faults() {
    // Facet 1 (transcript identity): three independent fresh processes over an
    // indexed tree produce fully identical transcripts -- full response values,
    // not just shapes. Every call is first-in-session, so no session memory
    // may perturb any byte. Hit-shape spot checks fold in the indexed chain.
    let temp = two_symbol_tree();
    let chain = || {
        vec![
            tools_list(1),
            ping(2),
            tool_call(3, "index_status", json!({})),
            search_call(4, "target_symbol"),
            search_call(5, "helper"),
            tool_call(6, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ]
    };
    let runs: Vec<Vec<Value>> = (0..3).map(|_| rpc_session(chain(), Some(temp.path()))).collect();
    for run in &runs {
        assert_eq!(run.len(), 6);
    }
    assert_eq!(runs[1], runs[0], "second transcript differs");
    assert_eq!(runs[2], runs[0], "third transcript differs");
    assert_tools_list_ok(&runs[0][0], 1);
    assert_ping_ok(&runs[0][1], 2);
    for response in runs[0].iter().skip(2) {
        assert_tool_success(response);
    }
    assert_eq!(tool_body(&runs[0][2])["file_count"], 1);
    for response in [&runs[0][3], &runs[0][4]] {
        let envelope = tool_body(response);
        let hits = envelope["h"].as_array().unwrap();
        assert!(!hits.is_empty(), "{envelope:#}");
        assert_eq!(envelope["zn"].as_u64().unwrap() as usize, hits.len());
        for hit in hits {
            assert_eq!(hit.as_array().unwrap().len(), 5, "{hit:#}");
        }
    }
    assert_eq!(tool_body(&runs[0][5])["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(tool_body(&runs[0][5])["nodes"][0]["id"], "src/lib.rs#L1-L1");

    // Facet 2 (catalog stability): tools/list bytes are invariant under an
    // invalid-envelope fault, a torn index db, and a deleted workspace root,
    // plus the restarts that follow each. (Garbage-line tolerance itself is
    // pinned in `recovery_contracts`; here the stream fault is the
    // invalid envelope, which needs only `Value` sends.)
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tools_list(1));
    let list = session.recv();
    assert_tools_list_ok(&list, 1);
    let baseline = catalog_bytes(&list);
    session.send(&json!({"jsonrpc": "2.0", "id": 77, "method": "missing"}));
    let error = session.recv();
    assert_eq!(error["id"], 77, "{error:#}");
    assert!(error["error"].is_object(), "{error:#}");
    assert!(error["error"]["code"].is_i64(), "{error:#}");
    assert!(error.get("result").is_none(), "{error:#}");
    session.send(&tools_list(2));
    assert_eq!(catalog_bytes(&session.recv()), baseline, "catalog drifted across envelope fault");
    let db = index_db(temp.path());
    assert!(std::fs::metadata(&db).expect("stat db").len() > 4096);
    truncate_file(&db, 7);
    session.send(&tools_list(3));
    assert_eq!(catalog_bytes(&session.recv()), baseline, "catalog drifted across index tear");
    session.send(&tool_call(4, "index_status", json!({})));
    let status = session.recv();
    assert_eq!(status["id"], 4, "{status:#}");
    assert_tool_error_shape(&status);
    session.close_stdin();
    assert!(session.wait_clean().success());
    std::fs::remove_file(index_db(temp.path())).expect("remove db");
    index_tree(temp.path());
    let restarted = rpc_session(vec![tools_list(1)], Some(temp.path()));
    assert_eq!(catalog_bytes(&restarted[0]), baseline, "catalog drifted across heal+restart");

    // Root-fault cycle on a second tree: discovery answers identically while
    // the workspace is deleted, after live recreation, and after a restart.
    let temp2 = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp2.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    let mut root_session = LiveSession::spawn(Some(temp2.path()));
    root_session.handshake();
    root_session.send(&tools_list(1));
    let root_baseline = catalog_bytes(&root_session.recv());
    std::fs::remove_dir_all(temp2.path()).expect("remove root");
    assert!(!temp2.path().exists());
    root_session.send(&tools_list(2));
    assert_eq!(
        catalog_bytes(&root_session.recv()),
        root_baseline,
        "catalog drifted while root deleted"
    );
    root_session.send(&ping(3));
    assert_ping_ok(&root_session.recv(), 3);
    std::fs::create_dir_all(temp2.path()).expect("recreate root");
    std::fs::write(temp2.path().join("a.rs"), "fn hey() {}\n").expect("write source");
    root_session.send(&tools_list(4));
    assert_eq!(
        catalog_bytes(&root_session.recv()),
        root_baseline,
        "catalog drifted after live root heal"
    );
    root_session.close_stdin();
    assert!(root_session.wait_clean().success());
    let root_restarted = rpc_session(vec![tools_list(1)], Some(temp2.path()));
    assert_eq!(
        catalog_bytes(&root_restarted[0]),
        root_baseline,
        "catalog drifted across root-fault restart"
    );
    assert_eq!(root_baseline, baseline, "catalog differs between trees");

    // Facet 3 (error determinism): invalid calls fail with byte-identical tool
    // errors in every fresh process; every failure keeps the uniform shape.
    let chain = || {
        vec![
            tool_call(1, "keyword_search", json!({"query": "target_symbol", "limit": 0})),
            tool_call(2, "keyword_search", json!({"query": "", "limit": 4})),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L99"]})),
            tool_call(4, "no_such_tool", json!({})),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 4);
    assert_eq!(second.len(), 4);
    for (index, response) in first.iter().enumerate() {
        assert_tool_error_shape(response);
        assert_tool_error_shape(&second[index]);
        assert_eq!(
            tool_text(response),
            tool_text(&second[index]),
            "error {index} drifted across restarts"
        );
    }
}

/// INTENT: the server's semantic relations are restart invariants -- the
/// search→compact-id→read link, the `resend_seen` stateless encoding, and
/// omitted-vs-explicit-default root equivalence all reproduce byte-identically
/// across fresh sessions.
/// KILLS: link-rot, position-drift, and root-equivalence mutants.
/// FACETS: compact/stable read agreement within and across sessions;
/// `resend_seen` bytes across position/interleave/restart with no elision
/// markers; explicit-root == default-root within and across sessions.
#[test]
fn search_read_link_resend_seen_and_root_equivalence_invariants() {
    let temp = two_symbol_tree();
    let stable = "src/lib.rs#L1-L1";

    // Facet 1 (link consistency): in EVERY fresh session the search ->
    // compact-id -> code_read link resolves to the same stable node with the
    // same bytes, and the whole link reproduces across restarts.
    let probe_a = rpc_session(vec![search_call(1, "target_symbol")], Some(temp.path()));
    let compact_a = tool_body(&probe_a[0])["h"][0][0]
        .as_str()
        .expect("compact id")
        .to_owned();
    let linked_a = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": [compact_a]})),
            tool_call(3, "code_read", json!({"ids": [stable]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(linked_a.len(), 3);
    assert_eq!(
        tool_text(&linked_a[0]),
        tool_text(&probe_a[0]),
        "first-search bytes differ between fresh sessions"
    );
    let probe_b = rpc_session(vec![search_call(1, "target_symbol")], Some(temp.path()));
    assert_eq!(tool_text(&probe_b[0]), tool_text(&probe_a[0]), "search drifted across restarts");
    let compact_b = tool_body(&probe_b[0])["h"][0][0]
        .as_str()
        .expect("compact id")
        .to_owned();
    let linked_b = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": [compact_b]})),
            tool_call(3, "code_read", json!({"ids": [stable]})),
        ],
        Some(temp.path()),
    );
    for (index, response) in linked_a.iter().enumerate() {
        assert_tool_success(response);
        assert_eq!(
            tool_text(&linked_b[index]),
            tool_text(response),
            "linked response {index} drifted across restarts"
        );
    }
    for linked in [&linked_a, &linked_b] {
        let via_compact = tool_body(&linked[1]);
        let via_stable = tool_body(&linked[2]);
        assert_eq!(via_compact["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(via_stable["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(via_compact["nodes"][0]["id"], stable);
        assert_eq!(via_stable["nodes"][0]["id"], stable);
        assert_eq!(
            via_compact["nodes"][0]["content"], via_stable["nodes"][0]["content"],
            "compact and stable reads disagree within one session"
        );
    }

    // Facet 2 (`resend_seen` statelessness): with `resend_seen` the search
    // encoding is a pure function of (query, index) -- identical at every call
    // position, interleaved with other calls, and across restarts, with no
    // elision markers anywhere.
    let pair = rpc_session(
        vec![search_call(1, "target_symbol"), search_call(2, "target_symbol")],
        Some(temp.path()),
    );
    assert_eq!(tool_text(&pair[0]), tool_text(&pair[1]), "resend_seen bytes differ by position");
    let restarted = rpc_session(vec![search_call(1, "target_symbol")], Some(temp.path()));
    assert_eq!(
        tool_text(&restarted[0]),
        tool_text(&pair[0]),
        "resend_seen bytes differ across restart"
    );
    let interleaved = rpc_session(
        vec![
            tool_call(1, "index_status", json!({})),
            search_call(2, "target_symbol"),
            tool_call(3, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            search_call(4, "target_symbol"),
        ],
        Some(temp.path()),
    );
    assert_eq!(
        tool_text(&interleaved[1]),
        tool_text(&pair[0]),
        "resend_seen bytes differ when interleaved"
    );
    assert_eq!(
        tool_text(&interleaved[3]),
        tool_text(&pair[0]),
        "resend_seen bytes differ at a later position"
    );
    for response in [&pair[0], &pair[1], &restarted[0], &interleaved[1], &interleaved[3]] {
        assert_tool_success(response);
        let envelope = tool_body(response);
        let hits = envelope["h"].as_array().unwrap();
        assert!(!hits.is_empty(), "{envelope:#}");
        assert!(envelope.get("ze").is_none(), "stateless encoding must not elide: {envelope:#}");
        for hit in hits {
            assert_ne!(hit[4], "~", "snippet elided despite resend_seen: {hit:#}");
        }
    }

    // Facet 3 (root equivalence): f(root omitted) == f(root = default)
    // byte-identically within each session, and both sides reproduce across
    // restarts -- the equivalence class itself is the restart invariant.
    let root = temp.path().display().to_string();
    let chain = || {
        vec![
            tool_call(1, "index_status", json!({})),
            tool_call(2, "index_status", json!({"root": root.as_str()})),
            search_call(3, "target_symbol"),
            tool_call(
                4,
                "keyword_search",
                json!({"query": "target_symbol", "limit": 4, "resend_seen": true, "root": root.as_str()}),
            ),
            tool_call(5, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
            tool_call(
                6,
                "code_read",
                json!({"ids": ["src/lib.rs#L1-L1"], "root": root.as_str()}),
            ),
        ]
    };
    let first = rpc_session(chain(), Some(temp.path()));
    let second = rpc_session(chain(), Some(temp.path()));
    assert_eq!(first.len(), 6);
    assert_eq!(second.len(), 6);
    for (omitted, explicit) in [(0usize, 1usize), (2, 3), (4, 5)] {
        assert_tool_success(&first[omitted]);
        assert_eq!(
            tool_text(&first[explicit]),
            tool_text(&first[omitted]),
            "explicit root differs from default in the first session"
        );
        assert_eq!(
            tool_text(&second[explicit]),
            tool_text(&second[omitted]),
            "explicit root differs from default in the second session"
        );
        assert_eq!(
            tool_text(&second[omitted]),
            tool_text(&first[omitted]),
            "default-root response drifted across restarts"
        );
    }
}

/// INTENT: source-byte faults are tracked live and survive restarts unchanged
/// -- a restart neither heals nor masks durable bytes -- and restoring the
/// exact bytes plus a restart reproduces the baseline search and read.
/// KILLS: heal-or-mask mutants (restart hiding the fault, or restore drifting).
/// FACETS: fault bytes != baseline bytes; fault reproduces identically across
/// two fresh processes; restore+restart == baseline bytes.
#[test]
fn source_fault_tracked_across_restarts_restore_recovers_baseline() {
    let temp = two_symbol_tree();
    let lib = temp.path().join("src").join("lib.rs");
    let baseline = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    let baseline_search = tool_text(&baseline[0]).to_owned();
    let baseline_read = tool_text(&baseline[1]).to_owned();

    std::fs::write(&lib, FAULT_SOURCE).expect("write fault");
    let mut faulty = Vec::new();
    for _ in 0..2 {
        let responses = rpc_session(
            vec![tool_call(1, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]}))],
            Some(temp.path()),
        );
        faulty.push(tool_text(&responses[0]).to_owned());
        assert_tool_success(&responses[0]);
        assert_eq!(tool_body(&responses[0])["nodes"][0]["id"], "src/lib.rs#L1-L1");
    }
    assert_eq!(faulty[0], faulty[1], "fault must reproduce across restarts");
    assert_ne!(faulty[0], baseline_read, "reads must track the live tree under fault");

    std::fs::write(&lib, FIXTURE_SOURCE).expect("restore source");
    let healed = rpc_session(
        vec![
            search_call(1, "target_symbol"),
            tool_call(2, "code_read", json!({"ids": ["src/lib.rs#L1-L1"]})),
        ],
        Some(temp.path()),
    );
    assert_eq!(tool_text(&healed[0]), baseline_search, "search drifted after fault roundtrip");
    assert_eq!(tool_text(&healed[1]), baseline_read, "read drifted after fault roundtrip");
}
