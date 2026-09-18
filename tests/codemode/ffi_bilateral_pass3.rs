//! F3 FFI bilateral-agreement tests for `ast-sgrep-codemode-napi` (pass 3).
//!
//! F1 pinned the export inventory, F2 the boundary validation. F3 proves the
//! NAPI wrapper and direct core agree on IDENTICAL fixtures: one tempdir root
//! plus one index db shared by a napi [`Session`] and a direct
//! [`CodeModeSession`], so byte-identical JSON is the assertion — not shape.
//!
//! * every fast-lookup tool: `Session::call_now` == `CodeModeSession::call`,
//!   byte-identical (`find` on indexed hits; `read` on a fixture window;
//!   `defs` / `callers` / `imports` / `index_status` / `catalog_search` /
//!   `catalog_describe`)
//! * `search` cache hit: napi `call_now` == core `take_cached_search`
//!   (byte-identical render, both bump); cold search falls through as
//!   Null / `None`, uncounted on both sides
//! * batch validation: napi `Session::batch` sync reasons == core `run_batch`
//!   `InvalidArgs` texts, byte-identical, for every violation class
//! * `call_count` parity after an identical mixed ok/error sequence
//! * error-case agreement: the same bad call fails on both sides, napi reason
//!   byte-identical to the core `Display` text, core discriminant pinned
//! * budget exhaustion: both sides fail the 10_001st call identically
//!
//! Honestly out of scope (require Node): `SessionCallTask::compute` and
//! `SessionBatchTask::compute` cannot be driven from a Rust test. Probed:
//! both structs hold private fields with no constructor, and napi-3
//! `AsyncTask` exposes only `new` / `with_signal` / `with_optional_signal` /
//! `on_abort` — no accessor for the inner task. So per-call ok/error shapes
//! of napi batch execution vs `run_batch` stay unpinned here; F3 pins the
//! validation layer both sides share instead.
//!
//! Link note (macOS): same as passes 1-2 —
//! `RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup" cargo test -p
//! ast-sgrep-codemode-napi --test ffi_bilateral_pass3`.

use ast_sgrep_codemode::{
    run_batch, BatchCall, BatchRequest, CallError, CodeModeSession, SessionConfig,
    MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_codemode_napi::{JsBatchCall, JsSessionConfig, Session};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::TempDir;

/// Napi wrapper + direct core on the SAME root and index db, with mirrored
/// config (limit 25, no embed), so outputs must be byte-identical.
fn pair(root: &Path, db: &str) -> (Session, CodeModeSession) {
    let index_path = root.join(db);
    let napi = Session::new(Some(JsSessionConfig {
        root: Some(root.display().to_string()),
        index_path: Some(index_path.display().to_string()),
        limit: Some(25),
        use_embed: Some(false),
    }))
    .expect("Session::new");
    let core = CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(index_path),
        limit: 25,
        use_embed: false,
        ..SessionConfig::default()
    });
    (napi, core)
}

/// `index_status` on both sides materializes the shared empty schema; each
/// side counts its own call.
fn materialize_both(napi: &Session, core: &mut CodeModeSession) {
    napi.call_now("index_status".to_string(), Some(json!({})))
        .expect("napi materialize");
    core.call("index_status", json!({})).expect("core materialize");
}

/// Byte-identity over canonical serialization, not just `Value` equality.
fn assert_byte_identical(napi_value: &Value, core_value: &Value, tool: &str) {
    let a = serde_json::to_vec(napi_value).expect("serialize napi");
    let b = serde_json::to_vec(core_value).expect("serialize core");
    assert_eq!(a, b, "{tool} bilateral mismatch");
}

fn write(root: &Path, name: &str, content: &str) {
    std::fs::write(root.join(name), content).expect("fixture");
}

// ---------------------------------------------------------------------------
// 1-2. find (indexed hits) + read (fixture window)
// ---------------------------------------------------------------------------

#[test]
fn find_on_indexed_hits_is_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    write(temp.path(), "a.rs", "fn alpha() {}\nfn beta() {}\n");
    write(temp.path(), "b.rs", "fn gamma() {}\n");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    // Indexing runs core-side (call_now gates slow tools by design); both
    // sides then read the same db through independent warm Searchers.
    core.call("index_repo", json!({})).expect("index fixture");
    let args = json!({"query": "alpha"});
    let napi_value = napi
        .call_now("find".to_string(), Some(args.clone()))
        .expect("napi find");
    let core_value = core.call("find", args).expect("core find");
    assert_byte_identical(&napi_value, &core_value, "find");
    // Non-vacuous: the hits path (ordering, excerpts) was exercised.
    assert!(napi_value["hit_count"].as_u64().unwrap_or(0) >= 1);
}

#[test]
fn read_on_fixture_window_is_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    write(temp.path(), "hello.rs", "fn alpha() {}\nfn beta() {}\n");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    materialize_both(&napi, &mut core);
    let args = json!({"path": "hello.rs", "start": 1, "end": 2});
    let napi_value = napi
        .call_now("read".to_string(), Some(args.clone()))
        .expect("napi read");
    let core_value = core.call("read", args).expect("core read");
    assert_byte_identical(&napi_value, &core_value, "read");
    assert_eq!(napi_value["ok"], json!(true));
}

// ---------------------------------------------------------------------------
// 3-5. defs / callers / imports capsules
// ---------------------------------------------------------------------------

#[test]
fn defs_capsule_is_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    materialize_both(&napi, &mut core);
    let args = json!({"symbol": "ZzzNoSuchSymbol"});
    let napi_value = napi
        .call_now("defs".to_string(), Some(args.clone()))
        .expect("napi defs");
    let core_value = core.call("defs", args).expect("core defs");
    assert_byte_identical(&napi_value, &core_value, "defs");
    assert_eq!(napi_value["mode"], json!("capsule"));
    assert_eq!(napi_value["hit_count"], json!(0));
}

#[test]
fn callers_capsule_is_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    materialize_both(&napi, &mut core);
    let args = json!({"symbol": "ZzzNoSuchSymbol"});
    let napi_value = napi
        .call_now("callers".to_string(), Some(args.clone()))
        .expect("napi callers");
    let core_value = core.call("callers", args).expect("core callers");
    assert_byte_identical(&napi_value, &core_value, "callers");
    assert_eq!(napi_value["mode"], json!("capsule"));
    assert_eq!(napi_value["hit_count"], json!(0));
}

#[test]
fn imports_capsule_is_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    materialize_both(&napi, &mut core);
    let args = json!({"module": "zzz_no_such_module"});
    let napi_value = napi
        .call_now("imports".to_string(), Some(args.clone()))
        .expect("napi imports");
    let core_value = core.call("imports", args).expect("core imports");
    assert_byte_identical(&napi_value, &core_value, "imports");
    assert_eq!(napi_value["mode"], json!("capsule"));
}

// ---------------------------------------------------------------------------
// 6-8. index_status + catalog tools
// ---------------------------------------------------------------------------

#[test]
fn index_status_is_byte_identical_on_shared_db() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    // Shared root + db, so root/index_path/counts embed identically.
    let napi_value = napi
        .call_now("index_status".to_string(), Some(json!({})))
        .expect("napi index_status");
    let core_value = core.call("index_status", json!({})).expect("core index_status");
    assert_byte_identical(&napi_value, &core_value, "index_status");
    assert_eq!(napi_value["file_count"], json!(0));
}

#[test]
fn catalog_search_is_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    let args = json!({"query": "search"});
    let napi_value = napi
        .call_now("catalog_search".to_string(), Some(args.clone()))
        .expect("napi catalog_search");
    let core_value = core.call("catalog_search", args).expect("core catalog_search");
    assert_byte_identical(&napi_value, &core_value, "catalog_search");
    assert!(!napi_value["tools"].as_array().expect("tools").is_empty());
}

#[test]
fn catalog_describe_is_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    let args = json!({"name": "search"});
    let napi_value = napi
        .call_now("catalog_describe".to_string(), Some(args.clone()))
        .expect("napi catalog_describe");
    let core_value = core
        .call("catalog_describe", args)
        .expect("core catalog_describe");
    assert_byte_identical(&napi_value, &core_value, "catalog_describe");
    assert_eq!(napi_value["name"], json!("search"));
}

// ---------------------------------------------------------------------------
// 9-10. search cache hit / cold fall-through
// ---------------------------------------------------------------------------

#[test]
fn search_cache_hit_agrees_and_counts_on_both() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    materialize_both(&napi, &mut core);
    // find("N") renders under key "word:N" on each side's own cache.
    let found_args = json!({"query": "needle_xyz"});
    let napi_found = napi
        .call_now("find".to_string(), Some(found_args.clone()))
        .expect("napi find");
    let core_found = core.call("find", found_args).expect("core find");
    assert_byte_identical(&napi_found, &core_found, "find");
    // Same cache key via napi call_now vs direct take_cached_search.
    let hit_args = json!({"query": "word:needle_xyz"});
    let napi_hit = napi
        .call_now("search".to_string(), Some(hit_args.clone()))
        .expect("napi cached search");
    let core_hit = core
        .take_cached_search(&hit_args)
        .expect("core cached search")
        .expect("cache hit");
    assert_byte_identical(&napi_hit, &core_hit, "search(hit)");
    assert_eq!(napi_hit, napi_found);
    // materialize + find + hit on each side.
    assert_eq!(napi.call_count(), 3);
    assert_eq!(core.call_count(), 3);
}

#[test]
fn search_cold_falls_through_uncounted_on_both() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    // No materialize: the cache probe touches no store on either side.
    let args = json!({"query": "zzz_unique_cold_needle"});
    assert_eq!(
        napi.call_now("search".to_string(), Some(args.clone()))
            .expect("napi cold search"),
        Value::Null
    );
    assert_eq!(core.take_cached_search(&args).expect("core cold probe"), None);
    assert_eq!(napi.call_count(), 0);
    assert_eq!(core.call_count(), 0);
}

// ---------------------------------------------------------------------------
// 11. batch validation: napi reasons == core run_batch reasons
// ---------------------------------------------------------------------------

fn run_batch_on(root: &Path, calls: Vec<BatchCall>) -> Result<(), CallError> {
    let config = SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(root.join("ffi_pass3.db")),
        limit: 25,
        use_embed: false,
        ..SessionConfig::default()
    };
    run_batch(
        config,
        &BatchRequest {
            root: None,
            index_path: None,
            use_embed: None,
            limit: None,
            parallel: None,
            parallel_mode: None,
            calls,
        },
    )
    .map(|_| ())
}

#[test]
fn batch_validation_reasons_agree_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, _) = pair(temp.path(), "ffi_pass3.db");
    // (napi calls, core calls): identical identities on both sides.
    let overcount = MAX_BATCH_CALLS + 1;
    let cases: Vec<(Vec<JsBatchCall>, Vec<BatchCall>)> = vec![
        (vec![], vec![]),
        (
            (0..overcount)
                .map(|i| JsBatchCall {
                    id: format!("id-{i}"),
                    tool: "search".into(),
                    args: None,
                })
                .collect(),
            (0..overcount)
                .map(|i| BatchCall {
                    id: format!("id-{i}"),
                    tool: "search".into(),
                    args: Value::Null,
                })
                .collect(),
        ),
        (
            vec![JsBatchCall {
                id: String::new(),
                tool: "search".into(),
                args: None,
            }],
            vec![BatchCall {
                id: String::new(),
                tool: "search".into(),
                args: Value::Null,
            }],
        ),
        (
            vec![JsBatchCall {
                id: "i".repeat(MAX_BATCH_ID_BYTES + 1),
                tool: "search".into(),
                args: None,
            }],
            vec![BatchCall {
                id: "i".repeat(MAX_BATCH_ID_BYTES + 1),
                tool: "search".into(),
                args: Value::Null,
            }],
        ),
        (
            vec![JsBatchCall {
                id: "a".into(),
                tool: String::new(),
                args: None,
            }],
            vec![BatchCall {
                id: "a".into(),
                tool: String::new(),
                args: Value::Null,
            }],
        ),
        (
            vec![JsBatchCall {
                id: "a".into(),
                tool: "t".repeat(MAX_BATCH_TOOL_BYTES + 1),
                args: None,
            }],
            vec![BatchCall {
                id: "a".into(),
                tool: "t".repeat(MAX_BATCH_TOOL_BYTES + 1),
                args: Value::Null,
            }],
        ),
    ];
    for (napi_calls, core_calls) in cases {
        let napi_err = napi
            .batch(napi_calls, None)
            .err()
            .expect("napi batch rejects");
        let core_err = run_batch_on(temp.path(), core_calls)
            .err()
            .expect("core run_batch rejects");
        assert!(matches!(core_err, CallError::InvalidArgs(_)));
        assert_eq!(napi_err.reason, core_err.to_string());
    }
    assert_eq!(napi.call_count(), 0);
}

// ---------------------------------------------------------------------------
// 12. call_count parity after an identical mixed sequence
// ---------------------------------------------------------------------------

#[test]
fn call_count_matches_after_identical_mixed_sequence() {
    let temp = TempDir::new().expect("tempdir");
    write(temp.path(), "hello.rs", "fn alpha() {}\n");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    materialize_both(&napi, &mut core);
    let steps: &[(&str, Value, bool)] = &[
        ("find", json!({"query": "zzz_no_such_needle"}), true),
        ("defs", json!({"symbol": "ZzzNoSuch"}), true),
        ("catalog_describe", json!({"name": "search"}), true),
        (
            "read",
            json!({"path": "hello.rs", "start": 1, "end": 1}),
            true,
        ),
        ("defs", json!({}), false),
    ];
    for (tool, args, ok) in steps {
        let napi_result = napi.call_now(tool.to_string(), Some(args.clone()));
        let core_result = core.call(tool, args.clone());
        assert_eq!(napi_result.is_ok(), *ok, "{tool} napi");
        assert_eq!(core_result.is_ok(), *ok, "{tool} core");
    }
    // materialize + 5 dispatched steps (failures bump on both sides).
    assert_eq!(napi.call_count(), 6);
    assert_eq!(core.call_count(), 6);
}

// ---------------------------------------------------------------------------
// 13-14. error-case agreement: invalid args + budget exhaustion
// ---------------------------------------------------------------------------

#[test]
fn invalid_args_fail_on_both_with_identical_reasons() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    // (tool, args, core discriminant is InvalidArgs? find/read use Other).
    let cases: &[(&str, Value, bool)] = &[
        ("defs", json!({}), true),
        ("callers", json!({}), true),
        ("imports", json!({}), true),
        ("catalog_search", json!({}), true),
        ("catalog_describe", json!({}), true),
        ("catalog_describe", json!({"name": "zzz_no_such_catalog_tool"}), true),
        ("find", json!({}), false),
        ("read", json!({}), false),
    ];
    for (tool, args, invalid_args) in cases {
        let core_err = core.call(tool, args.clone()).expect_err("core fails");
        assert_eq!(
            matches!(core_err, CallError::InvalidArgs(_)),
            *invalid_args,
            "{tool} discriminant"
        );
        let napi_err = napi
            .call_now(tool.to_string(), Some(args.clone()))
            .expect_err("napi fails");
        assert_eq!(napi_err.reason, core_err.to_string(), "{tool} reason");
    }
    // Every marshalled failure bumped both budgets.
    assert_eq!(napi.call_count(), cases.len() as u32);
    assert_eq!(core.call_count(), cases.len());
}

#[test]
fn budget_exhaustion_fails_identically_on_both() {
    let temp = TempDir::new().expect("tempdir");
    let (napi, mut core) = pair(temp.path(), "ffi_pass3.db");
    // Napi sessions run with max_calls = 10_000; mirror it core-side.
    core.max_calls = 10_000;
    let args = json!({"name": "search"});
    for _ in 0..10_000 {
        napi.call_now("catalog_describe".to_string(), Some(args.clone()))
            .expect("napi within budget");
        core.call("catalog_describe", args.clone())
            .expect("core within budget");
    }
    assert_eq!(napi.call_count(), 10_000);
    assert_eq!(core.call_count(), 10_000);
    let core_err = core
        .call("catalog_describe", args.clone())
        .expect_err("core exhausted");
    assert!(matches!(core_err, CallError::BudgetExhausted(10_000)));
    let napi_err = napi
        .call_now("catalog_describe".to_string(), Some(args))
        .expect_err("napi exhausted");
    assert_eq!(napi_err.reason, core_err.to_string());
    assert_eq!(napi.call_count(), 10_000);
    assert_eq!(core.call_count(), 10_000);
}
