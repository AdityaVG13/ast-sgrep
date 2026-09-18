//! Napi-vs-core bilateral agreement for `ast-sgrep-codemode-napi`.
//!
//! Proves the wrapper and direct core agree on IDENTICAL fixtures: one
//! tempdir root plus one index db shared by a napi `Session` and a direct
//! `CodeModeSession`, so byte-identical JSON is the assertion — not shape.
//! Covers every fast lookup, the search cache hit/miss pair, batch
//! validation reasons, mixed-sequence counts, error reasons, and budget
//! exhaustion.
//!
//! Honestly out of scope (require Node): per-call ok/error shapes of napi
//! batch execution vs `run_batch` stay unpinned — `compute()` is unreachable
//! from Rust. Only the validation layer both sides share is compared.
//!
//! Link note (macOS): `RUSTFLAGS="-C link-arg=-undefined -C
//! link-arg=dynamic_lookup" cargo test -p ast-sgrep-codemode-napi --test
//! ffi_bilateral`. Tests never call Node FFI.

use ast_sgrep_codemode::{
    run_batch, CallError, MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_testkit::{
    assert_json_byte_identical, batch_call, batch_request, config_at_indexed, file_tree,
};
use serde_json::{json, Value};

// `shared` serves all four FFI suites; each target uses a subset, so the
// per-target dead-code lint would fire on siblings' helpers. The allow lives
// on each suite's opt-in, not in the shared file.
#[allow(dead_code)]
#[path = "ffi_shared.rs"]
mod shared;
use shared::{empty_root, js_call, materialize_both, pair};

/// INTENT: `find` over real indexed hits, `read` over a fixture window, and
/// the `defs`/`callers`/`imports` capsules all agree byte-identical between
/// napi `call_now` and core `call`. KILLS: render-divergence, ordering-swap,
/// window-slice-divergence, capsule-shape-divergence.
#[test]
fn symbol_and_text_lookups_agree_byte_identical() {
    let temp = file_tree(&[
        ("a.rs", "fn alpha() {}\nfn beta() {}\n"),
        ("b.rs", "fn gamma() {}\n"),
        ("hello.rs", "fn alpha() {}\nfn beta() {}\n"),
    ]);
    let (napi, mut core) = pair(temp.path(), "ffi_bi.db");
    // Indexing runs core-side (call_now gates slow tools by design); both
    // sides then read the same db through independent warm Searchers.
    core.call("index_repo", json!({})).expect("index fixture");

    let args = json!({"query": "alpha"});
    let napi_value = napi
        .call_now("find".to_string(), Some(args.clone()))
        .expect("napi find");
    let core_value = core.call("find", args).expect("core find");
    assert_json_byte_identical(&napi_value, &core_value, "find");
    // Non-vacuous: the hits path (ordering, excerpts) was exercised.
    assert!(napi_value["hit_count"].as_u64().unwrap_or(0) >= 1);

    let args = json!({"path": "hello.rs", "start": 1, "end": 2});
    let napi_value = napi
        .call_now("read".to_string(), Some(args.clone()))
        .expect("napi read");
    let core_value = core.call("read", args).expect("core read");
    assert_json_byte_identical(&napi_value, &core_value, "read");
    assert_eq!(napi_value["ok"], json!(true));

    // The three symbol capsules share one loop: identical bodies, zero
    // coverage loss versus three per-tool tests.
    let capsules: &[(&str, Value)] = &[
        ("defs", json!({"symbol": "ZzzNoSuchSymbol"})),
        ("callers", json!({"symbol": "ZzzNoSuchSymbol"})),
        ("imports", json!({"module": "zzz_no_such_module"})),
    ];
    for (tool, args) in capsules {
        let napi_value = napi
            .call_now(tool.to_string(), Some(args.clone()))
            .unwrap_or_else(|e| panic!("napi {tool}: {}", e.reason));
        let core_value = core
            .call(tool, args.clone())
            .unwrap_or_else(|e| panic!("core {tool}: {e}"));
        assert_json_byte_identical(&napi_value, &core_value, tool);
        assert_eq!(napi_value["mode"], json!("capsule"), "{tool}");
    }
}

/// INTENT: `index_status` (including embedded root/index_path/counts) plus
/// both catalog tools agree byte-identical. KILLS: config-mapping-divergence,
/// catalog-dispatch-divergence.
#[test]
fn metadata_tools_agree_byte_identical() {
    let temp = empty_root();
    let (napi, mut core) = pair(temp.path(), "ffi_bi.db");
    // Shared root + db, so root/index_path/counts embed identically.
    let napi_value = napi
        .call_now("index_status".to_string(), Some(json!({})))
        .expect("napi index_status");
    let core_value = core.call("index_status", json!({})).expect("core index_status");
    assert_json_byte_identical(&napi_value, &core_value, "index_status");
    assert_eq!(napi_value["file_count"], json!(0));

    let args = json!({"query": "search"});
    let napi_value = napi
        .call_now("catalog_search".to_string(), Some(args.clone()))
        .expect("napi catalog_search");
    let core_value = core.call("catalog_search", args).expect("core catalog_search");
    assert_json_byte_identical(&napi_value, &core_value, "catalog_search");
    assert!(!napi_value["tools"].as_array().expect("tools").is_empty());

    let args = json!({"name": "search"});
    let napi_value = napi
        .call_now("catalog_describe".to_string(), Some(args.clone()))
        .expect("napi catalog_describe");
    let core_value = core
        .call("catalog_describe", args)
        .expect("core catalog_describe");
    assert_json_byte_identical(&napi_value, &core_value, "catalog_describe");
    assert_eq!(napi_value["name"], json!("search"));
}

/// INTENT: a primed search key agrees via napi `call_now` vs core
/// `take_cached_search` (byte-identical render, both bump); a cold key falls
/// through as `Null`/`None` on both, both uncounted. KILLS:
/// cache-key-divergence, count-divergence, fallthrough-divergence.
#[test]
fn search_cache_agrees_on_hit_and_miss() {
    let temp = empty_root();
    let (napi, mut core) = pair(temp.path(), "ffi_bi.db");
    materialize_both(&napi, &mut core);
    // find("N") renders under key "word:N" on each side's own cache.
    let found_args = json!({"query": "needle_xyz"});
    let napi_found = napi
        .call_now("find".to_string(), Some(found_args.clone()))
        .expect("napi find");
    let core_found = core.call("find", found_args).expect("core find");
    assert_json_byte_identical(&napi_found, &core_found, "find");
    // Same cache key via napi call_now vs direct take_cached_search.
    let hit_args = json!({"query": "word:needle_xyz"});
    let napi_hit = napi
        .call_now("search".to_string(), Some(hit_args.clone()))
        .expect("napi cached search");
    let core_hit = core
        .take_cached_search(&hit_args)
        .expect("core cached search")
        .expect("cache hit");
    assert_json_byte_identical(&napi_hit, &core_hit, "search(hit)");
    assert_eq!(napi_hit, napi_found);
    // materialize + find + hit on each side.
    assert_eq!(napi.call_count(), 3);
    assert_eq!(core.call_count(), 3);

    // No materialize: the cache probe touches no store on either side.
    let temp = empty_root();
    let (napi, mut core) = pair(temp.path(), "ffi_bi.db");
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

/// INTENT: every batch violation class fails on both layers with
/// byte-identical reasons, and the core discriminant is `InvalidArgs`. KILLS:
/// validation-layer-fork.
#[test]
fn batch_validation_reasons_agree_byte_identical() {
    let temp = empty_root();
    let db = temp.path().join("ffi_bi.db");
    let (napi, _) = pair(temp.path(), "ffi_bi.db");
    // (napi calls, core calls): identical identities on both sides. The core
    // side reuses the testkit batch builders; the napi side the shared
    // `js_call` (JsBatchCall is napi-typed, so it cannot come from testkit).
    let overcount = MAX_BATCH_CALLS + 1;
    let cases: Vec<(Vec<_>, Vec<_>)> = vec![
        (vec![], vec![]),
        (
            (0..overcount)
                .map(|i| js_call(&format!("id-{i}"), "search", None))
                .collect(),
            (0..overcount)
                .map(|i| batch_call(&format!("id-{i}"), "search", Value::Null))
                .collect(),
        ),
        (
            vec![js_call("", "search", None)],
            vec![batch_call("", "search", Value::Null)],
        ),
        (
            vec![js_call(
                &"i".repeat(MAX_BATCH_ID_BYTES + 1),
                "search",
                None,
            )],
            vec![batch_call(
                &"i".repeat(MAX_BATCH_ID_BYTES + 1),
                "search",
                Value::Null,
            )],
        ),
        (
            vec![js_call("a", "", None)],
            vec![batch_call("a", "", Value::Null)],
        ),
        (
            vec![js_call(
                "a",
                &"t".repeat(MAX_BATCH_TOOL_BYTES + 1),
                None,
            )],
            vec![batch_call(
                "a",
                &"t".repeat(MAX_BATCH_TOOL_BYTES + 1),
                Value::Null,
            )],
        ),
    ];
    for (napi_calls, core_calls) in cases {
        let napi_err = napi
            .batch(napi_calls, None)
            .err()
            .expect("napi batch rejects");
        let core_err = run_batch(
            config_at_indexed(temp.path(), &db),
            &batch_request(core_calls),
        )
        .err()
        .expect("core run_batch rejects");
        assert!(matches!(core_err, CallError::InvalidArgs(_)));
        assert_eq!(napi_err.reason, core_err.to_string());
    }
    assert_eq!(napi.call_count(), 0);
}

/// INTENT: after an identical mixed ok/error sequence both sides agree on
/// ok-ness, on every failure reason, and on the budget count. KILLS:
/// count-divergence, marshalling-drift, taxonomy-divergence.
#[test]
fn mixed_sequence_counts_and_reasons_agree() {
    let temp = empty_root();
    let (napi, mut core) = pair(temp.path(), "ffi_bi.db");
    materialize_both(&napi, &mut core);
    let steps: &[(&str, Value, bool)] = &[
        ("find", json!({"query": "zzz_no_such_needle"}), true),
        ("defs", json!({"symbol": "ZzzNoSuch"}), true),
        ("catalog_describe", json!({"name": "search"}), true),
        ("defs", json!({}), false),
    ];
    for (tool, args, ok) in steps {
        let napi_result = napi.call_now(tool.to_string(), Some(args.clone()));
        let core_result = core.call(tool, args.clone());
        assert_eq!(napi_result.is_ok(), *ok, "{tool} napi");
        assert_eq!(core_result.is_ok(), *ok, "{tool} core");
    }

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
    // materialize + dispatched steps + marshalled failures: failures bump on
    // both sides.
    let total = 1 + steps.len() + cases.len();
    assert_eq!(napi.call_count(), total as u32);
    assert_eq!(core.call_count(), total);
}

/// INTENT: both sides fail the 10_001st call identically — same reason, core
/// discriminant `BudgetExhausted(10_000)`, counts pinned at the cap. KILLS:
/// budget-cap-divergence. (Pairs with the js_client error-flow test, which
/// pins the JS-visible text this test never states.)
#[test]
fn budget_exhaustion_fails_identically_on_both() {
    let temp = empty_root();
    let (napi, mut core) = pair(temp.path(), "ffi_bi.db");
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
