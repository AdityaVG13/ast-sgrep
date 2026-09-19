//! Cross-cutting numerical keeps: determinism + end-to-end pipelines.
//!
//! Holds the three KEEP verdicts that cannot live in one function contract:
//! thread determinism, the text->embed->dot->rank pipeline (weakened per the
//! catalog TAUTOLOGY-RISK note), and the normalize->dot->rank hostile pipeline.
//! Tolerance table: BIT-EXACT (`assert_eq!`) for determinism reruns, index
//! orders, self-first slot; 1e-6 inline for normalized-dot hand scores;
//! 1e-5 inline for the 256-lane text self-dot (f32 accumulation band).

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, normalize_vec, top_by_similarity,
    top_k_flat_similarity, top_k_similarity, SemanticLocalEmbedding,
};
use ast_sgrep_testkit::{lcg_vec, ranked_indices};
use std::thread;

/// INTENT: identical cosine/dot/normalize/rank/embed workload (including the
/// n>=64 rayon flat path) reproduces bit-exactly across 8 threads.
/// KILLS: nondeterminism/rayon-race mutant.
/// ABSORBS: none — KEEP standalone (cross-cutting; N3 verbatim).
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

/// INTENT: full text->embed->dot->rank pipeline — the self-match wins with
/// separation over cross-scores and bit-exact rerun determinism.
/// KILLS: ranking/nondeterminism mutants (self-first + separation +
/// determinism arms). The exact cross-order assert is TAUTOLOGY-RISK per the
/// catalog (it snapshots observed blake3 hashes, not hand values) and is gone.
/// ABSORBS: none — KEEP standalone, WEAKENED (cross-function; N4, order line dropped).
#[test]
fn text_embed_dot_rank_self_match_wins_deterministically() {
    // Full text->embed->dot->rank pipeline. Hand fact: the query embedded
    // against itself is a unit self-dot ~= 1.0 (1e-5 band, N1 precedent),
    // and distinct hash vectors sit far below it, so doc 1 wins exactly;
    // two independent runs agree bit-exactly on the whole order.
    // WEAKENED (catalog TAUTOLOGY-RISK): the exact-order assert on the
    // observed blake3 cross-scores is replaced by self-first + separation.
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
    assert_eq!(ranked[0].0, 1, "self-match must win: {ranked:?}");
    assert!((ranked[0].1 - 1.0).abs() < 1e-5, "got {ranked:?}");
    for (_, s) in &ranked[1..] {
        assert!(*s < ranked[0].1, "got {ranked:?}");
        assert!(*s < 0.5, "got {ranked:?}");
    }
    assert_eq!(run(), ranked);
}

/// INTENT: normalize->dot->rank over a hostile corpus (NaN row included)
/// yields the exact hand order with hand margins.
/// KILLS: pipeline-composition (wrong-normalized-score-reorder) mutant.
/// ABSORBS: none — KEEP standalone (cross-function; N4 verbatim).
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
    assert_eq!(ranked_indices(&ranked), vec![0, 3, 1, 2]);
    assert!((ranked[0].1 - 1.0).abs() < 1e-6, "got {ranked:?}");
    assert!((ranked[1].1 - 0.8).abs() < 1e-6, "got {ranked:?}");
    assert!(ranked[2].1.abs() < 1e-6, "got {ranked:?}");
    assert!((ranked[3].1 + 1.0).abs() < 1e-6, "got {ranked:?}");
}
