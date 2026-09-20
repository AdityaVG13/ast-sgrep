//! CLI numerical: search surface contract suite.
//!
//! Canonical per-surface successor of the search arms of
//! `numerical_pass{3,4}.rs` per `tests/catalog/numerical-cli.md`: limit
//! growth/prefix/counts, exact file membership with term bytes in excerpts,
//! search hit determinism, and eval metric determinism. Eval-arithmetic arms
//! live in `numerical_eval.rs`; flag-cap/window arms in
//! `numerical_limits.rs`.
//!
//! Absorption map (catalog VERDICTs): 2 KEEP anchors stay standalone;
//! `search_limit_contract` absorbs 3, `search_determinism_contract` absorbs
//! 1. 4 tests.

use ast_sgrep_testkit::{
    indexed_project_n, run_eval_ok, run_index_json_noembed, run_search_json, write_fixture,
    write_gold,
};
use tempfile::TempDir;

/// INTENT: Repeated search yields byte-identical hit payload.
/// KILLS: nondeterminism (hash order / timestamps in hits).
/// ABSORBS: none (KEEP anchor).
#[test]
fn search_rerun_yields_identical_hit_bytes() {
    // Same query twice: the serialized hit payload must be byte-identical.
    let (_temp, root) = indexed_project_n("wobblebuild_n3det", 6);
    let a = run_search_json(&root, 10, "wobblebuild_n3det");
    let b = run_search_json(&root, 10, "wobblebuild_n3det");
    let bytes_a = serde_json::to_string(&a["hits"]).unwrap();
    let bytes_b = serde_json::to_string(&b["hits"]).unwrap();
    assert!(!a["hits"].as_array().unwrap().is_empty(), "{a}");
    assert_eq!(
        bytes_a, bytes_b,
        "rerun hit payload bytes must be identical"
    );
}

/// INTENT: Limits 1..7 pin counts 1..5,5,5 with limit echo and exact file
/// set.
/// KILLS: limit-count-off-by-one / limit-echo-drop.
/// ABSORBS: none (KEEP anchor).
#[test]
fn search_limit_sweep_hit_counts_match_hand_values() {
    // Five files each hold one matching line (dedup merges the lexical and
    // symbol evidence per file locus into one hit), so the hit count is
    // exactly min(limit, 5): limits 1..7 -> 1,2,3,4,5,5,5. The envelope
    // echoes the requested limit, and at full coverage the sorted file set
    // is exactly the five fixtures.
    let (_temp, root) = indexed_project_n("wobble_n4_lim", 5);
    for (limit, expected) in [
        (1usize, 1usize),
        (2, 2),
        (3, 3),
        (4, 4),
        (5, 5),
        (6, 5),
        (7, 5),
    ] {
        let value = run_search_json(&root, limit, "wobble_n4_lim");
        assert_eq!(value["ok"], true, "limit={limit}: {value}");
        assert_eq!(value["limit"], limit, "limit echo: {value}");
        let hits = value["hits"].as_array().expect("hits array");
        assert_eq!(hits.len(), expected, "limit={limit}: {value}");
    }
    let value = run_search_json(&root, 10, "wobble_n4_lim");
    let mut files: Vec<&str> = value["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["file"].as_str().expect("file string"))
        .collect();
    files.sort();
    let mut basenames: Vec<&str> = files
        .iter()
        .map(|f| f.rsplit('/').next().expect("basename"))
        .collect();
    basenames.sort();
    assert_eq!(
        basenames,
        ["m00.rs", "m01.rs", "m02.rs", "m03.rs", "m04.rs"]
    );
}

/// INTENT: One contract for search limit semantics: growing --limit never
/// shrinks hits and never reports more than the limit; smaller-limit hits
/// are an exact-JSON prefix of larger-limit hits (rank-then-truncate); and
/// membership is exact with query-term bytes in every excerpt.
/// KILLS: limit-count-inversion, rank-then-truncate-violation (per-limit
/// re-rank), membership-pollution / excerpt-drop.
/// ABSORBS: search_limit_growth_never_shrinks_hits,
/// search_smaller_limit_hits_are_a_prefix_of_larger,
/// search_exact_file_membership_and_term_bytes.
#[test]
fn search_limit_contract() {
    // Arm 1 (absorbed: search_limit_growth_never_shrinks_hits). Twelve files
    // share one term; sweeping the limit upward must never drop a hit, and
    // no run may report more hits than its limit.
    {
        let (_temp, root) = indexed_project_n("wobblebuild_n3lim", 12);
        let mut prev = 0usize;
        for limit in [1usize, 2, 5, 10] {
            let value = run_search_json(&root, limit, "wobblebuild_n3lim");
            assert_eq!(value["ok"], true, "{value}");
            let n = value["hits"].as_array().expect("hits array").len();
            assert!(
                n <= limit,
                "hits ({n}) must not exceed limit ({limit}): {value}"
            );
            assert!(
                n >= prev,
                "larger limit must never yield fewer hits: {prev} -> {n} at limit {limit}"
            );
            prev = n;
        }
        assert!(prev > 1, "sweep must discriminate: final hit count {prev}");
    }
    // Arm 2 (absorbed: search_smaller_limit_hits_are_a_prefix_of_larger).
    // Rank-then-truncate: the limit-2 hit list must equal the first two
    // entries of the limit-10 hit list, as full JSON objects.
    {
        let (_temp, root) = indexed_project_n("wobblebuild_n3pfx", 12);
        let small = run_search_json(&root, 2, "wobblebuild_n3pfx");
        let large = run_search_json(&root, 10, "wobblebuild_n3pfx");
        let small_hits = small["hits"].as_array().expect("hits array");
        let large_hits = large["hits"].as_array().expect("hits array");
        assert_eq!(
            small_hits.len(),
            2,
            "limit 2 must yield exactly 2 hits: {small}"
        );
        assert!(
            large_hits.len() >= small_hits.len(),
            "larger limit must cover smaller: {large}"
        );
        assert_eq!(
            &large_hits[..small_hits.len()],
            &small_hits[..],
            "small-limit hits must be a prefix of large-limit hits"
        );
    }
    // Arm 3 (absorbed: search_exact_file_membership_and_term_bytes). Three
    // files hold the term; a fourth holds a token-disjoint symbol, so no
    // weak channel match can pollute the result: at limit 10 the hit set is
    // exactly the three matching files, and every hit excerpt carries the
    // query-term bytes.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        for name in ["m0.rs", "m1.rs", "m2.rs"] {
            write_fixture(&root, name, "fn wobble_n4_mem() {}\n");
        }
        write_fixture(&root, "nomatch.rs", "fn plain_unrelated_symbol() {}\n");
        run_index_json_noembed(&root);
        let value = run_search_json(&root, 10, "wobble_n4_mem");
        assert_eq!(value["ok"], true, "{value}");
        assert_eq!(value["limit"], 10, "{value}");
        let hits = value["hits"].as_array().expect("hits array").clone();
        assert_eq!(hits.len(), 3, "{value}");
        let mut basenames: Vec<&str> = hits
            .iter()
            .map(|h| {
                h["file"]
                    .as_str()
                    .expect("file string")
                    .rsplit('/')
                    .next()
                    .expect("basename")
            })
            .collect();
        basenames.sort();
        assert_eq!(basenames, ["m0.rs", "m1.rs", "m2.rs"]);
        for hit in &hits {
            let excerpt = hit["excerpt"].as_str().expect("excerpt string");
            assert!(
                excerpt.contains("wobble_n4_mem"),
                "excerpt must carry the term bytes: {excerpt}"
            );
        }
    }
}

/// INTENT: Repeated eval yields byte-identical metric payloads (queries +
/// aggregate serialization; the full envelope embeds a fresh temp index path
/// per run, so full-stdout comparison is excluded by design).
/// KILLS: nondeterminism in metric payload.
/// ABSORBS: eval_rerun_yields_identical_metric_bytes.
#[test]
fn search_determinism_contract() {
    // Same gold + root twice: the serialized metric payload (queries +
    // aggregate) must be byte-identical. The full envelope is excluded
    // because it embeds a fresh temp index path per run.
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["d1.rs", "d2.rs", "d3.rs"] {
        write_fixture(&root, name, "fn wobblebuild_n3det() {}\n");
    }
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n3", "queries": [
            {"name": "trio", "query": "wobblebuild_n3det", "k": 2,
             "relevant": [{"file": "d1.rs"}, {"file": "d2.rs"}, {"file": "d3.rs"}]},
            {"name": "miss", "query": "qqqxq_missing_zzz", "k": 2,
             "relevant": [{"file": "d1.rs"}]}
        ]}),
    );
    let a = run_eval_ok(&gold, &root);
    let b = run_eval_ok(&gold, &root);
    for pointer in ["/queries", "/aggregate"] {
        let bytes_a = serde_json::to_string(a.pointer(pointer).unwrap()).unwrap();
        let bytes_b = serde_json::to_string(b.pointer(pointer).unwrap()).unwrap();
        assert_eq!(bytes_a, bytes_b, "rerun {pointer} bytes must be identical");
    }
}
