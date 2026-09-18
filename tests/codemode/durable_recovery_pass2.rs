//! R2 fault-injection oracles for codemode durable state.
//!
//! Pass 1 owns STATIC recovery contracts (corrupt fixtures written before any
//! flow starts). This file owns ACTIVE faults: every test verifies a healthy
//! flow FIRST, injects a fault MID-FLOW, then asserts the refusal or recovery
//! contract. Each test names its fault class in a `Fault class:` comment.
//!
//! Discipline: failures assert `CallError` discriminants via `matches!`,
//! never Display text. Filesystem facts are asserted as bytes/presence, never
//! messages. Every fixture is a fixed-content tempdir; no sample-fixture
//! dependence, no timing asserts. No new dependencies.

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, BatchCall, BatchRequest, CallError, CodeModeSession,
    ParallelMode, Plan, SessionConfig,
};
use serde_json::{json, Value};
use std::path::Path;

/// Load a host-persisted plan file: missing bytes are `Other` (io), corrupt
/// bytes are `Json` (syntax), well-formed values delegate to `parse_plan`.
fn load_plan_file(path: &Path) -> Result<Plan, CallError> {
    let bytes = std::fs::read(path).map_err(|e| CallError::Other(e.into()))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(CallError::from)?;
    parse_plan(&value)
}

/// Load a host-persisted batch file with the same layering: io -> `Other`,
/// syntax -> `Json`, per-call validation stays in `run_batch`.
fn load_batch_file(path: &Path) -> Result<BatchRequest, CallError> {
    let bytes = std::fs::read(path).map_err(|e| CallError::Other(e.into()))?;
    let request: BatchRequest = serde_json::from_slice(&bytes).map_err(CallError::from)?;
    Ok(request)
}

fn config_for(root: &Path, index_name: Option<&str>) -> SessionConfig {
    SessionConfig {
        root: root.to_path_buf(),
        index_path: index_name.map(|name| root.join(name)),
        limit: 5,
        use_embed: false,
        ..SessionConfig::default()
    }
}

/// Fixed-content repo indexed through the public session API. The unique
/// token lives in exactly one file, so shaped counts are hand-computable.
fn indexed_repo() -> (tempfile::TempDir, SessionConfig) {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path().join("src");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("a.rs"), "pub fn needle_unique_r2_xyz() {}\n").expect("write a");
    std::fs::write(dir.join("b.rs"), "pub fn other_fn() {}\n").expect("write b");
    let config = config_for(temp.path(), Some("index.db"));
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("index_repo", json!({"force": false}))
        .expect("index");
    (temp, config)
}

fn batch_request(calls: Vec<BatchCall>, mode: ParallelMode) -> BatchRequest {
    BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit: None,
        parallel: None,
        parallel_mode: Some(mode),
        calls,
    }
}

fn set_readonly(path: &Path, readonly: bool) {
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_readonly(readonly);
    std::fs::set_permissions(path, perms).expect("chmod");
}

#[test]
fn fault_torn_plan_file_midflow_refused_as_json() {
    // Fault class: TORN-PLAN. A plan file that loaded and executed cleanly is
    // torn mid-flow (valid prefix, cut tail, as from a crashed writer). Resume
    // is refused loudly at the syntax layer (`Json`); the live in-memory plan
    // is unaffected, proving the fault hit durable bytes, not live state.
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("plan dir");
    let plan_raw = json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": "needle_unique_r2_xyz", "format": "capsule", "limit": 5}},
        {"id": "narrow", "tool": "filter_hits",
         "args": {"hits": "$seed", "path_contains": "src/a.rs", "limit": 5}},
        {"id": "out", "tool": "select",
         "args": {"value": "$narrow", "fields": ["hit_count"]}},
    ], "return": "$out"});
    let path = dir.path().join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan_raw).expect("plan json")).expect("write plan");

    // Flow starts healthy: load + execute once.
    let plan = load_plan_file(&path).expect("plan loads before fault");
    let mut session = CodeModeSession::new(config.clone());
    let first = run_plan(&mut session, &plan).expect("plan runs before fault");
    assert!(first.ok);
    assert_eq!(first.return_value, json!({"hit_count": 1}));

    // ACTIVE FAULT: torn write mid-flow.
    let raw = std::fs::read(&path).expect("read plan");
    std::fs::write(&path, &raw[..raw.len() / 2]).expect("torn plan");

    let err = load_plan_file(&path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    let mut resumed = CodeModeSession::new(config);
    let again = run_plan(&mut resumed, &plan).expect("live plan unaffected");
    assert_eq!(again.return_value, first.return_value);
}

#[test]
fn fault_truncated_batch_file_midflow_refused_as_json() {
    // Fault class: TRUNCATED-BATCH. A batch file that loaded and executed
    // cleanly is truncated mid-flow. Resume is refused loudly at the syntax
    // layer (`Json`); the live in-memory request still executes, proving the
    // fault hit durable bytes, not live state.
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("batch dir");
    let batch_raw = batch_request(
        vec![
            BatchCall {
                id: "w1".into(),
                tool: "catalog_search".into(),
                args: json!({"query": "search"}),
            },
            BatchCall {
                id: "w2".into(),
                tool: "catalog_search".into(),
                args: json!({"query": "chain"}),
            },
        ],
        ParallelMode::Serial,
    );
    let path = dir.path().join("batch.json");
    std::fs::write(&path, serde_json::to_vec(&batch_raw).expect("batch json"))
        .expect("write batch");

    // Flow starts healthy: load + execute once.
    let request = load_batch_file(&path).expect("batch loads before fault");
    let first = run_batch(config.clone(), &request).expect("batch runs before fault");
    assert!(first.all_ok);
    assert_eq!(first.call_count, 2);

    // ACTIVE FAULT: truncate mid-flow.
    let raw = std::fs::read(&path).expect("read batch");
    std::fs::write(&path, &raw[..raw.len() / 2]).expect("truncate batch");

    let err = load_batch_file(&path)
        .map(|_| ())
        .expect_err("truncated batch must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    let again = run_batch(config, &request).expect("live batch unaffected");
    assert!(again.all_ok);
    assert_eq!(again.results.len(), first.results.len());
}

#[test]
fn fault_truncated_index_midflow_refused_loudly_without_quarantine() {
    // Fault class: TRUNCATED-INDEX. A serving index is cut mid-file AFTER a
    // verified search. Host resume (fresh session) fails closed with `Other`
    // on read, write, and plan paths alike, and no silent `.corrupt`
    // quarantine appears: refusal is total until the operator intervenes.
    let (_temp, config) = indexed_repo();
    let db = config.index_path.clone().expect("index path");
    let mut session = CodeModeSession::new(config.clone());
    let before = session
        .call("search", json!({"query": "needle_unique_r2_xyz", "limit": 5}))
        .expect("search before fault");
    assert!(!before["hits"].as_array().expect("hits").is_empty());

    // ACTIVE FAULT: truncate durable index mid-flow (header intact, body cut).
    let bytes = std::fs::read(&db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    std::fs::write(&db, &bytes[..512]).expect("truncate db");

    let mut resumed = CodeModeSession::new(config.clone());
    let err = resumed
        .call("search", json!({"query": "needle_unique_r2_xyz", "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = resumed
        .call("index_status", json!({}))
        .expect_err("status on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "search", "args": {"query": "needle_unique_r2_xyz", "limit": 5}},
    ]}))
    .expect("parse");
    let err = run_plan(&mut resumed, &plan).expect_err("plan on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    let quarantines: Vec<_> = std::fs::read_dir(_temp.path())
        .expect("readdir")
        .filter_map(|entry| entry.ok().map(|e| e.file_name()))
        .filter(|name| name.to_string_lossy().contains(".corrupt"))
        .collect();
    assert!(quarantines.is_empty(), "no silent quarantine: {quarantines:?}");
}

#[test]
fn fault_session_dir_deleted_midrun_fails_closed() {
    // Fault class: ROOT-DELETED. The session root is deleted mid-run after a
    // verified search (index lives in a separate tempdir so the fault removes
    // the source of truth, not just the index). The warm session AND a fresh
    // resume fail closed with `Other` on every root-touching path.
    let root_temp = tempfile::tempdir().expect("root tempdir");
    let index_temp = tempfile::tempdir().expect("index tempdir");
    let dir = root_temp.path().join("src");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("a.rs"), "pub fn needle_unique_r2_gone() {}\n").expect("write a");
    let config = SessionConfig {
        root: root_temp.path().to_path_buf(),
        index_path: Some(index_temp.path().join("index.db")),
        limit: 5,
        use_embed: false,
        ..SessionConfig::default()
    };
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("index_repo", json!({"force": false}))
        .expect("index");
    session
        .call("search", json!({"query": "needle_unique_r2_gone", "limit": 5}))
        .expect("search before fault");

    // ACTIVE FAULT: session dir deleted mid-run.
    std::fs::remove_dir_all(root_temp.path()).expect("delete root");
    assert!(!root_temp.path().exists());

    let err = session
        .call("search", json!({"query": "needle_unique_r2_gone", "limit": 5}))
        .expect_err("warm search after root deletion must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    let mut resumed = CodeModeSession::new(config.clone());
    let err = resumed
        .call("read", json!({"path": "src/a.rs", "start": 1, "end": 2}))
        .expect_err("read after root deletion must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "search", "args": {"query": "needle_unique_r2_gone", "limit": 5}},
    ]}))
    .expect("parse");
    let err = run_plan(&mut resumed, &plan).expect_err("plan after root deletion must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
}

#[test]
fn fault_readonly_session_dir_writes_fail_cleanly_as_other() {
    // Fault class: READONLY-DIR. The session dir goes read-only mid-run after
    // a verified search. Writers (`edit`, `index_repo`) error cleanly with
    // `Other` (never a panic, never a partial write); reads either still
    // serve or fail with the same discriminant. Durable bytes are unchanged.
    let (temp, config) = indexed_repo();
    let db = config.index_path.clone().expect("index path");
    let src_dir = temp.path().join("src");
    let a_rs = src_dir.join("a.rs");
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("search", json!({"query": "needle_unique_r2_xyz", "limit": 5}))
        .expect("search before fault");

    // ACTIVE FAULT: session dir read-only mid-run.
    let targets = [
        temp.path().to_path_buf(),
        src_dir.clone(),
        a_rs.clone(),
        src_dir.join("b.rs"),
        db.clone(),
    ];
    for path in &targets {
        set_readonly(path, true);
    }
    // Prove the fault bites in this environment; privileged uids bypass
    // permission bits, in which case there is no fault to test.
    let probe = src_dir.join(".r2-probe");
    if std::fs::write(&probe, b"probe").is_ok() {
        let _ = std::fs::remove_file(&probe);
        for path in &targets {
            set_readonly(path, false);
        }
        return;
    }

    // Capture results while read-only, restore before asserting so tempdir
    // cleanup can never strand (asserts below cannot leak the fixture).
    let edit_result = session.call(
        "edit",
        json!({"path": "src/a.rs", "oldText": "needle_unique_r2_xyz", "newText": "needle_r2_ro"}),
    );
    let index_result = session.call("index_repo", json!({"force": false}));
    let search_result = session.call("search", json!({"query": "needle_unique_r2_xyz", "limit": 5}));
    for path in &targets {
        set_readonly(path, false);
    }

    let err = edit_result.expect_err("edit on read-only dir must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = index_result.expect_err("index_repo on read-only dir must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    match search_result {
        Ok(_) => {}
        Err(err) => assert!(matches!(err, CallError::Other(_)), "got {err:?}"),
    }
    let body = std::fs::read_to_string(&a_rs).expect("reread");
    assert!(
        body.contains("needle_unique_r2_xyz"),
        "failed writes must leave durable bytes unchanged: {body}"
    );
}

#[test]
fn fault_interrupted_batch_failed_call_atomic_siblings_preserved() {
    // Fault class: INTERRUPTED-BATCH-ATOMICITY. A batch wave is interrupted by
    // a failing middle call (a multi-edit whose later edit can never match).
    // The failed call commits NOTHING (atomic within the call), the wave does
    // not abort (siblings keep ids/order/values), and the error is present as
    // data (presence discriminant, never message text) — the resume point.
    let (_temp, config) = indexed_repo();
    let a_rs = config.root.join("src/a.rs");
    let before = std::fs::read_to_string(&a_rs).expect("read before");
    let request = batch_request(
        vec![
            BatchCall {
                id: "w1".into(),
                tool: "search".into(),
                args: json!({"query": "needle_unique_r2_xyz", "limit": 5}),
            },
            BatchCall {
                id: "w2".into(),
                tool: "edit".into(),
                args: json!({"edits": [
                    {"path": "src/a.rs",
                     "oldText": "needle_unique_r2_xyz", "newText": "needle_mutated_r2_xyz"},
                    {"path": "src/a.rs",
                     "oldText": "not-present-anywhere", "newText": "x"},
                ]}),
            },
            BatchCall {
                id: "w3".into(),
                tool: "search".into(),
                args: json!({"query": "needle_unique_r2_xyz", "limit": 5}),
            },
        ],
        ParallelMode::Serial,
    );

    let response = run_batch(config, &request).expect("batch runs");
    assert!(!response.all_ok);
    assert_eq!(response.call_count, 3);
    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].id, "w1");
    assert_eq!(response.results[1].id, "w2");
    assert_eq!(response.results[2].id, "w3");
    assert!(response.results[0].ok);
    assert!(!response.results[1].ok);
    assert!(response.results[1].value.is_none());
    assert!(response.results[1].error.is_some());
    assert!(response.results[2].ok);

    let after = std::fs::read_to_string(&a_rs).expect("reread");
    assert_eq!(after, before, "failed call must commit nothing");
    assert!(!after.contains("needle_mutated_r2_xyz"), "{after}");
}

#[test]
fn fault_interrupted_batch_commits_prefix_resumes_from_failed_id() {
    // Fault class: INTERRUPTED-BATCH-RESUME. A batch wave `[edit-ok,
    // edit-bad]` commits its prefix (cross-call non-atomicity is DOCUMENTED,
    // not silent) and the host resumes by re-running ONLY the failed id with
    // corrected args: the resume wave is all_ok and the final bytes show both
    // mutations in order.
    let (_temp, config) = indexed_repo();
    let a_rs = config.root.join("src/a.rs");
    let request = batch_request(
        vec![
            BatchCall {
                id: "step-1".into(),
                tool: "edit".into(),
                args: json!({"path": "src/a.rs",
                    "oldText": "needle_unique_r2_xyz", "newText": "needle_resumed_r2_xyz"}),
            },
            BatchCall {
                id: "step-2".into(),
                tool: "edit".into(),
                args: json!({"path": "src/a.rs",
                    "oldText": "not-present-anywhere", "newText": "x"}),
            },
        ],
        ParallelMode::Serial,
    );

    let response = run_batch(config.clone(), &request).expect("batch runs");
    assert!(!response.all_ok);
    assert!(response.results[0].ok);
    assert!(!response.results[1].ok);
    assert_eq!(response.results[1].id, "step-2");

    // Committed prefix visible on disk: no cross-call rollback (documented).
    let mid = std::fs::read_to_string(&a_rs).expect("reread mid");
    assert!(mid.contains("needle_resumed_r2_xyz"), "{mid}");

    // Resume: re-run only the failed id with corrected args.
    let resume = batch_request(
        vec![BatchCall {
            id: "step-2".into(),
            tool: "edit".into(),
            args: json!({"path": "src/a.rs",
                "oldText": "needle_resumed_r2_xyz", "newText": "needle_final_r2_xyz"}),
        }],
        ParallelMode::Serial,
    );
    let resumed = run_batch(config, &resume).expect("resume runs");
    assert!(resumed.all_ok);
    assert_eq!(resumed.results.len(), 1);
    assert_eq!(resumed.results[0].id, "step-2");
    assert!(resumed.results[0].ok);

    let final_body = std::fs::read_to_string(&a_rs).expect("reread final");
    assert!(final_body.contains("needle_final_r2_xyz"), "{final_body}");
    assert!(!final_body.contains("needle_resumed_r2_xyz"), "{final_body}");
}

#[test]
fn fault_concurrent_readers_and_parallel_agree_exactly() {
    // Fault class: CONCURRENT-READ. Two live sessions over one index serve
    // byte-identical capsules, including on interleaved repeats (no
    // session-private drift); a parallel fan-out wave agrees per-call with
    // the serial wave (same ids, same ok bits, same values).
    let (_temp, config) = indexed_repo();
    let args = json!({"query": "needle_unique_r2_xyz", "format": "capsule", "limit": 5});
    let mut first = CodeModeSession::new(config.clone());
    let mut second = CodeModeSession::new(config.clone());
    let r1 = first.call("search", args.clone()).expect("reader 1");
    let r2 = second.call("search", args.clone()).expect("reader 2");
    assert_eq!(r1, r2);
    assert!(!r1["hits"].as_array().expect("hits").is_empty());
    let r1_again = first.call("search", args.clone()).expect("reader 1 again");
    let r2_again = second.call("search", args).expect("reader 2 again");
    assert_eq!(r1_again, r1);
    assert_eq!(r2_again, r1);

    let calls = || {
        vec![
            BatchCall {
                id: "p1".into(),
                tool: "search".into(),
                args: json!({"query": "needle_unique_r2_xyz", "format": "capsule", "limit": 5}),
            },
            BatchCall {
                id: "p2".into(),
                tool: "catalog_search".into(),
                args: json!({"query": "search"}),
            },
        ]
    };
    let serial = run_batch(config.clone(), &batch_request(calls(), ParallelMode::Serial))
        .expect("serial wave");
    let parallel = run_batch(config, &batch_request(calls(), ParallelMode::Parallel))
        .expect("parallel wave");
    assert_eq!(serial.mode, "serial");
    assert_eq!(parallel.mode, "parallel");
    assert_eq!(serial.results.len(), parallel.results.len());
    for (s, p) in serial.results.iter().zip(parallel.results.iter()) {
        assert_eq!(s.id, p.id);
        assert_eq!(s.ok, p.ok);
        assert_eq!(s.value, p.value);
    }
}

#[test]
fn fault_writer_while_reader_no_stale_error() {
    // Fault class: CONCURRENT-WRITE. A reader warms its Searcher cache, then
    // a SECOND session mutates + reindexes mid-flow. The reader's next call
    // must not serve a stale error or stale rows: the writer-generation bump
    // invalidates the warm Searcher, and the reader agrees exactly with a
    // fresh session on both the new and the old token.
    let (_temp, config) = indexed_repo();
    let mut reader = CodeModeSession::new(config.clone());
    let before = reader
        .call(
            "search",
            json!({"query": "needle_unique_r2_xyz", "format": "capsule", "limit": 5}),
        )
        .expect("reader warms cache");
    assert!(!before["hits"].as_array().expect("hits").is_empty());

    // ACTIVE FAULT (from the reader's view): concurrent writer mutates.
    let mut writer = CodeModeSession::new(config.clone());
    let edited = writer
        .call(
            "edit",
            json!({"path": "src/a.rs",
                "oldText": "needle_unique_r2_xyz", "newText": "needle_moved_r2_xyz"}),
        )
        .expect("writer edits");
    assert_eq!(edited["ok"], true);

    let mut fresh = CodeModeSession::new(config);
    let new_args = json!({"query": "needle_moved_r2_xyz", "format": "capsule", "limit": 5});
    let after = reader.call("search", new_args.clone()).expect("reader after write");
    let expected = fresh.call("search", new_args).expect("fresh");
    assert_eq!(after, expected);
    assert!(!after["hits"].as_array().expect("hits").is_empty());

    let old_args = json!({"query": "needle_unique_r2_xyz", "limit": 5});
    let gone = reader.call("search", old_args.clone()).expect("old token via reader");
    let gone_fresh = fresh.call("search", old_args).expect("old token via fresh");
    assert_eq!(gone, gone_fresh);
}

#[test]
fn fault_corrupt_stamp_midflow_failopen_writer_restores() {
    // Fault class: TORN-STAMP. The writer-generation stamp (the session
    // file) is torn mid-flow after a verified search. Reads keep serving
    // (fail-open hint per contract, generation reads 0), and the next writer
    // op restores a valid nonzero stamp; a resumed session then serves the
    // token again. (Pass 1 owns static fail-open; this owns mid-flow tear +
    // writer recovery.)
    let (_temp, config) = indexed_repo();
    let stamp =
        ast_sgrep_core::writer_generation_path(&config.root, config.index_path.as_deref());
    let advertised: u64 = std::fs::read_to_string(&stamp)
        .expect("indexing bumps the stamp")
        .trim()
        .parse()
        .expect("stamp parses");
    assert_ne!(advertised, 0);
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("search", json!({"query": "needle_unique_r2_xyz", "limit": 5}))
        .expect("search before fault");

    // ACTIVE FAULT: torn stamp write mid-flow.
    std::fs::write(&stamp, "not-a-number!!!").expect("corrupt stamp");
    assert_eq!(
        ast_sgrep_core::read_writer_generation(&config.root, config.index_path.as_deref()),
        0,
        "unparsable stamp must read as 0 mid-flow"
    );
    session
        .call("search", json!({"query": "needle_unique_r2_xyz", "limit": 5}))
        .expect("session serves despite torn stamp");

    // Writer recovery: the next index write republishes a valid stamp.
    session
        .call("index_repo", json!({"force": false}))
        .expect("writer restores stamp");
    let restored: u64 = std::fs::read_to_string(&stamp)
        .expect("stamp restored")
        .trim()
        .parse()
        .expect("restored stamp parses");
    assert_ne!(restored, 0);

    let mut resumed = CodeModeSession::new(config);
    let out = resumed
        .call(
            "search",
            json!({"query": "needle_unique_r2_xyz", "format": "capsule", "limit": 5}),
        )
        .expect("resumed search serves");
    assert!(!out["hits"].as_array().expect("hits").is_empty(), "{out}");
}
