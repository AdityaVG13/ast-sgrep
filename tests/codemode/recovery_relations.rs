//! Recovery RELATIONS for codemode durable state (consolidated suite).
//!
//! Consolidates `durable_recovery_pass3.rs` (R3 metamorphic relations) and the
//! relation-shaped `durable_recovery_pass2.rs` (R2 fault-injection) tests into
//! intent-grouped tests: each `#[test]` owns ONE intent and asserts it as an
//! equality between runs related by a fault/repair/reopen transform. Single-
//! run contracts live in `recovery_contracts.rs`, crash drills in
//! `recovery_drills.rs`. Catalog: `tests/catalog/recovery.md` (codemode rows).
//!
//! Discipline (inherited): `CallError` discriminants via `matches!`, never
//! Display text; batch/serve errors by presence, never message content;
//! fixed-content tempdirs; `wall_ms` never compared. Testkit supplies
//! config/batch/file/fault/serve builders plus the shared recovery fixtures
//! (`codemode_recovery`); file-local helpers below exist only where testkit
//! offers nothing (each carries a why-comment).

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, BatchCall, CallError, CodeModeSession, ParallelMode,
    ServeResponse,
};
use ast_sgrep_testkit as testkit;
use serde_json::{json, Value};

/// Unique token: lives in exactly one file of the fixed-content repo, so
/// shaped counts (`{"hit_count": 1}`) are hand-computable.
const TOKEN: &str = "needle_unique_rr_xyz";

// WHY file-local: TOKEN-bound search/read probes stay suite-local; the
// root-independent `status_counts` projection lives in testkit.
fn search_capsule(session: &mut CodeModeSession) -> Value {
    session
        .call(
            "search",
            json!({"query": TOKEN, "format": "capsule", "limit": 5}),
        )
        .expect("capsule search")
}

fn read_window(session: &mut CodeModeSession) -> Value {
    session
        .call("read", json!({"path": "src/a.rs", "start": 1, "end": 2}))
        .expect("read window")
}

// WHY file-local: relations-only record shapes (the recovering 3-step plan
// and the index-free catalog batch); token-bound, used within this file.
fn recovering_plan_json() -> Value {
    json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": TOKEN, "format": "capsule", "limit": 5}},
        {"id": "narrow", "tool": "filter_hits",
         "args": {"hits": "$seed", "path_contains": "src/a.rs", "limit": 5}},
        {"id": "out", "tool": "select",
         "args": {"value": "$narrow", "fields": ["hit_count"]}},
    ], "return": "$out"})
}

fn catalog_batch_calls() -> Vec<BatchCall> {
    vec![
        testkit::catalog_call("w1", "search"),
        testkit::catalog_call("w2", "chain"),
    ]
}

/// INTENT=interrupted-batch semantics: a failing middle call commits nothing
/// while the wave continues with siblings intact; a prefix commits
/// (documented cross-call non-atomicity) and resumes from the failed id; the
/// interrupted+resumed twin equals the uninterrupted twin on bytes and
/// per-call values.
/// KILLS=partial-commit/abort-wave mutants, rollback-prefix mutants,
/// resume-divergence mutants.
/// ABSORBS=R2 fault_interrupted_batch_failed_call_atomic_siblings_preserved,
/// R2 fault_interrupted_batch_commits_prefix_resumes_from_failed_id,
/// R3 mr_interrupted_batch_resume_equals_uninterrupted_wave.
#[test]
fn interrupted_batch_atomicity_prefix_and_resume_equals_uninterrupted() {
    // Facet 1 (intra-call atomicity): a batch wave interrupted by a failing
    // middle call (a multi-edit whose later edit can never match). The
    // failed call commits NOTHING, the wave does not abort (siblings keep
    // ids/order/values), and the error is present as data — the resume point.
    let (_temp1, config) = testkit::indexed_codemode_repo(TOKEN);
    let a_rs = config.root.join("src/a.rs");
    let before = std::fs::read_to_string(&a_rs).expect("read before");
    let request = testkit::batch_request_with_mode(vec![
        testkit::batch_call("w1", "search", json!({"query": TOKEN, "limit": 5})),
        testkit::batch_call(
            "w2",
            "edit",
            json!({"edits": [
                {"path": "src/a.rs",
                 "oldText": TOKEN, "newText": "needle_mutated_rr_xyz"},
                {"path": "src/a.rs",
                 "oldText": "not-present-anywhere", "newText": "x"},
            ]}),
        ),
        testkit::batch_call("w3", "search", json!({"query": TOKEN, "limit": 5})),
    ], ParallelMode::Serial);
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
    assert!(!after.contains("needle_mutated_rr_xyz"), "{after}");

    // Facet 2 (prefix commits, resume converges): `[edit-ok, edit-bad]`
    // commits its prefix (documented, not silent) and the host resumes by
    // re-running ONLY the failed id with corrected args: the resume wave is
    // all_ok and the final bytes show both mutations in order.
    let (_temp2, config) = testkit::indexed_codemode_repo(TOKEN);
    let a_rs = config.root.join("src/a.rs");
    let request = testkit::batch_request_with_mode(vec![
        testkit::batch_call(
            "step-1",
            "edit",
            json!({"path": "src/a.rs",
                "oldText": TOKEN, "newText": "needle_mid_rr_xyz"}),
        ),
        testkit::batch_call(
            "step-2",
            "edit",
            json!({"path": "src/a.rs",
                "oldText": "not-present-anywhere", "newText": "x"}),
        ),
    ], ParallelMode::Serial);
    let response = run_batch(config.clone(), &request).expect("batch runs");
    assert!(!response.all_ok);
    assert!(response.results[0].ok);
    assert!(!response.results[1].ok);
    assert_eq!(response.results[1].id, "step-2");
    // Committed prefix visible on disk: no cross-call rollback (documented).
    let mid = std::fs::read_to_string(&a_rs).expect("reread mid");
    assert!(mid.contains("needle_mid_rr_xyz"), "{mid}");
    let resume = testkit::batch_request_with_mode(vec![testkit::batch_call(
        "step-2",
        "edit",
        json!({"path": "src/a.rs",
            "oldText": "needle_mid_rr_xyz", "newText": "needle_final_rr_xyz"}),
    )], ParallelMode::Serial);
    let resumed = run_batch(config, &resume).expect("resume runs");
    assert!(resumed.all_ok);
    assert_eq!(resumed.results.len(), 1);
    assert_eq!(resumed.results[0].id, "step-2");
    assert!(resumed.results[0].ok);
    let final_body = std::fs::read_to_string(&a_rs).expect("reread final");
    assert!(final_body.contains("needle_final_rr_xyz"), "{final_body}");
    assert!(!final_body.contains("needle_mid_rr_xyz"), "{final_body}");

    // Facet 3 (resume == uninterrupted): twin repos start identical. Twin A
    // runs [edit-ok, edit-bad] (interrupted), then resumes with the
    // corrected step-2 alone. Twin B runs the corrected wave uninterrupted.
    // Final bytes agree, the committed prefix values agree, and the resumed
    // step-2 value equals the uninterrupted step-2 value.
    let (_a_temp, a_config, _b_temp, b_config) = testkit::twin_repos(TOKEN);
    let a_rs = a_config.root.join("src/a.rs");
    let b_rs = b_config.root.join("src/a.rs");
    let step_1 = testkit::batch_call(
        "step-1",
        "edit",
        json!({"path": "src/a.rs",
            "oldText": TOKEN, "newText": "needle_mid_rr_xyz"}),
    );
    let step_2_bad = testkit::batch_call(
        "step-2",
        "edit",
        json!({"path": "src/a.rs",
            "oldText": "not-present-anywhere", "newText": "x"}),
    );
    let step_2_fixed = testkit::batch_call(
        "step-2",
        "edit",
        json!({"path": "src/a.rs",
            "oldText": "needle_mid_rr_xyz", "newText": "needle_final_rr_xyz"}),
    );
    let interrupted = run_batch(
        a_config.clone(),
        &testkit::batch_request_with_mode(vec![step_1.clone(), step_2_bad], ParallelMode::Serial),
    )
    .expect("interrupted wave runs");
    assert!(!interrupted.all_ok);
    assert!(interrupted.results[0].ok);
    assert!(!interrupted.results[1].ok);
    assert!(interrupted.results[1].value.is_none());
    assert!(interrupted.results[1].error.is_some());
    let resumed = run_batch(a_config, &testkit::batch_request_with_mode(vec![step_2_fixed.clone()], ParallelMode::Serial)).expect("resume runs");
    assert!(resumed.all_ok);
    assert_eq!(resumed.results.len(), 1);
    assert_eq!(resumed.results[0].id, "step-2");
    assert!(resumed.results[0].ok);
    let uninterrupted = run_batch(b_config, &testkit::batch_request_with_mode(vec![step_1, step_2_fixed], ParallelMode::Serial))
        .expect("uninterrupted wave runs");
    assert!(uninterrupted.all_ok);
    assert_eq!(uninterrupted.results.len(), 2);
    let a_final = std::fs::read_to_string(&a_rs).expect("reread a");
    let b_final = std::fs::read_to_string(&b_rs).expect("reread b");
    assert!(a_final.contains("needle_final_rr_xyz"), "{a_final}");
    assert_eq!(a_final, b_final, "resumed bytes must equal uninterrupted bytes");
    assert_eq!(interrupted.results[0].value, uninterrupted.results[0].value);
    assert_eq!(resumed.results[0].value, uninterrupted.results[1].value);
}

/// INTENT=session and mode agreement: concurrent live sessions serve
/// identical capsules; a warm reader sees a concurrent writer's mutation via
/// generation-bump invalidation; serial/parallel waves agree per-call before
/// and after an index rebuild with values preserved.
/// KILLS=session-drift/mode-divergence mutants, stale-cache mutants.
/// ABSORBS=R2 fault_concurrent_readers_and_parallel_agree_exactly,
/// R2 fault_writer_while_reader_no_stale_error,
/// R3 mr_serial_parallel_agreement_survives_index_rebuild.
#[test]
fn sessions_modes_and_writer_invalidation_agree_exactly() {
    // Facet 1 (concurrent readers): two live sessions over one index serve
    // byte-identical capsules, including on interleaved repeats (no
    // session-private drift).
    let (_temp1, config) = testkit::indexed_codemode_repo(TOKEN);
    let args = json!({"query": TOKEN, "format": "capsule", "limit": 5});
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

    // Facet 2 (writer invalidation): a reader warms its Searcher cache, then
    // a SECOND session mutates mid-flow. The reader's next call must not
    // serve a stale error or stale rows: the writer-generation bump
    // invalidates the warm Searcher, and the reader agrees exactly with a
    // fresh session on both the new and the old token.
    let (_temp2, config) = testkit::indexed_codemode_repo(TOKEN);
    let mut reader = CodeModeSession::new(config.clone());
    let before = reader
        .call(
            "search",
            json!({"query": TOKEN, "format": "capsule", "limit": 5}),
        )
        .expect("reader warms cache");
    assert!(!before["hits"].as_array().expect("hits").is_empty());
    let mut writer = CodeModeSession::new(config.clone());
    let edited = writer
        .call(
            "edit",
            json!({"path": "src/a.rs",
                "oldText": TOKEN, "newText": "needle_moved_rr_xyz"}),
        )
        .expect("writer edits");
    assert_eq!(edited["ok"], true);
    let mut fresh = CodeModeSession::new(config);
    let new_args = json!({"query": "needle_moved_rr_xyz", "format": "capsule", "limit": 5});
    let after = reader.call("search", new_args.clone()).expect("reader after write");
    let expected = fresh.call("search", new_args).expect("fresh");
    assert_eq!(after, expected);
    assert!(!after["hits"].as_array().expect("hits").is_empty());
    let old_args = json!({"query": TOKEN, "limit": 5});
    let gone = reader.call("search", old_args.clone()).expect("old token via reader");
    let gone_fresh = fresh.call("search", old_args).expect("old token via fresh");
    assert_eq!(gone, gone_fresh);

    // Facet 3 (mode equivalence across rebuild): serial/parallel per-call
    // agreement holds before the fault, holds again after corrupt -> remove
    // -> rebuild, and post-rebuild serial answers equal pre-fault answers.
    // Recovery preserves both the mode relation and the served values.
    let (_temp3, config) = testkit::indexed_codemode_repo(TOKEN);
    let calls = || {
        vec![
            testkit::batch_call(
                "p1",
                "search",
                json!({"query": TOKEN, "format": "capsule", "limit": 5}),
            ),
            testkit::catalog_call("p2", "search"),
        ]
    };
    // WHY inline closure: per-call id/ok/value projection (`wall_ms`
    // excluded by design) used only by this facet's four comparisons.
    let answers = |response: &ast_sgrep_codemode::BatchResponse| -> Value {
        let triples: Vec<Value> = response
            .results
            .iter()
            .map(|r| json!({"id": r.id, "ok": r.ok, "value": r.value}))
            .collect();
        serde_json::to_value(&triples).expect("serialize triples")
    };
    let serial_before =
        run_batch(config.clone(), &testkit::batch_request_with_mode(calls(), ParallelMode::Serial)).expect("serial before");
    let parallel_before =
        run_batch(config.clone(), &testkit::batch_request_with_mode(calls(), ParallelMode::Parallel)).expect("parallel before");
    assert_eq!(serial_before.mode, "serial");
    assert_eq!(parallel_before.mode, "parallel");
    assert_eq!(answers(&parallel_before), answers(&serial_before));
    let db = config.index_path.clone().expect("index path");
    testkit::write_garbage(&db);
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    testkit::remove_db_with_sidecars(&db);
    let mut repaired = CodeModeSession::new(config.clone());
    repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    let serial_after =
        run_batch(config.clone(), &testkit::batch_request_with_mode(calls(), ParallelMode::Serial)).expect("serial after");
    let parallel_after = run_batch(config, &testkit::batch_request_with_mode(calls(), ParallelMode::Parallel)).expect("parallel after");
    assert_eq!(answers(&parallel_after), answers(&serial_after));
    assert_eq!(
        answers(&serial_after),
        answers(&serial_before),
        "recovery must preserve served values"
    );
}

/// INTENT=plan re-execution and session reopen are stable: the same plan
/// runs identically on a warm session (exact call-count doubling) and across
/// fresh sessions (fully identical results); drop+reopen preserves served
/// state exactly with only the call budget reset.
/// KILLS=warm-state-rot mutants, session-nondeterminism mutants,
/// reopen-drift mutants.
/// ABSORBS=R3 mr_plan_reexecution_on_warm_session_is_identical,
/// R3 mr_plan_reexecution_across_fresh_sessions_is_identical,
/// R3 mr_session_reopen_preserves_served_state_exactly.
#[test]
fn plan_reexecution_and_reopen_preserve_state_exactly() {
    let (_temp, config) = testkit::indexed_codemode_repo(TOKEN);
    let plan = parse_plan(&recovering_plan_json()).expect("parse plan");

    // Facet 1 (warm re-execution): the same plan run twice on one warm
    // session yields identical step outputs and return value; the only
    // difference is the exact call-count doubling (3 steps + 3 steps).
    let mut session = CodeModeSession::new(config.clone());
    let first = run_plan(&mut session, &plan).expect("first run");
    let second = run_plan(&mut session, &plan).expect("second run");
    assert!(first.ok);
    assert!(second.ok);
    assert_eq!(first.return_value, json!({"hit_count": 1}));
    assert_eq!(second.return_value, first.return_value);
    assert_eq!(second.steps, first.steps);
    assert_eq!(first.call_count, 3);
    assert_eq!(second.call_count, 2 * first.call_count);
    assert_eq!(session.call_count(), 6);

    // Facet 2 (fresh sessions): the same plan on two independently opened
    // sessions yields fully identical PlanResults, including the per-session
    // call count (fresh budgets agree exactly).
    let mut first_session = CodeModeSession::new(config.clone());
    let mut second_session = CodeModeSession::new(config.clone());
    let first = run_plan(&mut first_session, &plan).expect("first run");
    let second = run_plan(&mut second_session, &plan).expect("second run");
    let first_json = serde_json::to_value(&first).expect("serialize first");
    let second_json = serde_json::to_value(&second).expect("serialize second");
    assert_eq!(second_json, first_json);
    assert_eq!(first.call_count, second.call_count);

    // Facet 3 (reopen): dropping a warm session and reopening the same
    // config preserves served state exactly — search, read, and
    // status-count outputs are identical, source bytes are untouched, and
    // the schema stamp is unchanged. The only prescribed difference is the
    // fresh call budget (0 before replay, N after).
    let mut warm = CodeModeSession::new(config.clone());
    let search_warm = search_capsule(&mut warm);
    let read_warm = read_window(&mut warm);
    let status_warm = warm.call("index_status", json!({})).expect("status warm");
    let warm_calls = warm.call_count();
    assert_eq!(warm_calls, 3);
    let src_before = std::fs::read(config.root.join("src/a.rs")).expect("read src");
    let db = config.index_path.clone().expect("index path");
    let stamp_before = testkit::db_user_version(&db);
    drop(warm);
    let mut reopened = CodeModeSession::new(config.clone());
    assert_eq!(reopened.call_count(), 0, "reopen resets the call budget");
    assert_eq!(search_capsule(&mut reopened), search_warm);
    assert_eq!(read_window(&mut reopened), read_warm);
    let status_reopened = reopened
        .call("index_status", json!({}))
        .expect("status reopened");
    assert_eq!(testkit::status_counts(&status_reopened), testkit::status_counts(&status_warm));
    assert_eq!(reopened.call_count(), warm_calls);
    let src_after = std::fs::read(config.root.join("src/a.rs")).expect("reread src");
    assert_eq!(src_after, src_before);
    assert_eq!(testkit::db_user_version(&db), stamp_before);
}

/// INTENT=corrupt -> remove -> rebuild loses nothing: pre-corruption
/// search/read/status outputs are identical after rebuild (deterministic on
/// repeat), and a truncated-then-rebuilt twin serves identically to a
/// never-faulted twin.
/// KILLS=lossy-repair mutants, recovery-residue mutants, rebuild-failure
/// mutants.
/// ABSORBS=R3 mr_corrupt_remove_rebuild_restores_identical_outputs,
/// R1 index_repo_rebuilds_after_corrupt_index_is_removed,
/// R3 mr_recovered_repo_matches_never_faulted_twin.
#[test]
fn corrupt_remove_rebuild_restores_identical_outputs_and_matches_pristine_twin() {
    // Facet 1 (roundtrip): search, read, and status-count outputs captured
    // before corruption are byte-identical after operator removal plus
    // `index_repo` rebuild. The mid-transform refusal (`Other`) proves the
    // fault bit; the relation proves the repair lost nothing.
    let (_temp, config) = testkit::indexed_codemode_repo(TOKEN);
    let mut baseline_session = CodeModeSession::new(config.clone());
    let search_before = search_capsule(&mut baseline_session);
    let read_before = read_window(&mut baseline_session);
    let status_before = baseline_session
        .call("index_status", json!({}))
        .expect("status before");
    let db = config.index_path.clone().expect("index path");
    testkit::write_garbage(&db);
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    testkit::remove_db_with_sidecars(&db);
    let mut repaired = CodeModeSession::new(config);
    let rebuilt = repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    assert_eq!(rebuilt["ok"], json!(true));
    assert_eq!(search_capsule(&mut repaired), search_before);
    assert_eq!(read_window(&mut repaired), read_before);
    let status_after = repaired
        .call("index_status", json!({}))
        .expect("status after");
    assert_eq!(testkit::status_counts(&status_after), testkit::status_counts(&status_before));
    // Deterministic repeat on the rebuilt index.
    assert_eq!(search_capsule(&mut repaired), search_before);

    // Facet 2 (pristine twin): twin A is faulted (truncated index, refused
    // as `Other`), then repaired by operator removal plus rebuild. Twin B is
    // never faulted. The recovered twin serves identically to the pristine
    // twin across search, read, and status counts.
    let (_a_temp, a_config, _b_temp, b_config) = testkit::twin_repos(TOKEN);
    let db = a_config.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    testkit::truncate_file(&db, 512);
    let mut faulted = CodeModeSession::new(a_config.clone());
    let err = faulted
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    testkit::remove_db_with_sidecars(&db);
    let mut recovered = CodeModeSession::new(a_config);
    recovered
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    let mut pristine = CodeModeSession::new(b_config);
    assert_eq!(
        search_capsule(&mut recovered),
        search_capsule(&mut pristine),
        "recovered search must match never-faulted twin"
    );
    assert_eq!(
        read_window(&mut recovered),
        read_window(&mut pristine),
        "recovered read must match never-faulted twin"
    );
    let recovered_status = recovered
        .call("index_status", json!({}))
        .expect("recovered status");
    let pristine_status = pristine
        .call("index_status", json!({}))
        .expect("pristine status");
    assert_eq!(
        testkit::status_counts(&recovered_status),
        testkit::status_counts(&pristine_status)
    );
}

/// INTENT=torn record repair restores identical results: a torn plan or
/// truncated batch refused as Json, repaired by rewriting the original bytes,
/// reruns to the identical full PlanResult / batch response.
/// KILLS=lossy-repair mutants.
/// ABSORBS=R3 mr_torn_plan_file_repair_restores_identical_result,
/// R3 mr_truncated_batch_file_repair_restores_identical_response
/// (refusal facets also covered in recovery_contracts file-records test).
#[test]
fn torn_record_repair_restores_identical_results() {
    let (_temp, config) = testkit::indexed_codemode_repo(TOKEN);
    let dir = tempfile::tempdir().expect("records dir");

    // Facet 1 (plan): a plan file that ran cleanly is torn (resume refused
    // as `Json`), then repaired by rewriting the original bytes. The
    // post-repair run equals the pre-fault run as a full PlanResult:
    // run(repair(corrupt(f))) == run(f).
    let raw = recovering_plan_json();
    let path = dir.path().join("plan.json");
    testkit::write_file(&path, &serde_json::to_vec(&raw).expect("plan json"));
    let plan = testkit::load_plan_file(&path).expect("plan loads before fault");
    let mut session = CodeModeSession::new(config.clone());
    let before = run_plan(&mut session, &plan).expect("plan runs before fault");
    assert!(before.ok);
    let len = std::fs::metadata(&path).expect("metadata").len();
    testkit::truncate_file(&path, len / 2);
    let err = testkit::load_plan_file(&path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    testkit::write_file(&path, &serde_json::to_vec(&raw).expect("plan json"));
    let repaired = testkit::load_plan_file(&path).expect("repaired plan loads");
    assert_eq!(repaired.steps.len(), plan.steps.len());
    let mut resumed = CodeModeSession::new(config.clone());
    let after = run_plan(&mut resumed, &repaired).expect("repaired plan runs");
    let before_json = serde_json::to_value(&before).expect("serialize before");
    let after_json = serde_json::to_value(&after).expect("serialize after");
    assert_eq!(after_json, before_json);

    // Facet 2 (batch): the batch analogue. Post-repair response equals the
    // pre-fault response on every deterministic field (`wall_ms` excluded by
    // design).
    let request = testkit::batch_request_with_mode(catalog_batch_calls(), ParallelMode::Serial);
    let path = dir.path().join("batch.json");
    testkit::write_file(&path, &serde_json::to_vec(&request).expect("batch json"));
    let loaded = testkit::load_batch_file(&path).expect("batch loads before fault");
    let before = run_batch(config.clone(), &loaded).expect("batch runs before fault");
    assert!(before.all_ok);
    let len = std::fs::metadata(&path).expect("metadata").len();
    testkit::truncate_file(&path, len / 2);
    let err = testkit::load_batch_file(&path)
        .map(|_| ())
        .expect_err("truncated batch must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    testkit::write_file(&path, &serde_json::to_vec(&request).expect("batch json"));
    let repaired = testkit::load_batch_file(&path).expect("repaired batch loads");
    let after = run_batch(config, &repaired).expect("repaired batch runs");
    assert_eq!(after.all_ok, before.all_ok);
    assert_eq!(after.call_count, before.call_count);
    assert_eq!(after.mode, before.mode);
    let before_results = serde_json::to_value(&before.results).expect("serialize before");
    let after_results = serde_json::to_value(&after.results).expect("serialize after");
    assert_eq!(after_results, before_results);
}

/// INTENT=a serve stream with corrupt lines interleaved answers its valid
/// calls identically to the clean stream: garbage and truncated-JSON lines
/// fail as id-less per-record Errors, surrounding calls are answered, and
/// both runs end with Bye.
/// KILLS=stream-abort mutants, perturbation mutants.
/// ABSORBS=R1 serve_stream_recovers_after_corrupt_records,
/// R3 mr_serve_valid_answers_unaffected_by_corrupt_lines.
#[test]
fn serve_stream_answers_valid_calls_identically_despite_corrupt_lines() {
    let temp = tempfile::tempdir().expect("tempdir");
    let c1 = json!({
        "type": "call", "id": "c1",
        "tool": "catalog_search", "args": {"query": "search"},
    });
    let c2 = json!({
        "type": "call", "id": "c2",
        "tool": "catalog_search", "args": {"query": "chain"},
    });
    let end = json!({"type": "end"});
    let mut corrupt_input = String::new();
    corrupt_input.push_str(&serde_json::to_string(&c1).expect("c1"));
    corrupt_input.push('\n');
    corrupt_input.push_str("{{{not json at all\x00\x01\n");
    corrupt_input.push_str("{\"type\":\"call\",\"id\":\"t\",\"tool\":\"select\",\"args\":\n");
    corrupt_input.push_str(&serde_json::to_string(&c2).expect("c2"));
    corrupt_input.push('\n');
    corrupt_input.push_str(&serde_json::to_string(&end).expect("end"));
    corrupt_input.push('\n');
    let mut clean_input = String::new();
    for line in [&c1, &c2, &end] {
        clean_input.push_str(&serde_json::to_string(line).expect("line"));
        clean_input.push('\n');
    }

    // WHY inline closures: testkit::serve_lines returns raw lines; shaping
    // them into ServeResponse plus the id/ok/value projection is this test's
    // one-use comparator.
    let parse = |lines: Vec<String>| -> Vec<ServeResponse> {
        lines
            .iter()
            .filter(|l| !l.is_empty())
            .map(|line| serde_json::from_str(line).expect("response json"))
            .collect()
    };
    let (corrupt_result, corrupt_lines) = testkit::serve_lines(corrupt_input, temp.path());
    corrupt_result.expect("serve survives corrupt lines");
    let (clean_result, clean_lines) = testkit::serve_lines(clean_input, temp.path());
    clean_result.expect("clean serve runs");
    let corrupt = parse(corrupt_lines);
    let clean = parse(clean_lines);
    assert_eq!(clean.len(), 3, "c1 + c2 + bye");
    assert_eq!(corrupt.len(), 5, "c1 + 2 errors + c2 + bye");

    let answer = |r: &ServeResponse| -> Value {
        let v = serde_json::to_value(r).expect("serialize response");
        json!({"id": v["id"], "ok": v["ok"], "value": v["value"]})
    };
    assert_eq!(answer(&corrupt[0]), answer(&clean[0]), "c1 unaffected");
    assert_eq!(answer(&corrupt[3]), answer(&clean[1]), "c2 unaffected");
    for (i, response) in corrupt[1..3].iter().enumerate() {
        match response {
            ServeResponse::Error { id, error } => {
                assert_eq!(*id, None, "corrupt line {i} carries no id");
                assert!(!error.is_empty());
            }
            other => panic!("corrupt line {i} must fail closed, got {other:?}"),
        }
    }
    assert!(matches!(corrupt[4], ServeResponse::Bye));
    assert!(matches!(clean[2], ServeResponse::Bye));
}
