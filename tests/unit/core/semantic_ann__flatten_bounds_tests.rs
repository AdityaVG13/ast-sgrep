use super::flatten_vectors_for_search;
use ast_sgrep_embed::SemanticChunkRow;

#[test]
fn flatten_rejects_zero_dim_with_chunks() {
    let chunks: Vec<SemanticChunkRow> =
        vec![("a.rs".into(), 1u32, 1u32, "sym".into(), "x".into(), vec![])];
    let err = flatten_vectors_for_search(&chunks, 0).expect_err("dim=0 must fail");
    assert!(
        err.to_string().contains("dimension is 0"),
        "unexpected: {err}"
    );
}

#[test]
fn flatten_allows_empty_chunks_with_zero_dim() {
    let out = flatten_vectors_for_search(&[], 0).expect("empty ok");
    assert!(out.is_empty());
}

#[test]
fn flatten_rejects_len_times_dim_overflow() {
    // Overflow is checked before row-length validation / allocation, so empty
    // vectors are enough to exercise the edge without multi-GB allocs.
    let dim = usize::MAX / 2 + 1;
    let chunks: Vec<SemanticChunkRow> = vec![
        ("a.rs".into(), 1u32, 1u32, "s".into(), "x".into(), vec![]),
        ("b.rs".into(), 1u32, 1u32, "s".into(), "x".into(), vec![]),
    ];
    let err = flatten_vectors_for_search(&chunks, dim).expect_err("overflow");
    assert!(err.to_string().contains("overflow"), "unexpected: {err}");
}
