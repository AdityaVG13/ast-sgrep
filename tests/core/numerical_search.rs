//! Canonical numerical contracts: flat ANN search, finish margins, ANN gates,
//! chain decay, IVF selection.
//!
//! Consolidates the N1 (exactness) + N2 (totality) legs of
//! `numerical_pass{1,2}.rs` for the search/finish layer into ONE contract test
//! per function, plus the three KEEP standalone tests that own their surface.
//! Each test carries INTENT + KILLS + ABSORBS.
//!
//! Every expectation is HAND-DERIVED (rational or float derivation in the
//! comment above it). Nothing here snapshots production output.

use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::search::{finish_response, margin_is_decisive, HitKind};
use ast_sgrep_core::semantic_ann::{
    ann_result_is_sufficient, ann_threshold, should_use_ann, SemanticAnnIndex,
    DEFAULT_ADAPTIVE_PROBE_PERCENT, DEFAULT_ANN_THRESHOLD,
};
use ast_sgrep_core::{IndexStore, ParsedQuery};
use ast_sgrep_testkit::{chain_store, finish_options, mk_hit};

/// INTENT: `SemanticAnnIndex::search_flat` cosine ranking + 0.08 gate +
/// hostile-vector gating (brute-force-path contract).
/// KILLS: gate/normalize-mutant, zero-fill/guard-mutant.
/// ABSORBS: `search_flat_brute_force_cosine_oracle` (N1) +
/// `search_flat_hostile_vectors_gate_or_normalize` (N2).
#[test]
fn search_flat_contract() {
    // --- Exactness: empty build -> no centroids -> search_flat takes
    // brute_force_flat: cosine(query, row) gated by
    // exceeds_threshold(sim, 0.08), top-k sorted.
    let index = SemanticAnnIndex::build_from_flat(&[], 2);
    assert_eq!(index.centroid_count(), 0);
    assert!(index.candidate_indices(&[1.0, 0.0], None).is_empty());
    // Rows: [1,0] -> cos 1.0; [0,1] -> cos 0.0 (gated: 0 is not > 0.08+1ulp);
    // [1,1] -> 1/sqrt(2) ~= 0.70710678.
    let flat = [1.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
    let got = index.search_flat(&flat, 2, &[1.0, 0.0], 10);
    assert_eq!(got.len(), 2, "orthogonal row must be gated out: {got:?}");
    assert_eq!(got[0], (0, 1.0)); // 1/(1*1) exactly
    assert_eq!(got[1].0, 2);
    // Epsilon 1e-6: production casts an f64 1/sqrt(2) to f32 (possible 1-ulp
    // double-rounding vs f32-native); 1e-6 still kills gate/order mutants.
    assert!(
        (got[1].1 - 0.70710677f32).abs() < 1e-6,
        "cos = {}, want 1/sqrt(2)",
        got[1].1
    );
    // Limit truncates after ranking; limit 0 and empty corpora are empty.
    assert_eq!(index.search_flat(&flat, 2, &[1.0, 0.0], 1), vec![(0, 1.0)]);
    assert!(index.search_flat(&flat, 2, &[1.0, 0.0], 0).is_empty());
    assert!(index.search_flat(&[], 2, &[1.0, 0.0], 5).is_empty());
    // Query normalization pin: [3,0]/|..| = [1,0] exactly -> bit-identical rows.
    assert_eq!(
        index.search_flat(&flat, 2, &[3.0, 0.0], 10),
        index.search_flat(&flat, 2, &[1.0, 0.0], 10)
    );

    // --- Totality: non-finite query components are zeroed before the norm; a
    // fully-zeroed query has norm 0 -> every cosine is 0.0 -> gated -> empty.
    for query in [
        [f32::NAN, 0.0],
        [f32::NAN, f32::NAN],
        [f32::INFINITY, 0.0],
        [f32::NEG_INFINITY, f32::INFINITY],
        [0.0, 0.0],
    ] {
        assert!(
            index.search_flat(&flat, 2, &query, 10).is_empty(),
            "query {query:?} must gate to empty"
        );
    }
    // NaN corpus row: cosine skips non-finite components, leaving nb == 0 ->
    // sim 0.0 -> gated. The good row still ranks first with sim exactly 1.0.
    let nan_flat = [1.0f32, 0.0, f32::NAN, f32::NAN];
    let got = index.search_flat(&nan_flat, 2, &[1.0, 0.0], 10);
    assert_eq!(got, vec![(0, 1.0)]);
    // f32 norm overflow: 1e30^2 overflows f32 -> norm +inf -> zero-fill ->
    // gated to empty (extreme magnitude drops, no panic, no garbage rank).
    assert!(index.search_flat(&flat, 2, &[1e30, 0.0], 10).is_empty());
    // f32 norm underflow: 1e-40^2 underflows to 0 -> norm 0 -> same drop.
    assert!(index.search_flat(&flat, 2, &[1e-40, 0.0], 10).is_empty());
    // Safe extremes normalize exactly: [1e10,0]/1e10 = [1,0]; row [1e-10,0]
    // has f64 cosine 1e-10/(1*1e-10) = 1.0 -> sim exactly 1.0.
    assert_eq!(
        index.search_flat(&flat, 2, &[1e10, 0.0], 10),
        index.search_flat(&flat, 2, &[1.0, 0.0], 10)
    );
    let tiny_row = [1e-10f32, 0.0];
    assert_eq!(
        index.search_flat(&tiny_row, 2, &[1.0, 0.0], 10),
        vec![(0, 1.0)]
    );
    // dim 0 is guarded before any `/ dim` (no integer-div panic); a huge
    // limit is just a cap, not an allocation.
    assert!(index.search_flat(&flat, 0, &[1.0, 0.0], 5).is_empty());
    assert_eq!(index.search_flat(&flat, 2, &[1.0, 0.0], usize::MAX).len(), 2);
    // build_from_flat on all-NaN vectors: rows zero-fill, k-means runs on
    // zeros deterministically, search still gates everything to empty.
    let nan_index = SemanticAnnIndex::build_from_flat(&[f32::NAN; 8], 2);
    assert!(nan_index
        .search_flat(&[f32::NAN; 8], 2, &[1.0, 0.0], 10)
        .is_empty());
}

/// INTENT: `finish_response` margins/confidence + `margin_is_decisive`
/// boundary + hostile-score retention (finish contract).
/// KILLS: margin/confidence-formula-mutant, margin-gate/drop-mutant.
/// ABSORBS: `finish_margins_confidence_and_decisive_boundary` (N1) +
/// `finish_hostile_scores_survive_with_zero_margin` (N2).
#[test]
fn finish_margin_contract() {
    // --- Exactness: signal ladder with bit-exact margins.
    // 0.75 -> margin 0.75-0.5 = 0.25; the 0.5 tie -> both 0.0 (tie rule);
    // 0.125 -> 0.125-0.0625 = 0.0625; last -> 0.0.
    // (All operands exact in binary, so all margins below are bit-exact.)
    let dir = tempfile::tempdir().unwrap();
    let parsed = ParsedQuery::parse("foo"); // single term -> score ordering
    let mut x1 = mk_hit(HitKind::Asgrep, "f.rs", 1, 0.125);
    x1.contributors = vec![
        HitKind::Def,
        HitKind::Caller,
        HitKind::Graph,
        HitKind::Anchor,
        HitKind::Embed,
    ];
    let mut y1 = mk_hit(HitKind::Asgrep, "g.rs", 1, 0.0625);
    y1.contributors = vec![HitKind::Def, HitKind::Caller, HitKind::Graph, HitKind::Anchor];
    let mut d1 = mk_hit(HitKind::Def, "d.rs", 1, 1.0);
    d1.contributors = vec![HitKind::Caller, HitKind::Embed];
    let hits = vec![
        mk_hit(HitKind::Asgrep, "a.rs", 1, 0.75),
        mk_hit(HitKind::Asgrep, "b.rs", 2, 0.5),
        mk_hit(HitKind::Asgrep, "c.rs", 3, 0.5),
        d1,
        mk_hit(HitKind::Caller, "e.rs", 4, 0.5),
        x1,
        y1,
    ];
    let response = finish_response(&parsed, &finish_options(dir.path(), 10), hits, false);
    assert_eq!(response.hits.len(), 7);
    let by_file = |f: &str| response.hits.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("a.rs").margin, 0.25);
    assert_eq!(by_file("b.rs").margin, 0.0);
    assert_eq!(by_file("c.rs").margin, 0.0);
    assert_eq!(by_file("f.rs").margin, 0.0625);
    assert_eq!(by_file("g.rs").margin, 0.0);
    // Structural ladder (Def 1.0, Caller 0.5): margins 0.5 and 0.0.
    assert_eq!(by_file("d.rs").margin, 0.5);
    assert_eq!(by_file("e.rs").margin, 0.0);
    // Confidence = base(strongest signal) + min(n_contrib-1,3)*0.08, <= 0.99.
    // Bases: Exact 0.75, Structural 0.60, Semantic 0.35.
    assert_eq!(by_file("a.rs").confidence, 0.75); // Exact, no contributors
    assert_eq!(by_file("e.rs").confidence, 0.60); // Structural, none
    // d.rs: strongest Structural, 2 contributors -> 0.60 + 1*0.08 ~= 0.68.
    // Epsilon 1e-12: 0.6 and 0.08 are inexact in binary; the sum is within
    // 1 ulp of the 0.68 literal either way.
    assert!((by_file("d.rs").confidence - 0.68).abs() < 1e-12);
    // f.rs: Exact base + min(5-1,3)=3 steps: 0.75 + 3*0.08 == 0.99 exactly
    // in IEEE double (verified), then clamped to exactly 0.99.
    assert_eq!(by_file("f.rs").confidence, 0.99);
    // g.rs: min(4-1,3)=3, same agreement -> saturation pin, bit-identical.
    assert_eq!(by_file("g.rs").confidence, 0.99);
    // margin_is_decisive: score > 0 AND margin >= 0.10 * score.
    let mut h = mk_hit(HitKind::Def, "a.rs", 1, 1.0);
    h.margin = 0.1;
    assert!(margin_is_decisive(&h)); // 0.1 >= 0.10*1.0, inclusive edge
    h.margin = 0.0999999999;
    assert!(!margin_is_decisive(&h)); // just below the 10% line
    h.score = 2.0;
    h.margin = 0.2;
    assert!(margin_is_decisive(&h)); // ratio scales: 0.2 >= 0.2
    h.score = 0.0;
    h.margin = 0.0;
    assert!(!margin_is_decisive(&h)); // score > 0 gate
    h.score = -1.0;
    h.margin = 5.0;
    assert!(!margin_is_decisive(&h)); // negative scores never decisive

    // --- Totality: hostile scores retained with zero margin, never dropped.
    // Embed group: 0.75/0.5 rank normally (margin 0.25 exact), NaN is gated
    // out of margin ranking (is_finite filter) but the HIT is retained.
    // Asgrep +-inf: same gate -> margin 0, retained. Def 1e308 vs Caller
    // 5e-324 share the Structural group: delta rounds back to exactly 1e308.
    let dir = tempfile::tempdir().unwrap();
    let parsed = ParsedQuery::parse("foo"); // single term -> score ordering
    let hits = vec![
        mk_hit(HitKind::Embed, "e1.rs", 1, 0.75),
        mk_hit(HitKind::Embed, "e2.rs", 2, 0.5),
        mk_hit(HitKind::Embed, "enan.rs", 3, f64::NAN),
        mk_hit(HitKind::Asgrep, "pinf.rs", 1, f64::INFINITY),
        mk_hit(HitKind::Asgrep, "ninf.rs", 2, f64::NEG_INFINITY),
        mk_hit(HitKind::Def, "big.rs", 1, 1e308),
        mk_hit(HitKind::Caller, "tiny.rs", 2, 5e-324),
    ];
    let response = finish_response(&parsed, &finish_options(dir.path(), 10), hits, false);
    assert_eq!(response.hits.len(), 7, "finish must not drop hostile scores");
    let by_file = |f: &str| response.hits.iter().find(|h| h.file == f).unwrap();
    assert_eq!(by_file("e1.rs").margin, 0.25); // 0.75-0.5 exact in binary
    assert_eq!(by_file("e2.rs").margin, 0.0); // last finite in group
    assert_eq!(by_file("enan.rs").margin, 0.0); // non-finite gated from ranking
    assert!(by_file("enan.rs").score.is_nan()); // ...but the score is kept
    assert_eq!(by_file("pinf.rs").margin, 0.0);
    assert_eq!(by_file("ninf.rs").margin, 0.0);
    assert_eq!(by_file("big.rs").margin, 1e308); // 1e308-5e-324 rounds to 1e308
    assert_eq!(by_file("tiny.rs").margin, 0.0);
    // Confidence never touches the score: always finite and within the cap.
    for hit in &response.hits {
        assert!(
            hit.confidence.is_finite() && (0.0..=0.99).contains(&hit.confidence),
            "{}: confidence = {}",
            hit.file,
            hit.confidence
        );
    }
    // margin_is_decisive totality: `score > 0 && margin >= 0.10 * score`.
    let mut h = mk_hit(HitKind::Def, "a.rs", 1, f64::NAN);
    h.margin = 1.0;
    assert!(!margin_is_decisive(&h)); // NaN > 0 is false
    h.score = f64::INFINITY;
    h.margin = 1e308;
    assert!(!margin_is_decisive(&h)); // 1e308 >= 0.1 * inf = inf is false
    h.margin = f64::INFINITY;
    assert!(margin_is_decisive(&h)); // inf > 0 and inf >= 0.1*inf
    h.score = 1e308;
    h.margin = 1e308;
    assert!(margin_is_decisive(&h)); // 1e308 >= ~1.0000000000000001e307
    // 0.1 * 5e-324 underflows to exactly 0.0, so any positive margin wins
    // while the score>0 gate still holds.
    h.score = 5e-324;
    h.margin = 5e-324;
    assert!(margin_is_decisive(&h));
    h.score = 0.0;
    h.margin = 5e-324;
    assert!(!margin_is_decisive(&h)); // score > 0 gate
}

/// INTENT: ANN threshold default/override/env/garbage + should_use_ann edge +
/// sufficiency truth table; sole toucher of ASGREP_ANN_THRESHOLD.
/// KILLS: threshold-comparison-mutant.
/// ABSORBS: none (KEEP standalone: `ann_threshold_boundary_table`).
#[test]
fn ann_threshold_boundary_table() {
    // SOLE test touching ASGREP_ANN_THRESHOLD (parallel-safe).
    let key = "ASGREP_ANN_THRESHOLD";
    std::env::remove_var(key);
    assert_eq!(DEFAULT_ANN_THRESHOLD, 2000);
    assert_eq!(DEFAULT_ADAPTIVE_PROBE_PERCENT, 90);
    assert_eq!(ann_threshold(None), 2000);
    assert_eq!(ann_threshold(Some(5)), 5); // explicit override wins
    // should_use_ann is `count >= threshold` (inclusive lower edge).
    assert!(!should_use_ann(1999, None));
    assert!(should_use_ann(2000, None));
    assert!(!should_use_ann(9, Some(10)));
    assert!(should_use_ann(10, Some(10)));
    // Env supplies the default; override still wins; garbage falls back.
    std::env::set_var(key, "50");
    assert_eq!(ann_threshold(None), 50);
    assert_eq!(ann_threshold(Some(7)), 7);
    std::env::set_var(key, "bogus");
    assert_eq!(ann_threshold(None), 2000);
    std::env::remove_var(key);
    // ann_result_is_sufficient: found >= min(limit, total).
    assert!(ann_result_is_sufficient(5, 10, 5)); // 5 >= min(5,10)=5
    assert!(!ann_result_is_sufficient(4, 10, 5)); // 4 < 5
    assert!(ann_result_is_sufficient(10, 3, 5)); // 10 >= min(5,3)=3
    assert!(!ann_result_is_sufficient(2, 3, 5)); // 2 < 3
    assert!(ann_result_is_sufficient(0, 0, 0)); // 0 >= 0
    assert!(!ann_result_is_sufficient(0, 5, 3)); // empty never sufficient
}

/// INTENT: hostile chain decay bit-propagates to hops without panic; total
/// order kept; NaN-decay determinism.
/// KILLS: hop-formula-mutant (self-derived via same multiply but kills
/// non-multiplicative formulas).
/// ABSORBS: none (KEEP standalone: `chain_decay_hostile_propagates_without_panic`).
#[test]
fn chain_decay_hostile_propagates_without_panic() {
    let temp = tempfile::tempdir().unwrap();
    let store = chain_store(&temp);
    // hop_score = seed.score * decay, one multiply, no guards: the decay
    // propagates bit-exactly. Ordering uses partial_cmp+Equal fallback plus
    // file/symbol/line tiebreaks, so even NaN scores keep a total order.
    for decay in [f64::NAN, f64::INFINITY, -2.0, -0.0] {
        let response = expand_chain(
            &store,
            "defs:foobar",
            &ChainConfig {
                max_depth: 1,
                top_n: 4,
                limit: 8,
                decay_factor: decay,
            },
        )
        .unwrap();
        assert!(
            response.seeds.iter().any(|n| n.symbol.as_deref() == Some("FooBar")),
            "fixture must seed FooBar (decay {decay})"
        );
        assert!(
            response.seeds.iter().all(|n| n.score.is_finite()),
            "seeds come from the search pipeline: always finite"
        );
        let depth1: Vec<_> = response.nodes.iter().filter(|n| n.depth == 1).collect();
        assert_eq!(depth1.len(), 1, "one hop: FooBar -calls-> baz");
        assert_eq!(depth1[0].file, "callee.rs");
        // Every depth-1 node descends from some seed via the identical op, so
        // its bits must equal one of the per-seed expectations bit-for-bit.
        // (NaN hops: same multiply -> same payload on this machine.)
        let expected: Vec<u64> = response
            .seeds
            .iter()
            .map(|s| (s.score * decay).to_bits())
            .collect();
        for node in &depth1 {
            assert!(
                expected.contains(&node.score.to_bits()),
                "hop score {:x} not in {expected:x?} (decay {decay})",
                node.score.to_bits()
            );
        }
        if decay == -0.0 {
            assert_eq!(depth1[0].score, 0.0); // x * -0.0 == 0 either sign
        }
        if decay.is_nan() {
            assert!(depth1[0].score.is_nan());
        }
    }

    // NaN-decay determinism: same store + same query -> identical node stream
    // (file, symbol, depth, score bits), twice in a row.
    let key = |store: &IndexStore| {
        expand_chain(
            store,
            "defs:foobar",
            &ChainConfig {
                max_depth: 1,
                top_n: 4,
                limit: 8,
                decay_factor: f64::NAN,
            },
        )
        .unwrap()
        .nodes
        .iter()
        .map(|n| {
            (
                n.file.clone(),
                n.symbol.clone(),
                n.depth,
                n.score.to_bits(),
            )
        })
        .collect::<Vec<_>>()
    };
    assert_eq!(key(&store), key(&store));
}

/// INTENT: IVF degenerate inputs — NaN query ≡ zero query, probe clamp,
/// reassign_all fails closed.
/// KILLS: zero-fill/clamp-mutant.
/// ABSORBS: none (KEEP standalone: `ivf_candidate_selection_degenerate_inputs`).
#[test]
fn ivf_candidate_selection_degenerate_inputs() {
    // n=4, dim=2: k = clamp(sqrt(4))=16 -> min(n) = 4 centroids, k-means over
    // normalized rows, deterministic (index-ordered parallel collect).
    let flat = [1.0f32, 0.0, 0.0, 1.0, 1.0, 1.0, -1.0, 0.0];
    let index = SemanticAnnIndex::build_from_flat(&flat, 2);
    assert_eq!(index.centroid_count(), 4);

    // NaN query components zero-fill before centroid dots, so a NaN query is
    // bit-identical to the zero query: total order via stable sort + sorted
    // member emission, every index in range.
    let via_nan = index.candidate_indices(&[f32::NAN, f32::NAN], None);
    let via_zero = index.candidate_indices(&[0.0, 0.0], None);
    assert_eq!(via_nan, via_zero);
    let mut sorted = via_nan.clone();
    sorted.sort_unstable();
    assert_eq!(via_nan, sorted, "members must be emitted sorted");
    assert!(via_nan.iter().all(|&i| i < 4));

    // probes clamps to [1, populated]: usize::MAX and 1000 take the same path
    // and must agree bit-for-bit; search honors the same clamp without panic.
    let q = [1.0f32, 0.0];
    assert_eq!(
        index.candidate_indices(&q, Some(usize::MAX)),
        index.candidate_indices(&q, Some(1000))
    );
    assert_eq!(
        index.search_flat_with_probes(&flat, 2, &q, 10, Some(usize::MAX)),
        index.search_flat_with_probes(&flat, 2, &q, 10, Some(1000))
    );

    // reassign_all fails closed (false, no panic): empty input, dim mismatch
    // against len-2 centroids, and an index with no centroids at all.
    let mut index = index;
    assert!(!index.reassign_all(&[], 2));
    assert!(!index.reassign_all(&flat, 3));
    let mut empty = SemanticAnnIndex::build_from_flat(&[], 2);
    assert!(!empty.reassign_all(&flat, 2));
}
