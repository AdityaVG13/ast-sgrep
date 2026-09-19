//! batch numeric contract: byte caps + limit clamp + order/determinism + sweeps.
//!
//! Single-test contract suite absorbing every `run_batch` numeric from
//! `numerical_pass{1,2,3,4}` plus the byte-cap half of the split constants
//! test and the batch half of the split empty-envelope test. Pure
//! `catalog_search`/`filter_hits` batches only — no index I/O.

use ast_sgrep_codemode::{
    BatchRequest, CallError, MAX_BATCH_ERROR_BYTES, MAX_BATCH_RESPONSE_BYTES,
    MAX_BATCH_VALUE_BYTES, run_batch,
};
use ast_sgrep_testkit as testkit;
use ast_sgrep_testkit::scored_hits6;
use serde_json::json;

/// INTENT=batch numeric surface: 4MiB/4MiB-64KiB/8KiB byte caps, limit clamp [1,500], empty-batch InvalidArgs under degenerate limits (validation precedes clamp), call-order independence, rerun determinism, exact size-sweep counts, and exact per-id threshold-sweep counts.
/// KILLS=constant-value (byte-arithmetic literal), clamp-bound (batch-limit), error-discriminant + validation-order (InvalidArgs-vs-Ok), order-dependence (positional-result-mapping), nondeterministic-batch (payload/mode-wobble), batch-counting (call_count/results-length), per-id-mapping (crossed-results).
/// ABSORBS=budget_constants_and_fresh_session_are_hand_computed (caps half), batch_limit_zero_and_huge_clamp_without_panic, empty_batch_and_empty_plan_reject_under_degenerate_budgets (batch half), batch_call_order_does_not_change_counts_or_per_id_results, batch_rerun_is_deterministic, batch_size_sweep_exact_counts, batch_threshold_sweep_exact_per_id_counts
#[test]
fn batch_contract() {
    // §1 ABSORBED: budget_constants caps half — 4 MiB response cap minus
    // 64 KiB envelope reserve: 4194304 - 65536 = 4128768; 8 KiB errors.
    assert_eq!(MAX_BATCH_RESPONSE_BYTES, 4_194_304);
    assert_eq!(MAX_BATCH_VALUE_BYTES, 4_128_768);
    assert_eq!(MAX_BATCH_VALUE_BYTES + 64 * 1024, MAX_BATCH_RESPONSE_BYTES);
    assert_eq!(MAX_BATCH_ERROR_BYTES, 8 * 1024);
    // §2 ABSORBED: batch_limit_zero_and_huge_clamp_without_panic — limit
    // clamps [1,500] (0→1, MAX→500); batch stays Ok/all_ok/serial.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        for limit in [Some(0usize), Some(usize::MAX)] {
            let req = BatchRequest {
                limit,
                ..testkit::batch_request(vec![testkit::catalog_call("solo", "search")])
            };
            let resp =
                run_batch(testkit::config_at(temp.path()), &req).expect("degenerate batch limit runs");
            assert!(resp.all_ok);
            assert_eq!(resp.call_count, 1);
            assert_eq!(resp.mode, "serial");
            assert_eq!(resp.results.len(), 1);
            assert!(resp.results[0].ok);
        }
    }
    // §3 ABSORBED: empty_batch batch half — empty batch rejects with
    // InvalidArgs even when the limit is 0 or MAX (envelope check precedes
    // config clamp).
    {
        let temp = tempfile::tempdir().expect("tempdir");
        for limit in [Some(0usize), Some(usize::MAX)] {
            let req = BatchRequest {
                limit,
                ..testkit::batch_request(vec![])
            };
            let err = run_batch(testkit::config_at(temp.path()), &req)
                .expect_err("empty batch never runs");
            assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
        }
    }
    // §4 ABSORBED: batch_call_order_does_not_change_counts_or_per_id_results
    // — forward vs reversed agree on counts/mode/per-id (ok,value); compare
    // by id since result position may follow input order.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let forward = vec![
            testkit::catalog_call("a", "search"),
            testkit::catalog_call("b", "filter"),
            testkit::catalog_call("c", "batch"),
        ];
        let mut reversed = forward.clone();
        reversed.reverse();
        let r1 = run_batch(
            testkit::config_at(temp.path()),
            &testkit::batch_request(forward),
        )
        .expect("batch runs");
        let r2 = run_batch(
            testkit::config_at(temp.path()),
            &testkit::batch_request(reversed),
        )
        .expect("reversed runs");
        assert_eq!(r1.call_count, r2.call_count);
        assert_eq!(r1.call_count, 3);
        assert_eq!(r1.all_ok, r2.all_ok);
        assert_eq!(r1.mode, r2.mode);
        let by_id = |r: &ast_sgrep_codemode::BatchResponse| {
            r.results
                .iter()
                .map(|res| (res.id.clone(), (res.ok, res.value.clone())))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        assert_eq!(by_id(&r1), by_id(&r2));
    }
    // §5 ABSORBED: batch_rerun_is_deterministic — same batch twice agrees
    // on mode/counts/payloads (wall_ms excluded: timing, not semantics).
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let req = testkit::batch_request(vec![
            testkit::catalog_call("a", "search"),
            testkit::catalog_call("b", "filter"),
        ]);
        let r1 = run_batch(testkit::config_at(temp.path()), &req).expect("first runs");
        let r2 = run_batch(testkit::config_at(temp.path()), &req).expect("second runs");
        assert_eq!(r1.mode, r2.mode);
        assert_eq!(r1.call_count, r2.call_count);
        assert_eq!(r1.all_ok, r2.all_ok);
        assert_eq!(r1.results.len(), r2.results.len());
        for (a, b) in r1.results.iter().zip(r2.results.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.ok, b.ok);
            assert_eq!(a.value, b.value);
            assert_eq!(a.error, b.error);
        }
    }
    // §6 ABSORBED: batch_size_sweep_exact_counts — sizes 1..32 (the
    // MAX_BATCH_CALLS ceiling): call_count==N, N results, all_ok, serial.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        for n in [1usize, 2, 4, 8, 16, 32] {
            let calls: Vec<ast_sgrep_codemode::BatchCall> = (0..n)
                .map(|i| testkit::catalog_call(&format!("c{i}"), "search"))
                .collect();
            let resp = run_batch(
                testkit::config_at(temp.path()),
                &testkit::batch_request(calls),
            )
            .expect("batch runs");
            assert_eq!(resp.call_count, n, "size {n}");
            assert_eq!(resp.results.len(), n, "size {n}");
            assert!(resp.all_ok, "size {n}");
            assert_eq!(resp.mode, "serial", "size {n}");
            assert_eq!(
                resp.results.iter().filter(|r| r.ok).count(),
                n,
                "size {n}"
            );
        }
    }
    // §7 ABSORBED: batch_threshold_sweep_exact_per_id_counts — one 5-call
    // batch fans a threshold sweep: exact per-id map {t5:2,t4:3,t3:4,t2:5,t1:6}.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let thresholds = [("t5", 5.0), ("t4", 4.0), ("t3", 3.0), ("t2", 2.0), ("t1", 1.0)];
        let calls: Vec<ast_sgrep_codemode::BatchCall> = thresholds
            .iter()
            .map(|(id, min)| {
                testkit::batch_call(
                    id,
                    "filter_hits",
                    json!({"hits": scored_hits6(), "min_score": min}),
                )
            })
            .collect();
        let resp = run_batch(
            testkit::config_at(temp.path()),
            &testkit::batch_request(calls),
        )
        .expect("batch runs");
        assert!(resp.all_ok);
        assert_eq!(resp.call_count, 5);
        assert_eq!(resp.results.len(), 5);
        assert_eq!(resp.mode, "serial");
        let counts: std::collections::BTreeMap<&str, u64> = resp
            .results
            .iter()
            .map(|r| {
                assert!(r.ok, "row {} must succeed", r.id);
                let n = r.value.as_ref().expect("value")["hit_count"]
                    .as_u64()
                    .expect("count");
                (r.id.as_str(), n)
            })
            .collect();
        assert_eq!(
            counts,
            std::collections::BTreeMap::from([("t1", 6), ("t2", 5), ("t3", 4), ("t4", 3), ("t5", 2)])
        );
    }
}
