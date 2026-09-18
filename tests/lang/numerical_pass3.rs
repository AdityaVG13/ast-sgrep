//! N3 metamorphic-numeric tests for ast-sgrep-lang math.
//!
//! N1 (numerical_pass1) owns exactness pins and N2 (numerical_pass2) owns
//! degenerate totality; L1/L2 own SIMD-vs-scalar and heap-vs-sort agreement.
//! This file owns RELATIONS ONLY and does NOT duplicate those pins: every
//! test asserts that a transformation preserves a value or an ordering —
//! never an absolute hand value and never diagnostic message text.
//!
//! Relations covered (14 tests):
//! - symmetry: sim(a,b) == sim(b,a) for dot + cosine, scalar and SIMD lanes
//! - self-similarity maximality: self >= cross (cosine ~= 1.0 ceiling)
//! - normalization idempotence: normalize twice == once (+ in-place parity)
//! - scale behavior: positive rescale preserves cosine scores / dot order
//!   (negative rescale reverses dot order on distinct dyadic scores)
//! - permutation invariance: input/row order never changes ranking content
//! - top-k nesting: top-2 is the prefix of top-5 on every ranker
//! - threshold monotonicity: a threshold only filters, never reorders
//! - determinism: identical results across threads
//!
//! Tolerance table:
//! - BIT-EXACT (`assert_eq!`): scalar-lane symmetry, ranker orders under
//!   permutation, nesting prefixes, threshold filtering, in-place parity,
//!   degenerate idempotence, thread determinism.
//! - 1e-6 approx: SIMD-lane cosine symmetry, cosine self-scores, rescaled
//!   cosine scores, short-vector dot self-scores.
//! - 1e-5 approx: SIMD-lane (64-dim, f32 accumulation) dot symmetry.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, normalize_vec, normalize_vec_in_place,
    top_by_similarity, top_k_flat_similarity, top_k_similarity, SemanticLocalEmbedding,
};
use std::thread;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

/// Deterministic xorshift-free LCG; fixed seeds keep every test reproducible.
fn lcg_vec(seed: &mut u64, len: usize, scale: f32) -> Vec<f32> {
    (0..len)
        .map(|_| {
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((*seed >> 33) as f32 / u32::MAX as f32 * 2.0 - 1.0) * scale
        })
        .collect()
}

// ─── Symmetry: dot, scalar lane ──────────────────────────────────────────────

#[test]
fn dot_symmetry_bit_exact_scalar_lane() {
    // f32 mul is commutative and the scalar fold order is fixed, so swapping
    // arguments is bit-identical — including hostile lanes (all collapse to
    // the same 0.0 on both sides).
    let mut seed = 0x1234_5678_9ABC_DEF0;
    let mut pairs: Vec<(Vec<f32>, Vec<f32>)> = vec![
        (vec![3.0, -4.0, 0.5], vec![1.0, 2.0, -7.0]),
        (vec![0.0, -0.0], vec![-0.0, 0.0]),
        (vec![f32::MIN_POSITIVE, 1.0], vec![1.0, f32::MIN_POSITIVE]),
        (vec![1e30, -1e30], vec![1.0, 1.0]),
        (vec![f32::INFINITY, 1.0], vec![1.0, 2.0]),
        (vec![f32::NAN], vec![1.0]),
        (vec![], vec![]),
        (vec![1.0], vec![1.0, 2.0]),
    ];
    for _ in 0..4 {
        pairs.push((lcg_vec(&mut seed, 7, 1.0), lcg_vec(&mut seed, 7, 1.0)));
    }
    pairs.push((lcg_vec(&mut seed, 63, 1.0), lcg_vec(&mut seed, 63, 1.0)));
    for (a, b) in &pairs {
        assert_eq!(dot_similarity(a, b), dot_similarity(b, a));
    }
}

// ─── Symmetry: dot, SIMD lane ────────────────────────────────────────────────

#[test]
fn dot_symmetry_simd_lane_approx() {
    // 64-dim takes the SIMD path; accumulation order may differ from scalar,
    // but swapping arguments must agree to 1e-5 (vectors scaled so |dot|<=1).
    let mut seed = 0x0BAD_F00D_DEAD_BEEF;
    for _ in 0..4 {
        let a = lcg_vec(&mut seed, 64, 0.125);
        let b = lcg_vec(&mut seed, 64, 0.125);
        let ab = dot_similarity(&a, &b);
        let ba = dot_similarity(&b, &a);
        assert!((ab - ba).abs() < 1e-5, "dot {ab} vs {ba}");
    }
    let a = lcg_vec(&mut seed, 70, 0.125);
    let b = lcg_vec(&mut seed, 70, 0.125);
    assert!((dot_similarity(&a, &b) - dot_similarity(&b, &a)).abs() < 1e-5);
}

// ─── Symmetry: cosine, scalar lane ───────────────────────────────────────────

#[test]
fn cosine_symmetry_bit_exact_scalar_lane() {
    // The scalar fold skips non-finite pairs symmetrically and the f64
    // ratio is argument-order independent, so this is bit-identical.
    let mut seed = 0xC0FF_EE11_2345_6789;
    let mut pairs: Vec<(Vec<f32>, Vec<f32>)> = vec![
        (vec![3.0, 4.0], vec![1.0, 1.0]),
        (vec![f32::NAN, 3.0], vec![4.0, 4.0]),
        (vec![4.0, 4.0], vec![f32::NAN, 3.0]),
        (vec![f32::INFINITY, 3.0], vec![4.0, f32::NEG_INFINITY]),
        (vec![0.0, 0.0], vec![1.0, 2.0]),
        (vec![], vec![]),
        (vec![1.0], vec![1.0, 2.0]),
    ];
    for _ in 0..4 {
        pairs.push((lcg_vec(&mut seed, 9, 1.0), lcg_vec(&mut seed, 9, 1.0)));
    }
    pairs.push((lcg_vec(&mut seed, 63, 1.0), lcg_vec(&mut seed, 63, 1.0)));
    for (a, b) in &pairs {
        assert_eq!(cosine_similarity(a, b), cosine_similarity(b, a));
    }
}

// ─── Symmetry: cosine, SIMD lane ─────────────────────────────────────────────

#[test]
fn cosine_symmetry_simd_lane_approx() {
    // 64-dim takes the SIMD path; normalized ratios agree to 1e-6.
    let mut seed = 0x5EED_1234_ABCD_9876;
    for len in [64usize, 70, 128] {
        let a = lcg_vec(&mut seed, len, 1.0);
        let b = lcg_vec(&mut seed, len, 1.0);
        let ab = cosine_similarity(&a, &b);
        let ba = cosine_similarity(&b, &a);
        assert!(approx(ab, ba), "len={len}: cosine {ab} vs {ba}");
    }
}

// ─── Self-similarity maximality ──────────────────────────────────────────────

#[test]
fn self_similarity_is_maximal() {
    // Cosine self-score is ~= 1.0 and no cross-score exceeds the 1.0 ceiling
    // (1e-6 rounding band); unit-vector dot self-scores behave the same, and
    // every finite dot self-score is non-negative.
    let mut seed = 0xA11C_E5E1_F00D_1234;
    let mut vecs: Vec<Vec<f32>> = vec![vec![3.0, 4.0], vec![1.0, 1.0], vec![-2.0, 0.5, 7.0]];
    for _ in 0..3 {
        vecs.push(lcg_vec(&mut seed, 8, 1.0));
    }
    vecs.push(lcg_vec(&mut seed, 64, 1.0));
    for a in &vecs {
        let me = cosine_similarity(a, a);
        assert!(approx(me, 1.0), "self cosine {me} for {a:?}");
        for b in &vecs {
            let cross = cosine_similarity(a, b);
            assert!(cross <= 1.0 + 1e-6, "cross {cross} exceeds ceiling");
            assert!(cross <= me + 1e-6, "cross {cross} beats self {me}");
        }
        let u = normalize_vec(a);
        let udot = dot_similarity(&u, &u);
        assert!((udot - 1.0).abs() < 1e-6, "unit self-dot {udot}");
        for b in &vecs {
            let v = normalize_vec(b);
            assert!(dot_similarity(&u, &v) <= 1.0 + 1e-6);
        }
        assert!(dot_similarity(a, a) >= 0.0);
    }
    // Zero vector: self and every cross-score collapse to exactly 0.0.
    let z = vec![0.0, 0.0];
    assert_eq!(cosine_similarity(&z, &z), 0.0);
    assert_eq!(cosine_similarity(&z, &[1.0, 2.0]), 0.0);
    assert_eq!(dot_similarity(&z, &z), 0.0);
}

// ─── Normalization idempotence ───────────────────────────────────────────────

#[test]
fn normalize_idempotent_twice_equals_once() {
    // General vectors: the second pass divides by ~= 1.0, so per-element
    // drift stays under 1e-6. Degenerate lanes are exactly idempotent.
    let mut seed = 0x1DE4_907E_5EED_00CB;
    let general: Vec<Vec<f32>> = vec![
        vec![3.0, 4.0, 0.0],
        vec![0.1, -0.2, 0.3, -0.4],
        lcg_vec(&mut seed, 16, 1.0),
        lcg_vec(&mut seed, 70, 1.0),
    ];
    for v in &general {
        let once = normalize_vec(v);
        let twice = normalize_vec(&once);
        assert_eq!(once.len(), twice.len());
        for (x, y) in once.iter().zip(twice.iter()) {
            assert!((x - y).abs() < 1e-6, "drift {x} vs {y} for {v:?}");
        }
    }
    // Exact lanes: dyadic unit, zero fill, non-finite zeroing, overflow fill.
    for v in [
        vec![5.0],
        vec![0.0, -0.0],
        vec![f32::INFINITY, -3.0],
        vec![1e38, 1e38],
        Vec::<f32>::new(),
    ] {
        assert_eq!(normalize_vec(&normalize_vec(&v)), normalize_vec(&v));
    }
}

// ─── Normalization API parity ────────────────────────────────────────────────

#[test]
fn normalize_in_place_agrees_with_out_of_place() {
    // The two APIs share one code path; parity must be bit-exact on general
    // and hostile inputs alike (catches any future divergence).
    let mut seed = 0x9E37_79B9_7F4A_7C15;
    let mut vecs: Vec<Vec<f32>> = vec![
        vec![3.0, -4.0, 0.0],
        vec![f32::INFINITY, -3.0, f32::NAN],
        vec![0.0, -0.0],
        vec![1e38, 1e38],
        Vec::new(),
    ];
    vecs.push(lcg_vec(&mut seed, 70, 1.0));
    for v in &vecs {
        let mut inplace = v.clone();
        normalize_vec_in_place(&mut inplace);
        assert_eq!(inplace, normalize_vec(v));
    }
}

// ─── Positive rescale: cosine invariant, ranking preserved ───────────────────

#[test]
fn cosine_positive_rescale_preserves_scores_and_ranking() {
    // Cosine divides out magnitudes: per-row positive rescale leaves every
    // score ~= unchanged (1e-6) and the ranking order bit-identical.
    let query = vec![1.0, 0.2];
    let rows: Vec<Vec<f32>> = vec![
        vec![1.0, 0.0],
        vec![3.0, 4.0],
        vec![0.0, 1.0],
        vec![-1.0, 0.5],
        vec![1.0, 1.0],
    ];
    let factors = [0.25, 1.5, 3.0, 0.1, 7.0];
    let flat: Vec<f32> = rows.iter().flatten().copied().collect();
    let scaled_flat: Vec<f32> = rows
        .iter()
        .zip(factors)
        .flat_map(|(r, k)| r.iter().map(move |x| x * k))
        .collect();
    for (r, k) in rows.iter().zip(factors) {
        let plain = cosine_similarity(&query, r);
        let scaled_row: Vec<f32> = r.iter().map(|x| x * k).collect();
        assert!(approx(plain, cosine_similarity(&query, &scaled_row)));
    }
    // Query-side rescale is invariant too.
    let big_q: Vec<f32> = query.iter().map(|x| x * 4.0).collect();
    for r in &rows {
        assert!(approx(
            cosine_similarity(&query, r),
            cosine_similarity(&big_q, r)
        ));
    }
    // Ranking order is bit-identical under row rescale (scores are ~= but
    // well separated, so no adjacent pair can flip).
    let base = top_k_flat_similarity(&query, &flat, 2, 5, None);
    let scaled = top_k_flat_similarity(&query, &scaled_flat, 2, 5, None);
    assert_eq!(
        base.iter().map(|(i, _)| i).collect::<Vec<_>>(),
        scaled.iter().map(|(i, _)| i).collect::<Vec<_>>()
    );
    for (a, b) in base.iter().zip(scaled.iter()) {
        assert!(approx(a.1, b.1), "scores {a:?} vs {b:?}");
    }
}

// ─── Dot rescale: positive preserves order, negative reverses ────────────────

#[test]
fn dot_positive_rescale_preserves_order_negative_reverses() {
    // Well-separated dyadic dots: exact in f32, so scaling the query by k>0
    // keeps the order and scaling by k<0 reverses it — bit-exact.
    let a = vec![1.0, 2.0];
    let corpus = vec![
        vec![3.0, 4.0],   // 11
        vec![1.0, 1.0],   // 3
        vec![0.0, 0.0],   // 0
        vec![-1.0, 0.0],  // -1
    ];
    let order_of = |q: &[f32]| {
        let mut scored: Vec<(usize, f32)> = corpus
            .iter()
            .enumerate()
            .map(|(i, b)| (i, dot_similarity(q, b)))
            .collect();
        scored.sort_by(|l, r| r.1.partial_cmp(&l.1).unwrap().then(l.0.cmp(&r.0)));
        scored
    };
    let idx_order = |q: &[f32]| {
        order_of(q).iter().map(|(i, _)| *i).collect::<Vec<_>>()
    };
    let base = idx_order(&a);
    assert_eq!(base, vec![0, 1, 2, 3]);
    let pos: Vec<f32> = a.iter().map(|x| x * 2.5).collect();
    assert_eq!(idx_order(&pos), base);
    let neg: Vec<f32> = a.iter().map(|x| x * -2.0).collect();
    let mut reversed = base.clone();
    reversed.reverse();
    assert_eq!(idx_order(&neg), reversed);
}

// ─── Rankers invariant under input permutation ───────────────────────────────

#[test]
fn rankers_invariant_under_input_permutation() {
    // Score rankers sort a total order (score desc, index asc); input order
    // — including ties and dropped non-finite entries — cannot leak through.
    // BIT-EXACT: scores are moved, never recomputed.
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
    assert_eq!(first_sort, first_heap);
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
}

// ─── Flat ranker invariant under row permutation ─────────────────────────────

#[test]
fn flat_ranking_invariant_under_row_permutation() {
    // Permuting flat rows permutes which index carries each score; mapping
    // indices back through the permutation must recover the original order
    // with bit-identical scores (rows have distinct hand angles: 1.0, 0.6,
    // ~0.7071, 0.0, -1.0 — no ties to break differently).
    let query = vec![1.0, 0.0];
    let rows: Vec<[f32; 2]> = vec![
        [1.0, 0.0],
        [3.0, 4.0],
        [1.0, 1.0],
        [0.0, 1.0],
        [-1.0, 0.0],
    ];
    let flat: Vec<f32> = rows.iter().flatten().copied().collect();
    let base = top_k_flat_similarity(&query, &flat, 2, 5, None);
    assert_eq!(base.len(), 5);
    for perm in [vec![3, 0, 4, 1, 2], vec![4, 3, 2, 1, 0], vec![1, 2, 3, 4, 0]] {
        let pflat: Vec<f32> = perm.iter().flat_map(|&j| rows[j]).collect();
        let ranked = top_k_flat_similarity(&query, &pflat, 2, 5, None);
        assert_eq!(ranked.len(), 5);
        for (k, (new_idx, score)) in ranked.iter().enumerate() {
            assert_eq!(perm[*new_idx], base[k].0, "perm={perm:?} slot={k}");
            assert_eq!(*score, base[k].1, "perm={perm:?} slot={k}");
        }
    }
}

// ─── Top-k nesting ───────────────────────────────────────────────────────────

#[test]
fn top_k_nesting_prefix_property() {
    // top-k is a prefix operation: top-2 == top-5[..2] on every ranker, with
    // distinct scores and with ties (index tie-break keeps it total).
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
    // Flat ranker nests the same way (distinct hand angles, no ties).
    let flat = [1.0, 0.0, 3.0, 4.0, 1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 0.5, 0.5];
    let narrow = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 2, None);
    let wide = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(narrow, wide[..2]);
}

// ─── Threshold only filters, never reorders ──────────────────────────────────

#[test]
fn threshold_only_filters_never_reorders() {
    // With scores kept clear of the nextafter gap (0.9/0.7 above, 0.5/0.3/0.1
    // at-or-below Some(0.5)), the thresholded output is exactly the
    // unthresholded output filtered to s > min — same relative order.
    let corpus = vec![(0, 0.9), (1, 0.3), (2, 0.7), (3, 0.5), (4, 0.1), (5, -0.2)];
    let full_sort = top_by_similarity(corpus.clone(), 10, None);
    let full_heap = top_k_similarity(corpus.clone(), 10, None);
    assert_eq!(full_sort, full_heap);
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

// ─── Determinism across threads ──────────────────────────────────────────────

#[test]
fn determinism_across_threads() {
    // Eight threads recompute the same cosine/dot/normalize/rank/embed
    // workload (including the n>=64 rayon flat path); every thread must
    // match the main-thread reference bit-exactly.
    let mut seed = 0x7E57_1C0D_EF00_1234;
    let a = lcg_vec(&mut seed, 70, 1.0);
    let b = lcg_vec(&mut seed, 70, 1.0);
    let flat: Vec<f32> = (0..32).flat_map(|_| lcg_vec(&mut seed, 2, 1.0)).collect();
    let corpus: Vec<(usize, f32)> = (0..32)
        .map(|i| (i, cosine_similarity(&a[..2], &flat[i * 2..(i + 1) * 2])))
        .collect();
    let emb = SemanticLocalEmbedding;
    let texts = ["credential renewal", "sanitize user input", "hello world"];
    let reference = (
        cosine_similarity(&a, &b),
        dot_similarity(&a, &b),
        normalize_vec(&a),
        top_k_flat_similarity(&a[..2], &flat, 2, 8, None),
        top_k_similarity(corpus.clone(), 8, None),
        top_by_similarity(corpus.clone(), 8, None),
        texts.iter().map(|t| emb.embed_text(t)).collect::<Vec<_>>(),
    );
    // Flat corpus sized for the parallel path (n=70 >= 64).
    let big_flat: Vec<f32> = (0..70).flat_map(|_| lcg_vec(&mut seed, 2, 1.0)).collect();
    let big_ref = top_k_flat_similarity(&a[..2], &big_flat, 2, 8, None);
    thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(s.spawn(|| {
                let got = (
                    cosine_similarity(&a, &b),
                    dot_similarity(&a, &b),
                    normalize_vec(&a),
                    top_k_flat_similarity(&a[..2], &flat, 2, 8, None),
                    top_k_similarity(corpus.clone(), 8, None),
                    top_by_similarity(corpus.clone(), 8, None),
                    texts.iter().map(|t| emb.embed_text(t)).collect::<Vec<_>>(),
                );
                let got_big = top_k_flat_similarity(&a[..2], &big_flat, 2, 8, None);
                (got, got_big)
            }));
        }
        for h in handles {
            let (got, got_big) = h.join().expect("thread panicked");
            assert_eq!(got.0, reference.0);
            assert_eq!(got.1, reference.1);
            assert_eq!(got.2, reference.2);
            assert_eq!(got.3, reference.3);
            assert_eq!(got.4, reference.4);
            assert_eq!(got.5, reference.5);
            assert_eq!(got.6, reference.6);
            assert_eq!(got_big, big_ref);
        }
    });
}
