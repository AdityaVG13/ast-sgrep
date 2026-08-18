use super::*;
use std::collections::BTreeSet;

#[test]
fn cosine_similarity_is_scale_invariant() {
    assert!(
        (cosine_similarity(&[1.0, 2.0], &[3.0, 4.0])
            - cosine_similarity(&[10.0, 20.0], &[1.5, 2.0]))
        .abs()
            <= f32::EPSILON
    );
}

#[test]
fn similarity_rankers_filter_non_finite_scores() {
    assert_eq!(
        top_k_similarity([(0, f32::NAN), (1, 0.5)], 2, None),
        vec![(1, 0.5)]
    );
    // NaN components in flat rows are ignored; residual may be a finite 0.0
    // score which is dropped by the minimum-similarity gate.
    assert_eq!(
        top_k_flat_similarity(
            &[1.0, 0.0],
            &[f32::NAN, 0.0, 0.5, 0.0],
            2,
            2,
            Some(MIN_SIMILARITY)
        ),
        vec![(1, 1.0)]
    );
    assert_eq!(
        top_by_similarity(vec![(0, f32::NAN), (1, f32::INFINITY), (2, 0.4)], 3, None),
        vec![(2, 0.4)]
    );
}

#[test]
fn scored_constructor_rejects_non_finite() {
    assert!(Scored::new(0, 0.5).is_some());
    assert!(Scored::new(0, f32::NAN).is_none());
    assert!(Scored::new(0, f32::INFINITY).is_none());
    assert!(Scored::new(0, f32::NEG_INFINITY).is_none());
}

#[test]
fn scored_eq_ord_agree_on_finite_domain() {
    let a = Scored::new(1, 0.2).unwrap();
    let b = Scored::new(2, 0.2).unwrap();
    let c = Scored::new(0, 0.9).unwrap();
    assert_eq!(a.cmp(&b), Ordering::Greater); // higher idx loses ties → Reverse heap
    assert_eq!((a == b), (a.cmp(&b) == Ordering::Equal));
    assert_eq!((a == c), (a.cmp(&c) == Ordering::Equal));
    // Total order: no NaN equality loophole
    let mut set = BTreeSet::new();
    set.insert(a);
    set.insert(b);
    set.insert(c);
    assert_eq!(set.len(), 3);
}

#[test]
fn normalize_vec_canonicalizes_nan_residuals() {
    let out = normalize_vec(&[1.0, f32::NAN, 0.0]);
    assert!(out.iter().all(|x| x.is_finite()));
    let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5 || norm == 0.0);
    let all_nan = normalize_vec(&[f32::NAN, f32::NAN]);
    assert_eq!(all_nan, vec![0.0, 0.0]);
}

#[test]
fn cosine_ignores_nan_components() {
    let score = cosine_similarity(&[1.0, f32::NAN], &[1.0, 0.0]);
    assert!(score.is_finite());
    assert!((score - 1.0).abs() < 1e-5);
}

#[test]
fn minimum_similarity_uses_stable_ulp_boundary() {
    let min = 0.5_f32;
    let one = f32::from_bits(min.to_bits() + 1);
    let two = f32::from_bits(min.to_bits() + 2);
    assert!(top_k_similarity([(0, one)], 1, Some(min)).is_empty());
    assert_eq!(top_k_similarity([(0, two)], 1, Some(min)), vec![(0, two)]);
    assert!(top_by_similarity(vec![(0, one)], 1, Some(min)).is_empty());
    assert_eq!(
        top_by_similarity(vec![(0, two)], 1, Some(min)),
        vec![(0, two)]
    );
}
