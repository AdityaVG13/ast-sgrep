//! F1 FFI export-inventory tests for `ast-sgrep-codemode-napi` (pass 1).
//!
//! Pins every `#[napi]` export drivable from Rust without a JS `Env`:
//!
//! * identity markers: [`binding_version`], [`is_native`], [`async_api_version`]
//! * [`Session::new`] config mapping (`root` / `index_path` / `limit` / `use_embed`)
//! * [`Session::call_now`] fast-lookup success, search cache hit/miss, slow-tool
//!   rejection reasons, invalid-args counting, contention behavior
//! * [`Session::call`] / [`Session::batch`] construction without `Env` plus the
//!   synchronous batch-validation reasons
//! * struct field roundtrips for [`JsSessionConfig`], [`JsBatchCall`],
//!   [`JsBatchCallResult`], [`JsBatchResponse`]
//!
//! Honestly out of scope here (require Node): `AsyncTask` execution —
//! `SessionCallTask::compute` / `SessionBatchTask::compute` hold private fields
//! and `AsyncTask` exposes no accessor, so `compute()` / `resolve()` cannot be
//! driven from a Rust test. The async tests below pin construction (`Ok`
//! discriminant) and the synchronous validation reasons only.
//!
//! Link note (macOS): `napi-sys` builds without `dyn-symbols` here, so the test
//! binary needs the same flag `napi-build` already applies to the cdylib:
//! `RUSTFLAGS="-C link-arg=-Wl,-undefined,dynamic_lookup" cargo test -p
//! ast-sgrep-codemode-napi --test ffi_bilateral_pass1`. Tests never call Node
//! FFI, so the lazily-undefined symbols never resolve at runtime.

use ast_sgrep_codemode::{
    SessionConfig, MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_codemode_napi::{
    async_api_version, binding_version, is_native, JsBatchCall, JsBatchCallResult,
    JsBatchResponse, JsSessionConfig, Session,
};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::TempDir;

/// Fresh empty workspace root; sessions stay lazy (no Searcher opens here).
fn empty_root() -> TempDir {
    TempDir::new().expect("tempdir")
}

fn session_on(root: &Path) -> Session {
    // Explicit temp index_path keeps every test hermetic: with None, the first
    // store-touching call would materialize a db under the real index home.
    Session::new(Some(JsSessionConfig {
        root: Some(root.display().to_string()),
        index_path: Some(root.join("ffi_pass1.db").display().to_string()),
        limit: None,
        use_embed: Some(false),
    }))
    .expect("Session::new")
}

/// `index_status` opens the store writable, materializing an empty schema so
/// later readonly opens (find/search/read/defs) serve zero-hit results instead
/// of the fail-closed "index is empty" gate.
fn materialize(session: &Session) {
    session
        .call_now("index_status".to_string(), None)
        .expect("materialize empty schema");
}

/// `Session::new` stores `cfg.root.display()` verbatim, so the canonical root
/// the tools resolve must equal the canonical tempdir.
fn canonical(path: &Path) -> String {
    path.canonicalize()
        .expect("canonicalize")
        .display()
        .to_string()
}

// ---------------------------------------------------------------------------
// 1. Identity markers
// ---------------------------------------------------------------------------

#[test]
fn identity_markers_are_exact() {
    assert_eq!(binding_version(), env!("CARGO_PKG_VERSION"));
    assert!(!binding_version().is_empty());
    assert!(is_native());
    assert_eq!(async_api_version(), 1);
}

// ---------------------------------------------------------------------------
// 2-3. Session::new config mapping
// ---------------------------------------------------------------------------

#[test]
fn session_new_none_and_each_config_field_map() {
    // None: workspace default root, zero count.
    let session = Session::new(None).expect("Session::new(None)");
    assert_eq!(
        session.root(),
        SessionConfig::default().root.display().to_string()
    );
    assert_eq!(session.call_count(), 0);

    let temp = empty_root();
    let root = temp.path().display().to_string();

    // root: reflected verbatim by the root() getter.
    let session = Session::new(Some(JsSessionConfig {
        root: Some(root.clone()),
        index_path: None,
        limit: None,
        use_embed: None,
    }))
    .expect("root config");
    assert_eq!(session.root(), root);
    assert_eq!(session.call_count(), 0);

    // index_path + use_embed: accepted, session stays usable.
    let db = temp.path().join("custom.db");
    let session = Session::new(Some(JsSessionConfig {
        root: Some(root.clone()),
        index_path: Some(db.display().to_string()),
        limit: Some(25),
        use_embed: Some(true),
    }))
    .expect("full config");
    assert_eq!(session.root(), root);
    assert_eq!(session.call_count(), 0);

    // limit clamp edges (0 and 600 are outside 1..=500): into_rust clamps, so
    // construction and a first tool call must still succeed. Explicit temp
    // index_path per edge keeps the materialized db out of the real index home.
    for edge in [0u32, 1, 500, 600] {
        let session = Session::new(Some(JsSessionConfig {
            root: Some(root.clone()),
            index_path: Some(
                temp.path()
                    .join(format!("clamp-{edge}.db"))
                    .display()
                    .to_string(),
            ),
            limit: Some(edge),
            use_embed: Some(false),
        }))
        .expect("clamp-edge config");
        assert_eq!(session.root(), root);
        let status = session
            .call_now("index_status".to_string(), None)
            .expect("index_status with clamp-edge limit");
        assert!(status.is_object(), "edge {edge}: {status:?}");
    }
}

#[test]
fn session_new_index_path_flows_to_status() {
    let temp = empty_root();
    // Distinct from the default `index.db` so the pin proves *our* path flowed
    // through instead of the default index home.
    let db = temp.path().join("ffi_probe_custom.db");
    let session = Session::new(Some(JsSessionConfig {
        root: Some(temp.path().display().to_string()),
        index_path: Some(db.display().to_string()),
        limit: None,
        use_embed: Some(false),
    }))
    .expect("Session::new");
    let status = session
        .call_now("index_status".to_string(), None)
        .expect("index_status");
    // tools canonicalize the root (tempdir may sit under a symlinked /var).
    assert_eq!(status["root"], json!(canonical(temp.path())));
    // Exact-equality would be symlink-fragile; parent-dir + file-name pins the
    // mapping without coupling to canonicalization internals.
    let reported = status["index_path"]
        .as_str()
        .expect("index_path is a string");
    assert_eq!(Path::new(reported).file_name(), db.file_name());
    assert_eq!(session.call_count(), 1);
    // Empty root, fresh db: zero rows across every table.
    assert_eq!(status["file_count"], json!(0));
}

// ---------------------------------------------------------------------------
// 4-5. call_now fast-lookup success
// ---------------------------------------------------------------------------

#[test]
fn call_now_catalog_tools_return_contract_shapes() {
    let temp = empty_root();
    let session = session_on(temp.path());

    let found = session
        .call_now(
            "catalog_search".to_string(),
            Some(json!({"query": "search"})),
        )
        .expect("catalog_search");
    let tools = found["tools"].as_array().expect("tools array");
    assert!(!tools.is_empty());
    assert!(tools.iter().any(|t| t["name"] == json!("search")));
    assert_eq!(found["summary"]["surface"], json!("codemode"));

    let described = session
        .call_now(
            "catalog_describe".to_string(),
            Some(json!({"name": "search"})),
        )
        .expect("catalog_describe");
    assert_eq!(described["name"], json!("search"));
    assert!(described["description"].as_str().is_some_and(|d| !d.is_empty()));
    assert_eq!(session.call_count(), 2);
}

#[test]
fn call_now_fast_lookups_succeed_on_empty_root() {
    let temp = empty_root();
    let session = session_on(temp.path());

    // Before any db exists the readonly Searcher fails closed (discriminant
    // only: the reason text is core's, not the NAPI contract). Still counts:
    // bump precedes dispatch.
    assert!(session
        .call_now("find".to_string(), Some(json!({"query": "x"})))
        .is_err());
    assert_eq!(session.call_count(), 1);

    let status = session
        .call_now("index_status".to_string(), None)
        .expect("index_status");
    assert!(status.is_object());
    assert_eq!(status["file_count"], json!(0));

    // Every symbol/lexical fast lookup renders a capsule, even with zero hits.
    let cases = [
        ("find", json!({"query": "zzz_no_such_needle"})),
        ("defs", json!({"symbol": "ZzzNoSuchSymbol"})),
        ("callers", json!({"symbol": "ZzzNoSuchSymbol"})),
        ("imports", json!({"module": "zzz_no_such_module"})),
    ];
    for (tool, args) in cases {
        let value = session
            .call_now(tool.to_string(), Some(args))
            .unwrap_or_else(|e| panic!("{tool}: {}", e.reason));
        assert_eq!(value["mode"], json!("capsule"), "{tool}: {value:?}");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(value["hit_count"], json!(hits.len()));
    }
    assert_eq!(session.call_count(), 1 + 1 + 4);
}

#[test]
fn call_now_read_serves_fixture_window() {
    let temp = empty_root();
    std::fs::write(temp.path().join("hello.rs"), "fn alpha() {}\nfn beta() {}\n")
        .expect("fixture");
    let session = session_on(temp.path());
    materialize(&session);
    let value = session
        .call_now(
            "read".to_string(),
            Some(json!({"path": "hello.rs", "start": 1, "end": 2})),
        )
        .expect("read");
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["count"], json!(1));
    let text = value["windows"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains("fn alpha"), "windows: {:?}", value["windows"]);
    assert_eq!(session.call_count(), 2);
}

// ---------------------------------------------------------------------------
// 6-7. call_now search cache miss / hit
// ---------------------------------------------------------------------------

#[test]
fn call_now_search_cold_returns_null_without_counting() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // Unique search stays on call()/libuv: Null fall-through, no budget bump.
    let value = session
        .call_now(
            "search".to_string(),
            Some(json!({"query": "zzz_unique_cold_needle"})),
        )
        .expect("cold search");
    assert_eq!(value, Value::Null);
    assert_eq!(session.call_count(), 0);
}

#[test]
fn call_now_search_hit_after_find_returns_identical_render() {
    let temp = empty_root();
    let session = session_on(temp.path());
    materialize(&session);
    // find("N") renders under key "word:N"; the same key via search() must hit
    // take_cached_search and return the identical render.
    let found = session
        .call_now("find".to_string(), Some(json!({"query": "needle_xyz"})))
        .expect("find");
    assert_eq!(session.call_count(), 2);
    let hit = session
        .call_now(
            "search".to_string(),
            Some(json!({"query": "word:needle_xyz"})),
        )
        .expect("cached search");
    assert!(hit.is_object());
    assert_eq!(hit, found);
    assert_eq!(session.call_count(), 3);
}

// ---------------------------------------------------------------------------
// 8-9. call_now rejection + counting
// ---------------------------------------------------------------------------

const CALL_NOW_ONLY: &str = "callNow is only for bounded metadata/symbol lookups; use call() for search/index/semantic/chain";

#[test]
fn call_now_slow_and_unknown_tools_rejected_with_contract_reason() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // Slow tools stay on call(); unknown tools hit the fast gate before dispatch.
    for tool in [
        "index_repo",
        "semantic",
        "chain",
        "edit",
        "filter_hits",
        "select",
        "zzz_no_such_tool",
    ] {
        let err = session
            .call_now(tool.to_string(), Some(json!({})))
            .expect_err(&format!("{tool} rejected"));
        assert_eq!(err.reason, CALL_NOW_ONLY, "{tool}");
    }
    // Gate precedes the budget bump: rejections never count.
    assert_eq!(session.call_count(), 0);
}

#[test]
fn call_now_invalid_args_errors_but_still_counts() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // Fast tool, missing symbol: discriminant only (reason text is owned by
    // codemode's CallError, not the NAPI contract).
    let err = session
        .call_now("defs".to_string(), Some(json!({})))
        .expect_err("defs without symbol");
    assert!(!err.reason.is_empty());
    // bump_call precedes dispatch, so even failures mirror into call_count.
    assert_eq!(session.call_count(), 1);
}

// ---------------------------------------------------------------------------
// 10. call_now contention: only documented failure
// ---------------------------------------------------------------------------

#[test]
fn call_now_concurrent_failures_use_only_busy_reason() {
    let temp = empty_root();
    let session = session_on(temp.path());
    const THREADS: usize = 8;
    const ITERS: usize = 25;
    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|| {
                for _ in 0..ITERS {
                    match session.call_now(
                        "catalog_describe".to_string(),
                        Some(json!({"name": "search"})),
                    ) {
                        Ok(value) => assert_eq!(value["name"], json!("search")),
                        Err(err) => assert_eq!(err.reason, "session is busy"),
                    }
                }
            });
        }
    });
    // Contended losers never bump (try_lock precedes session.call).
    assert!(session.call_count() <= (THREADS * ITERS) as u32);
}

// ---------------------------------------------------------------------------
// 11-12. Async construction + batch validation (sync, no Env)
// ---------------------------------------------------------------------------

#[test]
fn async_call_and_batch_construct_without_env() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // Construction + arg defaulting need no JS Env; execution stays on libuv.
    assert!(session
        .call("search".to_string(), None, None)
        .map(|_| ())
        .is_ok());
    assert!(session
        .call(
            "defs".to_string(),
            Some(json!({"symbol": "Main"})),
            None
        )
        .map(|_| ())
        .is_ok());
    let task = session
        .batch(
            vec![JsBatchCall {
                id: "a".to_string(),
                tool: "catalog_describe".to_string(),
                args: Some(json!({"name": "search"})),
            }],
            None,
        )
        .expect("batch constructs");
    drop(task);
    // Construction alone never executes: count untouched.
    assert_eq!(session.call_count(), 0);
}

#[test]
fn batch_validation_reasons_are_exact_and_synchronous() {
    let temp = empty_root();
    let session = session_on(temp.path());
    let err = session.batch(vec![], None).err().expect("empty batch rejected");
    assert_eq!(err.reason, "batch.calls must be non-empty");

    let err = session
        .batch(
            vec![JsBatchCall {
                id: String::new(),
                tool: "search".to_string(),
                args: None,
            }],
            None,
        )
        .err().expect("empty id rejected");
    assert_eq!(err.reason, "batch call id must be non-empty");

    let err = session
        .batch(
            vec![JsBatchCall {
                id: "a".to_string(),
                tool: String::new(),
                args: None,
            }],
            None,
        )
        .err().expect("empty tool rejected");
    assert_eq!(err.reason, "batch call tool must be non-empty");

    let err = session
        .batch(
            vec![JsBatchCall {
                id: "x".repeat(MAX_BATCH_ID_BYTES + 1),
                tool: "search".to_string(),
                args: None,
            }],
            None,
        )
        .err().expect("oversize id rejected");
    assert_eq!(
        err.reason,
        format!("batch call id exceeds {MAX_BATCH_ID_BYTES} bytes")
    );

    let err = session
        .batch(
            vec![JsBatchCall {
                id: "a".to_string(),
                tool: "t".repeat(MAX_BATCH_TOOL_BYTES + 1),
                args: None,
            }],
            None,
        )
        .err().expect("oversize tool rejected");
    assert_eq!(
        err.reason,
        format!("batch call tool exceeds {MAX_BATCH_TOOL_BYTES} bytes")
    );

    let calls: Vec<JsBatchCall> = (0..MAX_BATCH_CALLS + 1)
        .map(|i| JsBatchCall {
            id: format!("id-{i}"),
            tool: "search".to_string(),
            args: None,
        })
        .collect();
    let err = session.batch(calls, None).err().expect("oversize batch rejected");
    assert_eq!(err.reason, format!("batch.calls exceeds max {MAX_BATCH_CALLS}"));
    assert_eq!(session.call_count(), 0);
}

// ---------------------------------------------------------------------------
// 13. Struct field roundtrips
// ---------------------------------------------------------------------------

#[test]
fn napi_object_struct_fields_roundtrip() {
    let cfg = JsSessionConfig {
        root: Some("/tmp/r".to_string()),
        index_path: Some("/tmp/r/index.db".to_string()),
        limit: Some(42),
        use_embed: Some(true),
    };
    assert_eq!(cfg.root.as_deref(), Some("/tmp/r"));
    assert_eq!(cfg.index_path.as_deref(), Some("/tmp/r/index.db"));
    assert_eq!(cfg.limit, Some(42));
    assert_eq!(cfg.use_embed, Some(true));

    let none_cfg = JsSessionConfig {
        root: None,
        index_path: None,
        limit: None,
        use_embed: None,
    };
    assert!(none_cfg.root.is_none());
    assert!(none_cfg.use_embed.is_none());

    let call = JsBatchCall {
        id: "k".to_string(),
        tool: "search".to_string(),
        args: Some(json!({"query": "q"})),
    };
    assert_eq!(call.id, "k");
    assert_eq!(call.tool, "search");
    assert_eq!(call.args, Some(json!({"query": "q"})));

    let result = JsBatchCallResult {
        id: "k".to_string(),
        ok: true,
        value: Some(json!({"hits": []})),
        error: None,
    };
    assert_eq!(result.id, "k");
    assert!(result.ok);
    assert_eq!(result.value, Some(json!({"hits": []})));
    assert!(result.error.is_none());

    let response = JsBatchResponse {
        all_ok: true,
        results: vec![],
        call_count: 3,
        wall_ms: 7,
        mode: "serial-napi".to_string(),
    };
    assert!(response.all_ok);
    assert!(response.results.is_empty());
    assert_eq!(response.call_count, 3);
    assert_eq!(response.wall_ms, 7);
    assert_eq!(response.mode, "serial-napi");
}
