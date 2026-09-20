//! Consolidated oracle suite: score ranking (order / truncate / threshold
//! / stability) and ranker-lane agreement.
//!
//! Replaces `oracle_foundry_pass{1,2,3}.rs` rank legs. Expectations are
//! hand-computed; errors assert discriminants, never messages.

use ast_sgrep_embed::{top_by_similarity, top_k_flat_similarity, top_k_similarity, MIN_SIMILARITY};

/// INTENT: ranking is score-desc/index-asc, truncates to the limit without
/// padding, drops NaN, applies an exclusive (`>`) threshold — incl the
/// denormal step above 0.0 — filters Inf, and is invariant under input
/// permutation.
///
/// KILLS: sort/tie-break-flip, limit-truncate/pad, NaN-filter-removal,
/// `>`→`>=`-flip, denormal-step-drop, Inf-filter-removal,
/// input-order-dependence.
///
/// ABSORBS: pass1::top_by_similarity_orders_truncates_and_drops_nan,
/// pass2::threshold_is_strict_nextafter_not_gte,
/// pass3::ranking_invariant_under_input_permutation_and_ties.
///
/// DEDUP: the MIN_SIMILARITY-exclusive line was pinned in pass1 and pass2; it
/// is pinned ONCE here (threshold leg).
#[test]
fn rank_matrix_orders_truncates_thresholds_stably() {
    // Descending score, ties broken by ascending index, NaN dropped.
    let scored = vec![(2, 0.5), (0, 0.9), (1, 0.9), (3, f32::NAN)];
    assert_eq!(
        top_by_similarity(scored, 10, None),
        vec![(0, 0.9), (1, 0.9), (2, 0.5)]
    );
    assert_eq!(
        top_by_similarity(vec![(0, 0.9), (1, 0.8)], 1, None),
        vec![(0, 0.9)]
    );
    assert!(top_by_similarity(vec![(0, 0.9)], 0, None).is_empty());
    assert_eq!(
        top_by_similarity(vec![(0, MIN_SIMILARITY + 0.01)], 10, Some(MIN_SIMILARITY)).len(),
        1
    );
    // Strict `>`: equal scores never pass, at 0.0, negatives, and the
    // MIN_SIMILARITY boundary; the denormal step above 0.0 does pass.
    assert!(top_by_similarity(vec![(0, 0.0)], 10, Some(0.0)).is_empty());
    assert_eq!(
        top_by_similarity(vec![(0, f32::MIN_POSITIVE)], 10, Some(0.0)).len(),
        1
    );
    assert!(top_by_similarity(vec![(0, -1.0)], 10, Some(-1.0)).is_empty());
    assert_eq!(top_by_similarity(vec![(0, -0.5)], 10, Some(-1.0)).len(), 1);
    assert!(top_by_similarity(vec![(0, MIN_SIMILARITY)], 10, Some(MIN_SIMILARITY)).is_empty());
    assert!(top_by_similarity(vec![(0, f32::INFINITY)], 10, None).is_empty());
    // Permutation invariance: every input order yields the hand order.
    let expected = vec![(0, 0.9), (1, 0.9), (2, 0.9), (3, 0.5), (4, 0.2)];
    let base = vec![(4, 0.2), (2, 0.9), (0, 0.9), (3, 0.5), (1, 0.9)];
    let mut reversed = base.clone();
    reversed.reverse();
    let mut rotated = base.clone();
    rotated.rotate_left(2);
    for perm in [base, reversed, rotated] {
        assert_eq!(top_by_similarity(perm, 10, None), expected);
    }
    // Limit beyond the corpus returns every survivor without padding.
    assert_eq!(
        top_by_similarity(vec![(1, 0.5), (0, 0.5)], 100, None),
        vec![(0, 0.5), (1, 0.5)]
    );
    // All-tie input still resolves to ascending index.
    assert_eq!(
        top_by_similarity(vec![(2, 0.5), (0, 0.5), (1, 0.5)], 10, None),
        vec![(0, 0.5), (1, 0.5), (2, 0.5)]
    );
}

/// INTENT: the heap and sort rankers agree bit-exactly on an adversarial
/// corpus, and the flat top-k lane returns honest empty on every degenerate
/// shape while ranking valid rows exactly.
///
/// KILLS: heap-vs-sort divergence (NaN/Inf filter, tie-break, threshold),
/// `checked_div`→`/`-panic on dim 0, shape-guard-removal, flat mis-rank.
///
/// ABSORBS: pass2::heap_and_sort_rankers_agree_on_adversarial_corpus,
/// pass2::top_k_flat_guards_division_and_shape.
#[test]
fn ranker_lanes_agree_and_flat_guards_total() {
    // Both rankers must produce the hand-sorted order: score desc, ties by
    // ascending index, NaN/Inf filtered, 0.5-threshold exclusive.
    let corpus = vec![
        (5, 0.9),
        (2, 0.9),
        (7, f32::NAN),
        (3, 0.5),
        (9, f32::INFINITY),
        (1, 0.7),
        (4, -2.0),
        (6, 0.5001),
    ];
    let expected = vec![(2, 0.9), (5, 0.9), (1, 0.7)];
    assert_eq!(top_by_similarity(corpus.clone(), 3, Some(0.5)), expected);
    assert_eq!(top_k_similarity(corpus, 3, Some(0.5)), expected);
    // Degenerate shapes collapse to empty instead of panicking on div-by-zero.
    assert!(top_k_flat_similarity(&[1.0], &[1.0, 0.0], 0, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[1.0, 0.0, 0.0], &[1.0, 0.0], 2, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[1.0, 0.0], &[], 2, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[1.0, 0.0], &[1.0, 0.0], 2, 0, None).is_empty());
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0, 1.0], 2, 5, None);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, 0);
    assert!((ranked[0].1 - 1.0).abs() < 1e-6, "got {ranked:?}");
    assert_eq!(ranked[1].0, 1);
}
