//! find numeric contract: limit totality + rank-then-truncate + determinism.
//!
//! Single-test contract suite absorbing every `find` numeric from
//! `numerical_pass{2,3}`. Lexical find over indexed tempdir fixtures only
//! (deterministic order).

use ast_sgrep_testkit::indexed_session_at;
use serde_json::json;

/// INTENT=find limit surface: limit 0→1 hit, huge→all, negative/string→config default; ranking precedes truncation (limit-1 hits equal the first hit of the limit-5 run); identical reruns yield identical hits.
/// KILLS=clamp-bound + type-coercion (`unwrap_or` config default), rank-vs-truncate-order (truncate-before-rank), nondeterministic-ranking.
/// ABSORBS=search_limit_zero_huge_and_negative_totality, find_limit_topk_is_prefix_of_topm, find_rerun_is_deterministic
#[test]
fn find_contract() {
    // §1 ABSORBED: search_limit_zero_huge_and_negative_totality — limit
    // maps `unwrap_or(config 5).clamp(1,500)`: 0→1, huge→all 3,
    // -1/string→default 3 on the 3-file fixture.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        for (name, func) in [("a.rs", "a_one"), ("b.rs", "b_two"), ("c.rs", "c_three")] {
            std::fs::write(
                temp.path().join(name),
                format!("// n2totality marker\npub fn {func}() {{}}\n"),
            )
            .expect("write");
        }
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let floored = session
            .call("find", json!({"query": "n2totality", "limit": 0}))
            .expect("limit 0 runs");
        assert_eq!(floored["hits"].as_array().expect("hits").len(), 1);
        let capped = session
            .call(
                "find",
                json!({"query": "n2totality", "limit": 1_000_000_000u64}),
            )
            .expect("huge limit runs");
        assert_eq!(capped["hits"].as_array().expect("hits").len(), 3);
        let negative = session
            .call("find", json!({"query": "n2totality", "limit": -1}))
            .expect("negative limit runs");
        assert_eq!(negative["hits"].as_array().expect("hits").len(), 3);
        let stringy = session
            .call("find", json!({"query": "n2totality", "limit": "bad"}))
            .expect("string limit runs");
        assert_eq!(stringy["hits"].as_array().expect("hits").len(), 3);
    }
    // §2 ABSORBED: find_limit_topk_is_prefix_of_topm — ranking precedes
    // truncation: limit-1 hits equal the first hit of the limit-5 run.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        for (name, func) in [("a.rs", "a_one"), ("b.rs", "b_two"), ("c.rs", "c_three")] {
            std::fs::write(
                temp.path().join(name),
                format!("// n3nest marker\npub fn {func}() {{}}\n"),
            )
            .expect("write");
        }
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let narrow = session
            .call("find", json!({"query": "n3nest", "limit": 1}))
            .expect("limit 1 runs");
        let wide = session
            .call("find", json!({"query": "n3nest", "limit": 5}))
            .expect("limit 5 runs");
        let narrow_hits = narrow["hits"].as_array().expect("hits").clone();
        let wide_hits = wide["hits"].as_array().expect("hits").clone();
        assert!(narrow_hits.len() <= wide_hits.len());
        assert_eq!(narrow_hits, wide_hits[..narrow_hits.len()]);
    }
    // §3 ABSORBED: find_rerun_is_deterministic — same lexical query twice
    // on one indexed session yields identical hits and identical JSON.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        for (name, func) in [("a.rs", "a_one"), ("b.rs", "b_two")] {
            std::fs::write(
                temp.path().join(name),
                format!("// n3det marker\npub fn {func}() {{}}\n"),
            )
            .expect("write");
        }
        let (_index_dir, mut session) = indexed_session_at(temp.path());
        let first = session
            .call("find", json!({"query": "n3det", "limit": 5}))
            .expect("first runs");
        let second = session
            .call("find", json!({"query": "n3det", "limit": 5}))
            .expect("second runs");
        assert_eq!(first["hits"], second["hits"]);
        assert_eq!(first, second);
    }
}
