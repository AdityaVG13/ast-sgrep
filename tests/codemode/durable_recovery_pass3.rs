//! R3 metamorphic-recovery oracles for codemode durable state.
//!
//! Pass 1 owns STATIC recovery contracts (corrupt fixtures fail closed with a
//! discriminant). Pass 2 owns ACTIVE faults (healthy flow, mid-flow fault,
//! refusal). This file owns RELATIONS over recovery: each test executes two or
//! more runs related by a fault/repair/reopen transform and asserts the runs
//! agree (or disagree) in a prescribed way. No test here asserts a lone point
//! contract; every assertion is an equality (or exact count relation) between
//! runs. Nothing here duplicates pass 1/2 shapes: where a fixture recurs (torn
//! plan, truncated index, stale stamp) the assertion is cross-run equivalence,
//! not the refusal itself.
//!
//! Discipline: failures assert `CallError` discriminants via `matches!`,
//! never Display text. Batch/serve errors are asserted by presence (`is_some`
//! / `is_none`), never message content. Every fixture is a fixed-content
//! tempdir; no sample-fixture dependence, no timing asserts (`wall_ms` is
//! never compared). No new dependencies.

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, run_serve, BatchCall, BatchRequest, CallError,
    CodeModeSession, ParallelMode, Plan, ServeResponse, SessionConfig,
};
use serde_json::{json, Value};
use std::io::Cursor;
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
    write_repo_files(temp.path());
    let config = config_for(temp.path(), Some("index.db"));
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("index_repo", json!({"force": false}))
        .expect("index");
    (temp, config)
}

fn write_repo_files(root: &Path) {
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("a.rs"), "pub fn needle_unique_r3_xyz() {}\n").expect("write a");
    std::fs::write(dir.join("b.rs"), "pub fn other_fn() {}\n").expect("write b");
}

/// Two byte-identical repos indexed independently: the never-faulted twin is
/// the oracle for the recovered twin. Search/read/edit values are
/// root-relative, so full-Value equality across tempdirs is well-defined;
/// `index_status` embeds absolute paths, so twins compare count fields only.
fn twin_repos() -> (tempfile::TempDir, SessionConfig, tempfile::TempDir, SessionConfig) {
    let (a_temp, a_config) = indexed_repo();
    let (b_temp, b_config) = indexed_repo();
    (a_temp, a_config, b_temp, b_config)
}

/// The recovering 3-step plan: seed on the unique token, narrow to its file,
/// project the shaped count. Hand-computed return: `{"hit_count": 1}`.
fn recovering_plan_json() -> Value {
    json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": "needle_unique_r3_xyz", "format": "capsule", "limit": 5}},
        {"id": "narrow", "tool": "filter_hits",
         "args": {"hits": "$seed", "path_contains": "src/a.rs", "limit": 5}},
        {"id": "out", "tool": "select",
         "args": {"value": "$narrow", "fields": ["hit_count"]}},
    ], "return": "$out"})
}

fn catalog_batch_calls() -> Vec<BatchCall> {
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
    ]
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

/// Operator removal of a dead index plus any SQLite sidecars.
fn remove_db_with_sidecars(db: &Path) {
    std::fs::remove_file(db).expect("remove db");
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = db.with_extension(format!("db{suffix}"));
        let _ = std::fs::remove_file(sidecar);
    }
}

/// Root-independent projection of `index_status` for cross-tempdir twins.
fn status_counts(status: &Value) -> Value {
    json!({
        "file_count": status["file_count"],
        "line_count": status["line_count"],
        "symbol_count": status["symbol_count"],
        "caller_count": status["caller_count"],
        "import_count": status["import_count"],
        "semantic_chunk_count": status["semantic_chunk_count"],
    })
}

fn search_capsule(session: &mut CodeModeSession) -> Value {
    session
        .call(
            "search",
            json!({"query": "needle_unique_r3_xyz", "format": "capsule", "limit": 5}),
        )
        .expect("capsule search")
}

fn read_window(session: &mut CodeModeSession) -> Value {
    session
        .call("read", json!({"path": "src/a.rs", "start": 1, "end": 2}))
        .expect("read window")
}

#[test]
fn mr_plan_reexecution_on_warm_session_is_identical() {
    // MR-DET-1 (re-execution determinism): the same plan run twice on one warm
    // session yields identical step outputs and return value; the only
    // difference is the exact call-count doubling (3 steps + 3 steps).
    let (_temp, config) = indexed_repo();
    let plan = parse_plan(&recovering_plan_json()).expect("parse plan");
    let mut session = CodeModeSession::new(config);
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
}

#[test]
fn mr_plan_reexecution_across_fresh_sessions_is_identical() {
    // MR-DET-2 (re-execution determinism): the same plan on two independently
    // opened sessions yields fully identical PlanResults, including the
    // per-session call count (fresh budgets agree exactly).
    let (_temp, config) = indexed_repo();
    let plan = parse_plan(&recovering_plan_json()).expect("parse plan");
    let mut first_session = CodeModeSession::new(config.clone());
    let mut second_session = CodeModeSession::new(config);
    let first = run_plan(&mut first_session, &plan).expect("first run");
    let second = run_plan(&mut second_session, &plan).expect("second run");
    let first_json = serde_json::to_value(&first).expect("serialize first");
    let second_json = serde_json::to_value(&second).expect("serialize second");
    assert_eq!(second_json, first_json);
    assert_eq!(first.call_count, second.call_count);
}

#[test]
fn mr_corrupt_remove_rebuild_restores_identical_outputs() {
    // MR-REPAIR-1 (corrupt -> repair/resume -> verify roundtrip): search,
    // read, and status-count outputs captured before corruption are
    // byte-identical after operator removal plus `index_repo` rebuild. The
    // mid-transform refusal (`Other`) proves the fault bit; the relation
    // proves the repair lost nothing.
    let (_temp, config) = indexed_repo();
    let mut baseline_session = CodeModeSession::new(config.clone());
    let search_before = search_capsule(&mut baseline_session);
    let read_before = read_window(&mut baseline_session);
    let status_before = baseline_session
        .call("index_status", json!({}))
        .expect("status before");

    let db = config.index_path.clone().expect("index path");
    std::fs::write(&db, "R3-NOT-SQLITE".repeat(400)).expect("corrupt db");
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": "needle_unique_r3_xyz", "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    remove_db_with_sidecars(&db);
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
    assert_eq!(status_counts(&status_after), status_counts(&status_before));
}

#[test]
fn mr_torn_plan_file_repair_restores_identical_result() {
    // MR-REPAIR-2 (corrupt -> repair/resume -> verify roundtrip): a plan file
    // that ran cleanly is torn (resume refused as `Json`), then repaired by
    // rewriting the original bytes. The post-repair run equals the pre-fault
    // run as a full PlanResult: run(repair(corrupt(f))) == run(f).
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("plan dir");
    let raw = recovering_plan_json();
    let path = dir.path().join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&raw).expect("plan json")).expect("write plan");

    let plan = load_plan_file(&path).expect("plan loads before fault");
    let mut session = CodeModeSession::new(config.clone());
    let before = run_plan(&mut session, &plan).expect("plan runs before fault");
    assert!(before.ok);

    let bytes = std::fs::read(&path).expect("read plan");
    std::fs::write(&path, &bytes[..bytes.len() / 2]).expect("torn plan");
    let err = load_plan_file(&path).expect_err("torn plan must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    std::fs::write(&path, serde_json::to_vec(&raw).expect("plan json")).expect("repair plan");
    let repaired = load_plan_file(&path).expect("repaired plan loads");
    assert_eq!(repaired.steps.len(), plan.steps.len());
    let mut resumed = CodeModeSession::new(config);
    let after = run_plan(&mut resumed, &repaired).expect("repaired plan runs");
    let before_json = serde_json::to_value(&before).expect("serialize before");
    let after_json = serde_json::to_value(&after).expect("serialize after");
    assert_eq!(after_json, before_json);
}

#[test]
fn mr_truncated_batch_file_repair_restores_identical_response() {
    // MR-REPAIR-3 (corrupt -> repair/resume -> verify roundtrip): the batch
    // analogue of MR-REPAIR-2. Post-repair response equals the pre-fault
    // response on every deterministic field (`wall_ms` excluded by design).
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("batch dir");
    let request = batch_request(catalog_batch_calls(), ParallelMode::Serial);
    let path = dir.path().join("batch.json");
    std::fs::write(&path, serde_json::to_vec(&request).expect("batch json"))
        .expect("write batch");

    let loaded = load_batch_file(&path).expect("batch loads before fault");
    let before = run_batch(config.clone(), &loaded).expect("batch runs before fault");
    assert!(before.all_ok);

    let bytes = std::fs::read(&path).expect("read batch");
    std::fs::write(&path, &bytes[..bytes.len() / 2]).expect("truncate batch");
    let err = load_batch_file(&path)
        .map(|_| ())
        .expect_err("truncated batch must fail");
    assert!(matches!(err, CallError::Json(_)), "got {err:?}");

    std::fs::write(&path, serde_json::to_vec(&request).expect("batch json"))
        .expect("repair batch");
    let repaired = load_batch_file(&path).expect("repaired batch loads");
    let after = run_batch(config, &repaired).expect("repaired batch runs");
    assert_eq!(after.all_ok, before.all_ok);
    assert_eq!(after.call_count, before.call_count);
    assert_eq!(after.mode, before.mode);
    let before_results = serde_json::to_value(&before.results).expect("serialize before");
    let after_results = serde_json::to_value(&after.results).expect("serialize after");
    assert_eq!(after_results, before_results);
}

#[test]
fn mr_interrupted_batch_resume_equals_uninterrupted_wave() {
    // MR-RESUME-1 (interrupted-batch resume equivalence): twin repos start
    // identical. Twin A runs [edit-ok, edit-bad] (interrupted: all_ok false),
    // then resumes with the corrected step-2 alone. Twin B runs the corrected
    // [edit-ok, edit-fixed] wave uninterrupted (all_ok true). Relation: final
    // file bytes agree, the committed prefix values agree, and the resumed
    // step-2 value equals the uninterrupted step-2 value.
    let (_a_temp, a_config, _b_temp, b_config) = twin_repos();
    let a_rs = a_config.root.join("src/a.rs");
    let b_rs = b_config.root.join("src/a.rs");

    let step_1 = BatchCall {
        id: "step-1".into(),
        tool: "edit".into(),
        args: json!({"path": "src/a.rs",
            "oldText": "needle_unique_r3_xyz", "newText": "needle_mid_r3_xyz"}),
    };
    let step_2_bad = BatchCall {
        id: "step-2".into(),
        tool: "edit".into(),
        args: json!({"path": "src/a.rs",
            "oldText": "not-present-anywhere", "newText": "x"}),
    };
    let step_2_fixed = BatchCall {
        id: "step-2".into(),
        tool: "edit".into(),
        args: json!({"path": "src/a.rs",
            "oldText": "needle_mid_r3_xyz", "newText": "needle_final_r3_xyz"}),
    };

    let interrupted = run_batch(
        a_config.clone(),
        &batch_request(vec![step_1.clone(), step_2_bad], ParallelMode::Serial),
    )
    .expect("interrupted wave runs");
    assert!(!interrupted.all_ok);
    assert!(interrupted.results[0].ok);
    assert!(!interrupted.results[1].ok);
    assert!(interrupted.results[1].value.is_none());
    assert!(interrupted.results[1].error.is_some());

    let resumed = run_batch(
        a_config,
        &batch_request(vec![step_2_fixed.clone()], ParallelMode::Serial),
    )
    .expect("resume runs");
    assert!(resumed.all_ok);
    assert_eq!(resumed.results.len(), 1);
    assert_eq!(resumed.results[0].id, "step-2");
    assert!(resumed.results[0].ok);

    let uninterrupted = run_batch(
        b_config,
        &batch_request(vec![step_1, step_2_fixed], ParallelMode::Serial),
    )
    .expect("uninterrupted wave runs");
    assert!(uninterrupted.all_ok);
    assert_eq!(uninterrupted.results.len(), 2);

    let a_final = std::fs::read_to_string(&a_rs).expect("reread a");
    let b_final = std::fs::read_to_string(&b_rs).expect("reread b");
    assert!(a_final.contains("needle_final_r3_xyz"), "{a_final}");
    assert_eq!(a_final, b_final, "resumed bytes must equal uninterrupted bytes");
    assert_eq!(interrupted.results[0].value, uninterrupted.results[0].value);
    assert_eq!(resumed.results[0].value, uninterrupted.results[1].value);
}

#[test]
fn mr_session_reopen_preserves_served_state_exactly() {
    // MR-REOPEN (session reopen stability): dropping a warm session and
    // reopening the same config preserves served state exactly — search,
    // read, and status-count outputs are identical, source bytes are
    // untouched, and the schema stamp is unchanged. The only prescribed
    // difference is the fresh call budget (0 before replay, N after).
    let (_temp, config) = indexed_repo();
    let mut warm = CodeModeSession::new(config.clone());
    let search_warm = search_capsule(&mut warm);
    let read_warm = read_window(&mut warm);
    let status_warm = warm.call("index_status", json!({})).expect("status warm");
    let warm_calls = warm.call_count();
    assert_eq!(warm_calls, 3);
    let src_before = std::fs::read(config.root.join("src/a.rs")).expect("read src");
    let db = config.index_path.clone().expect("index path");
    let stamp_before = db_user_version(&db);
    drop(warm);

    let mut reopened = CodeModeSession::new(config.clone());
    assert_eq!(reopened.call_count(), 0, "reopen resets the call budget");
    assert_eq!(search_capsule(&mut reopened), search_warm);
    assert_eq!(read_window(&mut reopened), read_warm);
    let status_reopened = reopened
        .call("index_status", json!({}))
        .expect("status reopened");
    assert_eq!(status_counts(&status_reopened), status_counts(&status_warm));
    assert_eq!(reopened.call_count(), warm_calls);
    let src_after = std::fs::read(config.root.join("src/a.rs")).expect("reread src");
    assert_eq!(src_after, src_before);
    assert_eq!(db_user_version(&db), stamp_before);
}

#[test]
fn mr_recovered_repo_matches_never_faulted_twin() {
    // MR-PARITY-1 (recovered-session output parity): twin A is faulted
    // (truncated index, refused as `Other`), then repaired by operator
    // removal plus rebuild. Twin B is never faulted. Relation: the recovered
    // twin serves outputs identical to the pristine twin across search, read,
    // and status counts.
    let (_a_temp, a_config, _b_temp, b_config) = twin_repos();

    let db = a_config.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    std::fs::write(&db, &bytes[..512]).expect("truncate db");
    let mut faulted = CodeModeSession::new(a_config.clone());
    let err = faulted
        .call("search", json!({"query": "needle_unique_r3_xyz", "limit": 5}))
        .expect_err("search on truncated index must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    remove_db_with_sidecars(&db);
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
        status_counts(&recovered_status),
        status_counts(&pristine_status)
    );
}

#[test]
fn mr_stale_schema_migration_preserves_search_output_exactly() {
    // MR-MIGRATE (stale-schema repair parity): capsule search and read outputs
    // captured before version surgery are byte-identical after the stale
    // reader refuses (`Other`), the writer migrates in place, and reads
    // resume. Pass 1 asserts migrated rows serve; this asserts the migration
    // preserves served output exactly (surgery is relative to the live stamp,
    // never a hardcoded version).
    let (_temp, config) = indexed_repo();
    let mut baseline_session = CodeModeSession::new(config.clone());
    let search_before = search_capsule(&mut baseline_session);
    let read_before = read_window(&mut baseline_session);
    // Drop the warm session before surgery: its open read connection would pin
    // the WAL and keep the migrated header out of the raw file bytes, making
    // the byte-level stamp assert observe a stale 15 instead of the migrated
    // value. The relation compares captured outputs, not live sessions.
    drop(baseline_session);

    let db = config.index_path.clone().expect("index path");
    let current = db_user_version(&db);
    set_db_user_version(&db, current - 1);
    let mut stale = CodeModeSession::new(config.clone());
    let err = stale
        .call("search", json!({"query": "needle_unique_r3_xyz", "limit": 5}))
        .expect_err("stale schema search must refuse");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    stale.call("index_status", json!({})).expect("writer migrates");
    assert_eq!(db_user_version(&db), current, "writer must stamp current");

    let mut resumed = CodeModeSession::new(config);
    assert_eq!(
        search_capsule(&mut resumed),
        search_before,
        "post-migration search must equal pre-fault output"
    );
    assert_eq!(
        read_window(&mut resumed),
        read_before,
        "post-migration read must equal pre-fault output"
    );
}

#[test]
fn mr_serve_valid_answers_unaffected_by_corrupt_lines() {
    // MR-STREAM (serve resumption equivalence): a serve stream with corrupt
    // lines interleaved answers its valid calls identically to the clean
    // stream with those lines removed. The corrupt run additionally carries
    // exactly two id-less `Error` records; both runs end with `Bye`.
    let temp = tempfile::tempdir().expect("tempdir");
    let config = config_for(temp.path(), None);
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
    clean_input.push_str(&serde_json::to_string(&c1).expect("c1"));
    clean_input.push('\n');
    clean_input.push_str(&serde_json::to_string(&c2).expect("c2"));
    clean_input.push('\n');
    clean_input.push_str(&serde_json::to_string(&end).expect("end"));
    clean_input.push('\n');

    let mut corrupt_out = Vec::new();
    run_serve(
        config.clone(),
        Cursor::new(corrupt_input),
        &mut corrupt_out,
    )
    .expect("serve survives corrupt lines");
    let mut clean_out = Vec::new();
    run_serve(config, Cursor::new(clean_input), &mut clean_out).expect("clean serve runs");

    let parse = |out: Vec<u8>| -> Vec<ServeResponse> {
        String::from_utf8(out)
            .expect("utf8")
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| serde_json::from_str(line).expect("response json"))
            .collect()
    };
    let corrupt = parse(corrupt_out);
    let clean = parse(clean_out);
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
            ServeResponse::Error { id, .. } => {
                assert_eq!(*id, None, "corrupt line {i} carries no id");
            }
            other => panic!("corrupt line {i} must fail closed, got {other:?}"),
        }
    }
    assert!(matches!(corrupt[4], ServeResponse::Bye));
    assert!(matches!(clean[2], ServeResponse::Bye));
}

#[test]
fn mr_serial_parallel_agreement_survives_index_rebuild() {
    // MR-MODE (mode-equivalence preservation): the serial/parallel
    // per-call agreement holds before the fault, holds again after
    // corrupt -> remove -> rebuild, and the post-rebuild serial answers equal
    // the pre-fault serial answers. Recovery preserves both the mode relation
    // and the served values.
    let (_temp, config) = indexed_repo();
    let calls = || {
        vec![
            BatchCall {
                id: "p1".into(),
                tool: "search".into(),
                args: json!({"query": "needle_unique_r3_xyz", "format": "capsule", "limit": 5}),
            },
            BatchCall {
                id: "p2".into(),
                tool: "catalog_search".into(),
                args: json!({"query": "search"}),
            },
        ]
    };
    let answers = |response: &ast_sgrep_codemode::BatchResponse| -> Value {
        let triples: Vec<Value> = response
            .results
            .iter()
            .map(|r| json!({"id": r.id, "ok": r.ok, "value": r.value}))
            .collect();
        serde_json::to_value(&triples).expect("serialize triples")
    };

    let serial_before = run_batch(
        config.clone(),
        &batch_request(calls(), ParallelMode::Serial),
    )
    .expect("serial before");
    let parallel_before = run_batch(
        config.clone(),
        &batch_request(calls(), ParallelMode::Parallel),
    )
    .expect("parallel before");
    assert_eq!(serial_before.mode, "serial");
    assert_eq!(parallel_before.mode, "parallel");
    assert_eq!(answers(&parallel_before), answers(&serial_before));

    let db = config.index_path.clone().expect("index path");
    std::fs::write(&db, "R3-NOT-SQLITE".repeat(400)).expect("corrupt db");
    let mut faulted = CodeModeSession::new(config.clone());
    let err = faulted
        .call("search", json!({"query": "needle_unique_r3_xyz", "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    remove_db_with_sidecars(&db);
    let mut repaired = CodeModeSession::new(config.clone());
    repaired
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");

    let serial_after = run_batch(
        config.clone(),
        &batch_request(calls(), ParallelMode::Serial),
    )
    .expect("serial after");
    let parallel_after = run_batch(config, &batch_request(calls(), ParallelMode::Parallel))
        .expect("parallel after");
    assert_eq!(answers(&parallel_after), answers(&serial_after));
    assert_eq!(
        answers(&serial_after),
        answers(&serial_before),
        "recovery must preserve served values"
    );
}
