//! NAPI export surface + async construction contract for
//! `ast-sgrep-codemode-napi`.
//!
//! Pins what exists and how construction validates, all drivable from Rust
//! without a JS `Env`: addon identity markers, `Session::new` config mapping,
//! fast-lookup contract shapes, and `call`/`batch` construction with the
//! synchronous identity-only batch validation.
//!
//! Honestly out of scope (require Node): `SessionCallTask::compute` /
//! `SessionBatchTask::compute` hold private fields with no accessor, so async
//! execution is unobservable here — only construction (`Ok` discriminant) and
//! the sync validation reasons are pinned.
//!
//! Link note: node symbols stay unresolved via the napi crate's build.rs
//! (tests never call Node FFI); plain `cargo test -p
//! ast-sgrep-codemode-napi` works on macOS/Linux.

use ast_sgrep_codemode::{
    SessionConfig, MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_codemode_napi::{
    async_api_version, binding_version, is_native, JsSessionConfig, Session,
};
use ast_sgrep_testkit::file_tree;
use serde_json::json;
use std::path::Path;

// `shared` serves all four FFI suites; each target uses a subset, so the
// per-target dead-code lint would fire on siblings' helpers. The allow lives
// on each suite's opt-in, not in the shared file.
#[allow(dead_code)]
#[path = "ffi_shared.rs"]
mod shared;
use shared::{empty_root, js_call, materialize, session_on};

/// INTENT: pin the addon identity markers Pi verifies against the extension
/// contract. KILLS: constant-change (`async_api_version`), version drift.
#[test]
fn identity_markers_pin_binding_contract() {
    assert_eq!(binding_version(), env!("CARGO_PKG_VERSION"));
    assert!(!binding_version().is_empty());
    assert!(is_native());
    assert_eq!(async_api_version(), 1);
}

/// INTENT: `Session::new` maps `None`/defaults plus every config field
/// faithfully; out-of-range limits clamp instead of rejecting; the custom
/// `index_path` flows through to `index_status` (not the default home).
/// KILLS: mapping-swap, clamp-drop, default-path fallback.
#[test]
fn session_construction_maps_every_config_field() {
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

    // index_path + limit + use_embed: accepted, session stays usable.
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

    // Limit clamp edges (0 and 600 are outside 1..=500): into_rust clamps, so
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

    // Distinct from the default `index.db` so the pin proves *our* path flowed
    // through instead of the default index home.
    let probe = temp.path().join("ffi_probe_custom.db");
    let session = Session::new(Some(JsSessionConfig {
        root: Some(root.clone()),
        index_path: Some(probe.display().to_string()),
        limit: None,
        use_embed: Some(false),
    }))
    .expect("Session::new");
    let status = session
        .call_now("index_status".to_string(), None)
        .expect("index_status");
    // Tools canonicalize the root (tempdir may sit under a symlinked /var).
    assert_eq!(
        status["root"],
        json!(temp
            .path()
            .canonicalize()
            .expect("canonicalize")
            .display()
            .to_string())
    );
    // Exact-equality would be symlink-fragile; file-name pins the mapping
    // without coupling to canonicalization internals.
    let reported = status["index_path"]
        .as_str()
        .expect("index_path is a string");
    assert_eq!(Path::new(reported).file_name(), probe.file_name());
    assert_eq!(session.call_count(), 1);
    // Empty root, fresh db: zero rows across every table.
    assert_eq!(status["file_count"], json!(0));
}

/// INTENT: every fast lookup renders its contract shape — catalog tools,
/// fail-closed-then-capsule symbol lookups, and exact-window `read` bytes.
/// KILLS: dispatch-swap, shape-regression, gate-reorder, count-on-fail-swap,
/// path/window-slice-swap, start-ignored-swap.
#[test]
fn fast_lookups_render_contract_shapes() {
    // Catalog tools: search finds "search"; describe returns name + nonempty
    // description.
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_surface.db");
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
    assert!(described["description"]
        .as_str()
        .is_some_and(|d| !d.is_empty()));
    assert_eq!(session.call_count(), 2);

    // Before any db exists the readonly Searcher fails closed (discriminant
    // only: the reason text is core's, not the NAPI contract). Still counts:
    // bump precedes dispatch.
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_surface.db");
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

    // `read` serves exact window bytes: first-lines window plus a middle
    // window over a second fixture (the start-ignored mutant needs both).
    let temp = file_tree(&[
        ("hello.rs", "fn alpha() {}\nfn beta() {}\n"),
        ("tri.rs", "line_one\nline_two\nline_three\n"),
    ]);
    let session = session_on(temp.path(), "ffi_surface.db");
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
    let value = session
        .call_now(
            "read".to_string(),
            Some(json!({"path": "tri.rs", "start": 2, "end": 3})),
        )
        .expect("read middle window");
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["count"], json!(1));
    assert_eq!(value["windows"][0]["path"], json!("tri.rs"));
    assert_eq!(value["windows"][0]["text"], json!("line_two\nline_three"));
    assert_eq!(session.call_count(), 3);
}

/// INTENT: `call`/`batch` construct without `Env`, and batch sync validation
/// is identity-only — exact reasons, exclusive boundaries, byte semantics,
/// first-violation ordering — while semantic failures (unknown tools, bad
/// args, duplicates) always construct `Ok` for libuv to sort out.
/// KILLS: construction-reject-regression, reason-change, missing-check,
/// off-by-one (`>`→`>=`), bytes-vs-chars-swap, check-reorder,
/// first-only-scan, over-validation (semantic checks at construction).
#[test]
fn async_construction_validates_identity_only() {
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_surface.db");

    // call() performs NO sync validation: unknown, empty, and absurdly long
    // tools all construct Ok; arg defaulting needs no Env either.
    for tool in [
        "search".to_string(),
        "zzz_no_such_tool".to_string(),
        String::new(),
        "t".repeat(10_000),
    ] {
        assert!(session
            .call(tool, Some(json!({"query": "q"})), None)
            .map(|_| ())
            .is_ok());
    }
    assert!(session
        .call("search".to_string(), None, None)
        .map(|_| ())
        .is_ok());
    assert!(session
        .call("defs".to_string(), Some(json!({"symbol": "Main"})), None)
        .map(|_| ())
        .is_ok());

    // batch() with valid + unknown-tool + invalid-args calls constructs Ok:
    // semantic failures surface per-call inside compute() on libuv.
    let task = session
        .batch(
            vec![
                js_call("ok", "catalog_describe", Some(json!({"name": "search"}))),
                js_call("bad-tool", "zzz_no_such_tool", None),
                js_call("bad-args", "defs", Some(json!({}))),
            ],
            None,
        )
        .expect("mixed-semantics batch constructs");
    drop(task);

    // Identity-only scope: duplicate ids, whitespace ids/tools, and non-object
    // args JSON all construct Ok.
    let task = session
        .batch(
            vec![
                js_call("dup", "search", Some(json!(42))),
                js_call("dup", "find", Some(json!([1, 2, {"nested": true}]))),
                js_call(" ", " ", None),
            ],
            None,
        )
        .expect("identity-only batch constructs");
    drop(task);

    // All 6 violation classes rejected with exact reasons (canonical text).
    let err = session
        .batch(vec![], None)
        .err()
        .expect("empty batch rejected");
    assert_eq!(err.reason, "batch.calls must be non-empty");
    let err = session
        .batch(vec![js_call("", "search", None)], None)
        .err()
        .expect("empty id rejected");
    assert_eq!(err.reason, "batch call id must be non-empty");
    let err = session
        .batch(vec![js_call("a", "", None)], None)
        .err()
        .expect("empty tool rejected");
    assert_eq!(err.reason, "batch call tool must be non-empty");
    let err = session
        .batch(
            vec![js_call(&"x".repeat(MAX_BATCH_ID_BYTES + 1), "search", None)],
            None,
        )
        .err()
        .expect("oversize id rejected");
    assert_eq!(
        err.reason,
        format!("batch call id exceeds {MAX_BATCH_ID_BYTES} bytes")
    );
    let err = session
        .batch(
            vec![js_call("a", &"t".repeat(MAX_BATCH_TOOL_BYTES + 1), None)],
            None,
        )
        .err()
        .expect("oversize tool rejected");
    assert_eq!(
        err.reason,
        format!("batch call tool exceeds {MAX_BATCH_TOOL_BYTES} bytes")
    );
    let calls: Vec<_> = (0..MAX_BATCH_CALLS + 1)
        .map(|i| js_call(&format!("id-{i}"), "search", None))
        .collect();
    let err = session
        .batch(calls, None)
        .err()
        .expect("oversize batch rejected");
    assert_eq!(
        err.reason,
        format!("batch.calls exceeds max {MAX_BATCH_CALLS}")
    );

    // Exact-max boundaries construct: every `>` comparison is exclusive.
    let mut calls: Vec<_> = (0..MAX_BATCH_CALLS - 1)
        .map(|i| js_call(&format!("id-{i}"), "search", None))
        .collect();
    calls.push(js_call(
        &"i".repeat(MAX_BATCH_ID_BYTES),
        &"t".repeat(MAX_BATCH_TOOL_BYTES),
        Some(json!({"query": "q"})),
    ));
    assert_eq!(calls.len(), MAX_BATCH_CALLS);
    let task = session
        .batch(calls, None)
        .expect("exact-max batch constructs");
    drop(task);

    // Limits count bytes, not chars: 'é' is 2 bytes, so 64 are exactly 128
    // bytes (accepted) and 65 are 130 bytes (rejected). Pins 128 literally.
    assert_eq!(MAX_BATCH_ID_BYTES, 128);
    assert_eq!(MAX_BATCH_TOOL_BYTES, 128);
    let ok_task = session
        .batch(vec![js_call(&"é".repeat(64), "search", None)], None)
        .expect("128-byte multibyte id constructs");
    drop(ok_task);
    let err = session
        .batch(vec![js_call(&"é".repeat(65), "search", None)], None)
        .err()
        .expect("130-byte multibyte id rejected");
    assert_eq!(
        err.reason,
        format!("batch call id exceeds {MAX_BATCH_ID_BYTES} bytes")
    );
    let err = session
        .batch(vec![js_call("a", &"é".repeat(65), None)], None)
        .err()
        .expect("130-byte multibyte tool rejected");
    assert_eq!(
        err.reason,
        format!("batch call tool exceeds {MAX_BATCH_TOOL_BYTES} bytes")
    );

    // First-violation ordering: overcount wins over per-call violations (the
    // len check precedes the loop), and the scan covers the whole list.
    let mut calls: Vec<_> = (0..MAX_BATCH_CALLS + 1)
        .map(|i| js_call(&format!("id-{i}"), "search", None))
        .collect();
    calls[0].id.clear();
    let err = session
        .batch(calls, None)
        .err()
        .expect("overcount + empty id rejected");
    assert_eq!(
        err.reason,
        format!("batch.calls exceeds max {MAX_BATCH_CALLS}")
    );
    let err = session
        .batch(
            vec![js_call("ok", "search", None), js_call("second", "", None)],
            None,
        )
        .err()
        .expect("second-call violation rejected");
    assert_eq!(err.reason, "batch call tool must be non-empty");

    // Construction alone never executes: count untouched throughout.
    assert_eq!(session.call_count(), 0);
}
