//! Intent tests for the native static embedding engine.
//!
//! Expected token ids were produced by an independent Python reference
//! implementation of BERT WordPiece over the pinned
//! `minishlab/potion-code-16M-v2` tokenizer.json (revision
//! e9d2a44ca6a05ac6685f3b23709ea57eb7352d5b) -- they are not derived from the
//! code under test. Semantic thresholds come from the same reference over
//! the pinned f16 table: cos(read,load)=0.514, cos(read,sort)=0.008.

use ast_sgrep_embed::{embed_text, StaticEmbedder, STATIC_DIM, STATIC_MODEL_ID};

fn cos(a: &[f32], b: &[f32]) -> f32 {
    let num: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let da: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let db: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    num / (da * db)
}

#[test]
fn wordpiece_matches_reference_tokenizations() {
    let e = StaticEmbedder::new();
    // Single-vocab words, case folding, punctuation isolation.
    assert_eq!(e.token_ids("function"), vec![2856]);
    assert_eq!(e.token_ids("Function"), vec![2856]);
    assert_eq!(e.token_ids("fn main() {"), vec![29611, 1367, 9, 10, 66]);
    assert_eq!(e.token_ids("Hello, world!"), vec![6595, 13, 1091, 2]);
    // Greedy longest-match decomposition into ## continuations.
    assert_eq!(e.token_ids("unaffable"), vec![13480, 19964, 2471]);
    // Apostrophe is punctuation; code-trained vocab keeps camelCase whole
    // after lowercasing.
    assert_eq!(e.token_ids("don't"), vec![1126, 8, 59]);
    assert_eq!(e.token_ids("getUserName"), vec![31247]);
    // Accent stripping, precomposed and decomposed forms agree.
    assert_eq!(e.token_ids("café"), vec![6671]);
    assert_eq!(e.token_ids("cafe\u{301}"), vec![6671]);
    // Whitespace runs collapse; empty and unknown-only text pool nothing.
    assert_eq!(e.token_ids("  multiple   spaces  "), vec![2677, 6261]);
    assert!(e.token_ids("").is_empty());
    assert!(e.token_ids(&"a".repeat(101)).is_empty());
    assert!(e.token_ids("🦀").is_empty());
}

#[test]
fn embed_is_deterministic_unit_norm_and_semantic() {
    let e = StaticEmbedder::new();
    let read = e.embed("read a file from disk");
    assert_eq!(read.len(), STATIC_DIM);
    assert_eq!(read, e.embed("read a file from disk"));
    let norm: f32 = read.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");

    let load = e.embed("load file contents into memory");
    let sort = e.embed("sort the array in ascending order");
    assert!(
        cos(&read, &load) > 0.35,
        "near paraphrase must be close: {}",
        cos(&read, &load)
    );
    assert!(
        cos(&read, &sort) < 0.15,
        "unrelated text must be far: {}",
        cos(&read, &sort)
    );

    // Batch path must produce exactly the single-text vectors.
    let batch = e.embed_batch(&["read a file from disk", "sort the array in ascending order"]);
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0], read);
    assert_eq!(batch[1], sort);
}

#[test]
fn unknown_only_text_embeds_to_zero_vector() {
    let e = StaticEmbedder::new();
    let v = e.embed("🦀");
    assert_eq!(v.len(), STATIC_DIM);
    assert!(v.iter().all(|x| *x == 0.0));
}

#[test]
fn static_identity_is_pinned() {
    assert_eq!(STATIC_DIM, 256);
    assert_eq!(STATIC_MODEL_ID, "static:potion-code-16M-v2:256");
    // The free-function facade is the same engine.
    assert_eq!(
        embed_text("refresh token"),
        StaticEmbedder::new().embed("refresh token")
    );
}

#[test]
fn per_text_embed_cost_stays_recompute_friendly() {
    // Contract: per-text embed cost is a first-class engine cost input.
    // Measured 2026-10-25: ~1.9 ms/text (wordpiece binary-search lookups +
    // scalar f16 decode + scalar accumulate), which is what DISQUALIFIED
    // storing field texts instead of vectors (S2): on-demand recompute at
    // rescore-pool scale would cost seconds. This tripwire guards against
    // further regression (bar = 1.5x measured) and prints the number for
    // the queued hot-path lever (f16 SIMD accumulate, vocab hash lookup).
    // Bar is 4x the quiet-machine cost: loaded machines inflate it ~1.5x.
    let e = StaticEmbedder::new();
    let sample = "symbol: embed_parallel kind: fn scope: store excerpt: pub(super) fn \
embed_parallel(conn: &Connection, chunks: &[SemanticChunkInput], source_path: &str) \
-> Result<Vec<EmbeddedChunk>> { let mid = active_model_id(); let mut jobs = Vec::new(); }";
    let texts: Vec<String> = (0..64)
        .map(|i| format!("{sample} variant {i} tail tokens to vary the hash input"))
        .collect();
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    // Warm the weight table into cache, then time two full batches.
    let _ = e.embed_batch(&refs);
    let t0 = std::time::Instant::now();
    let n = 4;
    for _ in 0..n {
        let _ = e.embed_batch(&refs);
    }
    let per_text_us = t0.elapsed().as_micros() as f64 / (n as f64 * 64.0);
    eprintln!("per-text embed cost: {per_text_us:.1} us");
    assert!(
        per_text_us < 8000.0,
        "per-text embed cost {per_text_us:.1}us regressed past the 8ms tripwire"
    );
}

/// The two pooling sources must agree bit-for-bit: `embed` decodes f16 rows
/// on demand (no table materialization), `embed_batch` pools from the
/// materialized f32 table. f16→f32 widening is exact and the summation
/// order is shared, so any divergence is a decode or pooling bug on one of
/// the paths. Edge texts cover empty input, unknown-only text (zero
/// vector), subword-heavy identifiers, accents, CJK, and a long body.
/// KILLS: a change to either path's decode or accumulation order.
#[test]
fn lazy_row_embed_matches_materialized_table_bit_for_bit() {
    let e = ast_sgrep_embed::StaticEmbedder::new();
    let cases = [
        "",
        "zzzzqqqqxxxx",
        "semantic_ivf search",
        "hybrid_lexical_prefilter::cascade_prefers_file()",
        "café naïve résumé",
        "配置文件与中间件",
        "The function is designed to select the appropriate middleware stack \
         for a Web3.js provider, combining multiple layers of interceptors.",
    ];
    for text in cases {
        let lazy = e.embed(text);
        let materialized = e.embed_batch(&[text]).pop().expect("batch result");
        assert_eq!(
            lazy, materialized,
            "lazy row-decode diverged from the materialized table for {text:?}"
        );
    }
}
