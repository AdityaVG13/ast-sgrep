//! `call_now` gate/count/reason contract for `ast-sgrep-codemode-napi`.
//!
//! Pins the sync fast path drivable from Rust without a JS `Env`: the exact
//! fast/slow partition over catalog tools and aliases, exact invalid-args
//! reason texts with the InvalidArgs-vs-Other taxonomy, search cache
//! hit/miss counting, the exact busy reason under contention, unicode/huge
//! string handling, and lazy fail-closed roots.
//!
//! Honestly out of scope (require Node): slow-tool execution and unique-search
//! dispatch live behind `call()` on libuv; `call_now` only pins their gate.
//!
//! Link note (macOS): `RUSTFLAGS="-C link-arg=-undefined -C
//! link-arg=dynamic_lookup" cargo test -p ast-sgrep-codemode-napi --test
//! ffi_call_now`. Tests never call Node FFI.

use ast_sgrep_codemode::CallError;
use ast_sgrep_codemode_napi::{JsSessionConfig, Session};
use ast_sgrep_testkit::{
    session_at_indexed, write_file, CALL_NOW_ONLY, DEFS_NEEDS_SYMBOL, MAX_QUERY_CHARS,
    SESSION_BUSY,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};

// `shared` serves all four FFI suites; each target uses a subset, so the
// per-target dead-code lint would fire on siblings' helpers. The allow lives
// on each suite's opt-in, not in the shared file.
#[allow(dead_code)]
#[path = "ffi_shared.rs"]
mod shared;
use shared::{empty_root, materialize, session_on};

/// INTENT: `call_now` serves exactly the 8 fast lookups (+ cold-search Null)
/// and gates the 6 slow tools plus every alias spelling with the exact
/// reason, uncounted. KILLS: gate-literal-vs-parse-swap (aliases of fast
/// tools must still gate), reason-change, gate-after-bump.
#[test]
fn fast_slow_partition_is_exact() {
    let temp = empty_root();
    write_file(&temp.path().join("hello.rs"), b"fn alpha() {}\n");
    let session = session_on(temp.path(), "ffi_call_now.db");
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

/// INTENT: invalid-args failures carry the exact contract texts (including
/// the InvalidArgs-vs-Other taxonomy) and still count; unknown spellings are
/// gated before dispatch; the napi reason is byte-identical to the core
/// `Display` for the same failure. KILLS: reason-change, taxonomy-swap,
/// count-on-fail-swap, gate-after-dispatch, marshalling-drift.
#[test]
fn invalid_args_reasons_are_exact_and_counted() {
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_call_now.db");
    // (tool, args, exact reason). require_* failures happen before any store
    // touch, so no materialize is needed; bump precedes dispatch, so every
    // marshalled failure still counts. The defs/callers text is the shared
    // testkit pin; the rest are single-use contract texts local to this table.
    let cases: &[(&str, Value, &str)] = &[
        ("defs", json!({}), DEFS_NEEDS_SYMBOL),
        ("callers", json!({}), DEFS_NEEDS_SYMBOL),
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
    assert_eq!(session.call_count(), cases.len() as u32);

    // The core taxonomy the gate shadows: pinned directly against a core
    // session by discriminant + shape.
    let mut core = session_at_indexed(temp.path(), &temp.path().join("core_taxonomy.db"));
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
    assert_eq!(session.call_count(), cases.len() as u32 + 1);
}

/// INTENT: a cold search key falls through as `Null` without counting (the
/// JS host's cue to retry via `call()`), while a `find`-primed key hits the
/// cache and returns the identical render, counted. KILLS:
/// Null-vs-error-swap, count-bump-on-fallthrough, cache-key-swap,
/// render-divergence.
#[test]
fn search_cache_hit_returns_identical_render_and_counts() {
    // Unique search stays on call()/libuv: Null fall-through, no budget bump.
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_call_now.db");
    let value = session
        .call_now(
            "search".to_string(),
            Some(json!({"query": "zzz_unique_cold_needle"})),
        )
        .expect("cold search");
    assert_eq!(value, Value::Null);
    assert_eq!(session.call_count(), 0);

    // find("N") renders under key "word:N"; the same key via search() must hit
    // take_cached_search and return the identical render.
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_call_now.db");
    materialize(&session);
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

/// INTENT: under contention every failure is exactly "session is busy"
/// (non-vacuously observed), losers never bump the budget, and the session
/// is healthy afterwards. KILLS: busy-reason-change, bump-on-busy-swap,
/// post-contention-poison.
#[test]
fn busy_gate_is_exact_and_session_recovers() {
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_call_now.db");
    // catalog_describe is pure (no store touch): 16 threads in a tight loop
    // maximize try_lock collisions. Rounds repeat until at least one busy
    // loser is observed, so the pin cannot pass vacuously.
    // One-use hammer kept local: it is Session-typed, so it cannot move to
    // testkit without a testkit→napi dependency, and no other suite contends.
    let successes = AtomicU32::new(0);
    let observed_busy = AtomicU32::new(0);
    for _round in 0..50 {
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| {
                    for _ in 0..200 {
                        match session.call_now(
                            "catalog_describe".to_string(),
                            Some(json!({"name": "search"})),
                        ) {
                            Ok(value) => {
                                assert_eq!(value["name"], json!("search"));
                                successes.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(err) => {
                                assert_eq!(err.reason, SESSION_BUSY);
                                observed_busy.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });
        if observed_busy.load(Ordering::Relaxed) > 0 {
            break;
        }
    }
    assert!(
        observed_busy.load(Ordering::Relaxed) > 0,
        "expected at least one busy collision under 16-thread hammering"
    );
    // Contended losers never bump: the mirrored count reflects only successes
    // (upper bound; concurrent stores may lag, so no exact equality).
    assert!(session.call_count() <= successes.load(Ordering::Relaxed));
    assert!(session.call_count() >= 1);

    // Post-contention the session is healthy: a sequential retry succeeds.
    let value = session
        .call_now(
            "catalog_describe".to_string(),
            Some(json!({"name": "search"})),
        )
        .expect("post-contention retry succeeds");
    assert_eq!(value["name"], json!("search"));
}

/// INTENT: multibyte/empty args never panic or truncate; oversize queries
/// fail closed naming the char limit (re-exported from core, no literal);
/// huge `search` still falls through as `Null`, uncounted. KILLS:
/// panic-on-multibyte, truncation, encoding-mangle, reason-change,
/// compose-before-validate-swap.
#[test]
fn string_args_roundtrip_and_fail_closed_by_name() {
    let temp = empty_root();
    let name = "héllo_世界.rs";
    let content = "fn unicodé_α() {}\n// 🦀 crab\n";
    write_file(&temp.path().join(name), content.as_bytes());
    let session = session_on(temp.path(), "ffi_call_now.db");
    materialize(&session);

    // Multibyte queries render well-formed capsules (hit_count consistent).
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

    // find validates raw chars: MAX+1 fails with the exact limit reason and
    // still counts.
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
    // defs composes "defs:{symbol}" before validating: the composed length
    // (5-char prefix + MAX+1 symbol = MAX+6) is what the reason names.
    let err = session
        .call_now(
            "defs".to_string(),
            Some(json!({"symbol": "s".repeat(MAX_QUERY_CHARS + 1)})),
        )
        .expect_err("oversize defs rejected");
    assert_eq!(
        err.reason,
        format!(
            "query exceeds maximum of {MAX_QUERY_CHARS} characters ({})",
            MAX_QUERY_CHARS + 6
        )
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

    // Unicode filename + content reads back byte-exact: no truncation, no
    // replacement chars, no newline mangling.
    let value = session
        .call_now(
            "read".to_string(),
            Some(json!({"path": name, "start": 1, "end": 2})),
        )
        .expect("unicode read");
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["count"], json!(1));
    assert_eq!(value["windows"][0]["path"], json!(name));
    assert_eq!(
        value["windows"][0]["text"],
        json!("fn unicodé_α() {}\n// 🦀 crab")
    );
}

/// INTENT: `Session::new` is lazy/infallible (stores the root verbatim), so
/// missing roots, escaping or unresolvable per-call roots, and unopenable
/// index paths fail closed at first use with documented reasons. KILLS:
/// eager-validation, jail-bypass.
#[test]
fn invalid_roots_fail_closed_at_first_use() {
    let temp = empty_root();
    // A nonexistent root stores verbatim and only fails when a tool resolves it.
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
    let session = session_on(temp.path(), "ffi_call_now.db");
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
    write_file(&blocker, b"not a directory");
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
