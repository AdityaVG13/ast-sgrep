//! Score-ranker contract suite: top_by + top_k heap behavior in one test.
//!
//! Absorbs the eight per-function score-ranker clauses from the N1/N2/N3 pass
//! files. Everything here is BIT-EXACT (`assert_eq!` / `to_bits`): scores are
//! moved, never recomputed. Heap-vs-sort agreement is L1/L2-owned and is NOT
//! re-pinned (catalog line-drops).

use ast_sgrep_embed::{top_by_similarity, top_k_similarity};

/// INTENT: score-ranker total contract — strict threshold bit-boundaries and
/// nonfinite-min admission refusal, heap tie-eviction under truncation,
/// negative retention with signed-zero index order, all-nonfinite/k-edge
/// totality, -inf drop, permutation invariance, top-k nesting, and
/// threshold-filters-never-reorders.
/// KILLS: threshold-comparison (strict->nonstrict, ulp-off-by-one,
/// nonfinite-min admit-all), tie-eviction-order, negative-drop,
/// tie-break/sign-payload, nonfinite-admit, heap-limit-edge, -inf-admit,
/// negative-order, input-order-leak, truncation-reorder, threshold-reorder mutants.
/// ABSORBS: threshold_nextafter_bits_are_exact_all_signs (N1), top_k_heap_tie_eviction_keeps_ascending_indices (N1),
/// top_by_keeps_negative_and_orders_signed_zero_by_index (N1), rankers_all_nonfinite_empty_and_limit_edges (N2),
/// rankers_drop_negative_infinity_and_keep_negative_order (N2), rankers_invariant_under_input_permutation (N3),
/// top_k_nesting_prefix_property sort/heap arms (N3), threshold_only_filters_never_reorders (N3).
/// LINE-DROPS: heap-vs-sort differential lines in heap_tie, rankers_permutation,
/// threshold_filters (weaker vs L1/L2-owned agreement).
#[test]
fn score_ranker_contract() {
    // ── Clause threshold_nextafter (N1): strict bit-boundaries ──
    // min = 1.0 (0x3F800000): next = 0x3F800001. sim == min fails,
    // sim == next fails (strict >), sim == next+1ulp passes.
    let next_up = f32::from_bits(0x3F80_0001);
    let past_up = f32::from_bits(0x3F80_0002);
    assert!(top_by_similarity(vec![(0, 1.0)], 10, Some(1.0)).is_empty());
    assert!(top_by_similarity(vec![(0, next_up)], 10, Some(1.0)).is_empty());
    assert_eq!(top_by_similarity(vec![(0, past_up)], 10, Some(1.0)).len(), 1);
    // min = 0.0: next = from_bits(1), the smallest subnormal. Equal fails,
    // one ulp above passes. (MIN_POSITIVE passing is L2; this pins the floor.)
    assert!(top_by_similarity(vec![(0, f32::from_bits(1))], 10, Some(0.0)).is_empty());
    assert_eq!(
        top_by_similarity(vec![(0, f32::from_bits(2))], 10, Some(0.0)).len(),
        1
    );
    // min = -1.0 (0xBF800000): next = bits-1 = 0xBF7FFFFF (-0.99999994).
    // For negatives value rises as bits fall, so the passing neighbor is
    // next-bits-1 = 0xBF7FFFFE (-0.99999988).
    let next_neg = f32::from_bits(0xBF7F_FFFF);
    let past_neg = f32::from_bits(0xBF7F_FFFE);
    assert!(next_neg > -1.0 && past_neg > next_neg);
    assert!(top_by_similarity(vec![(0, -1.0)], 10, Some(-1.0)).is_empty());
    assert!(top_by_similarity(vec![(0, next_neg)], 10, Some(-1.0)).is_empty());
    assert_eq!(top_by_similarity(vec![(0, past_neg)], 10, Some(-1.0)).len(), 1);
    // Non-finite thresholds admit nothing, not everything.
    for bad_min in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            top_by_similarity(vec![(0, 0.9)], 10, Some(bad_min)).is_empty(),
            "min={bad_min}"
        );
        assert!(
            top_k_similarity(vec![(0, 0.9)], 10, Some(bad_min)).is_empty(),
            "heap min={bad_min}"
        );
    }

    // ── Clause heap_tie (N1): tie-eviction keeps lowest indices ──
    // All five tie at 0.9; the min-heap pops the largest index among ties,
    // so limit 2 keeps indices 0 and 1 however the input is permuted.
    // LINE-DROP (catalog): the sort-differential line
    // (top_by == expected) is deleted; heap-vs-sort agreement is L1/L2-owned.
    let expected = vec![(0, 0.9), (1, 0.9)];
    let perms = [
        vec![(5, 0.9), (3, 0.9), (1, 0.9), (0, 0.9), (4, 0.9)],
        vec![(0, 0.9), (1, 0.9), (3, 0.9), (4, 0.9), (5, 0.9)],
        vec![(4, 0.9), (5, 0.9), (3, 0.9), (1, 0.9), (0, 0.9)],
    ];
    for perm in perms {
        assert_eq!(top_k_similarity(perm, 2, None), expected);
    }

    // ── Clause top_by_neg (N1): negatives kept, signed-zero by index ──
    // Unthresholded: negatives survive and sort below positives. BIT-EXACT.
    assert_eq!(
        top_by_similarity(vec![(1, -0.5), (0, 0.5), (2, -2.0)], 10, None),
        vec![(0, 0.5), (1, -0.5), (2, -2.0)]
    );
    // -0.0 == 0.0 under partial_cmp, so the tie breaks by ascending index;
    // pin the PAYLOAD BITS too (assert_eq alone cannot see the sign).
    let ranked = top_by_similarity(vec![(1, 0.0), (0, -0.0)], 10, None);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, 0);
    assert_eq!(ranked[1].0, 1);
    assert_eq!(ranked[0].1.to_bits(), (-0.0f32).to_bits());
    assert_eq!(ranked[1].1.to_bits(), 0.0f32.to_bits());

    // ── Clause rankers_nonfinite_edges (N2): honest empties + heap edges ──
    // All-nonfinite corpora are honest empties on BOTH rankers. BIT-EXACT.
    for corpus in [
        vec![(0, f32::NAN), (1, f32::NAN)],
        vec![
            (0, f32::INFINITY),
            (1, f32::NEG_INFINITY),
            (2, f32::NAN),
        ],
    ] {
        assert!(top_by_similarity(corpus.clone(), 10, None).is_empty());
        assert!(top_k_similarity(corpus, 10, None).is_empty());
    }
    // Heap edges: empty input and k=0 admit nothing (sort arms are L1).
    assert!(top_k_similarity(Vec::new(), 10, None).is_empty());
    assert!(top_k_similarity(vec![(0, 0.9), (1, f32::NAN)], 0, None).is_empty());
    // Heap k>len returns every survivor without padding (sort arm is L2).
    // BIT-EXACT: scores are moved, never recomputed.
    assert_eq!(
        top_k_similarity(vec![(1, 0.5), (0, 0.9)], 100, None),
        vec![(0, 0.9), (1, 0.5)]
    );
    assert_eq!(
        top_k_similarity(vec![(0, f32::NAN), (1, 0.3)], 100, None),
        vec![(1, 0.3)]
    );

    // ── Clause rankers_neginf (N2): -inf drops, negatives ordered ──
    // -inf drops exactly like NaN/+inf on both rankers. BIT-EXACT.
    let corpus = vec![
        (0, f32::NEG_INFINITY),
        (1, 0.4),
        (2, f32::NEG_INFINITY),
        (3, f32::NAN),
        (4, -0.2),
    ];
    let expected = vec![(1, 0.4), (4, -0.2)];
    assert_eq!(top_by_similarity(corpus.clone(), 10, None), expected);
    assert_eq!(top_k_similarity(corpus, 10, None), expected);
    // Heap keeps negatives below positives (sort arm is N1). BIT-EXACT.
    assert_eq!(
        top_k_similarity(vec![(1, -0.5), (0, 0.5), (2, -2.0)], 10, None),
        vec![(0, 0.5), (1, -0.5), (2, -2.0)]
    );

    // ── Clause rankers_permutation (N3): input order never leaks ──
    // Score rankers sort a total order (score desc, index asc); input order
    // — including ties and dropped non-finite entries — cannot leak through.
    // LINE-DROP (catalog): the first_sort == first_heap differential is
    // deleted; each ranker is pinned against its own first permutation.
    let corpus = vec![
        (0, 0.9),
        (1, 0.9),
        (2, 0.4),
        (3, f32::NAN),
        (4, -0.2),
        (5, f32::INFINITY),
        (6, 0.9),
        (7, 0.4),
    ];
    let perms: Vec<Vec<(usize, f32)>> = vec![
        corpus.clone(),
        corpus.iter().rev().copied().collect(),
        vec![
            corpus[6], corpus[2], corpus[0], corpus[7], corpus[4], corpus[3], corpus[1],
            corpus[5],
        ],
        vec![
            corpus[3], corpus[5], corpus[7], corpus[1], corpus[4], corpus[0], corpus[6],
            corpus[2],
        ],
    ];
    let first_sort = top_by_similarity(perms[0].clone(), 5, None);
    let first_heap = top_k_similarity(perms[0].clone(), 5, None);
    for perm in &perms[1..] {
        assert_eq!(top_by_similarity(perm.clone(), 5, None), first_sort);
        assert_eq!(top_k_similarity(perm.clone(), 5, None), first_heap);
    }
    // Threshold arm is permutation-invariant too.
    let t0 = top_by_similarity(perms[0].clone(), 8, Some(0.5));
    for perm in &perms[1..] {
        assert_eq!(top_by_similarity(perm.clone(), 8, Some(0.5)), t0);
        assert_eq!(
            top_k_similarity(perm.clone(), 8, Some(0.5)),
            top_k_similarity(perms[0].clone(), 8, Some(0.5))
        );
    }

    // ── Clause nesting sort/heap arms (N3): top-k is a prefix op ──
    // top-2 == top-5[..2] on both rankers, with distinct scores and with
    // ties (index tie-break keeps it total). (Flat arm lives in flat_ranker.)
    let distinct = vec![(0, 0.9), (1, 0.7), (2, 0.5), (3, 0.3), (4, 0.1), (5, -0.4)];
    let ties = vec![(0, 0.9), (1, 0.9), (2, 0.9), (3, 0.9), (4, 0.9), (5, 0.9)];
    for corpus in [&distinct, &ties] {
        for (small, big) in [(0usize, 5usize), (1, 3), (2, 5), (2, 6)] {
            let narrow_sort = top_by_similarity(corpus.clone(), small, None);
            let wide_sort = top_by_similarity(corpus.clone(), big, None);
            assert_eq!(narrow_sort, &wide_sort[..narrow_sort.len()]);
            let narrow_heap = top_k_similarity(corpus.clone(), small, None);
            let wide_heap = top_k_similarity(corpus.clone(), big, None);
            assert_eq!(narrow_heap, &wide_heap[..narrow_heap.len()]);
        }
    }

    // ── Clause threshold_filters (N3): threshold never reorders ──
    // With scores kept clear of the nextafter gap (0.9/0.7 above, 0.5/0.3/0.1
    // at-or-below Some(0.5)), the thresholded output is exactly the
    // unthresholded output filtered to s > min — same relative order.
    // LINE-DROP (catalog): the full_sort == full_heap differential is
    // deleted; both rankers are pinned against the hand filter rule.
    let corpus = vec![(0, 0.9), (1, 0.3), (2, 0.7), (3, 0.5), (4, 0.1), (5, -0.2)];
    let full_sort = top_by_similarity(corpus.clone(), 10, None);
    let expected: Vec<(usize, f32)> =
        full_sort.iter().copied().filter(|(_, s)| *s > 0.5).collect();
    assert_eq!(expected.len(), 2);
    assert_eq!(top_by_similarity(corpus.clone(), 10, Some(0.5)), expected);
    assert_eq!(top_k_similarity(corpus.clone(), 10, Some(0.5)), expected);
    // Kept scores strictly exceed min; thresholding only shrinks.
    for (_, s) in top_k_similarity(corpus.clone(), 10, Some(0.5)) {
        assert!(s > 0.5);
    }
    assert!(top_k_similarity(corpus, 10, Some(0.5)).len() <= full_sort.len());
}
