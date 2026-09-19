//! select numeric contract: limit truncation without clamp or reorder.
//!
//! Single-test contract suite absorbing every `select` numeric from
//! `numerical_pass{1,2,3}`. Pure transforms only — no index I/O, no
//! file-local helpers (all fixtures are inline JSON).

use ast_sgrep_testkit as testkit;
use serde_json::json;

/// INTENT=select limit surface: limit 0 truncates to [] (no clamp — opposite of filter), negative/huge/non-numeric limits mean no-truncate (all rows kept), and limit k equals the first k rows of the unlimited projection.
/// KILLS=truncate-vs-clamp (added-clamp), type-coercion (`as_u64` None means no-truncate), ordering (select-reorder).
/// ABSORBS=select_limit_zero_empties_array, select_negative_huge_and_non_numeric_limit_keeps_all, select_limit_topk_is_prefix_of_unlimited
#[test]
fn select_contract() {
    // §1 ABSORBED: select_limit_zero_empties_array — select has NO clamp:
    // `truncate(0)` yields []. Boundary opposite of filter_hits.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let out = session
            .call(
                "select",
                json!({"value": [{"a": 1}, {"a": 2}], "fields": ["a"], "limit": 0}),
            )
            .expect("select runs");
        assert_eq!(out, json!([]));
    }
    // §2 ABSORBED: select_negative_huge_and_non_numeric_limit_keeps_all —
    // `as_u64()` None means no truncate: -1/huge/float/string/null keep
    // all 3 rows.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let value = json!([{"a": 1}, {"a": 2}, {"a": 3}]);
        let limits = vec![
            json!(-1),
            json!(u64::MAX),
            json!(1.5),
            json!("1"),
            json!(null),
        ];
        for limit in limits {
            let out = session
                .call(
                    "select",
                    json!({"value": value, "fields": ["a"], "limit": limit}),
                )
                .expect("degenerate select limit runs");
            assert_eq!(out.as_array().expect("array").len(), 3);
            assert_eq!(out, json!([{"a": 1}, {"a": 2}, {"a": 3}]));
        }
    }
    // §3 ABSORBED: select_limit_topk_is_prefix_of_unlimited — select
    // truncates without reordering: limit k rows equal the first k rows of
    // the unlimited projection for k in 1..3.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let value = json!([{"a": 1}, {"a": 2}, {"a": 3}, {"a": 4}]);
        let full = session
            .call("select", json!({"value": value, "fields": ["a"]}))
            .expect("unlimited select runs");
        let full_rows = full.as_array().expect("array").clone();
        assert_eq!(full_rows.len(), 4);
        for k in [1u64, 2, 3] {
            let out = session
                .call(
                    "select",
                    json!({"value": value, "fields": ["a"], "limit": k}),
                )
                .expect("limited select runs");
            let rows = out.as_array().expect("array").clone();
            assert_eq!(rows.len(), k as usize);
            assert_eq!(rows, full_rows[..k as usize]);
        }
    }
}
