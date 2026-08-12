use super::*;

#[test]
fn scored_heap_never_admits_nan_across_seeded_inputs() {
    // Lightweight property micro-harness (g799) without pulling proptest into
    // the default lib build graph for embed.
    let seeds: &[f32] = &[
        0.0,
        -0.0,
        1.0,
        -1.0,
        f32::MIN_POSITIVE,
        f32::MAX,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.08,
        0.0799999,
    ];
    for (i, &sim) in seeds.iter().enumerate() {
        let out = top_k_similarity([(i, sim), (i + 100, 0.5)], 2, None);
        assert!(out.iter().all(|(_, s)| s.is_finite()));
        assert!(!out.iter().any(|(idx, _)| *idx == i) || sim.is_finite());
        let scored = Scored::new(i, sim);
        assert_eq!(scored.is_some(), sim.is_finite());
    }
    let mixed: Vec<_> = seeds.iter().enumerate().map(|(i, s)| (i, *s)).collect();
    let ranked = top_by_similarity(mixed, 8, None);
    assert!(ranked.iter().all(|(_, s)| s.is_finite()));
    for window in ranked.windows(2) {
        let ord = score_order(window[0].1, window[1].1);
        assert!(
            matches!(ord, Ordering::Greater | Ordering::Equal),
            "expected non-ascending scores, got {:?} then {:?}",
            window[0].1,
            window[1].1
        );
    }
}

#[test]
fn normalize_then_rank_rejects_nan_query_residuals() {
    let q = normalize_vec(&[f32::NAN, 1.0, f32::INFINITY]);
    assert!(q.iter().all(|x| x.is_finite()));
    let flat = {
        let mut v = vec![1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0];
        normalize_vec_in_place(&mut v[0..3]);
        normalize_vec_in_place(&mut v[3..6]);
        v
    };
    let hits = top_k_flat_similarity(&q, &flat, 3, 2, Some(MIN_SIMILARITY));
    assert!(hits.iter().all(|(_, s)| s.is_finite()));
}

/// Product edge paths: empty corpus, zero dim, limit 0 / max, dim mismatch.
/// Must return empty — never panic (div-by-zero on dim=0 was a real crash).
#[test]
fn top_k_flat_edge_paths_return_empty_without_panic() {
    let row = [1.0f32, 0.0, 0.0];
    let flat = {
        let mut v = vec![1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0];
        normalize_vec_in_place(&mut v[0..3]);
        normalize_vec_in_place(&mut v[3..6]);
        v
    };
    // empty corpus
    assert!(top_k_flat_similarity(&row, &[], 3, 5, Some(MIN_SIMILARITY)).is_empty());
    // zero dim (empty and non-empty flat) — must not divide-by-zero
    assert!(top_k_flat_similarity(&[], &[], 0, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[], &[1.0, 2.0], 0, 5, None).is_empty());
    // limit 0
    assert!(top_k_flat_similarity(&row, &flat, 3, 0, Some(MIN_SIMILARITY)).is_empty());
    // query dim mismatch
    assert!(top_k_flat_similarity(&[1.0, 0.0], &flat, 3, 5, None).is_empty());
    // max limit: still ranks without OOM on tiny corpus
    let hits = top_k_flat_similarity(&row, &flat, 3, usize::MAX, None);
    assert_eq!(hits.len(), 2);
    assert!(hits[0].1 >= hits[1].1);
}
