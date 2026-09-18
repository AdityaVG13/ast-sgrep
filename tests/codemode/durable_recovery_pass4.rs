//! R4 end-to-end crash drills for codemode durable state.
//!
//! Pass 1 owns STATIC recovery contracts (a corrupt fixture fails closed with
//! a discriminant). Pass 2 owns ACTIVE faults (healthy flow, mid-flow fault,
//! refusal). Pass 3 owns RELATIONS over recovery (cross-run equivalence).
//! This file owns FULL drills: every test runs a working session executing a
//! plan plus a batch, captures a pre-crash baseline, crashes durable state
//! MID-FLOW (corrupt / truncate / delete), reopens through the documented
//! resume path, recovers through the documented repair path, and then SERVE
//! proves full function: the post-recovery serve transcript is byte-identical
//! to the pre-crash serve transcript.
//!
//! Discipline: failures assert `CallError` discriminants via `matches!`,
//! never Display text. Batch errors are asserted by presence, never message
//! content. Serve proof is raw-line equality, never content matching. Every
//! fixture is a fixed-content tempdir; no sample-fixture dependence, no timing
//! asserts (`wall_ms` is never compared). No new dependencies.

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, run_serve, BatchCall, BatchRequest, CallError,
    CodeModeSession, ParallelMode, Plan, SessionConfig,
};
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::{Path, PathBuf};

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
    std::fs::write(dir.join("a.rs"), "pub fn needle_unique_r4_xyz() {}\n").expect("write a");
    std::fs::write(dir.join("b.rs"), "pub fn other_fn() {}\n").expect("write b");
    let config = config_for(temp.path(), Some("index.db"));
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("index_repo", json!({"force": false}))
        .expect("index");
    (temp, config)
}

/// The drill plan: seed on the unique token, narrow to its file, project the
/// shaped count. Hand-computed return: `{"hit_count": 1}`.
fn drill_plan_json() -> Value {
    json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": "needle_unique_r4_xyz", "format": "capsule", "limit": 5}},
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
                args: json!({"query": "needle_unique_r4_xyz", "format": "capsule", "limit": 5}),
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
    std::fs::write(&path, serde_json::to_vec(&raw).expect("plan json")).expect("write plan");
    (path, raw)
}

fn write_batch_file(dir: &Path) -> (PathBuf, BatchRequest) {
    let request = drill_batch_request();
    let path = dir.join("batch.json");
    std::fs::write(&path, serde_json::to_vec(&request).expect("batch json"))
        .expect("write batch");
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
               "args": {"query": "needle_unique_r4_xyz", "format": "capsule", "limit": 5}}),
        json!({"type": "call", "id": "s2", "tool": "read",
               "args": {"path": "src/a.rs", "start": 1, "end": 2}}),
        json!({"type": "call", "id": "s3", "tool": "catalog_search",
               "args": {"query": "search"}}),
        json!({"type": "end"}),
    ]
}

/// Run the proof script through the public serve entrypoint and return the
/// raw NDJSON lines. Equality of these lines is the full-function proof.
fn serve_transcript(config: SessionConfig) -> Vec<String> {
    let mut input = String::new();
    for call in serve_script() {
        input.push_str(&serde_json::to_string(&call).expect("serve line"));
        input.push('\n');
    }
    let mut out = Vec::new();
    run_serve(config, Cursor::new(input), &mut out).expect("serve runs");
    let text = String::from_utf8(out).expect("utf8");
    let lines: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 4, "s1 + s2 + s3 + bye: {text}");
    lines
}

/// Operator removal of a dead index plus any SQLite sidecars.
fn remove_db_with_sidecars(db: &Path) {
    std::fs::remove_file(db).expect("remove db");
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = db.with_extension(format!("db{suffix}"));
        let _ = std::fs::remove_file(sidecar);
    }
}

/// SQLite header bytes 60..64 are the big-endian `user_version` stamp.
/// Byte surgery keeps version-skew fixtures dep-free (no rusqlite needed).
fn db_user_version(db: &Path) -> u32 {
    let bytes = std::fs::read(db).expect("read db");
    u32::from_be_bytes(bytes[60..64].try_into().expect("full header"))
}

fn set_db_user_version(db: &Path, version: u32) {
    let mut bytes = std::fs::read(db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    bytes[60..64].copy_from_slice(&version.to_be_bytes());
    std::fs::write(db, &bytes).expect("write db");
}

/// Baseline flow: a working session executes the drill plan and the drill
/// batch, and the serve transcript is captured. Returns the plan result, the
/// batch answers, and the serve lines all captured pre-crash.
fn baseline_flow(config: &SessionConfig, plan: &Plan, batch: &BatchRequest) -> (Value, Value, Vec<String>) {
    let mut session = CodeModeSession::new(config.clone());
    let plan_result = run_plan(&mut session, plan).expect("baseline plan runs");
    assert!(plan_result.ok);
    assert_eq!(plan_result.return_value, json!({"hit_count": 1}));
    let plan_json = serde_json::to_value(&plan_result).expect("serialize plan");

    let batch_response = run_batch(config.clone(), batch).expect("baseline batch runs");
    assert!(batch_response.all_ok);
    assert_eq!(batch_response.call_count, 2);
    let answers = batch_answers(&batch_response);

    let transcript = serve_transcript(config.clone());
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

    let transcript_after = serve_transcript(config.clone());
    assert_eq!(
        transcript_after, *transcript_before,
        "post-recovery serve must be byte-identical to pre-crash baseline"
    );
}

#[test]
fn drill_corrupt_index_crash_recovers_to_identical_serve() {
    // Crash kind: CORRUPT-INDEX. Garbage index bytes mid-flow; reopen refuses
    // with `Other`; operator removal plus `index_repo` rebuild recovers; SERVE
    // proves full function with a byte-identical transcript.
    let (_temp, config) = indexed_repo();
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH: corrupt durable index bytes mid-flow.
    let db = config.index_path.clone().expect("index path");
    std::fs::write(&db, "R4-NOT-SQLITE".repeat(400)).expect("corrupt db");

    // Reopen refuses loudly on both read and write paths.
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": "needle_unique_r4_xyz", "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = faulted
        .call("index_status", json!({}))
        .expect_err("status on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Documented recovery: remove the dead file, rebuild, serve.
    remove_db_with_sidecars(&db);
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
}

#[test]
fn drill_truncated_index_crash_recovers_to_identical_serve() {
    // Crash kind: TRUNCATED-INDEX. Header intact, body cut mid-file (as from
    // a crashed writer); reopen refuses with `Other`; removal plus rebuild
    // recovers; SERVE proves full function.
    let (_temp, config) = indexed_repo();
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH: truncate durable index mid-flow.
    let db = config.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    std::fs::write(&db, &bytes[..512]).expect("truncate db");

    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": "needle_unique_r4_xyz", "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    remove_db_with_sidecars(&db);
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
}

#[test]
fn drill_deleted_index_crash_recovers_to_identical_serve() {
    // Crash kind: DELETED-INDEX. The index file (plus sidecars) vanishes
    // mid-flow; reopen reads fail closed with `Other`; `index_repo` rebuilds
    // from source; SERVE proves full function.
    let (_temp, config) = indexed_repo();
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH: index file deleted mid-flow.
    let db = config.index_path.clone().expect("index path");
    remove_db_with_sidecars(&db);
    assert!(!db.exists());

    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": "needle_unique_r4_xyz", "limit": 5}))
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

#[test]
fn drill_torn_plan_file_crash_recovers_to_identical_serve() {
    // Crash kind: TORN-PLAN. A plan file that executed cleanly is torn
    // mid-flow (valid prefix, cut tail); resume is refused as `Json`; the
    // host repairs by rewriting the original bytes; plan, batch, and SERVE
    // all prove full function.
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("plan dir");
    let (path, raw) = write_plan_file(dir.path());
    let plan = load_plan_file(&path).expect("plan loads before crash");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH: torn write over the durable plan mid-flow.
    let bytes = std::fs::read(&path).expect("read plan");
    std::fs::write(&path, &bytes[..bytes.len() / 2]).expect("torn plan");
    let err = load_plan_file(&path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    // Documented recovery: rewrite the original bytes, resume.
    std::fs::write(&path, serde_json::to_vec(&raw).expect("plan json")).expect("repair plan");
    let repaired = load_plan_file(&path).expect("repaired plan loads");
    assert_eq!(repaired.steps.len(), plan.steps.len());

    prove_full_function(
        &config,
        &repaired,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}

#[test]
fn drill_corrupt_batch_file_crash_recovers_to_identical_serve() {
    // Crash kind: CORRUPT-BATCH. A batch file that executed cleanly is
    // overwritten with garbage mid-flow; resume is refused as `Json`; the
    // host repairs by rewriting the original bytes; plan, batch, and SERVE
    // all prove full function.
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("batch dir");
    let (path, request) = write_batch_file(dir.path());
    let loaded = load_batch_file(&path).expect("batch loads before crash");
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &loaded);

    // CRASH: garbage bytes over the durable batch mid-flow.
    std::fs::write(&path, b"\x00\x01\x02{{{not json at all\xff\xfe").expect("corrupt batch");
    let err = load_batch_file(&path)
        .map(|_| ())
        .expect_err("corrupt batch must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    // Documented recovery: rewrite the original bytes, resume.
    std::fs::write(&path, serde_json::to_vec(&request).expect("batch json"))
        .expect("repair batch");
    let repaired = load_batch_file(&path).expect("repaired batch loads");
    assert_eq!(repaired.calls.len(), loaded.calls.len());

    prove_full_function(
        &config,
        &plan,
        &repaired,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}

#[test]
fn drill_deleted_plan_and_batch_files_crash_recovers_to_identical_serve() {
    // Crash kind: DELETED-RECORDS. Both durable records vanish mid-flow;
    // resume of each is refused as `Other` (io); the host repairs by
    // rewriting the original bytes; plan, batch, and SERVE all prove full
    // function.
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("records dir");
    let (plan_path, plan_raw) = write_plan_file(dir.path());
    let (batch_path, batch_raw) = write_batch_file(dir.path());
    let plan = load_plan_file(&plan_path).expect("plan loads before crash");
    let batch = load_batch_file(&batch_path).expect("batch loads before crash");
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH: both durable records deleted mid-flow.
    std::fs::remove_file(&plan_path).expect("delete plan");
    std::fs::remove_file(&batch_path).expect("delete batch");
    let err = load_plan_file(&plan_path).expect_err("missing plan must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = load_batch_file(&batch_path)
        .map(|_| ())
        .expect_err("missing batch must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Documented recovery: rewrite the original bytes, resume.
    std::fs::write(&plan_path, serde_json::to_vec(&plan_raw).expect("plan json"))
        .expect("repair plan");
    std::fs::write(&batch_path, serde_json::to_vec(&batch_raw).expect("batch json"))
        .expect("repair batch");
    let plan_repaired = load_plan_file(&plan_path).expect("repaired plan loads");
    let batch_repaired = load_batch_file(&batch_path).expect("repaired batch loads");

    prove_full_function(
        &config,
        &plan_repaired,
        &batch_repaired,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}

#[test]
fn drill_stale_schema_crash_recovers_to_identical_serve() {
    // Crash kind: STALE-SCHEMA. The index stamp is knocked one behind the
    // live version (surgery relative to the live stamp, never hardcoded);
    // the stale reader refuses with `Other`; the writer migrates in place
    // via `index_status`; reads resume; SERVE proves full function.
    let (_temp, config) = indexed_repo();
    let plan = parse_plan(&drill_plan_json()).expect("parse plan");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH: version-skew surgery mid-flow (warm session dropped first so no
    // open read connection pins the WAL over the byte-level stamp asserts).
    let db = config.index_path.clone().expect("index path");
    let current = db_user_version(&db);
    set_db_user_version(&db, current - 1);

    let mut stale = CodeModeSession::new(config.clone());
    let err = stale
        .call("search", json!({"query": "needle_unique_r4_xyz", "limit": 5}))
        .expect_err("stale schema search must refuse");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Documented recovery: the writer migrates in place.
    stale.call("index_status", json!({})).expect("writer migrates");
    assert_eq!(db_user_version(&db), current, "writer must stamp current");

    prove_full_function(
        &config,
        &plan,
        &batch,
        &plan_before,
        &answers_before,
        &transcript_before,
    );
}

#[test]
fn drill_double_crash_truncated_index_then_torn_plan_recovers_to_identical_serve() {
    // Chained DOUBLE-CRASH. Crash 1 truncates the index mid-flow (refused as
    // `Other`, recovered by removal plus rebuild, mid-drill serve already
    // back to baseline). Crash 2 then tears the plan file mid-flow (refused
    // as `Json`, recovered by rewriting the original bytes). Final SERVE
    // proves full function against the ORIGINAL pre-crash baseline: neither
    // crash left residue.
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("records dir");
    let (plan_path, plan_raw) = write_plan_file(dir.path());
    let plan = load_plan_file(&plan_path).expect("plan loads before crash");
    let batch = drill_batch_request();
    let (plan_before, answers_before, transcript_before) = baseline_flow(&config, &plan, &batch);

    // CRASH 1: truncate durable index mid-flow.
    let db = config.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    std::fs::write(&db, &bytes[..512]).expect("truncate db");

    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": "needle_unique_r4_xyz", "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    // Recovery 1: removal plus rebuild; mid-drill serve already at baseline.
    remove_db_with_sidecars(&db);
    let mut repaired = CodeModeSession::new(config.clone());
    repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    assert_eq!(
        serve_transcript(config.clone()),
        transcript_before,
        "serve must be back to baseline after crash 1"
    );

    // CRASH 2: torn write over the durable plan mid-flow.
    let plan_bytes = std::fs::read(&plan_path).expect("read plan");
    std::fs::write(&plan_path, &plan_bytes[..plan_bytes.len() / 2]).expect("torn plan");
    let err = load_plan_file(&plan_path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    // Recovery 2: rewrite the original bytes, resume.
    std::fs::write(&plan_path, serde_json::to_vec(&plan_raw).expect("plan json"))
        .expect("repair plan");
    let plan_repaired = load_plan_file(&plan_path).expect("repaired plan loads");

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
