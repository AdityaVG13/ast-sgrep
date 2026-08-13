//! Durable regression-style checks for the pure APIs that cargo-fuzz targets
//! exercise. These drive the **shipped** functions (not harness re-implementations).

use ast_sgrep_core::rank::{fuse_rrf, score_symbol, SCORE_EXACT_SYMBOL};
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::{ParsedQuery, QueryMode};
use ast_sgrep_embed::{embed_from_bytes, embed_to_bytes};

/// Mirrors the structural oracle in `fuzz/fuzz_targets/query_grammar.rs`.
fn assert_query_structure(input: &str) {
    let parsed = ParsedQuery::parse(input);
    assert_eq!(parsed.raw, input.trim());
    match parsed.mode {
        QueryMode::Callers
        | QueryMode::Defs
        | QueryMode::Imports
        | QueryMode::Pattern
        | QueryMode::Literal
        | QueryMode::Regex
        | QueryMode::Word => {
            assert!(parsed.target.is_some());
        }
        QueryMode::Hybrid => assert!(parsed.target.is_none()),
    }
    let again = ParsedQuery::parse(&parsed.raw);
    assert_eq!(again.mode, parsed.mode);
    assert_eq!(again.target, parsed.target);
    assert_eq!(again.raw, parsed.raw);
}

#[test]
fn query_grammar_oracle_on_seed_like_inputs() {
    for q in [
        "",
        "process_request",
        "callers:Map",
        "defs:User_Id",
        "imports:std::io",
        "pattern:fn $NAME() {}",
        "literal:FooBar",
        "regex:Foo.*Bar",
        "word:Hello",
        "  callers: spaced  ",
    ] {
        assert_query_structure(q);
    }
}

#[test]
fn rank_oracle_finite_and_reverse_rrf() {
    let s = score_symbol("exact", "exact");
    assert!((s - SCORE_EXACT_SYMBOL).abs() < f64::EPSILON);
    let ranks = vec![0usize, 3, 10];
    let fused = fuse_rrf(&ranks, 60.0);
    let mut rev = ranks.clone();
    rev.reverse();
    let reversed = fuse_rrf(&rev, 60.0);
    assert!((fused - reversed).abs() <= f64::EPSILON * ranks.len() as f64);
}

#[test]
fn embed_roundtrip_oracle() {
    let v = vec![1.0f32, -0.5, 0.0, 42.0];
    let bytes = embed_to_bytes(&v);
    let decoded = embed_from_bytes(&bytes).expect("decode");
    assert_eq!(decoded.len(), v.len());
    for (a, b) in decoded.iter().zip(v.iter()) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
    assert!(embed_from_bytes(&[0u8, 1, 2]).is_err());
}

#[test]
fn ann_clusters_write_read_roundtrip() {
    let dim = 4;
    let flat: Vec<f32> = (0..16).map(|i| (i as f32) * 0.1).collect();
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let mut buf = Vec::new();
    index.write_to(&mut buf, dim).expect("serialize");
    let k = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    let n = flat.len() / dim;
    let rt = SemanticAnnIndex::read_clusters_bounded(&buf, k, dim, n);
    assert!(rt.is_ok(), "RT failed: {rt:?}");
}

#[test]
fn ann_clusters_rejects_truncated_garbage() {
    let garbage = [0u8, 0, 0, 1, 0xff, 0xff];
    let err = SemanticAnnIndex::read_clusters_bounded(&garbage, 1, 4, 2);
    assert!(err.is_err());
}
