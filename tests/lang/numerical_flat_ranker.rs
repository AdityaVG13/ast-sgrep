//! Flat-ranker contract suite: top_k_flat_similarity behavior in one test.
//!
//! Absorbs the thirteen per-function flat-ranker clauses from the N1/N2/N3/N4
//! pass files. Tolerance table: BIT-EXACT (`assert_eq!`) for all index orders,
//! ragged-tail/degenerate shapings, hostile/hostile-parallel orders,
//! dyadic-exact 1.0/0.0/-1.0/0.6 heads, tie bit-equality, empty cuts;
//! 1e-6 (`approx_eq`) for irrational heads (1/sqrt(2), 1/sqrt(1+e^2)) via
//! independent f64 formulas.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, top_k_flat_similarity, top_k_similarity,
    PARALLEL_CHUNK_THRESHOLD,
};
use ast_sgrep_testkit::{approx_eq, ranked_indices};

/// INTENT: flat-ranker total contract — ragged-tail truncation, limit/threshold
/// shaping, parallel-boundary agreement, degenerate-shape empties, nonfinite
/// corpus/query totality, hostile parallel order, row-permutation invariance,
/// nesting prefix, angle-vs-magnitude ranking, epsilon-ladder resolution,
/// hostile-mixed zeros-by-index, threshold-boundary strictness, duplicate
/// truncation, zero-query index order.
/// KILLS: row-count/ragged-tail, limit/threshold-shaping, parallel-fold/
/// reduce-divergence, shape-guard (div-by-zero/panic), k>len-pad,
/// nonfinite-row/query-collapse, parallel-hostile-order, row-index/
/// score-pairing, truncation-reorder, dot-for-cosine (magnitude-confound),
/// epsilon-resolution/tie, hostile-fail-open/order, strictness-at-0.6,
/// flat-tie-truncation, zero-query-collapse mutants.
/// ABSORBS: top_k_flat_truncates_ragged_tail_and_shapes (N1), top_k_flat_parallel_boundary_agrees_with_sequential (N1),
/// flat_degenerate_shapes_empty_and_limit_edges (N2), flat_nonfinite_corpus_and_query_totality (N2),
/// flat_parallel_path_with_hostile_rows_keeps_hand_order (N2), flat_ranking_invariant_under_row_permutation (N3),
/// top_k_nesting_prefix_property flat arm (N3), cosine_pipeline_ranks_by_angle_not_magnitude (N4),
/// near_tie_ladder_orders_by_epsilon_with_index_break (N4), hostile_mixed_corpus_keeps_clean_order_zeros_sort_by_index (N4),
/// flat_threshold_boundary_drops_equal_score_keeps_above (N4), duplicate_corpus_truncation_keeps_lowest_indices (N4),
/// zero_query_flat_pipeline_orders_by_index_threshold_empties (N4).
/// LINE-DROPS: none.
#[test]
fn flat_ranker_contract() {
    // ── Clause flat_ragged (N1): ragged tail + limit/threshold shaping ──
    // flat.len()=5, dim=2 -> n = 5/2 = 2 rows; the trailing 999.0 is not a
    // row and must be ignored (no panic, no third hit).
    // Hand cosines vs query [1,0]: row0 [1,0] -> 1/(1*1) = 1.0;
    // row1 [0,1] -> 0. Indices BIT-EXACT; scores 1e-6 (f64->f32 cast).
    let flat = [1.0, 0.0, 0.0, 1.0, 999.0];
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, 0);
    assert!(approx_eq(ranked[0].1, 1.0), "got {ranked:?}");
    assert_eq!(ranked[1], (1, 0.0));
    // Limit shaping keeps the head only.
    let head = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 1, None);
    assert_eq!(head.len(), 1);
    assert_eq!(head[0].0, 0);
    assert!(approx_eq(head[0].1, 1.0));
    // Threshold shaping is exclusive: 0.0 row drops at Some(0.5), the 1.0
    // head survives; at Some(1.0) even the head drops (strict >).
    assert_eq!(
        top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, Some(0.5)).len(),
        1
    );
    assert!(top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, Some(1.0)).is_empty());

    // ── Clause flat_parallel (N1): parallel boundary == sequential ──
    // n == PARALLEL_CHUNK_THRESHOLD takes the parallel fold/reduce path;
    // hand scores are axis-exact (1.0 on rows 5,60; 0.0 elsewhere), so both
    // the parallel flat path and the manual sequential path
    // (cosine_per_row + heap ranker) must equal the hand order.
    assert_eq!(PARALLEL_CHUNK_THRESHOLD, 64);
    for n in [64usize, 65] {
        let mut flat = vec![0.0f32; n * 2];
        for i in [5, 60] {
            flat[i * 2] = 1.0;
        }
        for i in 0..n {
            if i != 5 && i != 60 {
                flat[i * 2 + 1] = 1.0;
            }
        }
        let query = [1.0, 0.0];
        let flat_ranked = top_k_flat_similarity(&query, &flat, 2, 3, None);
        let manual: Vec<(usize, f32)> = (0..n)
            .map(|i| (i, cosine_similarity(&query, &flat[i * 2..(i + 1) * 2])))
            .collect();
        let seq_ranked = top_k_similarity(manual, 3, None);
        assert_eq!(flat_ranked.len(), 3, "n={n}");
        assert_eq!(seq_ranked.len(), 3, "n={n}");
        // Hand order: the two 1.0 rows by ascending index, then the first
        // 0.0 row (index 0). Indices BIT-EXACT, scores 1e-6.
        for ranked in [&flat_ranked, &seq_ranked] {
            assert_eq!([ranked[0].0, ranked[1].0, ranked[2].0], [5, 60, 0]);
            assert!(approx_eq(ranked[0].1, 1.0) && approx_eq(ranked[1].1, 1.0));
            assert_eq!(ranked[2].1, 0.0);
        }
        // Threshold arm on both paths: only the 1.0 rows survive Some(0.5).
        let flat_t = top_k_flat_similarity(&query, &flat, 2, 10, Some(0.5));
        assert_eq!(flat_t.len(), 2);
        assert_eq!([flat_t[0].0, flat_t[1].0], [5, 60]);
    }

    // ── Clause flat_degenerate (N2): degenerate shapes + edges ──
    // dim=0 with fully empty inputs: empty, not a div-by-zero panic.
    assert!(top_k_flat_similarity(&[], &[], 0, 5, None).is_empty());
    // flat shorter than one row: n = 1/2 = 0 rows -> empty.
    assert!(top_k_flat_similarity(&[1.0, 0.0], &[1.0], 2, 5, None).is_empty());
    // Empty query against nonzero dim: shape mismatch -> empty.
    assert!(top_k_flat_similarity(&[], &[1.0, 0.0, 0.0, 1.0], 2, 5, None).is_empty());
    // k>len rows returns every row without padding. Indices BIT-EXACT,
    // head score 1e-6 (f64->f32 cast), zero row BIT-EXACT 0.0.
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0, 1.0], 2, 100, None);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, 0);
    assert!(approx_eq(ranked[0].1, 1.0), "got {ranked:?}");
    assert_eq!(ranked[1], (1, 0.0));
    // All-zero rows score exactly 0.0 and survive unthresholded.
    let zeros = top_k_flat_similarity(&[1.0, 0.0], &[0.0, 0.0, 0.0, 0.0], 2, 5, None);
    assert_eq!(zeros, vec![(0, 0.0), (1, 0.0)]);
    // ... but the threshold arm drops them all.
    assert!(top_k_flat_similarity(&[1.0, 0.0], &[0.0, 0.0, 0.0, 0.0], 2, 5, Some(0.5)).is_empty());

    // ── Clause flat_nonfinite (N2): nonfinite corpus/query totality ──
    // All-NaN rows score exactly 0.0: kept unthresholded, dropped at
    // Some(0.5). BIT-EXACT (contrasts the NaN-QUERY pin owned by pass 3).
    let flat_nan = [f32::NAN; 4];
    assert_eq!(
        top_k_flat_similarity(&[1.0, 0.0], &flat_nan, 2, 5, None),
        vec![(0, 0.0), (1, 0.0)]
    );
    assert!(top_k_flat_similarity(&[1.0, 0.0], &flat_nan, 2, 5, Some(0.5)).is_empty());
    // Inf query, partial survival: pair 0 skipped; row0 [1,0] gives
    // dot=0,na=1,nb=1 -> 0.0; row1 [0,1] gives 1/1 -> 1.0. BIT-EXACT.
    assert_eq!(
        top_k_flat_similarity(&[f32::INFINITY, 1.0], &[1.0, 0.0, 0.0, 1.0], 2, 5, None),
        vec![(1, 1.0), (0, 0.0)]
    );
    // Inf row scores 0.0 (both pairs skipped) and sorts after the 1.0.
    assert_eq!(
        top_k_flat_similarity(
            &[1.0, 0.0],
            &[f32::INFINITY, f32::INFINITY, 1.0, 0.0],
            2,
            5,
            None
        ),
        vec![(1, 1.0), (0, 0.0)]
    );

    // ── Clause flat_parallel_hostile (N2): parallel + hostile rows ──
    // n == 64 takes the parallel fold/reduce path. Even rows [1,0]
    // score exactly 1.0; odd rows [0,1] and the NaN row 7 score exactly
    // 0.0. Limit 40 keeps the 32 ones then the first 8 zero indices.
    // Whole order BIT-EXACT.
    let mut flat = vec![0.0f32; 64 * 2];
    for i in 0..64 {
        if i % 2 == 0 {
            flat[i * 2] = 1.0;
        } else {
            flat[i * 2 + 1] = 1.0;
        }
    }
    flat[7 * 2] = f32::NAN;
    flat[7 * 2 + 1] = f32::NAN;
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 40, None);
    assert_eq!(ranked.len(), 40);
    let mut expected: Vec<(usize, f32)> = (0..64).step_by(2).map(|i| (i, 1.0)).collect();
    expected.extend([
        (1, 0.0),
        (3, 0.0),
        (5, 0.0),
        (7, 0.0),
        (9, 0.0),
        (11, 0.0),
        (13, 0.0),
        (15, 0.0),
    ]);
    assert_eq!(ranked, expected);
    assert!(ranked.iter().all(|(_, s)| s.is_finite()));

    // ── Clause flat_row_permutation (N3): row order never leaks ──
    // Permuting flat rows permutes which index carries each score; mapping
    // indices back through the permutation must recover the original order
    // with bit-identical scores (rows have distinct hand angles: 1.0, 0.6,
    // ~0.7071, 0.0, -1.0 — no ties to break differently).
    let query = vec![1.0, 0.0];
    let rows: Vec<[f32; 2]> = vec![[1.0, 0.0], [3.0, 4.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 0.0]];
    let flat: Vec<f32> = rows.iter().flatten().copied().collect();
    let base = top_k_flat_similarity(&query, &flat, 2, 5, None);
    assert_eq!(base.len(), 5);
    for perm in [
        vec![3, 0, 4, 1, 2],
        vec![4, 3, 2, 1, 0],
        vec![1, 2, 3, 4, 0],
    ] {
        let pflat: Vec<f32> = perm.iter().flat_map(|&j| rows[j]).collect();
        let ranked = top_k_flat_similarity(&query, &pflat, 2, 5, None);
        assert_eq!(ranked.len(), 5);
        for (k, (new_idx, score)) in ranked.iter().enumerate() {
            assert_eq!(perm[*new_idx], base[k].0, "perm={perm:?} slot={k}");
            assert_eq!(*score, base[k].1, "perm={perm:?} slot={k}");
        }
    }

    // ── Clause nesting flat arm (N3): top-k is a prefix op ──
    let flat = [1.0, 0.0, 3.0, 4.0, 1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 0.5, 0.5];
    let narrow = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 2, None);
    let wide = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(narrow, wide[..2]);

    // ── Clause angle_drill (N4): ranks by angle, not magnitude ──
    // query=[1,0]. row0 [100,100]: dot=100, |q|=1, |r|=sqrt(20000)=141.421356
    //   -> 100/141.421356 = 0.70710678. row1 [1,0] -> 1/1 = 1.0 BIT-EXACT.
    // row2 [0,1] -> 0/1 = 0.0 BIT-EXACT. row3 [-5,0] -> -5/5 = -1.0 BIT-EXACT.
    // A dot ranking would crown row0 (100.0); the cosine pipeline must rank
    // by angle: exact final order [1, 0, 2, 3].
    let flat = [100.0, 100.0, 1.0, 0.0, 0.0, 1.0, -5.0, 0.0];
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 4, None);
    assert_eq!(ranked.len(), 4);
    assert_eq!(ranked_indices(&ranked), vec![1, 0, 2, 3]);
    assert_eq!(ranked[0].1, 1.0);
    assert!(
        approx_eq(ranked[1].1, std::f32::consts::FRAC_1_SQRT_2),
        "got {ranked:?}"
    );
    assert_eq!(ranked[2].1, 0.0);
    assert_eq!(ranked[3].1, -1.0);
    // Control: the dot confound is real — row0 out-dots row1 a hundredfold.
    assert_eq!(dot_similarity(&[1.0, 0.0], &[100.0, 100.0]), 100.0);
    assert_eq!(dot_similarity(&[1.0, 0.0], &[1.0, 0.0]), 1.0);

    // ── Clause ladder_drill (N4): epsilon ladder + duplicate break ──
    // cos([1,0],[1,e]) = 1/sqrt(1+e^2): e=0 -> 1.0 BIT-EXACT; e=0.01 ->
    // 0.99995000; e=0.02 -> 0.99980006; e=0.03 -> 0.99955013. Adjacent gap
    // ~1.5e-4 dwarfs f32 rounding (~6e-8), so the ladder cannot flip.
    // idx1/idx2 share e=0.01 -> bit-identical scores -> ascending index.
    // Exact final order: [4, 1, 2, 0, 3].
    let flat = [1.0, 0.02, 1.0, 0.01, 1.0, 0.01, 1.0, 0.03, 1.0, 0.0];
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(ranked.len(), 5);
    assert_eq!(ranked_indices(&ranked), vec![4, 1, 2, 0, 3]);
    assert_eq!(ranked[0].1, 1.0);
    assert_eq!(ranked[1].1, ranked[2].1);
    assert!(ranked[0].1 > ranked[1].1, "got {ranked:?}");
    assert!(ranked[2].1 > ranked[3].1, "got {ranked:?}");
    assert!(ranked[3].1 > ranked[4].1, "got {ranked:?}");
    // Independent f64 check of every rung (expected values built from the
    // same f32 epsilons the pipeline consumed).
    for (slot, e) in [(1, 0.01f32), (3, 0.02f32), (4, 0.03f32)] {
        let ed = f64::from(e);
        let expected = 1.0 / (1.0 + ed * ed).sqrt();
        assert!(
            (f64::from(ranked[slot].1) - expected).abs() < 1e-6,
            "slot={slot} got={} expected={expected}",
            ranked[slot].1
        );
    }

    // ── Clause hostile_drill (N4): hostile zeros sort by index ──
    // query=[1,0]. idx0 [NaN,NaN]: every pair skipped -> na=nb=0 -> 0.0.
    // idx1 [1,0] -> 1.0. idx2 [0,0]: nb=0 -> 0.0. idx3 [inf,inf]: skipped
    // like NaN -> 0.0. idx4 [0,1] -> 0/1 = 0.0. idx5 [1,0] -> 1.0, tie with
    // idx1 -> ascending index. The four hostile/orthogonal zeros sort purely
    // by index after the clean ones. Whole order BIT-EXACT.
    let flat = [
        f32::NAN,
        f32::NAN,
        1.0,
        0.0,
        0.0,
        0.0,
        f32::INFINITY,
        f32::INFINITY,
        0.0,
        1.0,
        1.0,
        0.0,
    ];
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 6, None);
    assert_eq!(
        ranked,
        vec![(1, 1.0), (5, 1.0), (0, 0.0), (2, 0.0), (3, 0.0), (4, 0.0)]
    );

    // ── Clause flat_threshold_drill (N4): hand 0.6 strictness ──
    // query=[1,0]. idx0 [3,4]: 3/(1*5) = 0.6 exactly ((3/5)f64 cast to f32
    // is 0.6f32); exceeds_threshold is strict (sim > next(min)), so
    // Some(0.6) drops it. idx1 [1,1] -> 1/sqrt(2) = 0.70710678 survives;
    // idx2 [1,0] -> 1.0. Unthresholded control [2,1,0] proves the drop is
    // strictness, not value. Both orders BIT-EXACT on indices.
    let flat = [3.0, 4.0, 1.0, 1.0, 1.0, 0.0];
    let full = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 3, None);
    assert_eq!(ranked_indices(&full), vec![2, 1, 0]);
    assert_eq!(full[2].1, 0.6);
    let cut = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 3, Some(0.6));
    assert_eq!(cut.len(), 2);
    assert_eq!(cut[0], (2, 1.0));
    assert_eq!(cut[1].0, 1);
    assert!(
        approx_eq(cut[1].1, std::f32::consts::FRAC_1_SQRT_2),
        "got {cut:?}"
    );

    // ── Clause duplicate_drill (N4): truncation keeps lowest indices ──
    // 8 identical rows [1,1] vs [1,0]: every score is the same f32 bit
    // pattern (1/sqrt(2) = 0.70710678), so limit 5 keeps [0..5] and the
    // full limit keeps [0..8]. Indices BIT-EXACT, scores bit-equal.
    let flat: Vec<f32> = (0..8).flat_map(|_| [1.0, 1.0]).collect();
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(ranked_indices(&ranked), vec![0, 1, 2, 3, 4]);
    for (_, s) in &ranked {
        assert_eq!(*s, ranked[0].1);
        assert!(
            approx_eq(*s, std::f32::consts::FRAC_1_SQRT_2),
            "got {ranked:?}"
        );
    }
    let full = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 8, None);
    assert_eq!(ranked_indices(&full), vec![0, 1, 2, 3, 4, 5, 6, 7]);

    // ── Clause zero_query_drill (N4): zero query index order ──
    // query=[0,0]: every na is 0.0 — the [NaN,5] row skips pair 0 but its
    // survivor still accumulates na=0 — so all scores are exactly 0.0 and
    // the order is purely by index. Some(0.0) drops all of them
    // (0.0 > from_bits(1) is false). Whole order BIT-EXACT.
    let flat = [1.0, 0.0, 3.0, 4.0, 0.0, 0.0, f32::NAN, 5.0];
    let ranked = top_k_flat_similarity(&[0.0, 0.0], &flat, 2, 4, None);
    assert_eq!(ranked, vec![(0, 0.0), (1, 0.0), (2, 0.0), (3, 0.0)]);
    assert!(top_k_flat_similarity(&[0.0, 0.0], &flat, 2, 4, Some(0.0)).is_empty());
}
