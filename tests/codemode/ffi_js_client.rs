//! JS-client simulation drills for `ast-sgrep-codemode-napi`.
//!
//! Simulates a JS client driving the NAPI surface ONLY (`Session::new` /
//! `call_now` / construction): a full flow with exact values and counts, the
//! JS-visible error reasons through budget exhaustion, and determinism plus
//! per-session isolation. No core imports: everything here is observable
//! through the wrapper.
//!
//! Honestly out of scope (require a real Node env): slow-tool execution,
//! per-call batch shapes, and cancellation (`AbortSignal` has no Rust
//! constructor) — all live inside `compute()` on libuv.
//!
//! Link note (macOS): `RUSTFLAGS="-C link-arg=-undefined -C
//! link-arg=dynamic_lookup" cargo test -p ast-sgrep-codemode-napi --test
//! ffi_js_client`. Tests never call Node FFI.

use ast_sgrep_codemode_napi::Session;
use ast_sgrep_testkit::{
    assert_json_byte_identical, write_file, BUDGET_EXCEEDED, CALL_NOW_ONLY, DEFS_NEEDS_SYMBOL,
};
use serde_json::{json, Value};

// `shared` serves all four FFI suites; each target uses a subset, so the
// per-target dead-code lint would fire on siblings' helpers. The allow lives
// on each suite's opt-in, not in the shared file.
#[allow(dead_code)]
#[path = "ffi_shared.rs"]
mod shared;
use shared::{empty_root, session_on};

/// INTENT: the status→find→read→cold-search→find→cached-search flow serves
/// exact values with the exact count after every step. KILLS:
/// sequencing/count-choreography-swap. (The one end-to-end flow test.)
#[test]
fn full_flow_choreography_pins_values_and_counts() {
    let temp = empty_root();
    write_file(
        &temp.path().join("hello.rs"),
        b"fn alpha() {}\nfn beta() {}\n",
    );
    let session = session_on(temp.path(), "ffi_js.db");
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

/// INTENT: JS-visible errors are exact and stable — gated tools uncounted,
/// bad args counted, reasons byte-stable across repeats, and the 10_001st
/// call fails with the exact budget text with the count pinned at the cap.
/// KILLS: budget-reason-change, cap-swap, call-dependent reason text
/// (counter/timestamp in reasons, first-vs-rest divergence).
#[test]
fn error_flow_pins_js_visible_reasons_and_budget() {
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_js.db");

    // A JS client switching on reason strings sees identical text every time.
    // Stability first: after the budget fill below, every call fails with the
    // budget reason instead.
    for _ in 0..2 {
        let err = session
            .call_now("zzz_no_such_tool".to_string(), Some(json!({})))
            .expect_err("unknown tool gated");
        assert_eq!(err.reason, CALL_NOW_ONLY);
    }
    assert_eq!(session.call_count(), 0);
    for _ in 0..2 {
        let err = session
            .call_now("defs".to_string(), Some(json!({})))
            .expect_err("defs without symbol");
        assert_eq!(err.reason, DEFS_NEEDS_SYMBOL);
    }
    assert_eq!(session.call_count(), 2);

    // Fill the 10_000-call session budget, then prove the 10_001st call fails
    // with the exact JS-visible budget reason and the count pins at the cap.
    // Only pin of the JS-visible budget text (the bilateral suite asserts
    // dynamic equality with core instead).
    let args = json!({"name": "search"});
    for _ in 2..10_000 {
        session
            .call_now("catalog_describe".to_string(), Some(args.clone()))
            .expect("within budget");
    }
    assert_eq!(session.call_count(), 10_000);
    let err = session
        .call_now("catalog_describe".to_string(), Some(args))
        .expect_err("budget exhausted");
    assert_eq!(err.reason, BUDGET_EXCEEDED);
    assert_eq!(session.call_count(), 10_000);
}

/// INTENT: repeats on one session and fresh sessions over one store produce
/// byte-identical renders, while search caches and budgets stay per-session.
/// KILLS: nondeterminism/state-pollution, session-state-leak-into-renders,
/// cache-leak-across-sessions, shared-budget.
#[test]
fn sessions_are_deterministic_and_isolated() {
    // Repeats on one session are byte-identical.
    let temp = empty_root();
    let session = session_on(temp.path(), "ffi_js.db");
    session
        .call_now("index_status".to_string(), None)
        .expect("materialize");
    let first_find = session
        .call_now("find".to_string(), Some(json!({"query": "zzz_det_needle"})))
        .expect("first find");
    let second_find = session
        .call_now("find".to_string(), Some(json!({"query": "zzz_det_needle"})))
        .expect("second find");
    assert_json_byte_identical(&first_find, &second_find, "repeated find");
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
    assert_json_byte_identical(&first_desc, &second_desc, "repeated describe");

    // Same root, same db: two independent JS clients over one store agree
    // byte-identical on a 4-step flow, counts 4/4.
    let temp = empty_root();
    write_file(
        &temp.path().join("hello.rs"),
        b"fn alpha() {}\nfn beta() {}\n",
    );
    let first = session_on(temp.path(), "ffi_js.db");
    let second = session_on(temp.path(), "ffi_js.db");
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
        assert_json_byte_identical(x, y, "fresh-session flow step");
    }
    assert_eq!(first.call_count(), 4);
    assert_eq!(second.call_count(), 4);

    // Shared store, separate sessions: each session owns its search cache and
    // its call budget.
    let temp = empty_root();
    let a = session_on(temp.path(), "ffi_js.db");
    let b = session_on(temp.path(), "ffi_js.db");
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
