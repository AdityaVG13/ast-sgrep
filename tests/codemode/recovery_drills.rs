//! Recovery DRILLS for codemode durable state (consolidated suite).
//!
//! Consolidates `durable_recovery_pass4.rs` (R4 end-to-end crash drills) into
//! intent-grouped tests: each `#[test]` owns ONE crash kind with multiple
//! fault-shape facets. Every drill runs a working session executing a plan
//! plus a batch, captures a pre-crash baseline, crashes durable state
//! MID-FLOW, reopens through the documented resume path, recovers through the
//! documented repair path, and then SERVE proves full function: the
//! post-recovery serve transcript is byte-identical to the pre-crash
//! transcript. Contracts live in `recovery_contracts.rs`, relations in
//! `recovery_relations.rs`. Catalog: `tests/catalog/recovery.md` (codemode
//! rows).
//!
//! Discipline (inherited): `CallError` discriminants via `matches!`, never
//! Display text; batch errors by presence; serve proof is raw-line equality;
//! fixed-content tempdirs; `wall_ms` never compared. Testkit supplies
//! config/batch/file/fault builders plus the shared recovery fixtures
//! (`codemode_recovery`); file-local helpers below exist only where testkit
//! offers nothing (each carries a why-comment).

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, BatchCall, BatchRequest, CallError, CodeModeSession,
    ParallelMode, Plan, SessionConfig,
};
use ast_sgrep_testkit as testkit;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Unique token: lives in exactly one file of the fixed-content repo, so
/// shaped counts (`{"hit_count": 1}`) are hand-computable.
const TOKEN: &str = "needle_unique_rd_xyz";

// WHY file-local: drills-only record shapes and harness. The plan/batch
// fixtures, per-call projection, proof script, baseline capture, and
// full-function proof are specific to this file's crash drills (the indexed
// serve transcript runs through testkit's `serve_transcript_indexed`).
fn drill_plan_json() -> Value {
    json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": TOKEN, "format": "capsule", "limit": 5}},
        {"id": "narrow", "tool": "filter_hits",
         "args": {"hits": "$seed", "path_contains": "src/a.rs", "limit": 5}},
        {"id": "out", "tool": "select",
         "args": {"value": "$narrow", "fields": ["hit_count"]}},
    ], "return": "$out"})
}

fn drill_batch_request() -> BatchRequest {
    BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit: None,
        parallel: None,
        parallel_mode: Some(ParallelMode::Serial),
        calls: vec![
            BatchCall {
                id: "b1".into(),
                tool: "search".into(),
                args: json!({"query": TOKEN, "format": "capsule", "limit": 5}),
            },
            BatchCall {
                id: "b2".into(),
                tool: "catalog_search".into(),
                args: json!({"query": "search"}),
            },
        ],
    }
}

fn write_plan_file(dir: &Path) -> (PathBuf, Value) {
    let raw = drill_plan_json();
    let path = dir.join("plan.json");
    testkit::write_file(&path, &serde_json::to_vec(&raw).expect("plan json"));
    (path, raw)
}

fn write_batch_file(dir: &Path) -> (PathBuf, BatchRequest) {
    let request = drill_batch_request();
    let path = dir.join("batch.json");
    testkit::write_file(&path, &serde_json::to_vec(&request).expect("batch json"));
    (path, request)
}

/// Deterministic projection of a batch response: per-call id/ok/value
/// triples (`wall_ms` excluded by design).
fn batch_answers(response: &ast_sgrep_codemode::BatchResponse) -> Value {
    let triples: Vec<Value> = response
        .results
        .iter()
        .map(|r| json!({"id": r.id, "ok": r.ok, "value": r.value}))
        .collect();
    serde_json::to_value(&triples).expect("serialize triples")
}

/// The serve proof script: an index read, a source read, and an index-free
/// catalog call, then end. Full function in one sticky-worker transcript.
fn serve_script() -> Vec<Value> {
    vec![
        json!({"type": "call", "id": "s1", "tool": "search",
               "args": {"query": TOKEN, "format": "capsule", "limit": 5}}),
        json!({"type": "call", "id": "s2", "tool": "read",
               "args": {"path": "src/a.rs", "start": 1, "end": 2}}),
        json!({"type": "call", "id": "s3", "tool": "catalog_search",
               "args": {"query": "search"}}),
        json!({"type": "end"}),
    ]
}

// The indexed serve transcript runs through testkit's
// `serve_transcript_indexed` (testkit::serve_lines runs unindexed sessions
// only, but the drill proof script needs search+read over the drilled
// index); the TOKEN-bound proof script itself stays suite-local.

/// Baseline flow: a working session executes the drill plan and the drill
/// batch, and the serve transcript is captured. Returns the plan result, the
/// batch answers, and the serve lines all captured pre-crash.
fn baseline_flow(
    config: &SessionConfig,
    plan: &Plan,
    batch: &BatchRequest,
) -> (Value, Value, Vec<String>) {
    let mut session = CodeModeSession::new(config.clone());
    let plan_result = run_plan(&mut session, plan).expect("baseline plan runs");
    assert!(plan_result.ok);
    assert_eq!(plan_result.return_value, json!({"hit_count": 1}));
    let plan_json = serde_json::to_value(&plan_result).expect("serialize plan");

    let batch_response = run_batch(config.clone(), batch).expect("baseline batch runs");
    assert!(batch_response.all_ok);
    assert_eq!(batch_response.call_count, 2);
    let answers = batch_answers(&batch_response);

    let transcript = testkit::serve_transcript_indexed(config.clone(), &serve_script());
    (plan_json, answers, transcript)
}

/// Post-recovery proof: the same plan and batch rerun identically on a fresh
/// session, and SERVE proves full function with a byte-identical transcript.
fn prove_full_function(
    config: &SessionConfig,
    plan: &Plan,
    batch: &BatchRequest,
    plan_before: &Value,
    answers_before: &Value,
    transcript_before: &[String],
) {
    let mut resumed = CodeModeSession::new(config.clone());
    let plan_after = run_plan(&mut resumed, plan).expect("post-recovery plan runs");
    let plan_after_json = serde_json::to_value(&plan_after).expect("serialize plan");
    assert_eq!(
        plan_after_json, *plan_before,
        "post-recovery plan must equal pre-crash baseline"
    );

    let batch_after = run_batch(config.clone(), batch).expect("post-recovery batch runs");
    assert!(batch_after.all_ok);
    assert_eq!(
        batch_answers(&batch_after),
        *answers_before,
        "post-recovery batch must equal pre-crash baseline"
    );

    let transcript_after = testkit::serve_transcript_indexed(config.clone(), &serve_script());
    assert_eq!(
        transcript_after, *transcript_before,
        "post-recovery serve must be byte-identical to pre-crash baseline"
    );
}

/// INTENT=index crash-shape drills: corrupt, truncated, and deleted index
/// bytes mid-flow refuse loudly, recover via operator removal plus rebuild,
/// and serve byte-identically (plan + batch + serve transcript).
/// KILLS=crash-residue mutants, cold-start mutants, lossy-recovery mutants.
/// ABSORBS=R4 drill_corrupt_index_crash_recovers_to_identical_serve,
/// R4 drill_truncated_index_crash_recovers_to_identical_serve,
/// R4 drill_deleted_index_crash_recovers_to_identical_serve.
#[test]
fn index_crash_variants_recover_to_identical_serve() {
    // Arm 1 (CORRUPT-INDEX): garbage index bytes mid-flow; reopen refuses
    // with `Other` on both paths; operator removal plus `index_repo`
    // rebuild recovers; SERVE proves full function.
    let (_temp1, config) = testkit::indexed_codemode_repo(TOKEN);
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);
    let db = config.index_path.clone().expect("index path");
    testkit::write_garbage(&db);
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = faulted
        .call("index_status", json!({}))
        .expect_err("status on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    testkit::remove_db_with_sidecars(&db);
    let mut repaired = CodeModeSession::new(config.clone());
    let rebuilt = repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    assert_eq!(rebuilt["ok"], json!(true));
    prove_full_function(
        &config,
        &plan,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );

    // Arm 2 (TRUNCATED-INDEX): header intact, body cut mid-file (as from a
    // crashed writer); reopen refuses with `Other`; removal plus rebuild
    // recovers; SERVE proves full function.
    let (_temp2, config) = testkit::indexed_codemode_repo(TOKEN);
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);
    let db = config.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    testkit::truncate_file(&db, 512);
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    testkit::remove_db_with_sidecars(&db);
    let mut repaired = CodeModeSession::new(config.clone());
    repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    prove_full_function(
        &config,
        &plan,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );

    // Arm 3 (DELETED-INDEX): the index file (plus sidecars) vanishes
    // mid-flow; reopen reads fail closed with `Other`; `index_repo` rebuilds
    // from source; SERVE proves full function.
    let (_temp3, config) = testkit::indexed_codemode_repo(TOKEN);
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);
    let db = config.index_path.clone().expect("index path");
    testkit::remove_db_with_sidecars(&db);
    assert!(!db.exists());
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search without index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let mut repaired = CodeModeSession::new(config.clone());
    let rebuilt = repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after deletion");
    assert_eq!(rebuilt["ok"], json!(true));
    prove_full_function(
        &config,
        &plan,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}

/// INTENT=record crash-shape drills: a torn plan (Json), a corrupt batch
/// (Json), and deleted plan+batch records (Other) mid-flow refuse loudly,
/// recover by rewriting the original bytes, and serve byte-identically.
/// KILLS=record-recovery mutants.
/// ABSORBS=R4 drill_torn_plan_file_crash_recovers_to_identical_serve,
/// R4 drill_corrupt_batch_file_crash_recovers_to_identical_serve,
/// R4 drill_deleted_plan_and_batch_files_crash_recovers_to_identical_serve.
#[test]
fn record_crash_variants_recover_to_identical_serve() {
    // Arm 1 (TORN-PLAN): a plan file that executed cleanly is torn mid-flow
    // (valid prefix, cut tail); resume is refused as `Json`; the host
    // repairs by rewriting the original bytes; plan, batch, and SERVE all
    // prove full function.
    let (_temp1, config) = testkit::indexed_codemode_repo(TOKEN);
    let dir = tempfile::tempdir().expect("plan dir");
    let (path, raw) = write_plan_file(dir.path());
    let plan = testkit::load_plan_file(&path).expect("plan loads before crash");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);
    let len = std::fs::metadata(&path).expect("metadata").len();
    testkit::truncate_file(&path, len / 2);
    let err = testkit::load_plan_file(&path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    testkit::write_file(&path, &serde_json::to_vec(&raw).expect("plan json"));
    let repaired = testkit::load_plan_file(&path).expect("repaired plan loads");
    assert_eq!(repaired.steps.len(), plan.steps.len());
    prove_full_function(
        &config,
        &repaired,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );

    // Arm 2 (CORRUPT-BATCH): a batch file that executed cleanly is
    // overwritten with garbage mid-flow; resume is refused as `Json`; the
    // host repairs by rewriting the original bytes; plan, batch, and SERVE
    // all prove full function.
    let (_temp2, config) = testkit::indexed_codemode_repo(TOKEN);
    let dir = tempfile::tempdir().expect("batch dir");
    let (path, request) = write_batch_file(dir.path());
    let loaded = testkit::load_batch_file(&path).expect("batch loads before crash");
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &loaded);
    testkit::write_garbage(&path);
    let err = testkit::load_batch_file(&path)
        .map(|_| ())
        .expect_err("corrupt batch must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");
    testkit::write_file(&path, &serde_json::to_vec(&request).expect("batch json"));
    let repaired = testkit::load_batch_file(&path).expect("repaired batch loads");
    assert_eq!(repaired.calls.len(), loaded.calls.len());
    prove_full_function(
        &config,
        &plan,
        &repaired,
        &plan_before,
        &answers_before,
        &transcript_before,
    );

    // Arm 3 (DELETED-RECORDS): both durable records vanish mid-flow; resume
    // of each is refused as `Other` (io); the host repairs by rewriting the
    // original bytes; plan, batch, and SERVE all prove full function.
    let (_temp3, config) = testkit::indexed_codemode_repo(TOKEN);
    let dir = tempfile::tempdir().expect("records dir");
    let (plan_path, plan_raw) = write_plan_file(dir.path());
    let (batch_path, batch_raw) = write_batch_file(dir.path());
    let plan = testkit::load_plan_file(&plan_path).expect("plan loads before crash");
    let batch = testkit::load_batch_file(&batch_path).expect("batch loads before crash");
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);
    std::fs::remove_file(&plan_path).expect("delete plan");
    std::fs::remove_file(&batch_path).expect("delete batch");
    let err = testkit::load_plan_file(&plan_path).expect_err("missing plan must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = testkit::load_batch_file(&batch_path)
        .map(|_| ())
        .expect_err("missing batch must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    testkit::write_file(&plan_path, &serde_json::to_vec(&plan_raw).expect("plan json"));
    testkit::write_file(&batch_path, &serde_json::to_vec(&batch_raw).expect("batch json"));
    let plan_repaired = testkit::load_plan_file(&plan_path).expect("repaired plan loads");
    let batch_repaired = testkit::load_batch_file(&batch_path).expect("repaired batch loads");
    prove_full_function(
        &config,
        &plan_repaired,
        &batch_repaired,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}

/// INTENT=stale-schema crash drill: the index stamp knocked one behind the
/// live version refuses readers, the writer migrates in place, reads resume,
/// and SERVE proves full function byte-identically.
/// KILLS=migration-residue mutants.
/// ABSORBS=R4 drill_stale_schema_crash_recovers_to_identical_serve.
#[test]
fn stale_schema_crash_recovers_to_identical_serve() {
    let (_temp, config) = testkit::indexed_codemode_repo(TOKEN);
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH: version-skew surgery mid-flow (surgery relative to the live
    // stamp, never hardcoded; warm session dropped first so no open read
    // connection pins the WAL over the byte-level stamp asserts).
    let db = config.index_path.clone().expect("index path");
    let current = testkit::db_user_version(&db);
    testkit::set_db_user_version(&db, current - 1);

    let mut stale = CodeModeSession::new(config.clone());
    let err = stale
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("stale schema search must refuse");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Documented recovery: the writer migrates in place.
    stale.call("index_status", json!({})).expect("writer migrates");
    assert_eq!(testkit::db_user_version(&db), current, "writer must stamp current");

    prove_full_function(
        &config,
        &plan,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}

/// INTENT=chained double-crash drill: truncate index -> recover (mid-drill
/// serve already back to baseline) -> tear plan -> recover; final SERVE
/// proves full function against the ORIGINAL pre-crash baseline.
/// KILLS=chain-residue mutants.
/// ABSORBS=R4 drill_double_crash_truncated_index_then_torn_plan_recovers_to_identical_serve.
#[test]
fn double_crash_index_then_plan_recovers_to_original_baseline() {
    let (_temp, config) = testkit::indexed_codemode_repo(TOKEN);
    let dir = tempfile::tempdir().expect("records dir");
    let (plan_path, plan_raw) = write_plan_file(dir.path());
    let plan = testkit::load_plan_file(&plan_path).expect("plan loads before crash");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH 1: truncate durable index mid-flow.
    let db = config.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    testkit::truncate_file(&db, 512);

    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": TOKEN, "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Recovery 1: removal plus rebuild; mid-drill serve already at baseline.
    testkit::remove_db_with_sidecars(&db);
    let mut repaired = CodeModeSession::new(config.clone());
    repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    assert_eq!(
        testkit::serve_transcript_indexed(config.clone(), &serve_script()),
        transcript_before,
        "serve must be back to baseline after crash 1"
    );

    // CRASH 2: torn write over the durable plan mid-flow.
    let len = std::fs::metadata(&plan_path).expect("metadata").len();
    testkit::truncate_file(&plan_path, len / 2);
    let err = testkit::load_plan_file(&plan_path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    // Recovery 2: rewrite the original bytes, resume.
    testkit::write_file(&plan_path, &serde_json::to_vec(&plan_raw).expect("plan json"));
    let plan_repaired = testkit::load_plan_file(&plan_path).expect("repaired plan loads");

    // Final proof against the ORIGINAL baseline: no residue from either crash.
    prove_full_function(
        &config,
        &plan_repaired,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}
