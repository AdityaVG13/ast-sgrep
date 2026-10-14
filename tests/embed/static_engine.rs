//! Intent tests for the native static embedding engine.
//!
//! Expected token ids were produced by an independent Python reference
//! implementation of BERT WordPiece over the pinned
//! `minishlab/potion-code-16M-v2` tokenizer.json (revision
//! e9d2a44ca6a05ac6685f3b23709ea57eb7352d5b) -- they are not derived from the
//! code under test. Semantic thresholds come from the same reference over
//! the pinned f16 table: cos(read,load)=0.514, cos(read,sort)=0.008.

use ast_sgrep_embed::{
    configured_backend_model_id, embedder_for, EmbedBackendKind, Embedder, StaticEmbedder,
    STATIC_DIM,
};

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
    let read = e.embed("read a file from disk").unwrap();
    assert_eq!(read.len(), STATIC_DIM);
    assert_eq!(read, e.embed("read a file from disk").unwrap());
    let norm: f32 = read.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");

    let load = e.embed("load file contents into memory").unwrap();
    let sort = e.embed("sort the array in ascending order").unwrap();
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
    let batch = e
        .embed_batch(&["read a file from disk", "sort the array in ascending order"])
        .unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0], read);
    assert_eq!(batch[1], sort);
}

#[test]
fn unknown_only_text_embeds_to_zero_vector() {
    let e = StaticEmbedder::new();
    let v = e.embed("🦀").unwrap();
    assert_eq!(v.len(), STATIC_DIM);
    assert!(v.iter().all(|x| *x == 0.0));
}

#[test]
fn static_backend_is_wired() {
    assert_eq!(EmbedBackendKind::parse("static"), Some(EmbedBackendKind::Static));
    assert_eq!(EmbedBackendKind::Static.as_meta_str(), "static");
    let e = embedder_for(EmbedBackendKind::Static).expect("static embedder is always available");
    assert_eq!(e.dim(), STATIC_DIM);
    assert!(
        e.model_id().contains("potion-code-16M-v2"),
        "model_id={}",
        e.model_id()
    );
    assert_eq!(
        configured_backend_model_id(EmbedBackendKind::Static, STATIC_DIM).as_deref(),
        Some("static:potion-code-16M-v2:256")
    );
}
