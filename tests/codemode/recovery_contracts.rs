//! Recovery CONTRACTS for codemode durable state (consolidated suite).
//!
//! Consolidates `durable_recovery_pass1.rs` (R1 static contracts) and the
//! contract-shaped `durable_recovery_pass2.rs` (R2 fault-injection) tests into
//! intent-grouped tests: each `#[test]` owns ONE intent with multiple fault
//! facets (static + mid-flow arms). Relations live in `recovery_relations.rs`,
//! crash drills in `recovery_drills.rs`. Catalog: `tests/catalog/recovery.md`
//! (tests/codemode rows).
//!
//! Discipline (inherited): `CallError` discriminants via `matches!`, never
//! Display text; filesystem facts as bytes/presence; fixed-content tempdirs;
//! no timing asserts. Testkit supplies config/batch/file/fault builders plus
//! the shared recovery fixtures (`codemode_recovery`); file-local helpers
//! below exist only where testkit offers nothing (each carries a why-comment).

use ast_sgrep_codemode::{parse_plan, run_batch, run_plan, CallError, CodeModeSession};
use ast_sgrep_testkit as testkit;
use serde_json::json;
use std::path::Path;

/// Unique token: lives in exactly one file of the fixed-content repo, so
/// shaped counts (`{"hit_count": 1}`) are hand-computable.
const TOKEN: &str = "needle_unique_rc_xyz";

// WHY file-local: one-file comparator (quarantine-absence is asserted only
// in this file's total-refusal test); no shared shape to promote yet.
fn assert_no_quarantine(root: &Path) {
    let quarantines: Vec<_> = std::fs::read_dir(root)
        .expect("readdir")
        .filter_map(|entry| entry.ok().map(|e| e.file_name()))
        .filter(|name| name.to_string_lossy().contains(".corrupt"))
        .collect();
    assert!(
        quarantines.is_empty(),
        "no silent quarantine: {quarantines:?}"
    );
}

/// INTENT=file records refuse at the right layer (missing->Other/io,
/// garbage/truncated/empty->Json/syntax, mid-flow tears->Json) and valid
/// files resume deterministically with hand-computed values.
/// KILLS=silent-empty mutants, wrong-layer (InvalidArgs/Other-for-corrupt)
/// mutants, durable/live-confusion mutants, load-nondeterminism mutants.
/// ABSORBS=R1 missing_plan_and_batch_files_fail_closed_as_other,
/// R1 corrupt_truncated_and_empty_files_fail_closed_as_json,
/// R1 valid_plan_and_batch_files_resume_deterministically,
/// R2 fault_torn_plan_file_midflow_refused_as_json (refusal facet),
/// R2 fault_truncated_batch_file_midflow_refused_as_json (refusal facet).
#[test]
fn file_records_refuse_bad_layers_and_resume_good_deterministically() {
    // Facet 1 (missing -> Other): absent records are io failures, never
    // silent-empty plans/batches and never a validation discriminant.
    let temp = tempfile::tempdir().expect("tempdir");
    let err = testkit::load_plan_file(&temp.path().join("no-such-plan.json"))
        .expect_err("missing plan must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = testkit::load_batch_file(&temp.path().join("no-such-batch.json"))
        .expect_err("missing batch must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Facet 2 (static corrupt -> Json): garbage, truncation (valid prefix,
    // cut tail), and empty files fail at the syntax layer for BOTH record
    // kinds — distinct from the shape layer (`InvalidArgs`).
    let full_plan = json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ]});
    let full_batch = json!({"calls": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ]});
    for (name, full) in [("plan", full_plan), ("batch", full_batch)] {
        let raw = serde_json::to_vec(&full).expect("serialize");
        let garbage_path = temp.path().join(format!("{name}-garbage.json"));
        testkit::write_garbage(&garbage_path);
        let trunc_path = temp.path().join(format!("{name}-truncated.json"));
        testkit::write_file(&trunc_path, &raw);
        let len = std::fs::metadata(&trunc_path).expect("metadata").len();
        testkit::truncate_file(&trunc_path, len / 2);
        let empty_path = temp.path().join(format!("{name}-empty.json"));
        testkit::write_file(&empty_path, b"");
        for (case, path) in [
            ("garbage", garbage_path),
            ("truncated", trunc_path),
            ("empty", empty_path),
        ] {
            let err = if name == "plan" {
                testkit::load_plan_file(&path).expect_err("must fail")
            } else {
                testkit::load_batch_file(&path)
                    .map(|_| ())
                    .expect_err("must fail")
            };
            assert!(
                matches!(err, CallError::Json(_)),
                "{name}/{case}: expected Json, got {err:?}"
            );
        }
    }

    // Facet 3 (mid-flow tear -> Json, live state isolated): records that
    // loaded and executed cleanly are torn mid-flow; resume is refused at
    // the syntax layer while the live in-memory records still execute,
    // proving the fault hit durable bytes, not live state.
    let (_repo, config) = testkit::indexed_codemode_repo(TOKEN);
    let dir = tempfile::tempdir().expect("records dir");
    let plan_raw = json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": TOKEN, "format": "capsule", "limit": 5}},
        {"id": "narrow", "tool": "filter_hits",
         "args": {"hits": "$seed", "path_contains": "src/a.rs", "limit": 5}},
        {"id": "out", "tool": "select",
         "args": {"value": "$narrow", "fields": ["hit_count"]}},
    ], "return": "$out"});
    let plan_path = dir.path().join("plan.json");
    testkit::write_file(
        &plan_path,
        &serde_json::to_vec(&plan_raw).expect("plan json"),
    );
    let plan = testkit::load_plan_file(&plan_path).expect("plan loads before fault");
    let mut session = CodeModeSession::new(config.clone());
    let first = run_plan(&mut session, &plan).expect("plan runs before fault");
    assert!(first.ok);
    assert_eq!(first.return_value, json!({"hit_count": 1}));
    let len = std::fs::metadata(&plan_path).expect("metadata").len();
    testkit::truncate_file(&plan_path, len / 2);
    let err = testkit::load_plan_file(&plan_path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    let mut resumed = CodeModeSession::new(config.clone());
    let again = run_plan(&mut resumed, &plan).expect("live plan unaffected");
    assert_eq!(again.return_value, first.return_value);

    let batch_raw = testkit::batch_request(vec![
        testkit::catalog_call("w1", "search"),
        testkit::catalog_call("w2", "chain"),
    ]);
    let batch_path = dir.path().join("batch.json");
    testkit::write_file(
        &batch_path,
        &serde_json::to_vec(&batch_raw).expect("batch json"),
    );
    let request = testkit::load_batch_file(&batch_path).expect("batch loads before fault");
    let first = run_batch(config.clone(), &request).expect("batch runs before fault");
    assert!(first.all_ok);
    assert_eq!(first.call_count, 2);
    let len = std::fs::metadata(&batch_path).expect("metadata").len();
    testkit::truncate_file(&batch_path, len / 2);
    let err = testkit::load_batch_file(&batch_path)
        .map(|_| ())
        .expect_err("truncated batch must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    let again = run_batch(config.clone(), &request).expect("live batch unaffected");
    assert!(again.all_ok);
    assert_eq!(again.results.len(), first.results.len());

    // Facet 4 (valid roundtrip): the same bytes load and execute
    // deterministically with hand-computed values (unique token narrowed to
    // its file; catalog ranking facts).
    let plan_path = dir.path().join("plan-roundtrip.json");
    testkit::write_file(
        &plan_path,
        &serde_json::to_vec(&plan_raw).expect("plan json"),
    );
    let plan = testkit::load_plan_file(&plan_path).expect("plan loads");
    assert_eq!(plan.steps.len(), 3);
    for _ in 0..2 {
        let mut session = CodeModeSession::new(config.clone());
        let result = run_plan(&mut session, &plan).expect("plan runs");
        assert!(result.ok);
        assert_eq!(result.call_count, 3);
        assert_eq!(result.return_value, json!({"hit_count": 1}));
    }
    let batch_path = dir.path().join("batch-roundtrip.json");
    testkit::write_file(
        &batch_path,
        &serde_json::to_vec(&batch_raw).expect("batch json"),
    );
    let request = testkit::load_batch_file(&batch_path).expect("batch loads");
    assert_eq!(request.calls.len(), 2);
    let response = run_batch(config, &request).expect("batch runs");
    assert!(response.all_ok);
    assert_eq!(response.call_count, 2);
    assert_eq!(response.results[0].id, "w1");
    assert_eq!(response.results[1].id, "w2");
    assert_eq!(
        response.results[0].value.as_ref().expect("v1")["tools"][0]["name"],
        json!("search")
    );
    assert_eq!(
        response.results[1].value.as_ref().expect("v2")["tools"][0]["name"],
        json!("chain")
    );
}

/// INTENT=absent or unwritable host state fails closed as Other: missing
/// index refuses reads then the writer creates an empty index that serves
/// deterministic zeros; mid-run root deletion refuses warm+fresh sessions on
/// every path; read-only dirs refuse writes cleanly with bytes unchanged.
/// KILLS=serve-without-index/root mutants, panic/partial-write mutants.
/// ABSORBS=R1 absent_index_reads_fail_closed_then_empty_index_serves_deterministic_empty_results,
/// R2 fault_session_dir_deleted_midrun_fails_closed,
/// R2 fault_readonly_session_dir_writes_fail_cleanly_as_other.
#[test]
fn absent_or_unwritable_host_state_fails_closed() {
    // Facet 1 (missing index): root exists but no index was ever built.
    // Bound reads fail closed with `Other`; the writer creates the empty
    // index, which then serves deterministic empty results on repeat.
    let temp = tempfile::tempdir().expect("tempdir");
    let config = testkit::config_at_indexed(temp.path(), &temp.path().join("index.db"));
    assert!(!temp.path().join("index.db").exists());
    let mut session = CodeModeSession::new(config.clone());
    let err = session
        .call("search", json!({"query": "anything", "limit": 5}))
        .expect_err("search without index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call("read", json!({"path": "src/a.rs", "start": 1, "end": 2}))
        .expect_err("read without index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let status = session
        .call("index_status", json!({}))
        .expect("status creates");
    assert_eq!(status["file_count"], json!(0));
    assert!(temp.path().join("index.db").is_file());
    let first = session
        .call("search", json!({"query": "anything", "limit": 5}))
        .expect("empty search ok");
    let second = session
        .call("search", json!({"query": "anything", "limit": 5}))
        .expect("empty search ok twice");
    assert_eq!(first["hits"], json!([]));
    assert_eq!(first, second);

    // Facet 2 (root deleted mid-run): the warm session AND a fresh resume
    // fail closed with `Other` on every root-touching path. The index lives
    // in a separate tempdir so the fault removes the source of truth, not
    // just the index.
    let root_temp = tempfile::tempdir().expect("root tempdir");
    let index_temp = tempfile::tempdir().expect("index tempdir");
    testkit::write_file(
        &root_temp.path().join("src/a.rs"),
        b"pub fn needle_unique_rc_gone() {}\n",
    );
    let config = testkit::config_at_indexed(root_temp.path(), &index_temp.path().join("index.db"));
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("index_repo", json!({"force": false}))
        .expect("index");
    session
        .call(
            "search",
            json!({"query": "needle_unique_rc_gone", "limit": 5}),
        )
        .expect("search before fault");
    std::fs::remove_dir_all(root_temp.path()).expect("delete root");
    assert!(!root_temp.path().exists());
    let err = session
        .call(
            "search",
            json!({"query": "needle_unique_rc_gone", "limit": 5}),
        )
        .expect_err("warm search after root deletion must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let mut resumed = CodeModeSession::new(config.clone());
    let err = resumed
        .call("read", json!({"path": "src/a.rs", "start": 1, "end": 2}))
        .expect_err("read after root deletion must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "search", "args": {"query": "needle_unique_rc_gone", "limit": 5}},
    ]}))
    .expect("parse");
    let err = run_plan(&mut resumed, &plan).expect_err("plan after root deletion must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Facet 3 (read-only dir): the session dir goes read-only mid-run after
    // a verified search. Writers error cleanly with `Other` (never a panic,
    // never a partial write); reads either still serve or fail with the same
    // discriminant; durable bytes are unchanged.
    let (temp, config) = testkit::indexed_codemode_repo(TOKEN);
    let db = config.index_path.clone().expect("index path");
    let src_dir = temp.path().join("src");
    let a_rs = src_dir.join("a.rs");
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect("search before fault");
    let targets = [
        temp.path().to_path_buf(),
        src_dir.clone(),
        a_rs.clone(),
        src_dir.join("b.rs"),
        db.clone(),
    ];
    let chmod = |readonly: bool| {
        for path in &targets {
            let mut perms = std::fs::metadata(path).expect("metadata").permissions();
            perms.set_readonly(readonly);
            std::fs::set_permissions(path, perms).expect("chmod");
        }
    };
    chmod(true);
    // Prove the fault bites in this environment; privileged uids bypass
    // permission bits, in which case there is no fault to test.
    let probe = src_dir.join(".rc-probe");
    if std::fs::write(&probe, b"probe").is_ok() {
        let _ = std::fs::remove_file(&probe);
        chmod(false);
        return;
    }
    // Capture results while read-only, restore before asserting so tempdir
    // cleanup can never strand (asserts below cannot leak the fixture).
    let edit_result = session.call(
        "edit",
        json!({"path": "src/a.rs", "oldText": TOKEN, "newText": "needle_rc_ro"}),
    );
    let index_result = session.call("index_repo", json!({"force": false}));
    let search_result = session.call("search", json!({"query": TOKEN, "limit": 5}));
    chmod(false);
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
        body.contains(TOKEN),
        "failed writes must leave durable bytes unchanged: {body}"
    );
}

/// INTENT=dead index bytes (static garbage, static truncate, mid-flow
/// truncate) refuse totally as Other on read, write, and plan paths with no
/// silent quarantine and no silent rebuild.
/// KILLS=silent-rebuild/quarantine mutants, serve-dead-index mutants.
/// ABSORBS=R1 corrupt_and_truncated_index_files_fail_closed_without_quarantine,
/// R2 fault_truncated_index_midflow_refused_loudly_without_quarantine.
#[test]
fn dead_index_bytes_refuse_totally_without_quarantine() {
    // Facet 1 (static garbage): read AND write paths refuse with `Other`;
    // no `.corrupt` quarantine, no silent rebuild — codemode never sets
    // force_reindex, so refusal is total until the operator intervenes.
    let (temp, config) = testkit::indexed_codemode_repo(TOKEN);
    let db = config.index_path.clone().expect("index path");
    testkit::write_garbage(&db);
    let mut session = CodeModeSession::new(config.clone());
    let err = session
        .call("index_status", json!({}))
        .expect_err("status on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_no_quarantine(temp.path());

    // Facet 2 (static truncate): header intact, body cut mid-file.
    let (_temp2, config2) = testkit::indexed_codemode_repo(TOKEN);
    let db2 = config2.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db2).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    testkit::truncate_file(&db2, 512);
    let mut session2 = CodeModeSession::new(config2);
    let err = session2
        .call("index_status", json!({}))
        .expect_err("status on truncated must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session2
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on truncated must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Facet 3 (mid-flow truncate): a serving index is cut AFTER a verified
    // search; host resume refuses on read, write, AND plan paths alike,
    // still with no silent quarantine.
    let (temp3, config3) = testkit::indexed_codemode_repo(TOKEN);
    let db3 = config3.index_path.clone().expect("index path");
    let mut session3 = CodeModeSession::new(config3.clone());
    let before = session3
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect("search before fault");
    assert!(!before["hits"].as_array().expect("hits").is_empty());
    let bytes = std::fs::read(&db3).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    testkit::truncate_file(&db3, 512);
    let mut resumed = CodeModeSession::new(config3.clone());
    let err = resumed
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = resumed
        .call("index_status", json!({}))
        .expect_err("status on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let plan = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "search", "args": {"query": TOKEN, "limit": 5}},
    ]}))
    .expect("parse");
    let err = run_plan(&mut resumed, &plan).expect_err("plan on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_no_quarantine(temp3.path());
}

/// INTENT=schema versions gate loudly: future stamps refuse nonmutating on
/// both paths (peek survives); stale stamps refuse reads, the writer migrates
/// in place, and reads resume with served output preserved byte-identically.
/// KILLS=migrate-down/serve-future mutants, serve-stale mutants,
/// lossy-migration mutants.
/// ABSORBS=R1 future_schema_version_is_refused_loudly_and_nonmutating,
/// R1 stale_schema_version_readers_refuse_writers_migrate_then_reads_resume,
/// R3 mr_stale_schema_migration_preserves_search_output_exactly.
#[test]
fn schema_versions_refuse_future_and_migrate_stale_preserving_output() {
    // Tripwire: stale surgery below is relative to the live stamp, but the
    // suite pins the known-good version so a core bump fails loudly.
    assert_eq!(ast_sgrep_core::INDEX_SCHEMA_VERSION, 16);

    // Facet 1 (future): a newer writer's index (user_version 9999) is
    // refused with `Other` on both paths, nonmutating: the stamp still
    // reads 9999 afterwards and peek survives, so a newer binary can open it.
    let (_temp1, config) = testkit::indexed_codemode_repo(TOKEN);
    let db = config.index_path.clone().expect("index path");
    assert_eq!(testkit::db_user_version(&db), 16);
    testkit::set_db_user_version(&db, 9999);
    let mut session = CodeModeSession::new(config.clone());
    let err = session
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("future schema search must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call("index_status", json!({}))
        .expect_err("future schema status must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(
        testkit::db_user_version(&db),
        9999,
        "refusal must not migrate"
    );
    let peeked =
        ast_sgrep_core::IndexStore::peek_schema_version(&config.root, config.index_path.as_deref())
            .expect("peek survives refusal");
    assert_eq!(peeked, 9999);

    // Facet 2 (stale migrate + resume): a one-behind stamp refuses reads
    // with `Other` rather than serving pre-migration rows; the writer
    // migrates in place; reads resume on the migrated rows.
    let (_temp2, config) = testkit::indexed_codemode_repo(TOKEN);
    let db = config.index_path.clone().expect("index path");
    let current = testkit::db_user_version(&db);
    testkit::set_db_user_version(&db, current - 1);
    let mut session = CodeModeSession::new(config.clone());
    let err = session
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("stale schema search must refuse");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    session
        .call("index_status", json!({}))
        .expect("writer migrates");
    assert_eq!(
        testkit::db_user_version(&db),
        current,
        "writer must stamp current"
    );
    let mut resumed = CodeModeSession::new(config);
    let out = resumed
        .call(
            "search",
            json!({"query": TOKEN, "format": "capsule", "limit": 5}),
        )
        .expect("post-migration search resumes");
    let hits = out["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "migrated rows must serve: {out}");
    assert!(
        hits.iter()
            .any(|h| h["file"].as_str().unwrap_or("").contains("src/a.rs")),
        "{out}"
    );

    // Facet 3 (migration parity): capsule search and read outputs captured
    // before version surgery are byte-identical after refuse -> migrate ->
    // resume. The warm session is dropped before surgery: its open read
    // connection would pin the WAL and keep the migrated header out of the
    // raw file bytes.
    let (_temp3, config) = testkit::indexed_codemode_repo(TOKEN);
    let mut baseline = CodeModeSession::new(config.clone());
    let search_before = baseline
        .call(
            "search",
            json!({"query": TOKEN, "format": "capsule", "limit": 5}),
        )
        .expect("capsule search");
    let read_before = baseline
        .call("read", json!({"path": "src/a.rs", "start": 1, "end": 2}))
        .expect("read window");
    drop(baseline);
    let db = config.index_path.clone().expect("index path");
    let current = testkit::db_user_version(&db);
    testkit::set_db_user_version(&db, current - 1);
    let mut stale = CodeModeSession::new(config.clone());
    let err = stale
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("stale schema search must refuse");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    stale
        .call("index_status", json!({}))
        .expect("writer migrates");
    assert_eq!(
        testkit::db_user_version(&db),
        current,
        "writer must stamp current"
    );
    let mut resumed = CodeModeSession::new(config);
    assert_eq!(
        resumed
            .call(
                "search",
                json!({"query": TOKEN, "format": "capsule", "limit": 5}),
            )
            .expect("capsule search"),
        search_before,
        "post-migration search must equal pre-fault output"
    );
    assert_eq!(
        resumed
            .call("read", json!({"path": "src/a.rs", "start": 1, "end": 2}))
            .expect("read window"),
        read_before,
        "post-migration read must equal pre-fault output"
    );
}

/// INTENT=the writer-generation stamp is a fail-open staleness hint, not a
/// gate: static corrupt/missing and mid-flow tears read as generation 0 and
/// keep serving, and the next writer republishes a valid nonzero stamp.
/// KILLS=stamp-gating mutants, stamp-poison mutants.
/// ABSORBS=R1 corrupt_writer_generation_stamp_fails_open_per_contract,
/// R2 fault_corrupt_stamp_midflow_failopen_writer_restores.
#[test]
fn writer_generation_stamp_is_failopen_hint_restored_by_writer() {
    // Facet 1 (static fail-open): missing or unparsable content reads as
    // generation 0 (documented cold-start protocol) and the session keeps
    // serving instead of failing closed.
    let (_temp1, config) = testkit::indexed_codemode_repo(TOKEN);
    let stamp = ast_sgrep_core::writer_generation_path(&config.root, config.index_path.as_deref());
    // Indexing is a write, so it advertises a generation; the true positive
    // is that a valid stamp parses to its file content.
    let advertised: u64 = std::fs::read_to_string(&stamp)
        .expect("indexing bumps the stamp")
        .trim()
        .parse()
        .expect("stamp parses");
    assert_eq!(
        ast_sgrep_core::read_writer_generation(&config.root, config.index_path.as_deref()),
        advertised
    );
    std::fs::write(&stamp, "not-a-number!!!").expect("corrupt stamp");
    assert_eq!(
        ast_sgrep_core::read_writer_generation(&config.root, config.index_path.as_deref()),
        0,
        "unparsable stamp must read as 0, not error"
    );
    let mut session = CodeModeSession::new(config.clone());
    let out = session
        .call(
            "search",
            json!({"query": TOKEN, "format": "capsule", "limit": 5}),
        )
        .expect("session serves despite corrupt stamp");
    assert!(!out["hits"].as_array().expect("hits").is_empty(), "{out}");
    std::fs::remove_file(&stamp).expect("remove stamp");
    let out = session
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect("session serves with stamp absent");
    assert!(!out["hits"].as_array().expect("hits").is_empty(), "{out}");

    // Facet 2 (mid-flow tear + writer restore): the stamp torn after a
    // verified search still reads 0 and serves; the next writer op
    // republishes a valid nonzero stamp; a resumed session serves again.
    let (_temp2, config) = testkit::indexed_codemode_repo(TOKEN);
    let stamp = ast_sgrep_core::writer_generation_path(&config.root, config.index_path.as_deref());
    let advertised: u64 = std::fs::read_to_string(&stamp)
        .expect("indexing bumps the stamp")
        .trim()
        .parse()
        .expect("stamp parses");
    assert_ne!(advertised, 0);
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect("search before fault");
    std::fs::write(&stamp, "not-a-number!!!").expect("corrupt stamp");
    assert_eq!(
        ast_sgrep_core::read_writer_generation(&config.root, config.index_path.as_deref()),
        0,
        "unparsable stamp must read as 0 mid-flow"
    );
    session
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect("session serves despite torn stamp");
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
            json!({"query": TOKEN, "format": "capsule", "limit": 5}),
        )
        .expect("resumed search serves");
    assert!(!out["hits"].as_array().expect("hits").is_empty(), "{out}");
}
