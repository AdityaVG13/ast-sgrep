//! N1 numerical-exactness oracles for ast-sgrep-lang similarity math.
//!
//! L1/L2 (oracle_foundry_pass1/2) own: dot/cosine/normalize basics,
//! top_by ordering/truncation/NaN-drop, embed roundtrip, split/tokenize
//! hand-sets, threshold strictness incl. nextafter spot checks, heap-vs-sort
//! agreement, flat guards, SIMD/scalar dot agreement, cosine nonfinite-skip,
//! concept trigger precision, keyword roots, literal/trivia lanes.
//! Pass 3/4 own metamorphic/E2E pipelines. This file owns the REST of the
//! reachable float surface and does NOT duplicate those pins:
//! threshold bit-boundaries, heap tie-eviction under truncation, flat
//! ragged-tail + parallel-boundary shaping, the chunk ranker, normalize/dot
//! overflow lanes, the SIMD cosine nonfinite lane, embed byte layout,
//! exact concept-expansion strings, trigram-weight structure, and unit-norm
//! tables.
//!
//! Out of reach (NOT pinned here): keyword/pattern scoring in
//! ast-sgrep-core (tantivy/lexical) and the rerank/neural backends — none is
//! a dependency of this [[test]] target, and N1 adds no dependencies.
//!
//! Tolerance table (every expectation states its class inline):
//! - BIT-EXACT (`assert_eq!` / `to_bits`): threshold nextafter boundaries,
//!   heap tie-eviction order, ragged-tail indices, chunk cosine 50/50 = 1.0,
//!   normalize overflow/subnormal/dyadic, dot overflow/dyadic, SIMD-NaN
//!   cosine 0.0, LE byte layout, expansion strings, token-set equality,
//!   zero embeddings, signed-zero order bits, negative ordering, empty
//!   shapings, MIN_SIMILARITY literal.
//! - 1e-6 approx: flat cosine 1.0 heads, chunk 7/(5*sqrt(2)) hand formula,
//!   63-dim NaN-skip 24/25 formula, SIMD parallel-lane 1.0.
//! - 1e-5 approx: 256-dim embedding self-dot (f32 accumulation over 256
//!   lanes; same band as pass-3 precedent); cross-sim Cauchy bound 1+1e-6.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, embed_to_bytes, expand_concepts, normalize_vec,
    rank_chunk_indices_by_vector, tokenize, top_by_similarity, top_k_flat_similarity,
    top_k_similarity, SemanticChunkRow, SemanticLocalEmbedding, MIN_SIMILARITY,
    PARALLEL_CHUNK_THRESHOLD, SEMANTIC_DIM,
};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

fn chunk_row(emb: Vec<f32>) -> SemanticChunkRow {
    ("f.rs".to_string(), 0, 1, "sym".to_string(), "ex".to_string(), emb)
}

// ─── Threshold bit-boundaries (strict nextafter, all sign classes) ───────────

#[test]
fn threshold_nextafter_bits_are_exact_all_signs() {
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
}

// ─── Heap tie-eviction under truncation ──────────────────────────────────────

#[test]
fn top_k_heap_tie_eviction_keeps_ascending_indices() {
    // All five tie at 0.9; the min-heap pops the largest index among ties,
    // so limit 2 keeps indices 0 and 1 however the input is permuted.
    // BIT-EXACT: scores are moved, never recomputed.
    let expected = vec![(0, 0.9), (1, 0.9)];
    let perms = [
        vec![(5, 0.9), (3, 0.9), (1, 0.9), (0, 0.9), (4, 0.9)],
        vec![(0, 0.9), (1, 0.9), (3, 0.9), (4, 0.9), (5, 0.9)],
        vec![(4, 0.9), (5, 0.9), (3, 0.9), (1, 0.9), (0, 0.9)],
    ];
    for perm in perms {
        assert_eq!(top_k_similarity(perm.clone(), 2, None), expected);
        // Differential: the sort ranker agrees with the heap ranker here.
        assert_eq!(top_by_similarity(perm, 2, None), expected);
    }
}

// ─── Flat ranker: ragged tail, limit + threshold shaping ────────────────────

#[test]
fn top_k_flat_truncates_ragged_tail_and_shapes() {
    // flat.len()=5, dim=2 -> n = 5/2 = 2 rows; the trailing 999.0 is not a
    // row and must be ignored (no panic, no third hit).
    // Hand cosines vs query [1,0]: row0 [1,0] -> 1/(1*1) = 1.0;
    // row1 [0,1] -> 0. Indices BIT-EXACT; scores 1e-6 (f64->f32 cast).
    let flat = [1.0, 0.0, 0.0, 1.0, 999.0];
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, 0);
    assert!(approx(ranked[0].1, 1.0), "got {ranked:?}");
    assert_eq!(ranked[1], (1, 0.0));
    // Limit shaping keeps the head only.
    let head = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 1, None);
    assert_eq!(head.len(), 1);
    assert_eq!(head[0].0, 0);
    assert!(approx(head[0].1, 1.0));
    // Threshold shaping is exclusive: 0.0 row drops at Some(0.5), the 1.0
    // head survives; at Some(1.0) even the head drops (strict >).
    assert_eq!(
        top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, Some(0.5)).len(),
        1
    );
    assert!(top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, Some(1.0)).is_empty());
}

// ─── Flat ranker: parallel boundary vs sequential ───────────────────────────

#[test]
fn top_k_flat_parallel_boundary_agrees_with_sequential() {
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
            assert!(approx(ranked[0].1, 1.0) && approx(ranked[1].1, 1.0));
            assert_eq!(ranked[2].1, 0.0);
        }
        // Threshold arm on both paths: only the 1.0 rows survive Some(0.5).
        let flat_t = top_k_flat_similarity(&query, &flat, 2, 10, Some(0.5));
        assert_eq!(flat_t.len(), 2);
        assert_eq!([flat_t[0].0, flat_t[1].0], [5, 60]);
    }
}

// ─── Chunk ranker: hand cosine values + filters ─────────────────────────────

#[test]
fn rank_chunk_indices_pins_hand_cosine_and_filters() {
    // query=[3,4] (L2=5). Row0 [6,8]: dot=18+32=50, L2=10, 50/(5*10)=1.0
    // BIT-EXACT. Row1 [1,1]: dot=7, L2=sqrt(2), 7/(5*sqrt(2)) ~= 0.98994949
    // (1e-6, independent f64 formula). Row2 [4,-3]: dot=12-12=0 -> dropped
    // by the MIN_SIMILARITY threshold. Row3 [0,0]: den=0 -> 0.0 dropped.
    // Row4 [-6,-8]: -1.0 dropped. Row5 wrong dim -> filtered pre-score.
    assert_eq!(MIN_SIMILARITY, 0.08);
    let chunks = vec![
        chunk_row(vec![6.0, 8.0]),
        chunk_row(vec![1.0, 1.0]),
        chunk_row(vec![4.0, -3.0]),
        chunk_row(vec![0.0, 0.0]),
        chunk_row(vec![-6.0, -8.0]),
        chunk_row(vec![1.0, 2.0, 3.0]),
    ];
    let ranked = rank_chunk_indices_by_vector(&[3.0, 4.0], &chunks, 10);
    assert_eq!(ranked.len(), 2, "got {ranked:?}");
    assert_eq!(ranked[0], (0, 1.0));
    assert_eq!(ranked[1].0, 1);
    let expected = 7.0f64 / (5.0 * 2.0f64.sqrt());
    assert!((f64::from(ranked[1].1) - expected).abs() < 1e-6, "got {ranked:?}");
    // Limit shaping keeps the head of the same order.
    assert_eq!(
        rank_chunk_indices_by_vector(&[3.0, 4.0], &chunks, 1),
        vec![(0, 1.0)]
    );
    // Zero query: qn=0 so every den is 0 -> all 0.0 -> threshold drops all.
    let zero_q = rank_chunk_indices_by_vector(&[0.0, 0.0], &chunks, 10);
    assert!(zero_q.is_empty(), "got {zero_q:?}");
}

#[test]
fn rank_chunk_indices_empty_and_limit_shape() {
    // Shaping guards: empty corpus, zero limit, all-filtered, all-dropped.
    let good = vec![chunk_row(vec![6.0, 8.0])];
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &[], 10).is_empty());
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &good, 0).is_empty());
    let wrong_dim = vec![chunk_row(vec![1.0])];
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &wrong_dim, 10).is_empty());
    let orthogonal = vec![chunk_row(vec![4.0, -3.0])];
    assert!(rank_chunk_indices_by_vector(&[3.0, 4.0], &orthogonal, 10).is_empty());
}

// ─── Normalize / dot overflow and denormal lanes ────────────────────────────

#[test]
fn normalize_overflow_subnormal_and_dyadic_pins() {
    // [1e38,1e38]: squares overflow f32 -> norm=inf -> not finite -> zeros.
    // Kills the finite-guard removal (inf/inf = NaN). BIT-EXACT.
    assert_eq!(normalize_vec(&[1e38, 1e38]), vec![0.0, 0.0]);
    assert_eq!(normalize_vec(&[f32::MAX, f32::MAX]), vec![0.0, 0.0]);
    // [MIN_POSITIVE]: square (1.4e-45)^2 underflows to 0.0 -> norm=0 ->
    // zeros. Kills the zero-guard removal (0/0 = NaN). BIT-EXACT.
    assert_eq!(normalize_vec(&[f32::MIN_POSITIVE]), vec![0.0]);
    // Dyadic-exact: norm divides evenly, no rounding. BIT-EXACT.
    assert_eq!(normalize_vec(&[5.0]), vec![1.0]);
    assert_eq!(normalize_vec(&[-2.0, 0.0]), vec![-1.0, 0.0]);
}

#[test]
fn dot_overflow_and_dyadic_lanes() {
    // Scalar lane (len 2): 1e38*1e38 = inf per product -> sum inf -> 0.0.
    assert_eq!(dot_similarity(&[1e38, 1e38], &[1e38, 1e38]), 0.0);
    // SIMD-width lane (len 64): true sum 6.4e61 unrepresentable in f32
    // under either accumulation (f32 inf, or f64 6.4e61 -> `as f32` inf),
    // so both the simd and the scalar-fallback arm collapse to 0.0.
    assert_eq!(
        dot_similarity(&vec![1e30f32; 64], &vec![1e30f32; 64]),
        0.0
    );
    // Dyadic-exact: 0.25+0.25+0.25+0.25 = 1.0, no rounding. BIT-EXACT.
    assert_eq!(
        dot_similarity(&[0.5, 0.5, 0.5, 0.5], &[0.5, 0.5, 0.5, 0.5]),
        1.0
    );
}

// ─── Cosine SIMD lane does NOT skip nonfinite pairs ─────────────────────────

#[test]
fn cosine_simd_lane_does_not_skip_nonfinite() {
    // 64-dim takes the SIMD path: any NaN poisons dot/na/nb -> non-finite
    // -> 0.0. BIT-EXACT 0.0. (The scalar path would skip the pair instead;
    // this pins the deliberate lane divergence.)
    let mut a64 = vec![1.0f32; 64];
    a64[0] = f32::NAN;
    assert_eq!(cosine_similarity(&a64, &vec![1.0f32; 64]), 0.0);
    // 63-dim takes the scalar path: pair 0 is skipped ENTIRELY (neither
    // norm accumulates the survivor side), so 62 pairs of (1,1) give
    // dot=62, na=62, nb=62 -> 62/62 = 1.0. BIT-EXACT.
    let mut a63 = vec![1.0f32; 63];
    a63[0] = f32::NAN;
    assert_eq!(cosine_similarity(&a63, &vec![1.0f32; 63]), 1.0);
    // Non-trivial skip formula at boundary length: a=[NaN,3,4,0x60],
    // b=[5,4,3,0x60]. Pair 0 skipped; survivors give dot=12+12=24,
    // na=9+16=25, nb=16+9=25 -> 24/25 = 0.96 (1e-6; kills both the
    // NaN-poison mutant and the count-skipped-norm mutant, which would
    // give 24/sqrt(25*50) ~= 0.6788 instead).
    let mut a = vec![0.0f32; 63];
    let mut b = vec![0.0f32; 63];
    (a[0], a[1], a[2]) = (f32::NAN, 3.0, 4.0);
    (b[0], b[1], b[2]) = (5.0, 4.0, 3.0);
    let got = cosine_similarity(&a, &b);
    assert!((got - 0.96).abs() < 1e-6, "got {got}");
    // SIMD all-finite parallel vectors still normalize to 1.0:
    // dot=128, na=64, nb=256 -> 128/(8*16) = 1.0 (1e-6).
    let simd_one = cosine_similarity(&vec![1.0f32; 64], &vec![2.0f32; 64]);
    assert!((simd_one - 1.0).abs() < 1e-6, "got {simd_one}");
}

// ─── Embed byte layout (little-endian, not just roundtrip) ──────────────────

#[test]
fn embed_byte_layout_is_little_endian_exact() {
    // 1.0 = 0x3F800000 -> LE 00 00 80 3F. BIT-EXACT.
    assert_eq!(embed_to_bytes(&[1.0]), vec![0x00, 0x00, 0x80, 0x3F]);
    // -2.5 = -(1.25 * 2^1): exp 128, mantissa .25 -> 0xC0200000
    // -> LE 00 00 20 C0. BIT-EXACT.
    assert_eq!(embed_to_bytes(&[-2.5]), vec![0x00, 0x00, 0x20, 0xC0]);
    assert_eq!(embed_to_bytes(&[0.0]), vec![0x00, 0x00, 0x00, 0x00]);
}

// ─── Exact concept-expansion strings ────────────────────────────────────────

#[test]
fn expand_concepts_single_and_multi_group_strings_are_exact() {
    // "combine": tokens=[combine]; only the conjunction group fires.
    // parts = {combine} U {combine,conjunction,intersect,intersection},
    // sorted (combine < conjunction: 'm'<'n'; intersect < intersection:
    // prefix rule). BIT-EXACT whole string.
    assert_eq!(
        expand_concepts("combine"),
        "combine combine conjunction intersect intersection"
    );
    // "auth refresh": tokens=[auth,refresh]; auth group (9 terms) + refresh
    // group (7 terms), disjoint -> 16 parts, byte-sorted. BIT-EXACT.
    assert_eq!(
        expand_concepts("auth refresh"),
        "auth refresh auth authentication bearer credential identity login \
         oauth refresh reissue renew renewal revoke rotate session token update"
    );
    // The expanded token SET is exactly those 16 terms (query words add
    // nothing new). BIT-EXACT vec equality.
    assert_eq!(
        tokenize(&expand_concepts("auth refresh")),
        vec![
            "auth",
            "authentication",
            "bearer",
            "credential",
            "identity",
            "login",
            "oauth",
            "refresh",
            "reissue",
            "renew",
            "renewal",
            "revoke",
            "rotate",
            "session",
            "token",
            "update"
        ]
    );
}

// ─── Trigram-weight structure: order-sensitive, token-static ─────────────────

#[test]
fn embed_trigram_path_is_order_sensitive_while_tokens_static() {
    // Token features (weight 1.0) are a SET: order-invariant.
    assert_eq!(tokenize("ab cd"), tokenize("cd ab"));
    assert_eq!(tokenize("ab cd"), vec!["ab", "cd"]);
    // Trigram features (weight 0.35) run over the ordered compact string:
    // "ab cd ab cd" -> compact abcdabcd (windows abc,bcd,cda,dab,abc,bcd)
    // vs "cd ab ab cd" -> compact cdababcd (windows cda,dab,aba,bab,abc,bcd).
    // Different window multisets -> different blake3 signs -> the two
    // embeddings MUST differ, isolating the trigram path. Both unit-norm
    // (1e-5 band) and strictly below self-similarity.
    let emb = SemanticLocalEmbedding;
    let a = emb.embed_text("ab cd");
    let b = emb.embed_text("cd ab");
    assert_eq!(a.len(), SEMANTIC_DIM);
    assert_ne!(a, b);
    assert!((dot_similarity(&a, &a) - 1.0).abs() < 1e-5);
    assert!((dot_similarity(&b, &b) - 1.0).abs() < 1e-5);
    assert!(dot_similarity(&a, &b) < 1.0);
}

// ─── Embedding unit-norm + zero tables ──────────────────────────────────────

#[test]
fn embed_unit_norm_and_cauchy_schwarz_table() {
    // Hand rule: embed_text divides by the L2 norm, so every NONZERO
    // embedding is unit up to f32 rounding (1e-5 over 256 lanes), and every
    // cross-similarity obeys Cauchy-Schwarz (|s| <= 1 + rounding).
    let queries = [
        "credential renewal",
        "sanitize user input",
        "hello world",
        "ab",
        "throttle inbound",
        "a1B2",
        "HTTPStatusCode",
        "x y z", // no 2+ char tokens, but compact "xyz" feeds one trigram
    ];
    let emb = SemanticLocalEmbedding;
    let vecs: Vec<Vec<f32>> = queries.iter().map(|q| emb.embed_text(q)).collect();
    for (q, v) in queries.iter().zip(vecs.iter()) {
        assert_eq!(v.len(), SEMANTIC_DIM, "{q:?}");
        assert!(v.iter().all(|x| x.is_finite()), "{q:?}");
        assert!(v.iter().any(|x| *x != 0.0), "{q:?} must be nonzero");
        assert!(
            (dot_similarity(v, v) - 1.0).abs() < 1e-5,
            "{q:?} self-dot = {}",
            dot_similarity(v, v)
        );
    }
    for (i, a) in vecs.iter().enumerate() {
        for (j, b) in vecs.iter().enumerate() {
            if i != j {
                let s = dot_similarity(a, b);
                assert!(s.abs() <= 1.0 + 1e-6, "{i},{j}: {s}");
            }
        }
    }
    // Zero table, BIT-EXACT: single alphanumerics yield no tokens
    // (len < 2 split) and a compact shorter than one trigram window.
    assert_eq!(emb.embed_text("a"), vec![0.0; SEMANTIC_DIM]);
    assert_eq!(emb.embed_text("I"), vec![0.0; SEMANTIC_DIM]);
}

// ─── Sort ranker: negatives kept, signed-zero tie by index ──────────────────

#[test]
fn top_by_keeps_negative_and_orders_signed_zero_by_index() {
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
}
