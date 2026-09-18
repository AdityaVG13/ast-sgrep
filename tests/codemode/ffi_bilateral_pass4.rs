//! F4 JS-client simulation drills for `ast-sgrep-codemode-napi` (pass 4).
//!
//! F1 pinned the export inventory, F2 the boundary validation, F3 the
//! napi-vs-core bilateral agreement. F4 simulates a JS client driving the NAPI
//! surface ONLY ([`Session::new`] / [`Session::call_now`] / [`Session::call`] /
//! [`Session::batch`] construction): full flows with exact expected values,
//! JS-visible error reasons, per-session isolation, and determinism. No core
//! imports: everything here is observable through the wrapper.
//!
//! Honestly out of scope (require a real Node env):
//!
//! * `SessionCallTask::compute` / `SessionBatchTask::compute` — both structs
//!   hold private fields with no constructor, and napi-3 `AsyncTask` exposes
//!   only `new` / `with_signal` / `with_optional_signal` (verified against
//!   napi-3.12.2 sources: `inner` is private, no accessor). Per-call batch
//!   ok/error shapes and slow-tool execution (`index_repo`, `semantic`,
//!   `chain`, `search` unique queries) stay on libuv and cannot run here. The
//!   batch drill below pins the JS-visible construction half only.
//! * Cancellation — `AbortSignal` has no Rust constructor (`FromNapiValue`
//!   only), the pre-cancelled path lives inside `compute()`, and `call_now`
//!   takes no signal. `operation cancelled` is unobservable from Rust.
//! * `index_repo` flows — gated to `call()` by design, so indexed-hit `find`
//!   results are unreachable napi-only; the full-flow drill uses the
//!   materialized empty schema plus `read` fixtures instead.
//!
//! Link note (macOS): same as passes 1-3 —
//! `RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup" cargo test -p
//! ast-sgrep-codemode-napi --test ffi_bilateral_pass4`.

use ast_sgrep_codemode_napi::{JsBatchCall, JsSessionConfig, Session};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use tempfile::TempDir;

const CALL_NOW_ONLY: &str = "callNow is only for bounded metadata/symbol lookups; use call() for search/index/semantic/chain";
const BUSY: &str = "session is busy";
const BUDGET: &str = "codemode call budget exceeded (max_calls=10000)";
const DEFS_NEEDS_SYMBOL: &str =
    "symbol is required. Call asgrep.defs(\"Name\") or asgrep.defs({ symbol: \"Name\" })";

fn session_on(root: &Path, db: &str) -> Session {
    // Explicit temp index_path keeps every test hermetic: with None, the first
    // store-touching call would materialize a db under the real index home.
    Session::new(Some(JsSessionConfig {
        root: Some(root.display().to_string()),
        index_path: Some(root.join(db).display().to_string()),
        limit: None,
        use_embed: Some(false),
    }))
    .expect("Session::new")
}

/// Byte-identity over canonical serialization, not just `Value` equality.
fn assert_byte_identical(a: &Value, b: &Value, what: &str) {
    let a = serde_json::to_vec(a).expect("serialize a");
    let b = serde_json::to_vec(b).expect("serialize b");
    assert_eq!(a, b, "{what} mismatch");
}

// ---------------------------------------------------------------------------
// 1. Full JS-client flow: status -> find -> read -> search-cache chain
// ---------------------------------------------------------------------------

#[test]
fn js_client_full_flow_status_find_read_search_cache() {
    let temp = TempDir::new().expect("tempdir");
    std::fs::write(temp.path().join("hello.rs"), "fn alpha() {}\nfn beta() {}\n")
        .expect("fixture");
    let session = session_on(temp.path(), "ffi_pass4.db");
    assert_eq!(session.call_count(), 0);

    // index_status materializes the empty schema: exact zero counts.
    let status = session
        .call_now("index_status".to_string(), None)
        .expect("index_status");
    assert_eq!(status["file_count"], json!(0));
    assert_eq!(session.call_count(), 1);

    // find with zero hits: exact capsule shape.
    let found = session
        .call_now("find".to_string(), Some(json!({"query": "zzz_flow_needle"})))
        .expect("find");
    assert_eq!(found["mode"], json!("capsule"));
    assert_eq!(found["hit_count"], json!(0));
    assert_eq!(found["hits"], json!([]));
    assert_eq!(session.call_count(), 2);

    // read serves the exact window bytes.
    let read = session
        .call_now(
            "read".to_string(),
            Some(json!({"path": "hello.rs", "start": 1, "end": 2})),
        )
        .expect("read");
    assert_eq!(read["ok"], json!(true));
    assert_eq!(read["count"], json!(1));
    assert_eq!(read["windows"][0]["path"], json!("hello.rs"));
    assert_eq!(
        read["windows"][0]["text"],
        json!("fn alpha() {}\nfn beta() {}")
    );
    assert_eq!(session.call_count(), 3);

    // A never-cached key falls through as Null, uncounted — the JS host's cue
    // to retry via call().
    assert_eq!(
        session
            .call_now(
                "search".to_string(),
                Some(json!({"query": "word:zzz_never_cached"}))
            )
            .expect("cold search"),
        Value::Null
    );
    assert_eq!(session.call_count(), 3);

    // find("N") renders under key "word:N"; the same key via search() hits the
    // cache and returns the identical render, counted.
    let rendered = session
        .call_now("find".to_string(), Some(json!({"query": "cache_me"})))
        .expect("find cache_me");
    assert_eq!(session.call_count(), 4);
    let hit = session
        .call_now(
            "search".to_string(),
            Some(json!({"query": "word:cache_me"})),
        )
        .expect("cached search");
    assert_eq!(hit, rendered);
    assert_eq!(session.call_count(), 5);
}

// ---------------------------------------------------------------------------
// 2. Error flow: bad tool, bad args, budget — JS-visible reasons
// ---------------------------------------------------------------------------

#[test]
fn js_client_error_flow_bad_tool_bad_args_budget() {
    let temp = TempDir::new().expect("tempdir");
    let session = session_on(temp.path(), "ffi_pass4.db");

    // Bad tool: the fast gate rejects before dispatch, uncounted.
    let err = session
        .call_now("zzz_no_such_tool".to_string(), Some(json!({})))
        .expect_err("unknown tool gated");
    assert_eq!(err.reason, CALL_NOW_ONLY);
    assert_eq!(session.call_count(), 0);

    // Bad args: marshalled InvalidArgs reason, still bumps the budget.
    let err = session
        .call_now("defs".to_string(), Some(json!({})))
        .expect_err("defs without symbol");
    assert_eq!(err.reason, DEFS_NEEDS_SYMBOL);
    assert_eq!(session.call_count(), 1);

    // Fill the 10_000-call session budget, then prove the 10_001st call fails
    // with the exact JS-visible budget reason and the count pins at the cap.
    let args = json!({"name": "search"});
    for _ in 1..10_000 {
        session
            .call_now("catalog_describe".to_string(), Some(args.clone()))
            .expect("within budget");
    }
    assert_eq!(session.call_count(), 10_000);
    let err = session
        .call_now("catalog_describe".to_string(), Some(args))
        .expect_err("budget exhausted");
    assert_eq!(err.reason, BUDGET);
    assert_eq!(session.call_count(), 10_000);
}

// ---------------------------------------------------------------------------
// 3. Batch drill: construction half only (compute needs Node)
// ---------------------------------------------------------------------------

#[test]
fn js_client_batch_construction_accepts_mixed_semantics() {
    let temp = TempDir::new().expect("tempdir");
    let session = session_on(temp.path(), "ffi_pass4.db");

    // A JS client batching mixed ok/error-shaped calls (valid lookup, unknown
    // tool, invalid args) constructs Ok: semantic failures surface per-call
    // inside compute() on libuv, which needs a real Node env to observe.
    let task = session
        .batch(
            vec![
                JsBatchCall {
                    id: "ok".to_string(),
                    tool: "catalog_describe".to_string(),
                    args: Some(json!({"name": "search"})),
                },
                JsBatchCall {
                    id: "bad-tool".to_string(),
                    tool: "zzz_no_such_tool".to_string(),
                    args: None,
                },
                JsBatchCall {
                    id: "bad-args".to_string(),
                    tool: "defs".to_string(),
                    args: Some(json!({})),
                },
            ],
            None,
        )
        .expect("mixed-semantics batch constructs");
    drop(task);
    assert_eq!(session.call_count(), 0);

    // Only identity violations reject synchronously with exact reasons.
    let err = session
        .batch(
            vec![JsBatchCall {
                id: String::new(),
                tool: "search".to_string(),
                args: None,
            }],
            None,
        )
        .err()
        .expect("empty id rejected");
    assert_eq!(err.reason, "batch call id must be non-empty");
    assert_eq!(session.call_count(), 0);
}

// ---------------------------------------------------------------------------
// 4. Busy-gate drill: retry contract under contention
// ---------------------------------------------------------------------------

#[test]
fn js_client_busy_gate_retry_contract_under_contention() {
    let temp = TempDir::new().expect("tempdir");
    let session = session_on(temp.path(), "ffi_pass4.db");
    let successes = AtomicU32::new(0);
    let observed_busy = AtomicU32::new(0);

    // Hammer a pure tool until at least one busy loser is observed, so the
    // retry-contract pin cannot pass vacuously.
    for _round in 0..50 {
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..50 {
                        match session.call_now(
                            "catalog_describe".to_string(),
                            Some(json!({"name": "search"})),
                        ) {
                            Ok(value) => {
                                assert_eq!(value["name"], json!("search"));
                                successes.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(err) => {
                                // The JS-visible retry signal: exact, stable.
                                assert_eq!(err.reason, BUSY);
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
        "expected at least one busy collision"
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

// ---------------------------------------------------------------------------
// 5-6. Determinism: repeats on one session, fresh sessions on one fixture
// ---------------------------------------------------------------------------

#[test]
fn js_client_same_session_repeats_are_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    let session = session_on(temp.path(), "ffi_pass4.db");
    session
        .call_now("index_status".to_string(), None)
        .expect("materialize");

    let first_find = session
        .call_now("find".to_string(), Some(json!({"query": "zzz_det_needle"})))
        .expect("first find");
    let second_find = session
        .call_now("find".to_string(), Some(json!({"query": "zzz_det_needle"})))
        .expect("second find");
    assert_byte_identical(&first_find, &second_find, "repeated find");

    let first_desc = session
        .call_now(
            "catalog_describe".to_string(),
            Some(json!({"name": "search"})),
        )
        .expect("first describe");
    let second_desc = session
        .call_now(
            "catalog_describe".to_string(),
            Some(json!({"name": "search"})),
        )
        .expect("second describe");
    assert_byte_identical(&first_desc, &second_desc, "repeated describe");
}

#[test]
fn js_client_fresh_sessions_agree_byte_identical() {
    let temp = TempDir::new().expect("tempdir");
    std::fs::write(temp.path().join("hello.rs"), "fn alpha() {}\nfn beta() {}\n")
        .expect("fixture");
    // Same root, same db: two independent JS clients over one store.
    let first = session_on(temp.path(), "ffi_pass4.db");
    let second = session_on(temp.path(), "ffi_pass4.db");

    let flow = |session: &Session| -> Vec<Value> {
        vec![
            session
                .call_now("index_status".to_string(), None)
                .expect("index_status"),
            session
                .call_now("find".to_string(), Some(json!({"query": "zzz_det_needle"})))
                .expect("find"),
            session
                .call_now(
                    "read".to_string(),
                    Some(json!({"path": "hello.rs", "start": 1, "end": 2})),
                )
                .expect("read"),
            session
                .call_now(
                    "catalog_describe".to_string(),
                    Some(json!({"name": "search"})),
                )
                .expect("describe"),
        ]
    };
    let a = flow(&first);
    let b = flow(&second);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_byte_identical(x, y, "fresh-session flow step");
    }
    assert_eq!(first.call_count(), 4);
    assert_eq!(second.call_count(), 4);
}

// ---------------------------------------------------------------------------
// 7. Session isolation: caches and budgets are per-session
// ---------------------------------------------------------------------------

#[test]
fn js_client_sessions_are_isolated_caches_and_budgets() {
    let temp = TempDir::new().expect("tempdir");
    // Shared store, separate sessions: each session owns its search cache and
    // its call budget.
    let a = session_on(temp.path(), "ffi_pass4.db");
    let b = session_on(temp.path(), "ffi_pass4.db");
    a.call_now("index_status".to_string(), None)
        .expect("a materialize");
    b.call_now("index_status".to_string(), None)
        .expect("b materialize");

    // A caches key "word:iso_needle"; B probing the same key still falls
    // through as Null, uncounted — no cross-session cache leak.
    a.call_now("find".to_string(), Some(json!({"query": "iso_needle"})))
        .expect("a find");
    assert_eq!(a.call_count(), 2);
    assert_eq!(
        b.call_now(
            "search".to_string(),
            Some(json!({"query": "word:iso_needle"}))
        )
        .expect("b cold probe"),
        Value::Null
    );
    assert_eq!(b.call_count(), 1);
    assert_eq!(a.call_count(), 2);
}

// ---------------------------------------------------------------------------
// 8. Read drill: exact window bytes over a multi-line fixture
// ---------------------------------------------------------------------------

#[test]
fn js_client_read_window_returns_exact_bytes() {
    let temp = TempDir::new().expect("tempdir");
    std::fs::write(temp.path().join("tri.rs"), "line_one\nline_two\nline_three\n")
        .expect("fixture");
    let session = session_on(temp.path(), "ffi_pass4.db");
    session
        .call_now("index_status".to_string(), None)
        .expect("materialize");

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
}

// ---------------------------------------------------------------------------
// 9. Error reasons are stable across repeats
// ---------------------------------------------------------------------------

#[test]
fn js_client_error_reasons_are_stable_across_repeats() {
    let temp = TempDir::new().expect("tempdir");
    let session = session_on(temp.path(), "ffi_pass4.db");

    // A JS client switching on reason strings sees identical text every time.
    for _ in 0..2 {
        let err = session
            .call_now("defs".to_string(), Some(json!({})))
            .expect_err("defs without symbol");
        assert_eq!(err.reason, DEFS_NEEDS_SYMBOL);
    }
    assert_eq!(session.call_count(), 2);

    for _ in 0..2 {
        let err = session
            .call_now("zzz_no_such_tool".to_string(), Some(json!({})))
            .expect_err("unknown tool gated");
        assert_eq!(err.reason, CALL_NOW_ONLY);
    }
    assert_eq!(session.call_count(), 2);
}
