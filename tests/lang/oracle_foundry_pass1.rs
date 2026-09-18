//! Pass 1 (oracle-foundry, Mission 2): independent L1 oracles for embed
//! math/byte/tokenizer contracts and lang pattern-gate boundaries.
//!
//! Expectations are hand-computed (dot products, splits, sorts), and error
//! cases assert discriminants (`is_err`, empty-vs-`None`), never messages.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, embed_from_bytes, embed_to_bytes, normalize_vec,
    split_ident, tokenize, top_by_similarity, MIN_SIMILARITY,
};
use ast_sgrep_lang::{match_pattern, needs_ast_grep_fallback, Language};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn dot_similarity_matches_hand_products() {
    assert_eq!(dot_similarity(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]), 32.0);
    assert_eq!(dot_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    // Degenerate inputs collapse to zero, not NaN or panic.
    assert_eq!(dot_similarity(&[], &[]), 0.0);
    assert_eq!(dot_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::INFINITY], &[1.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::NAN], &[1.0]), 0.0);
}

#[test]
fn cosine_similarity_matches_hand_angles() {
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    assert!(approx(
        cosine_similarity(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]),
        1.0
    ));
    assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
    assert_eq!(cosine_similarity(&[1.0], &[1.0, 1.0]), 0.0);
}

#[test]
fn normalize_vec_matches_hand_norms() {
    let n = normalize_vec(&[3.0, 4.0]);
    assert!(approx(n[0], 0.6) && approx(n[1], 0.8), "got {n:?}");
    assert_eq!(normalize_vec(&[0.0, 0.0]), vec![0.0, 0.0]);
    // Non-finite components are zeroed before normalization.
    assert_eq!(normalize_vec(&[f32::NAN, 1.0]), vec![0.0, 1.0]);
    assert_eq!(normalize_vec(&[]), Vec::<f32>::new());
}

#[test]
fn top_by_similarity_orders_truncates_and_drops_nan() {
    let scored = vec![(2, 0.5), (0, 0.9), (1, 0.9), (3, f32::NAN)];
    // Descending score, ties broken by ascending index, NaN dropped.
    assert_eq!(
        top_by_similarity(scored, 10, None),
        vec![(0, 0.9), (1, 0.9), (2, 0.5)]
    );
    assert_eq!(
        top_by_similarity(vec![(0, 0.9), (1, 0.8)], 1, None),
        vec![(0, 0.9)]
    );
    assert!(top_by_similarity(vec![(0, 0.9)], 0, None).is_empty());
    // The threshold is exclusive: exactly MIN_SIMILARITY does not pass.
    assert!(top_by_similarity(vec![(0, MIN_SIMILARITY)], 10, Some(MIN_SIMILARITY)).is_empty());
    assert_eq!(
        top_by_similarity(vec![(0, MIN_SIMILARITY + 0.01)], 10, Some(MIN_SIMILARITY)).len(),
        1
    );
}

#[test]
fn embed_bytes_roundtrip_is_bit_exact() {
    let vec = vec![0.0, 1.0, -2.5, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
    let bytes = embed_to_bytes(&vec);
    assert_eq!(bytes.len(), 4 * vec.len());
    let back = embed_from_bytes(&bytes).expect("roundtrip");
    assert_eq!(back.len(), vec.len());
    for (a, b) in vec.iter().zip(back.iter()) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
    assert_eq!(embed_from_bytes(&[]).expect("empty ok"), Vec::<f32>::new());
    // Failure discriminant: ragged byte lengths are rejected.
    assert!(embed_from_bytes(&[0u8; 5]).is_err());
    assert!(embed_from_bytes(&[0u8; 3]).is_err());
}

#[test]
fn split_ident_matches_hand_splits() {
    assert_eq!(split_ident("fooBar"), vec!["foo", "bar"]);
    assert_eq!(split_ident("foo_bar"), vec!["foo", "bar"]);
    assert_eq!(split_ident("fooBAR"), vec!["foo", "bar"]);
    assert_eq!(split_ident("a-b"), vec!["a", "b"]);
    assert_eq!(split_ident("ABC"), vec!["abc"]);
    // No splittable parts: falls back to the whole ident, lowercased.
    assert_eq!(split_ident(""), vec![""]);
    assert_eq!(split_ident("__"), vec!["__"]);
}

#[test]
fn tokenize_matches_hand_sets() {
    assert_eq!(tokenize("FooBar"), vec!["bar", "foo", "foobar"]);
    assert_eq!(tokenize("a bc"), vec!["bc"]);
    assert!(tokenize("").is_empty());
    assert!(tokenize("a b c").is_empty());
}

#[test]
fn pattern_gates_treat_empty_and_literal_as_native() {
    assert!(!needs_ast_grep_fallback(""));
    assert!(!needs_ast_grep_fallback("process_request"));
    // Empty pattern is absence (Ok + no hits), not a failure.
    let hits = match_pattern(Language::Rust, "fn foo() {}\n", "").expect("empty ok");
    assert!(hits.is_empty());
}
