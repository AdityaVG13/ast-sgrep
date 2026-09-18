//! N2 nonfinite/degenerate-totality oracles for ast-sgrep-lang math.
//!
//! L1/L2 (oracle_foundry_pass1/2) own: dot/cosine/normalize basics,
//! top_by ordering/truncation/NaN-drop, embed roundtrip, split/tokenize
//! hand-sets, threshold strictness, heap-vs-sort agreement, flat guards,
//! SIMD/scalar dot agreement, cosine nonfinite-skip, concept triggers,
//! keyword roots, literal/trivia lanes. N1 (numerical_pass1) owns:
//! threshold bit-boundaries, heap tie-eviction, flat ragged-tail +
//! parallel-boundary shaping, the chunk ranker, normalize/dot overflow
//! lanes, the SIMD cosine nonfinite lane, embed byte layout, exact
//! expansion strings, trigram structure, unit-norm tables. Pass 3/4 own
//! metamorphic/E2E pipelines (incl. all-NaN 70-dim collapse, NaN-query
//! flat zeros, k>len sort shaping, empty-input top_by).
//!
//! This file owns the REST of the degenerate surface and does NOT
//! duplicate those pins: N2 proves TOTALITY over hostile inputs across
//! every numeric fn — NaN/inf elements, zero vectors, empty vectors,
//! mismatched dims, extreme magnitudes, all-NaN corpora, k=0/k>len.
//! Each test pins the DOCUMENTED outcome: never panic, never silent
//! garbage (NaN out of a normalizer/ranker), only honest zeros/empties.
//!
//! Out of reach (NOT pinned here): keyword/pattern scoring in
//! ast-sgrep-core and the rerank/neural backends — none is a dependency
//! of this [[test]] target, and N2 adds no dependencies.
//!
//! Tolerance table (every expectation states its class inline):
//! - BIT-EXACT (`assert_eq!` / `to_bits`): subnormal/signed-zero dot,
//!   MAX finite-vs-overflow split, exact cancellation, cosine skip
//!   survivors, normalize in-place lanes, signed-zero fill bits, ranker
//!   empty/shaping orders, flat degenerate shapes, flat nonfinite
//!   survival, parallel hostile order, chunk fail-closed rows, degenerate
//!   embed zeros, similarity delegation, byte edges.
//! - 1e-6 approx: cosine extreme-magnitude ratios (f64 sqrt/divide
//!   rounding; kills the overflow-collapse mutant, not the last ulp).

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, embed_from_bytes, embed_to_bytes, normalize_vec,
    normalize_vec_in_place, rank_chunk_indices_by_vector, top_by_similarity,
    top_k_flat_similarity, top_k_similarity, SemanticChunkRow, SemanticLocalEmbedding,
    SEMANTIC_DIM,
};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

fn chunk_row(emb: Vec<f32>) -> SemanticChunkRow {
    ("f.rs".to_string(), 0, 1, "sym".to_string(), "ex".to_string(), emb)
}

// ─── Dot: zero vectors, subnormal underflow, signed-zero payloads ────────────

#[test]
fn dot_zero_subnormal_and_signed_zero_lanes_are_exact() {
    // Zero vectors are honest zeros, not NaN. BIT-EXACT.
    assert_eq!(dot_similarity(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
    assert_eq!(dot_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    // MIN_POSITIVE^2 underflows to 0.0 (finite collapse). BIT-EXACT.
    assert_eq!(
        dot_similarity(&[f32::MIN_POSITIVE], &[f32::MIN_POSITIVE]),
        0.0
    );
    // MIN_POSITIVE * 1.0 is exactly representable. BIT-EXACT.
    assert_eq!(dot_similarity(&[f32::MIN_POSITIVE], &[1.0]), f32::MIN_POSITIVE);
    // Signed-zero payloads survive the finite gate; pin the BITS.
    assert_eq!(
        dot_similarity(&[-0.0], &[1.0]).to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(dot_similarity(&[-0.0], &[-0.0]).to_bits(), 0.0f32.to_bits());
}

// ─── Dot: extreme magnitudes split finite from overflow ─────────────────────

#[test]
fn dot_extreme_magnitudes_split_finite_from_overflow() {
    // MAX * 1.0 is finite and exact; MAX * 2.0 overflows to inf -> 0.0.
    // BIT-EXACT both arms (documents the f32-accumulation boundary).
    assert_eq!(dot_similarity(&[f32::MAX], &[1.0]), f32::MAX);
    assert_eq!(dot_similarity(&[f32::MAX], &[2.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::MAX], &[f32::MAX]), 0.0);
    // Exact cancellation at extreme magnitude: 1e30 + -1e30 = 0.0.
    // BIT-EXACT (kills the nonfinite-collapse mutant: this 0.0 is a real
    // sum, not an overflow guard).
    assert_eq!(dot_similarity(&[1e30, -1e30], &[1.0, 1.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::MAX, f32::MAX], &[1.0, -1.0]), 0.0);
}

// ─── Cosine: all-skipped pairs, one-sided survivors, SIMD inf ────────────────

#[test]
fn cosine_skip_all_pairs_and_one_sided_survivor() {
    // Every pair skipped -> both norms are 0.0 -> honest 0.0. BIT-EXACT.
    assert_eq!(cosine_similarity(&[f32::NAN, f32::NAN], &[1.0, 2.0]), 0.0);
    assert_eq!(
        cosine_similarity(&[f32::INFINITY, f32::NEG_INFINITY], &[1.0, 2.0]),
        0.0
    );
    // One-sided survivors still score: pair 0 skipped, survivors give
    // dot=12, na=9, nb=16 -> 12/(3*4) = 1.0. BIT-EXACT.
    assert_eq!(cosine_similarity(&[f32::INFINITY, 3.0], &[4.0, 4.0]), 1.0);
    // -inf on the right skips the same way: dot=4, na=4, nb=4 -> 1.0.
    assert_eq!(
        cosine_similarity(&[1.0, 2.0], &[f32::NEG_INFINITY, 2.0]),
        1.0
    );
    // SIMD lane (64-dim): a single +inf poisons dot/na/nb -> 0.0.
    // BIT-EXACT. (Rides N1's pin that the SIMD path is taken here.)
    let mut a64 = vec![1.0f32; 64];
    a64[0] = f32::INFINITY;
    assert_eq!(cosine_similarity(&a64, &vec![1.0f32; 64]), 0.0);
}

// ─── Cosine: extreme magnitudes stay exact via the f64 lane ─────────────────

#[test]
fn cosine_extreme_magnitudes_avoid_dot_overflow_collapse() {
    // The scalar cosine accumulates in f64, so magnitudes that collapse
    // dot_similarity (f32) to 0.0 still score ~1.0 here. 1e-6: the ratio
    // x/(sqrt(x)*sqrt(x)) is exact-class but not bit-promised.
    let got = cosine_similarity(&[1e30], &[1e30]);
    assert!(approx(got, 1.0), "got {got}");
    assert_eq!(dot_similarity(&[1e30], &[1e30]), 0.0);
    let max = cosine_similarity(&[f32::MAX], &[f32::MAX]);
    assert!(approx(max, 1.0), "got {max}");
    let anti = cosine_similarity(&[f32::MAX], &[-f32::MAX]);
    assert!(approx(anti, -1.0), "got {anti}");
    let two = cosine_similarity(&[f32::MAX, f32::MAX], &[f32::MAX, f32::MAX]);
    assert!(approx(two, 1.0), "got {two}");
}

// ─── Normalize: in-place totality + signed-zero fill bits ───────────────────

#[test]
fn normalize_in_place_totality_and_signed_zero_bits() {
    // Empty in place: no panic, stays empty.
    let mut empty: Vec<f32> = vec![];
    normalize_vec_in_place(&mut empty);
    assert!(empty.is_empty());
    // Overflow in place: norm=inf -> fill zeros (matches out-of-place).
    let mut huge = vec![1e38, 1e38];
    normalize_vec_in_place(&mut huge);
    assert_eq!(huge, vec![0.0, 0.0]);
    // Non-finite zeroing in place: [inf,-3] -> [0,-1]. BIT-EXACT.
    let mut mixed = vec![f32::INFINITY, -3.0];
    normalize_vec_in_place(&mut mixed);
    assert_eq!(mixed, vec![0.0, -1.0]);
    // Zero-norm fill writes +0.0 even for -0.0 inputs; pin the BITS
    // (assert_eq alone cannot see the sign).
    let filled = normalize_vec(&[0.0, -0.0]);
    assert_eq!(filled.len(), 2);
    assert_eq!(filled[0].to_bits(), 0.0f32.to_bits());
    assert_eq!(filled[1].to_bits(), 0.0f32.to_bits());
    assert_eq!(normalize_vec(&[-0.0])[0].to_bits(), 0.0f32.to_bits());
}

// ─── Rankers: all-nonfinite corpora, empty input, k=0/k>len ──────────────────

#[test]
fn rankers_all_nonfinite_empty_and_limit_edges() {
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
}

// ─── Rankers: -inf dropped, negative order agrees across rankers ─────────────

#[test]
fn rankers_drop_negative_infinity_and_keep_negative_order() {
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
}

// ─── Flat ranker: degenerate shapes return empty, k>len, zero rows ───────────

#[test]
fn flat_degenerate_shapes_empty_and_limit_edges() {
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
    assert!(approx(ranked[0].1, 1.0), "got {ranked:?}");
    assert_eq!(ranked[1], (1, 0.0));
    // All-zero rows score exactly 0.0 and survive unthresholded.
    let zeros = top_k_flat_similarity(&[1.0, 0.0], &[0.0, 0.0, 0.0, 0.0], 2, 5, None);
    assert_eq!(zeros, vec![(0, 0.0), (1, 0.0)]);
    // ... but the threshold arm drops them all.
    assert!(top_k_flat_similarity(&[1.0, 0.0], &[0.0, 0.0, 0.0, 0.0], 2, 5, Some(0.5)).is_empty());
}

// ─── Flat ranker: nonfinite corpus/query totality ────────────────────────────

#[test]
fn flat_nonfinite_corpus_and_query_totality() {
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
        top_k_flat_similarity(
            &[f32::INFINITY, 1.0],
            &[1.0, 0.0, 0.0, 1.0],
            2,
            5,
            None
        ),
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
}

// ─── Flat ranker: parallel path with hostile rows keeps hand order ───────────

#[test]
fn flat_parallel_path_with_hostile_rows_keeps_hand_order() {
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
    let mut expected: Vec<(usize, f32)> =
        (0..64).step_by(2).map(|i| (i, 1.0)).collect();
    expected.extend([(1, 0.0), (3, 0.0), (5, 0.0), (7, 0.0), (9, 0.0), (11, 0.0), (13, 0.0), (15, 0.0)]);
    assert_eq!(ranked, expected);
    assert!(ranked.iter().all(|(_, s)| s.is_finite()));
}

// ─── Chunk ranker: nonfinite query/rows fail closed, limit extreme ───────────

#[test]
fn chunk_ranker_nonfinite_query_and_rows_fail_closed() {
    let good = chunk_row(vec![6.0, 8.0]);
    // NaN query: unguarded l2 is NaN, den is NaN, `den > 0.0` is false,
    // so every row scores 0.0 and the threshold drops all. Empty.
    let nan_q = rank_chunk_indices_by_vector(
        &[f32::NAN, f32::NAN],
        &[chunk_row(vec![6.0, 8.0])],
        10,
    );
    assert!(nan_q.is_empty(), "got {nan_q:?}");
    // NaN/inf rows score 0.0 (NaN den / 0.0-per-nonfinite-dot) and drop;
    // the good row still scores exactly 1.0. BIT-EXACT.
    let mixed = vec![
        good,
        chunk_row(vec![f32::NAN, f32::NAN]),
        chunk_row(vec![f32::INFINITY, f32::INFINITY]),
    ];
    assert_eq!(
        rank_chunk_indices_by_vector(&[3.0, 4.0], &mixed, 10),
        vec![(0, 1.0)]
    );
    // Empty-dim degenerate: zero-length query and row pass the dim
    // filter but den is 0.0 -> 0.0 -> threshold drops. Empty.
    let empty_dim = vec![chunk_row(Vec::new())];
    assert!(rank_chunk_indices_by_vector(&[], &empty_dim, 10).is_empty());
    // Extreme limit admits every survivor without padding or panic.
    let one = vec![chunk_row(vec![6.0, 8.0])];
    assert_eq!(
        rank_chunk_indices_by_vector(&[3.0, 4.0], &one, usize::MAX),
        vec![(0, 1.0)]
    );
}

// ─── embed_text: degenerate inputs are total ─────────────────────────────────

#[test]
fn embed_text_degenerate_inputs_are_total() {
    let emb = SemanticLocalEmbedding;
    // Whitespace-only and punctuation-only yield no tokens and a compact
    // shorter than one trigram window: exact zero vectors. BIT-EXACT.
    assert_eq!(emb.embed_text("   "), vec![0.0; SEMANTIC_DIM]);
    assert_eq!(emb.embed_text("!!!"), vec![0.0; SEMANTIC_DIM]);
    // Non-ASCII alphanumerics and very long inputs stay shaped, finite,
    // and deterministic (totality: no panic, no NaN poisoning).
    for text in ["日本語", &"credential renewal ".repeat(500)] {
        let v = emb.embed_text(text);
        assert_eq!(v.len(), SEMANTIC_DIM, "{text:?} shape");
        assert!(v.iter().all(|x| x.is_finite()), "{text:?} finite");
        assert_eq!(v, emb.embed_text(text), "{text:?} deterministic");
    }
}

// ─── Provider similarity delegation + byte edges ─────────────────────────────

#[test]
fn similarity_delegation_and_byte_edges() {
    let emb = SemanticLocalEmbedding;
    // similarity delegates to dot, including the degenerate lanes.
    assert_eq!(emb.similarity(&[], &[]), 0.0);
    assert_eq!(emb.similarity(&[1.0], &[1.0, 2.0]), 0.0);
    assert_eq!(emb.similarity(&[f32::NAN], &[1.0]), 0.0);
    assert_eq!(emb.similarity(&[f32::INFINITY], &[1.0]), 0.0);
    // Byte edges: empty encodes to empty; aligned NaN payloads decode
    // Ok with bits preserved (never Err on aligned length). BIT-EXACT.
    assert!(embed_to_bytes(&[]).is_empty());
    let nan = embed_from_bytes(&[0xFF, 0xFF, 0xFF, 0xFF]).expect("aligned ok");
    assert_eq!(nan.len(), 1);
    assert_eq!(nan[0].to_bits(), 0xFFFF_FFFF);
    let pair = embed_from_bytes(&[0x00, 0x00, 0x80, 0x3F, 0x00, 0x00, 0x00, 0x00])
        .expect("aligned ok");
    assert_eq!(pair, vec![1.0, 0.0]);
}
