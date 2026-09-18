//! N4 end-to-end scoring drills for ast-sgrep-lang math.
//!
//! N1 (numerical_pass1) owns exactness pins, N2 (numerical_pass2) owns
//! degenerate totality, N3 (numerical_pass3) owns metamorphic relations,
//! and L1/L2 own ranker-agreement relations. This file owns FULL
//! similarity->rank PIPELINES ONLY and does NOT duplicate those pins:
//! every test runs an end-to-end pipeline (cosine/dot over a hand-built
//! corpus, then heap/sort/chunk ranking, optionally through normalize or
//! embed_text) over an ADVERSARIAL corpus and asserts the EXACT FINAL
//! ORDERING — never diagnostic message text.
//!
//! Drill table (9 tests):
//! - angle-vs-magnitude confound: cosine pipeline ranks by angle while a
//!   dot ranking would crown the high-magnitude row (hand arithmetic)
//! - near-tie epsilon ladder with a duplicate pair (deterministic breaks)
//! - hostile mixed corpus: NaN/zero/inf/duplicate rows among clean rows
//! - flat threshold boundary: hand-computed 0.6 dropped at Some(0.6)
//! - chunk threshold straddle: 0.0797 dropped / 0.0830 kept at MIN_SIMILARITY
//! - duplicate-vector corpus: truncation keeps lowest indices
//! - zero query: index order unthresholded, empty thresholded
//! - text embed->dot->rank: self-match wins, separation, determinism
//! - normalize->dot->rank over a hostile corpus (NaN row included)
//!
//! Tolerance table (every expectation states its class inline):
//! - BIT-EXACT (assert_eq! on indices / full vecs / scores): every final
//!   index ordering, 1.0/0.0/-1.0/0.6 dyadic-exact heads, tie bit-equality,
//!   empty cuts, determinism across runs.
//! - 1e-6 approx: irrational rungs (1/sqrt(2), 1/sqrt(1+e^2),
//!   2/sqrt(580)) checked against independent f64 formulas.
//! - 1e-5 approx: text self-dot only (256-lane f32 accumulation band,
//!   same as the N1 precedent).

use ast_sgrep_embed::{
    dot_similarity, normalize_vec, rank_chunk_indices_by_vector, top_k_flat_similarity,
    top_k_similarity, SemanticChunkRow, SemanticLocalEmbedding, MIN_SIMILARITY,
};

fn chunk_row(emb: Vec<f32>) -> SemanticChunkRow {
    ("f.rs".to_string(), 0, 1, "sym".to_string(), "ex".to_string(), emb)
}

fn indices(ranked: &[(usize, f32)]) -> Vec<usize> {
    ranked.iter().map(|(i, _)| *i).collect()
}

// ─── Angle-vs-magnitude confound ─────────────────────────────────────────────

#[test]
fn cosine_pipeline_ranks_by_angle_not_magnitude() {
    // query=[1,0]. row0 [100,100]: dot=100, |q|=1, |r|=sqrt(20000)=141.421356
    //   -> 100/141.421356 = 0.70710678. row1 [1,0] -> 1/1 = 1.0 BIT-EXACT.
    // row2 [0,1] -> 0/1 = 0.0 BIT-EXACT. row3 [-5,0] -> -5/5 = -1.0 BIT-EXACT.
    // A dot ranking would crown row0 (100.0); the cosine pipeline must rank
    // by angle: exact final order [1, 0, 2, 3].
    let flat = [100.0, 100.0, 1.0, 0.0, 0.0, 1.0, -5.0, 0.0];
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 4, None);
    assert_eq!(ranked.len(), 4);
    assert_eq!(indices(&ranked), vec![1, 0, 2, 3]);
    assert_eq!(ranked[0].1, 1.0);
    assert!((ranked[1].1 - 0.70710678).abs() < 1e-6, "got {ranked:?}");
    assert_eq!(ranked[2].1, 0.0);
    assert_eq!(ranked[3].1, -1.0);
    // Control: the dot confound is real — row0 out-dots row1 a hundredfold.
    assert_eq!(dot_similarity(&[1.0, 0.0], &[100.0, 100.0]), 100.0);
    assert_eq!(dot_similarity(&[1.0, 0.0], &[1.0, 0.0]), 1.0);
}

// ─── Near-tie epsilon ladder ─────────────────────────────────────────────────

#[test]
fn near_tie_ladder_orders_by_epsilon_with_index_break() {
    // cos([1,0],[1,e]) = 1/sqrt(1+e^2): e=0 -> 1.0 BIT-EXACT; e=0.01 ->
    // 0.99995000; e=0.02 -> 0.99980006; e=0.03 -> 0.99955013. Adjacent gap
    // ~1.5e-4 dwarfs f32 rounding (~6e-8), so the ladder cannot flip.
    // idx1/idx2 share e=0.01 -> bit-identical scores -> ascending index.
    // Exact final order: [4, 1, 2, 0, 3].
    let flat = [1.0, 0.02, 1.0, 0.01, 1.0, 0.01, 1.0, 0.03, 1.0, 0.0];
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(ranked.len(), 5);
    assert_eq!(indices(&ranked), vec![4, 1, 2, 0, 3]);
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
}

// ─── Hostile mixed corpus ────────────────────────────────────────────────────

#[test]
fn hostile_mixed_corpus_keeps_clean_order_zeros_sort_by_index() {
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
        vec![
            (1, 1.0),
            (5, 1.0),
            (0, 0.0),
            (2, 0.0),
            (3, 0.0),
            (4, 0.0)
        ]
    );
}

// ─── Flat threshold boundary ─────────────────────────────────────────────────

#[test]
fn flat_threshold_boundary_drops_equal_score_keeps_above() {
    // query=[1,0]. idx0 [3,4]: 3/(1*5) = 0.6 exactly ((3/5)f64 cast to f32
    // is 0.6f32); exceeds_threshold is strict (sim > next(min)), so
    // Some(0.6) drops it. idx1 [1,1] -> 1/sqrt(2) = 0.70710678 survives;
    // idx2 [1,0] -> 1.0. Unthresholded control [2,1,0] proves the drop is
    // strictness, not value. Both orders BIT-EXACT on indices.
    let flat = [3.0, 4.0, 1.0, 1.0, 1.0, 0.0];
    let full = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 3, None);
    assert_eq!(indices(&full), vec![2, 1, 0]);
    assert_eq!(full[2].1, 0.6);
    let cut = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 3, Some(0.6));
    assert_eq!(cut.len(), 2);
    assert_eq!(cut[0], (2, 1.0));
    assert_eq!(cut[1].0, 1);
    assert!((cut[1].1 - 0.70710678).abs() < 1e-6, "got {cut:?}");
}

// ─── Chunk threshold straddle ────────────────────────────────────────────────

#[test]
fn chunk_ranker_threshold_straddle_keeps_above_drops_below() {
    // query=[1,0] against MIN_SIMILARITY (design guard below checks the
    // straddle against the constant; the literal is N1's pin, not this one's).
    // idx0 [2,25]: 2/sqrt(629) = 2/25.079872 = 0.079745 < min -> dropped.
    // idx1 [2,24]: 2/sqrt(580) = 2/24.083189 = 0.083045 > next(min) -> kept.
    // idx2 [1,0] -> 1.0. Margins (2.5e-4 / 3e-3) dwarf f32 rounding.
    // Exact final order: [(2, 1.0), (1, 0.083045)].
    let below_ref = 2.0f64 / 629.0f64.sqrt();
    let above_ref = 2.0f64 / 580.0f64.sqrt();
    assert!(below_ref < f64::from(MIN_SIMILARITY));
    assert!(above_ref > f64::from(MIN_SIMILARITY));
    let chunks = vec![
        chunk_row(vec![2.0, 25.0]),
        chunk_row(vec![2.0, 24.0]),
        chunk_row(vec![1.0, 0.0]),
    ];
    let ranked = rank_chunk_indices_by_vector(&[1.0, 0.0], &chunks, 10);
    assert_eq!(ranked.len(), 2, "got {ranked:?}");
    assert_eq!(ranked[0], (2, 1.0));
    assert_eq!(ranked[1].0, 1);
    assert!(
        (f64::from(ranked[1].1) - above_ref).abs() < 1e-6,
        "got {ranked:?} expected {above_ref}"
    );
}

// ─── Duplicate-vector truncation ─────────────────────────────────────────────

#[test]
fn duplicate_corpus_truncation_keeps_lowest_indices() {
    // 8 identical rows [1,1] vs [1,0]: every score is the same f32 bit
    // pattern (1/sqrt(2) = 0.70710678), so limit 5 keeps [0..5] and the
    // full limit keeps [0..8]. Indices BIT-EXACT, scores bit-equal.
    let flat: Vec<f32> = (0..8).flat_map(|_| [1.0, 1.0]).collect();
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 5, None);
    assert_eq!(indices(&ranked), vec![0, 1, 2, 3, 4]);
    for (_, s) in &ranked {
        assert_eq!(*s, ranked[0].1);
        assert!((s - 0.70710678).abs() < 1e-6, "got {ranked:?}");
    }
    let full = top_k_flat_similarity(&[1.0, 0.0], &flat, 2, 8, None);
    assert_eq!(indices(&full), vec![0, 1, 2, 3, 4, 5, 6, 7]);
}

// ─── Zero query ──────────────────────────────────────────────────────────────

#[test]
fn zero_query_flat_pipeline_orders_by_index_threshold_empties() {
    // query=[0,0]: every na is 0.0 — the [NaN,5] row skips pair 0 but its
    // survivor still accumulates na=0 — so all scores are exactly 0.0 and
    // the order is purely by index. Some(0.0) drops all of them
    // (0.0 > from_bits(1) is false). Whole order BIT-EXACT.
    let flat = [1.0, 0.0, 3.0, 4.0, 0.0, 0.0, f32::NAN, 5.0];
    let ranked = top_k_flat_similarity(&[0.0, 0.0], &flat, 2, 4, None);
    assert_eq!(ranked, vec![(0, 0.0), (1, 0.0), (2, 0.0), (3, 0.0)]);
    assert!(top_k_flat_similarity(&[0.0, 0.0], &flat, 2, 4, Some(0.0)).is_empty());
}

// ─── Text embed->dot->rank ───────────────────────────────────────────────────

#[test]
fn text_embed_dot_rank_self_match_wins_deterministically() {
    // Full text->embed->dot->rank pipeline. Hand fact: the query embedded
    // against itself is a unit self-dot ~= 1.0 (1e-5 band, N1 precedent),
    // and distinct hash vectors sit far below it, so doc 1 wins exactly;
    // two independent runs agree bit-exactly on the whole order.
    let emb = SemanticLocalEmbedding;
    let docs = [
        "credential renewal",
        "sanitize user input",
        "throttle inbound",
        "hello world",
    ];
    let run = || {
        let qv = emb.embed_text(docs[1]);
        let scored: Vec<(usize, f32)> = docs
            .iter()
            .enumerate()
            .map(|(i, d)| (i, emb.similarity(&qv, &emb.embed_text(d))))
            .collect();
        top_k_similarity(scored, 4, None)
    };
    let ranked = run();
    assert_eq!(ranked.len(), 4);
    // Exact final order (deterministic blake3 hash vectors; observed cross
    // scores 0.077/0.046/0.003, all far below the self-dot ~= 1.0 head).
    assert_eq!(indices(&ranked), vec![1, 2, 0, 3]);
    assert!((ranked[0].1 - 1.0).abs() < 1e-5, "got {ranked:?}");
    for (_, s) in &ranked[1..] {
        assert!(*s < ranked[0].1, "got {ranked:?}");
        assert!(*s < 0.5, "got {ranked:?}");
    }
    assert_eq!(run(), ranked);
}

// ─── Normalize->dot->rank hostile pipeline ───────────────────────────────────

#[test]
fn normalize_dot_rank_hostile_pipeline_exact_order() {
    // query [3,4] -> norm 5 -> [0.6,0.8].
    // idx0 [30,40] -> norm 50 -> [0.6,0.8]: dot = 0.36+0.64 ~= 1.0.
    // idx1 [4,-3] -> norm 5 -> [0.8,-0.6]: dot = 0.48-0.48 ~= 0.0.
    // idx2 [-3,-4] -> [-0.6,-0.8]: dot = -0.36-0.64 ~= -1.0.
    // idx3 [NaN,1] -> nonfinite zeroed -> [0,1], norm 1: dot = 0.8.
    // Margins (>= 0.2) dwarf f32 rounding (~1e-7): order [0,3,1,2] exact.
    let q = normalize_vec(&[3.0, 4.0]);
    let rows = [
        vec![30.0, 40.0],
        vec![4.0, -3.0],
        vec![-3.0, -4.0],
        vec![f32::NAN, 1.0],
    ];
    let scored: Vec<(usize, f32)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (i, dot_similarity(&q, &normalize_vec(r))))
        .collect();
    let ranked = top_k_similarity(scored, 4, None);
    assert_eq!(indices(&ranked), vec![0, 3, 1, 2]);
    assert!((ranked[0].1 - 1.0).abs() < 1e-6, "got {ranked:?}");
    assert!((ranked[1].1 - 0.8).abs() < 1e-6, "got {ranked:?}");
    assert!(ranked[2].1.abs() < 1e-6, "got {ranked:?}");
    assert!((ranked[3].1 + 1.0).abs() < 1e-6, "got {ranked:?}");
}
