//! F2 FFI boundary-validation + error-marshalling tests for
//! `ast-sgrep-codemode-napi` (pass 2).
//!
//! F1 pinned the export inventory and happy paths. F2 proves the boundary
//! VALIDATES and MARSHALS errors:
//!
//! * [`Session::batch`] sync validation: exact-max boundaries construct,
//!   byte-vs-char limit semantics, first-violation ordering, identity-only
//!   scope (duplicates / whitespace / odd args JSON are NOT rejected)
//! * [`Session::call`] performs NO sync validation (unknown/empty/huge tools
//!   construct `Ok`; execution stays on libuv, unreachable without Node)
//! * [`Session::call_now`] fast/slow partition is exact over all 15 catalog
//!   tools plus aliases (the fast gate is literal, not `ToolName::parse`)
//! * `call_now` slow-tool and busy reasons are exact constants
//! * `call_now` invalid-args reasons are the exact `CallError` contract texts,
//!   byte-identical to what core `CodeModeSession::call` produces (marshalling
//!   fidelity), and still bump the budget
//! * unknown tools never reach dispatch through `call_now` (gate precedes
//!   dispatch); the core `UnknownTool` taxonomy is pinned directly against
//!   `CodeModeSession` to prove the two layers differ by design
//! * unicode / empty / huge string args: no panic, no truncation; huge fails
//!   closed naming the `MAX_QUERY_CHARS` limit
//! * invalid root configs: `Session::new` is lazy/infallible (stores the root
//!   verbatim), so failure surfaces at first use with documented reason
//!   prefixes — there is no construction-time rejection to pin
//!
//! Link note (macOS): same as pass 1 —
//! `RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup" cargo test -p
//! ast-sgrep-codemode-napi --test ffi_bilateral_pass2`.

use ast_sgrep_codemode::{
    CallError, CodeModeSession, SessionConfig, MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES,
    MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_codemode_napi::{JsBatchCall, JsSessionConfig, Session};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tempfile::TempDir;

/// `ast-sgrep-core/src/limits.rs::MAX_QUERY_CHARS` (core is not a direct
/// dev-dep of the napi crate, so the value is pinned literally here).
const MAX_QUERY_CHARS: usize = 4096;

const CALL_NOW_ONLY: &str = "callNow is only for bounded metadata/symbol lookups; use call() for search/index/semantic/chain";
const BUSY: &str = "session is busy";

/// Fresh empty workspace root; sessions stay lazy (no Searcher opens here).
fn empty_root() -> TempDir {
    TempDir::new().expect("tempdir")
}

fn session_on(root: &Path) -> Session {
    // Explicit temp index_path keeps every test hermetic: with None, the first
    // store-touching call would materialize a db under the real index home.
    Session::new(Some(JsSessionConfig {
        root: Some(root.display().to_string()),
        index_path: Some(root.join("ffi_pass2.db").display().to_string()),
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

// ---------------------------------------------------------------------------
// 1-3. batch() sync validation: boundaries, byte semantics, ordering
// ---------------------------------------------------------------------------

#[test]
fn batch_exact_max_boundaries_construct_without_counting() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // Exactly MAX_BATCH_CALLS, with one call sitting exactly on both per-call
    // byte limits: every `>` comparison must be exclusive.
    let mut calls: Vec<JsBatchCall> = (0..MAX_BATCH_CALLS - 1)
        .map(|i| JsBatchCall {
            id: format!("id-{i}"),
            tool: "search".to_string(),
            args: None,
        })
        .collect();
    calls.push(JsBatchCall {
        id: "i".repeat(MAX_BATCH_ID_BYTES),
        tool: "t".repeat(MAX_BATCH_TOOL_BYTES),
        args: Some(json!({"query": "q"})),
    });
    assert_eq!(calls.len(), MAX_BATCH_CALLS);
    let task = session.batch(calls, None).expect("exact-max batch constructs");
    drop(task);
    assert_eq!(session.call_count(), 0);
}

#[test]
fn batch_limits_count_bytes_not_chars() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // 'é' is 2 bytes: 64 of them are exactly 128 bytes (accepted), 65 are 130
    // bytes (rejected). A chars-based check would accept both.
    assert_eq!(MAX_BATCH_ID_BYTES, 128);
    assert_eq!(MAX_BATCH_TOOL_BYTES, 128);
    let ok_task = session
        .batch(
            vec![JsBatchCall {
                id: "é".repeat(64),
                tool: "search".to_string(),
                args: None,
            }],
            None,
        )
        .expect("128-byte multibyte id constructs");
    drop(ok_task);
    let err = session
        .batch(
            vec![JsBatchCall {
                id: "é".repeat(65),
                tool: "search".to_string(),
                args: None,
            }],
            None,
        )
        .err()
        .expect("130-byte multibyte id rejected");
    assert_eq!(
        err.reason,
        format!("batch call id exceeds {MAX_BATCH_ID_BYTES} bytes")
    );
    let err = session
        .batch(
            vec![JsBatchCall {
                id: "a".to_string(),
                tool: "é".repeat(65),
                args: None,
            }],
            None,
        )
        .err()
        .expect("130-byte multibyte tool rejected");
    assert_eq!(
        err.reason,
        format!("batch call tool exceeds {MAX_BATCH_TOOL_BYTES} bytes")
    );
    assert_eq!(session.call_count(), 0);
}

#[test]
fn batch_reports_first_violation_in_validation_order() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // Overcount wins over per-call violations: the len check precedes the loop.
    let mut calls: Vec<JsBatchCall> = (0..MAX_BATCH_CALLS + 1)
        .map(|i| JsBatchCall {
            id: format!("id-{i}"),
            tool: "search".to_string(),
            args: None,
        })
        .collect();
    calls[0].id.clear();
    let err = session
        .batch(calls, None)
        .err()
        .expect("overcount + empty id rejected");
    assert_eq!(err.reason, format!("batch.calls exceeds max {MAX_BATCH_CALLS}"));

    // The scan covers the whole list, not just the first call.
    let err = session
        .batch(
            vec![
                JsBatchCall {
                    id: "ok".to_string(),
                    tool: "search".to_string(),
                    args: None,
                },
                JsBatchCall {
                    id: "second".to_string(),
                    tool: String::new(),
                    args: None,
                },
            ],
            None,
        )
        .err()
        .expect("second-call violation rejected");
    assert_eq!(err.reason, "batch call tool must be non-empty");
    assert_eq!(session.call_count(), 0);
}

// ---------------------------------------------------------------------------
// 4. Validation scope: batch/call validate identity only, never semantics
// ---------------------------------------------------------------------------

#[test]
fn construction_validates_identity_only_and_never_executes() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // call() performs NO sync validation at all: unknown, empty, and absurdly
    // long tools all construct Ok. Enforcement lives in compute() on libuv.
    for tool in [
        "zzz_no_such_tool".to_string(),
        String::new(),
        "t".repeat(10_000),
    ] {
        assert!(session
            .call(tool, Some(json!({"query": "q"})), None)
            .map(|_| ())
            .is_ok());
    }
    // batch() checks non-empty + byte caps only: duplicate ids, whitespace
    // ids/tools, and non-object args JSON all construct Ok.
    let task = session
        .batch(
            vec![
                JsBatchCall {
                    id: "dup".to_string(),
                    tool: "search".to_string(),
                    args: Some(json!(42)),
                },
                JsBatchCall {
                    id: "dup".to_string(),
                    tool: "find".to_string(),
                    args: Some(json!([1, 2, {"nested": true}])),
                },
                JsBatchCall {
                    id: " ".to_string(),
                    tool: " ".to_string(),
                    args: None,
                },
            ],
            None,
        )
        .expect("identity-only batch constructs");
    drop(task);
    // Construction alone never executes: count untouched throughout.
    assert_eq!(session.call_count(), 0);
}

// ---------------------------------------------------------------------------
// 5-6. call_now partition + busy constant
// ---------------------------------------------------------------------------

#[test]
fn call_now_fast_slow_partition_is_exact_over_catalog_and_aliases() {
    let temp = empty_root();
    std::fs::write(temp.path().join("hello.rs"), "fn alpha() {}\n").expect("fixture");
    let session = session_on(temp.path());
    materialize(&session);

    // Every fast lookup succeeds here (never the gate reason): 8 sync tools.
    let fast: &[(&str, Value)] = &[
        ("find", json!({"query": "zzz_no_such_needle"})),
        (
            "read",
            json!({"path": "hello.rs", "start": 1, "end": 1}),
        ),
        ("defs", json!({"symbol": "ZzzNoSuchSymbol"})),
        ("callers", json!({"symbol": "ZzzNoSuchSymbol"})),
        ("imports", json!({"module": "zzz_no_such_module"})),
        ("index_status", json!({})),
        ("catalog_search", json!({"query": "search"})),
        ("catalog_describe", json!({"name": "search"})),
    ];
    for (tool, args) in fast {
        session
            .call_now(tool.to_string(), Some(args.clone()))
            .unwrap_or_else(|e| panic!("fast tool {tool} must not gate: {}", e.reason));
    }
    // search stays special: unique queries fall through as Null, still no gate.
    assert_eq!(
        session
            .call_now(
                "search".to_string(),
                Some(json!({"query": "zzz_unique_partition_needle"}))
            )
            .expect("cold search"),
        Value::Null
    );

    // Slow tools plus every ToolName alias spelling: the fast gate is literal,
    // so even aliases of fast tools ("grep", "code_search", "indexStatus")
    // stay on call(). Each rejected with the exact constant.
    for tool in [
        "index_repo",
        "semantic",
        "chain",
        "edit",
        "filter_hits",
        "select",
        "code_search",
        "grep",
        "keyword",
        "code_read",
        "code_edit",
        "define",
        "definition",
        "definitions",
        "references",
        "indexStatus",
        "indexRepo",
        "catalogSearch",
        "catalogDescribe",
    ] {
        let err = session
            .call_now(tool.to_string(), Some(json!({})))
            .expect_err(&format!("{tool} gated"));
        assert_eq!(err.reason, CALL_NOW_ONLY, "{tool}");
    }
    // materialize (1) + 8 fast successes; cold search and gates never bump.
    assert_eq!(session.call_count(), 1 + 8);
}

#[test]
fn call_now_busy_reason_is_exact_under_contention() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // catalog_describe is pure (no store touch): 16 threads in a tight loop
    // maximize try_lock collisions. Rounds repeat until at least one busy
    // loser is observed, so the pin cannot pass vacuously.
    let observed_busy = AtomicBool::new(false);
    for _round in 0..50 {
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| {
                    for _ in 0..200 {
                        match session.call_now(
                            "catalog_describe".to_string(),
                            Some(json!({"name": "search"})),
                        ) {
                            Ok(value) => assert_eq!(value["name"], json!("search")),
                            Err(err) => {
                                assert_eq!(err.reason, BUSY);
                                observed_busy.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });
        if observed_busy.load(Ordering::Relaxed) {
            break;
        }
    }
    assert!(
        observed_busy.load(Ordering::Relaxed),
        "expected at least one busy collision under 16-thread hammering"
    );
}

// ---------------------------------------------------------------------------
// 7-8. Error marshalling: exact CallError texts, gate precedes dispatch
// ---------------------------------------------------------------------------

#[test]
fn call_now_invalid_args_reasons_are_exact_contract_texts() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // (tool, args, exact reason). require_* failures happen before any store
    // touch, so no materialize is needed; bump precedes dispatch, so every
    // marshalled failure still counts.
    let cases: &[(&str, Value, &str)] = &[
        (
            "defs",
            json!({}),
            "symbol is required. Call asgrep.defs(\"Name\") or asgrep.defs({ symbol: \"Name\" })",
        ),
        (
            "callers",
            json!({}),
            "symbol is required. Call asgrep.defs(\"Name\") or asgrep.defs({ symbol: \"Name\" })",
        ),
        (
            "imports",
            json!({}),
            "module is required. Call asgrep.imports(\"os\") or asgrep.imports({ module: \"os\" })",
        ),
        (
            "catalog_search",
            json!({}),
            "query is required. Call asgrep.search(\"text\") or asgrep.search({ query: \"text\" })",
        ),
        (
            "catalog_describe",
            json!({}),
            "name is required. Call asgrep.catalogDescribe(\"search\")",
        ),
        (
            "catalog_describe",
            json!({"name": "zzz_no_such_catalog_tool"}),
            "unknown tool in catalog: zzz_no_such_catalog_tool",
        ),
        // find/read arg failures surface as Other (anyhow context), not
        // InvalidArgs — the taxonomy boundary the napi reason preserves.
        ("find", json!({}), "query is required"),
        ("read", json!({}), "path is required"),
    ];
    for (tool, args, expected) in cases {
        let err = session
            .call_now(tool.to_string(), Some(args.clone()))
            .expect_err(&format!("{tool} invalid args"));
        assert_eq!(err.reason, *expected, "{tool}");
    }
    assert_eq!(session.call_count(), cases.len() as u32);
}

#[test]
fn call_now_gate_precedes_dispatch_while_core_taxonomy_pins_directly() {
    let temp = empty_root();
    let session = session_on(temp.path());
    // Unknowns — including near-misses, empty, case, and padded spellings —
    // all hit the fast gate, never dispatch. Count untouched.
    for tool in [
        "zzz_no_such_tool",
        "serach",
        "",
        "SEARCH",
        "search ",
        "grep; rm -rf /",
    ] {
        let err = session
            .call_now(tool.to_string(), Some(json!({})))
            .expect_err(&format!("{tool:?} gated"));
        assert_eq!(err.reason, CALL_NOW_ONLY, "{tool:?}");
    }
    assert_eq!(session.call_count(), 0);

    // The core taxonomy the gate shadows: pinned directly against
    // CodeModeSession by discriminant + shape.
    let mut core = CodeModeSession::new(SessionConfig {
        root: temp.path().to_path_buf(),
        index_path: Some(temp.path().join("core_taxonomy.db")),
        limit: 25,
        use_embed: false,
        ..SessionConfig::default()
    });
    match core.call("zzz_no_such_tool", json!({})) {
        Err(CallError::UnknownTool(msg)) => {
            assert!(msg.starts_with("unknown tool:"), "taxonomy: {msg}");
        }
        other => panic!("expected UnknownTool, got {other:?}"),
    }
    match core.call("defs", json!({})) {
        Err(CallError::InvalidArgs(msg)) => {
            // Marshalling fidelity: the napi reason is byte-identical to the
            // core Display text for the same failure.
            let napi_err = session
                .call_now("defs".to_string(), Some(json!({})))
                .expect_err("defs invalid args");
            assert_eq!(napi_err.reason, msg);
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 9-10. String args: unicode/empty roundtrip, huge fails closed by name
// ---------------------------------------------------------------------------

#[test]
fn find_and_catalog_unicode_and_empty_roundtrip_without_panic() {
    let temp = empty_root();
    let session = session_on(temp.path());
    materialize(&session);
    // Multibyte query renders a well-formed capsule (hit_count consistent).
    for (tool, args) in [
        ("find", json!({"query": "héllo世界🦀"})),
        ("defs", json!({"symbol": "héllo世界🦀"})),
    ] {
        let value = session
            .call_now(tool.to_string(), Some(args))
            .unwrap_or_else(|e| panic!("{tool} unicode: {}", e.reason));
        assert_eq!(value["mode"], json!("capsule"), "{tool}: {value:?}");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(value["hit_count"], json!(hits.len()));
    }
    // Empty is allowed (mode parsers treat it as no hits), not a throw.
    let value = session
        .call_now("find".to_string(), Some(json!({"query": ""})))
        .expect("empty find");
    assert_eq!(value["mode"], json!("capsule"));
    // Unicode flows through the pure catalog path too.
    let value = session
        .call_now("catalog_search".to_string(), Some(json!({"query": "séarch"})))
        .expect("unicode catalog_search");
    assert!(value["tools"].is_array());
}

#[test]
fn huge_query_fails_closed_naming_the_char_limit() {
    let temp = empty_root();
    let session = session_on(temp.path());
    materialize(&session);
    // find validates raw chars: 4097 > 4096, exact limit reason, still counts.
    let huge = "a".repeat(MAX_QUERY_CHARS + 1);
    let err = session
        .call_now("find".to_string(), Some(json!({"query": huge})))
        .expect_err("oversize find rejected");
    assert_eq!(
        err.reason,
        format!(
            "query exceeds maximum of {MAX_QUERY_CHARS} characters ({})",
            MAX_QUERY_CHARS + 1
        )
    );
    assert_eq!(session.call_count(), 2);
    // defs composes "defs:{symbol}" before validating: the composed length
    // (5 + 4097 = 4102) is what the reason names.
    let err = session
        .call_now(
            "defs".to_string(),
            Some(json!({"symbol": "s".repeat(MAX_QUERY_CHARS + 1)})),
        )
        .expect_err("oversize defs rejected");
    assert_eq!(
        err.reason,
        format!("query exceeds maximum of {MAX_QUERY_CHARS} characters (4102)")
    );
    // search via call_now never throws on huge input: the cache probe fails
    // validation internally, so the query falls through as Null uncounted.
    let before = session.call_count();
    assert_eq!(
        session
            .call_now(
                "search".to_string(),
                Some(json!({"query": "q".repeat(MAX_QUERY_CHARS + 1)}))
            )
            .expect("huge search falls through"),
        Value::Null
    );
    assert_eq!(session.call_count(), before);
}

// ---------------------------------------------------------------------------
// 11. Unicode fixture roundtrip without truncation
// ---------------------------------------------------------------------------

#[test]
fn read_unicode_fixture_roundtrips_exact_text() {
    let temp = empty_root();
    let name = "héllo_世界.rs";
    let content = "fn unicodé_α() {}\n// 🦀 crab\n";
    std::fs::write(temp.path().join(name), content).expect("unicode fixture");
    let session = session_on(temp.path());
    materialize(&session);
    let value = session
        .call_now(
            "read".to_string(),
            Some(json!({"path": name, "start": 1, "end": 2})),
        )
        .expect("unicode read");
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["count"], json!(1));
    assert_eq!(value["windows"][0]["path"], json!(name));
    // Byte-exact: no truncation, no replacement chars, no newline mangling.
    assert_eq!(
        value["windows"][0]["text"],
        json!("fn unicodé_α() {}\n// 🦀 crab")
    );
}

// ---------------------------------------------------------------------------
// 12. Invalid configs fail closed at first use (construction is lazy)
// ---------------------------------------------------------------------------

#[test]
fn invalid_root_configs_fail_closed_at_first_use_with_documented_reasons() {
    let temp = empty_root();
    // Session::new is lazy/infallible: a nonexistent root stores verbatim and
    // only fails when a tool resolves it.
    let missing = temp.path().join("does-not-exist-zzz");
    let session = Session::new(Some(JsSessionConfig {
        root: Some(missing.display().to_string()),
        index_path: Some(temp.path().join("bad_root.db").display().to_string()),
        limit: None,
        use_embed: Some(false),
    }))
    .expect("construction is lazy");
    assert_eq!(session.root(), missing.display().to_string());
    let err = session
        .call_now("index_status".to_string(), None)
        .expect_err("missing root fails closed");
    assert!(
        err.reason.starts_with("cannot resolve session root:"),
        "taxonomy: {}",
        err.reason
    );
    assert_eq!(session.call_count(), 1);

    // Per-call root overrides are jailed: outside roots and unresolvable
    // roots fail with their documented prefixes.
    let session = session_on(temp.path());
    materialize(&session);
    let err = session
        .call_now(
            "find".to_string(),
            Some(json!({"query": "x", "root": "/etc"})),
        )
        .expect_err("escaping root fails closed");
    assert!(
        err.reason
            .starts_with("requested root is outside the configured session root:"),
        "taxonomy: {}",
        err.reason
    );
    let err = session
        .call_now(
            "find".to_string(),
            Some(json!({"query": "x", "root": "/nonexistent-zzz-root"})),
        )
        .expect_err("unresolvable root fails closed");
    assert!(
        err.reason.starts_with("cannot resolve requested root:"),
        "taxonomy: {}",
        err.reason
    );

    // index_path under a regular file (ENOTDIR parent): SQLite-owned text,
    // so discriminant only (is_err + non-empty), never message matching.
    // (A bare directory is NOT invalid — the Indexer treats it as the folder
    // holding index.db — so the blocker must be a non-directory parent.)
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, "not a directory").expect("blocker file");
    let session = Session::new(Some(JsSessionConfig {
        root: Some(temp.path().display().to_string()),
        index_path: Some(blocker.join("index.db").display().to_string()),
        limit: None,
        use_embed: Some(false),
    }))
    .expect("construction is lazy");
    let err = session
        .call_now("index_status".to_string(), None)
        .expect_err("unopenable index_path fails closed");
    assert!(!err.reason.is_empty());
}
