//! R1 recovery-contract oracles for codemode durable state.
//!
//! Pass 1/2 own in-memory contracts (plan/batch shape validation, budget,
//! taxonomy, catalog, scrubbers). Pass 3 owns cross-call relations. Pass 4
//! owns full flows including missing-ROOT fail-closed. session_plan/batch own
//! operational flows. This file owns only NEW recovery contracts over DURABLE
//! bytes: plan/batch JSON files, the SQLite index file, the writer-generation
//! stamp, and serve NDJSON records.
//!
//! Discipline: failures assert `CallError` discriminants via `matches!`,
//! never Display text. Filesystem facts (quarantine absence, version bytes)
//! are asserted as bytes, never messages. Every fixture is a fixed-content
//! tempdir; no sample-fixture dependence, no timing asserts.

use ast_sgrep_codemode::{
    parse_plan, run_batch, run_plan, run_serve, BatchCall, BatchRequest, CallError,
    CodeModeSession, Plan, ServeResponse, SessionConfig,
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
    let dir = temp.path().join("src");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("a.rs"), "pub fn needle_unique_r1_xyz() {}\n").expect("write a");
    std::fs::write(dir.join("b.rs"), "pub fn other_fn() {}\n").expect("write b");
    let config = config_for(temp.path(), Some("index.db"));
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("index_repo", json!({"force": false}))
        .expect("index");
    (temp, config)
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

#[test]
fn missing_plan_and_batch_files_fail_closed_as_other() {
    // Absent durable records are io failures (`Other`), never silent empty
    // plans/batches and never a validation discriminant.
    let temp = tempfile::tempdir().expect("tempdir");
    let plan = temp.path().join("no-such-plan.json");
    let batch = temp.path().join("no-such-batch.json");
    let err = load_plan_file(&plan).expect_err("missing plan must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = load_batch_file(&batch).expect_err("missing batch must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
}

#[test]
fn corrupt_truncated_and_empty_files_fail_closed_as_json() {
    // Byte-corrupt records fail at the syntax layer (`Json`), distinct from
    // the pass-1/2 shape layer (`InvalidArgs` for well-formed bad values).
    // Truncation (valid prefix, cut tail) and empty files share the bucket.
    let temp = tempfile::tempdir().expect("tempdir");
    let garbage = b"\x00\x01\x02{{{not json at all\xff\xfe";
    let full_plan = json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ]});
    let full_batch = json!({"calls": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
    ]});
    for (name, full) in [("plan", full_plan), ("batch", full_batch)] {
        let raw = serde_json::to_vec(&full).expect("serialize");
        let truncated = &raw[..raw.len() / 2];
        for (case, bytes) in [
            ("garbage", garbage.as_slice()),
            ("truncated", truncated),
            ("empty", b"".as_slice()),
        ] {
            let path = temp.path().join(format!("{name}-{case}.json"));
            std::fs::write(&path, bytes).expect("write fixture");
            let err = if name == "plan" {
                load_plan_file(&path).expect_err("must fail")
            } else {
                load_batch_file(&path).map(|_| ()).expect_err("must fail")
            };
            assert!(
                matches!(err, CallError::Json(_)),
                "{name}/{case}: expected Json, got {err:?}"
            );
        }
    }
}

#[test]
fn valid_plan_and_batch_files_resume_deterministically() {
    // File roundtrip: the same bytes load twice and execute identically.
    // Hand values mirror the pass-4 flow (unique token, path-narrowed).
    let (_temp, config) = indexed_repo();
    let dir = tempfile::tempdir().expect("plan dir");
    let plan_raw = json!({"steps": [
        {"id": "seed", "tool": "search",
         "args": {"query": "needle_unique_r1_xyz", "format": "capsule", "limit": 5}},
        {"id": "narrow", "tool": "filter_hits",
         "args": {"hits": "$seed", "path_contains": "src/a.rs", "limit": 5}},
        {"id": "out", "tool": "select",
         "args": {"value": "$narrow", "fields": ["hit_count"]}},
    ], "return": "$out"});
    let plan_path = dir.path().join("plan.json");
    std::fs::write(&plan_path, serde_json::to_vec(&plan_raw).expect("plan json"))
        .expect("write plan");
    let plan = load_plan_file(&plan_path).expect("plan loads");
    assert_eq!(plan.steps.len(), 3);
    for _ in 0..2 {
        let mut session = CodeModeSession::new(config.clone());
        let result = run_plan(&mut session, &plan).expect("plan runs");
        assert!(result.ok);
        assert_eq!(result.call_count, 3);
        assert_eq!(result.return_value, json!({"hit_count": 1}));
    }

    let batch_raw = BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit: None,
        parallel: None,
        parallel_mode: None,
        calls: vec![
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
    };
    let batch_path = dir.path().join("batch.json");
    std::fs::write(
        &batch_path,
        serde_json::to_vec(&batch_raw).expect("batch json"),
    )
    .expect("write batch");
    let request = load_batch_file(&batch_path).expect("batch loads");
    assert_eq!(request.calls.len(), 2);
    let response = run_batch(config, &request).expect("batch runs");
    assert!(response.all_ok);
    assert_eq!(response.call_count, 2);
    assert_eq!(response.results[0].id, "w1");
    assert_eq!(response.results[1].id, "w2");
    // Hand-computed catalog facts (pass 4): search ranks itself first,
    // chain matches only the chain tool.
    assert_eq!(response.results[0].value.as_ref().expect("v1")["tools"][0]["name"], json!("search"));
    assert_eq!(response.results[1].value.as_ref().expect("v2")["tools"][0]["name"], json!("chain"));
}

#[test]
fn absent_index_reads_fail_closed_then_empty_index_serves_deterministic_empty_results() {
    // Root exists but no index was ever built (pass 4 owns missing ROOT;
    // this owns missing INDEX): bound reads fail closed with `Other` while
    // the writer creates the empty index; the empty index then serves
    // deterministic empty results on repeat.
    let temp = tempfile::tempdir().expect("tempdir");
    let config = config_for(temp.path(), Some("index.db"));
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

    let status = session.call("index_status", json!({})).expect("status creates");
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
}

#[test]
fn corrupt_and_truncated_index_files_fail_closed_without_quarantine() {
    // Garbage or cut-off index bytes fail closed on BOTH read and write
    // paths with `Other`. Codemode never sets force_reindex, so unlike the
    // CLI reindex path no `.corrupt` quarantine is created and no silent
    // rebuild happens: refusal is total until the operator intervenes.
    let (_temp, config) = indexed_repo();
    let db = config.index_path.clone().expect("index path");
    std::fs::write(&db, "R1-NOT-SQLITE".repeat(400)).expect("corrupt db");
    let mut session = CodeModeSession::new(config.clone());
    let err = session
        .call("index_status", json!({}))
        .expect_err("status on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call("search", json!({"query": "needle_unique_r1_xyz", "limit": 5}))
        .expect_err("search on garbage must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let quarantines: Vec<_> = std::fs::read_dir(_temp.path())
        .expect("readdir")
        .filter_map(|entry| entry.ok().map(|e| e.file_name()))
        .filter(|name| name.to_string_lossy().contains(".corrupt"))
        .collect();
    assert!(quarantines.is_empty(), "no silent quarantine: {quarantines:?}");

    // Truncated: header intact, body cut mid-file.
    let (_temp2, config2) = indexed_repo();
    let db2 = config2.index_path.clone().expect("index path");
    let bytes = std::fs::read(&db2).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    std::fs::write(&db2, &bytes[..512]).expect("truncate db");
    let mut session2 = CodeModeSession::new(config2);
    let err = session2
        .call("index_status", json!({}))
        .expect_err("status on truncated must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session2
        .call("search", json!({"query": "needle_unique_r1_xyz", "limit": 5}))
        .expect_err("search on truncated must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
}

#[test]
fn future_schema_version_is_refused_loudly_and_nonmutating() {
    // A newer writer's index (user_version 9999) is refused with `Other` on
    // both paths, and the refusal mutates nothing: the stamp still reads
    // 9999 afterwards, so a newer binary can still open it.
    assert_eq!(ast_sgrep_core::INDEX_SCHEMA_VERSION, 16);
    let (_temp, config) = indexed_repo();
    let db = config.index_path.clone().expect("index path");
    assert_eq!(db_user_version(&db), 16);
    set_db_user_version(&db, 9999);
    let mut session = CodeModeSession::new(config.clone());
    let err = session
        .call("search", json!({"query": "needle_unique_r1_xyz", "limit": 5}))
        .expect_err("future schema search must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    let err = session
        .call("index_status", json!({}))
        .expect_err("future schema status must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(db_user_version(&db), 9999, "refusal must not migrate");
    let peeked = ast_sgrep_core::IndexStore::peek_schema_version(
        &config.root,
        config.index_path.as_deref(),
    )
    .expect("peek survives refusal");
    assert_eq!(peeked, 9999);
}

#[test]
fn stale_schema_version_readers_refuse_writers_migrate_then_reads_resume() {
    // One-behind stamp (15): read-only opens refuse with `Other` rather
    // than serve pre-migration rows authoritatively; the writer migrates in
    // place; reads then resume on the migrated rows.
    assert_eq!(ast_sgrep_core::INDEX_SCHEMA_VERSION, 16);
    let (_temp, config) = indexed_repo();
    let db = config.index_path.clone().expect("index path");
    set_db_user_version(&db, 15);
    let mut session = CodeModeSession::new(config.clone());
    let err = session
        .call("search", json!({"query": "needle_unique_r1_xyz", "limit": 5}))
        .expect_err("stale schema search must refuse");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    session.call("index_status", json!({})).expect("writer migrates");
    assert_eq!(db_user_version(&db), 16, "writer must stamp current");

    let mut resumed = CodeModeSession::new(config);
    let out = resumed
        .call(
            "search",
            json!({"query": "needle_unique_r1_xyz", "format": "capsule", "limit": 5}),
        )
        .expect("post-migration search resumes");
    let hits = out["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "migrated rows must serve: {out}");
    assert!(
        hits.iter().any(|h| h["file"].as_str().unwrap_or("").contains("src/a.rs")),
        "{out}"
    );
}

#[test]
fn serve_stream_recovers_after_corrupt_records() {
    // A garbage line and a truncated-JSON line each fail closed as a
    // per-record `Error` with no id, and the stream RESUMES: surrounding
    // valid calls are answered and the session ends cleanly with `Bye`.
    // (batch.rs owns valid-id preservation on schema errors; this owns
    // non-JSON bytes plus resumption.)
    let temp = tempfile::tempdir().expect("tempdir");
    let config = config_for(temp.path(), None);
    let mut input = String::new();
    input.push_str(
        &serde_json::to_string(&json!({
            "type": "call", "id": "c1",
            "tool": "catalog_search", "args": {"query": "search"},
        }))
        .expect("c1"),
    );
    input.push('\n');
    input.push_str("{{{not json at all\x00\x01\n");
    input.push_str("{\"type\":\"call\",\"id\":\"t\",\"tool\":\"select\",\"args\":{\"value\":{\"a\":1},\"fields\":\n");
    input.push_str(
        &serde_json::to_string(&json!({
            "type": "call", "id": "c2",
            "tool": "catalog_search", "args": {"query": "chain"},
        }))
        .expect("c2"),
    );
    input.push('\n');
    input.push_str(&serde_json::to_string(&json!({"type": "end"})).expect("end"));
    input.push('\n');

    let mut out = Vec::new();
    run_serve(config, Cursor::new(input), &mut out).expect("serve survives corrupt lines");
    let text = String::from_utf8(out).expect("utf8");
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 5, "c1 + 2 errors + c2 + bye: {text}");
    let responses: Vec<ServeResponse> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("response json"))
        .collect();
    match &responses[0] {
        ServeResponse::Result { id, ok, value, .. } => {
            assert_eq!(id, "c1");
            assert!(ok);
            assert!(value.as_ref().expect("c1 value").get("tools").is_some());
        }
        other => panic!("c1 must be answered, got {other:?}"),
    }
    for (i, response) in responses[1..3].iter().enumerate() {
        match response {
            ServeResponse::Error { id, error } => {
                assert_eq!(*id, None, "corrupt line {i} carries no id");
                assert!(!error.is_empty());
            }
            other => panic!("corrupt line {i} must fail closed, got {other:?}"),
        }
    }
    match &responses[3] {
        ServeResponse::Result { id, ok, .. } => {
            assert_eq!(id, "c2");
            assert!(ok, "stream must resume after corrupt lines");
        }
        other => panic!("c2 must be answered, got {other:?}"),
    }
    assert!(matches!(responses[4], ServeResponse::Bye), "got {:?}", responses[4]);
}

#[test]
fn corrupt_writer_generation_stamp_fails_open_per_contract() {
    // The stamp is a staleness hint, not a gate: missing or unparsable
    // content reads as generation 0 (documented cold-start protocol) and
    // the session keeps serving instead of failing closed.
    let (_temp, config) = indexed_repo();
    let stamp = ast_sgrep_core::writer_generation_path(
        &config.root,
        config.index_path.as_deref(),
    );
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
            json!({"query": "needle_unique_r1_xyz", "format": "capsule", "limit": 5}),
        )
        .expect("session serves despite corrupt stamp");
    assert!(!out["hits"].as_array().expect("hits").is_empty(), "{out}");
    std::fs::remove_file(&stamp).expect("remove stamp");
    let out = session
        .call("search", json!({"query": "needle_unique_r1_xyz", "limit": 5}))
        .expect("session serves with stamp absent");
    assert!(!out["hits"].as_array().expect("hits").is_empty(), "{out}");
}

#[test]
fn index_repo_rebuilds_after_corrupt_index_is_removed() {
    // Operator recovery: corrupt bytes fail closed; once the bad file (and
    // any SQLite sidecars) is removed, a plain `index_repo` rebuilds and
    // the same session serves the token again, deterministically.
    let (_temp, config) = indexed_repo();
    let db = config.index_path.clone().expect("index path");
    std::fs::write(&db, "R1-NOT-SQLITE".repeat(400)).expect("corrupt db");
    let mut session = CodeModeSession::new(config);
    let err = session
        .call("index_status", json!({}))
        .expect_err("corrupt index must fail closed");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");

    std::fs::remove_file(&db).expect("remove corrupt db");
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = db.with_extension(format!("db{suffix}"));
        let _ = std::fs::remove_file(sidecar);
    }
    let rebuilt = session
        .call("index_repo", json!({"force": false}))
        .expect("rebuild after removal");
    assert_eq!(rebuilt["ok"], true);
    let first = session
        .call("search", json!({"query": "needle_unique_r1_xyz", "limit": 5}))
        .expect("search after rebuild");
    let second = session
        .call("search", json!({"query": "needle_unique_r1_xyz", "limit": 5}))
        .expect("search after rebuild twice");
    assert_eq!(first, second);
    let hits = first["hits"].as_array().expect("hits");
    assert!(
        hits.iter().any(|h| h["file"].as_str().unwrap_or("").contains("src/a.rs")),
        "{first}"
    );
}
