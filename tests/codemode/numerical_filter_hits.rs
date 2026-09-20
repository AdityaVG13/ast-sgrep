//! filter_hits numeric contract: threshold/limit math + relations + sweep.
//!
//! Single-test contract suite absorbing every `filter_hits` numeric from
//! `numerical_pass{1,2,3,4}`. Pure transforms only — no index I/O.

use ast_sgrep_testkit as testkit;
use ast_sgrep_testkit::{hits_fixture, scored_hits5, scored_hits6};
use serde_json::json;

// Keep file-local: order-preserving nesting check for threshold relations;
// testkit has set projections but no subsequence predicate, and no sibling
// suite needs one.
fn is_subsequence(needle: &[String], haystack: &[String]) -> bool {
    let mut rest = haystack.iter();
    needle.iter().all(|n| rest.any(|h| h == n))
}

/// INTENT=filter_hits numeric surface: strict-`<` reject with inclusive boundary, missing/non-numeric scores read as 0.0, limit clamp [1,1000] with degenerate-limit fallback to max, threshold/limit monotonicity + prefix nesting, rerun determinism, and the exact 6..1 threshold sweep.
/// KILLS=comparison-flip (`<`-vs-`<=`, inverted-reject, absolute-sweep), default-value (`unwrap_or(0.0)`), clamp-bound (floor/ceiling), type-coercion (`as_f64`/`as_u64` None-paths), ordering (unstable-filter/truncate-reorder), nondeterministic-ordering (hash-iteration).
/// ABSORBS=filter_min_score_boundary_is_inclusive, filter_missing_score_defaults_to_zero, filter_limit_zero_clamps_to_one, filter_non_numeric_min_score_is_ignored, filter_non_numeric_score_defaults_to_zero, filter_negative_and_non_numeric_limit_defaults_to_max, min_score_lower_threshold_never_yields_fewer_hits, min_score_higher_result_nests_inside_lower, filter_limit_topk_is_prefix_of_topm, filter_rerun_is_deterministic, threshold_sweep_exact_survivor_counts
#[test]
fn filter_hits_contract() {
    // §1 ABSORBED: filter_min_score_boundary_is_inclusive — reject is
    // `score < min`, so score == min survives (2.0/2.001 kept, 1.999 cut).
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let out = session
            .call(
                "filter_hits",
                json!({"hits": hits_fixture(), "min_score": 2.0}),
            )
            .expect("filter runs");
        assert_eq!(out["hit_count"], json!(2));
        assert_eq!(out["hits"][0]["file"], json!("src/a.rs"));
        assert_eq!(out["hits"][1]["file"], json!("src/c.rs"));
    }
    // §2 ABSORBED: filter_missing_score_defaults_to_zero — missing score is
    // `unwrap_or(0.0)`: passes min 0.0/-1.0, fails min 0.5.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let hits = json!([
            {"kind": "def", "file": "src/noscore.rs"},
            {"kind": "def", "file": "src/zero.rs", "score": 0.0},
        ]);
        let at_zero = session
            .call("filter_hits", json!({"hits": hits, "min_score": 0.0}))
            .expect("min 0 runs");
        assert_eq!(at_zero["hit_count"], json!(2));
        let negative = session
            .call(
                "filter_hits",
                json!({"hits": hits_fixture(), "min_score": -1.0}),
            )
            .expect("negative min runs");
        assert_eq!(negative["hit_count"], json!(3));
        let positive = session
            .call("filter_hits", json!({"hits": hits, "min_score": 0.5}))
            .expect("min 0.5 runs");
        assert_eq!(positive["hit_count"], json!(0));
        assert_eq!(positive["hits"], json!([]));
    }
    // §3 ABSORBED: filter_limit_zero_clamps_to_one — limit clamps
    // [1,1000]: 0→1 hit survives, 99999→all 3, no error.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let floored = session
            .call("filter_hits", json!({"hits": hits_fixture(), "limit": 0}))
            .expect("limit 0 runs");
        assert_eq!(floored["hit_count"], json!(1));
        assert_eq!(floored["hits"][0]["file"], json!("src/a.rs"));
        let capped = session
            .call(
                "filter_hits",
                json!({"hits": hits_fixture(), "limit": 99999}),
            )
            .expect("huge limit runs");
        assert_eq!(capped["hit_count"], json!(3));
    }
    // §4 ABSORBED: filter_non_numeric_min_score_is_ignored — `as_f64()`
    // None (string/bool/null/object/array/NaN→Null) disables filtering.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let thresholds = vec![
            json!("2.0"),
            json!(true),
            json!(null),
            json!({"n": 1}),
            json!([1]),
            json!(f64::NAN),
        ];
        for min in thresholds {
            let out = session
                .call(
                    "filter_hits",
                    json!({"hits": hits_fixture(), "min_score": min}),
                )
                .expect("non-numeric min runs");
            assert_eq!(out["hit_count"], json!(3));
            assert_eq!(out["hits"].as_array().expect("hits").len(), 3);
        }
    }
    // §5 ABSORBED: filter_non_numeric_score_defaults_to_zero — string/
    // bool/null/missing scores read as 0.0 across min 0.0/0.5/-1.0.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let hits = json!([
            {"kind": "def", "file": "src/s.rs", "score": "high"},
            {"kind": "def", "file": "src/b.rs", "score": true},
            {"kind": "def", "file": "src/n.rs", "score": null},
            {"kind": "def", "file": "src/m.rs"},
            {"kind": "def", "file": "src/z.rs", "score": 0.0},
        ]);
        let at_zero = session
            .call("filter_hits", json!({"hits": hits, "min_score": 0.0}))
            .expect("min 0 runs");
        assert_eq!(at_zero["hit_count"], json!(5));
        let at_half = session
            .call("filter_hits", json!({"hits": hits, "min_score": 0.5}))
            .expect("min 0.5 runs");
        assert_eq!(at_half["hit_count"], json!(0));
        assert_eq!(at_half["hits"], json!([]));
        let negative = session
            .call("filter_hits", json!({"hits": hits, "min_score": -1.0}))
            .expect("negative min runs");
        assert_eq!(negative["hit_count"], json!(5));
    }
    // §6 ABSORBED: filter_negative_and_non_numeric_limit_defaults_to_max —
    // `as_u64()` None falls back to 1000 (all survive, never error).
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let limits = vec![
            json!(-1),
            json!(1.5),
            json!("3"),
            json!(true),
            json!(null),
            json!({"n": 1}),
        ];
        for limit in limits {
            let out = session
                .call(
                    "filter_hits",
                    json!({"hits": hits_fixture(), "limit": limit}),
                )
                .expect("degenerate limit runs");
            assert_eq!(out["hit_count"], json!(3));
        }
    }
    // §7 ABSORBED: min_score_lower_threshold_never_yields_fewer_hits —
    // counts non-decreasing as min descends 6.0→-1.0 through every score.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let mut prev_count = usize::MAX;
        let mut first = true;
        for min in [6.0, 5.0, 4.5, 4.0, 3.0, 2.0, 1.0, 0.0, -1.0] {
            let out = session
                .call(
                    "filter_hits",
                    json!({"hits": scored_hits5(), "min_score": min}),
                )
                .expect("filter runs");
            let count = out["hit_count"].as_u64().expect("count") as usize;
            assert_eq!(count, testkit::hit_files(&out).len());
            if !first {
                assert!(count >= prev_count, "min {min}: {count} < {prev_count}");
            }
            first = false;
            prev_count = count;
        }
    }
    // §8 ABSORBED: min_score_higher_result_nests_inside_lower — filter
    // preserves input order, so higher-threshold files nest as a
    // subsequence of every lower-threshold list.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let mut prev_files: Vec<String> = Vec::new();
        let mut first = true;
        for min in [5.0, 4.0, 3.0, 2.0, 1.0, 0.0] {
            let out = session
                .call(
                    "filter_hits",
                    json!({"hits": scored_hits5(), "min_score": min}),
                )
                .expect("filter runs");
            let files = testkit::hit_files(&out);
            if !first {
                assert!(
                    is_subsequence(&prev_files, &files),
                    "min-descending break: {prev_files:?} not subsequence of {files:?}"
                );
            }
            first = false;
            prev_files = files;
        }
    }
    // §9 ABSORBED: filter_limit_topk_is_prefix_of_topm — limit k<m yields
    // a strict prefix: counts grow and out_k==out_m[..k].
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let mut prev_files: Vec<String> = Vec::new();
        for limit in [1u64, 2, 3, 5, 100] {
            let out = session
                .call(
                    "filter_hits",
                    json!({"hits": scored_hits5(), "limit": limit}),
                )
                .expect("filter runs");
            let files = testkit::hit_files(&out);
            assert!(files.len() >= prev_files.len());
            assert_eq!(&files[..prev_files.len()], &prev_files[..]);
            prev_files = files;
        }
    }
    // §10 ABSORBED: filter_rerun_is_deterministic — same call twice on one
    // session yields byte-identical JSON.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let args = json!({"hits": scored_hits5(), "min_score": 3.0, "limit": 10});
        let first = session
            .call("filter_hits", args.clone())
            .expect("first runs");
        let second = session.call("filter_hits", args).expect("second runs");
        assert_eq!(first, second);
    }
    // §11 ABSORBED: threshold_sweep_exact_survivor_counts — absolute
    // survivor vector [1,2,3,4,5,6,6] over min 6..0, exact mid-threshold
    // order, exactly 8 calls.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        let expected = [
            (6.0, 1),
            (5.0, 2),
            (4.0, 3),
            (3.0, 4),
            (2.0, 5),
            (1.0, 6),
            (0.0, 6),
        ];
        for (min, count) in expected {
            let out = session
                .call(
                    "filter_hits",
                    json!({"hits": scored_hits6(), "min_score": min}),
                )
                .expect("filter runs");
            assert_eq!(out["hit_count"], json!(count), "min {min}");
            assert_eq!(testkit::hit_files(&out).len(), count);
            assert_eq!(
                testkit::hit_files(&out)[0],
                "f6.rs".to_string(),
                "min {min}"
            );
        }
        let out = session
            .call(
                "filter_hits",
                json!({"hits": scored_hits6(), "min_score": 3.0}),
            )
            .expect("mid threshold reruns");
        assert_eq!(
            testkit::hit_files(&out),
            vec!["f6.rs", "f5.rs", "f4.rs", "f3.rs"]
        );
        assert_eq!(session.call_count(), 8);
    }
}
