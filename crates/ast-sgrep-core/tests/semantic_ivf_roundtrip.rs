use std::collections::HashSet;

use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{
    compute_ann_fingerprint, load_semantic_ivf, load_semantic_ivf_unchecked, save_semantic_ivf,
};
use ast_sgrep_embed::{top_k_flat_similarity, MIN_SIMILARITY};

#[test]
fn semantic_ivf_roundtrip_and_fingerprint_gate() {
    let dim = 4usize;
    let vectors: Vec<f32> = (0..24).map(|i| i as f32 * 0.1).collect();
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let fingerprint = compute_ann_fingerprint(6, 6, dim, Some("test"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("semantic.ivf");

    save_semantic_ivf(&path, fingerprint, dim, &vectors, &index).unwrap();

    let loaded = load_semantic_ivf(&path, fingerprint)
        .unwrap()
        .expect("valid sidecar");
    assert_eq!(loaded.dim, dim);
    assert_eq!(loaded.vectors, vectors);
    assert_eq!(loaded.fingerprint, fingerprint);

    let wrong_fp = compute_ann_fingerprint(6, 5, dim, Some("test"));
    assert!(load_semantic_ivf(&path, wrong_fp).unwrap().is_none());

    let unchecked = load_semantic_ivf_unchecked(&path)
        .unwrap()
        .expect("unchecked load");
    assert_eq!(unchecked.vectors, vectors);

    let query = vec![0.1f32; dim];
    let before = index.search_flat(&vectors, dim, &query, 3);
    let after = loaded.index.search_flat(&loaded.vectors, dim, &query, 3);
    assert_eq!(before, after);
}

/// Deterministic LCG unit vectors for IVF regression (CE-003).
fn normalized_flat_vectors(count: usize, dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    let mut flat = Vec::with_capacity(count * dim);
    for _ in 0..count {
        let start = flat.len();
        for _ in 0..dim {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let unit = ((state >> 32) as u32) as f32 / u32::MAX as f32;
            flat.push(unit * 2.0 - 1.0);
        }
        normalize_row_in_place(&mut flat[start..start + dim]);
    }
    flat
}

fn normalize_row_in_place(row: &mut [f32]) {
    let norm: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in row.iter_mut() {
            *x /= norm;
        }
    }
}

fn normalize_query(query: &[f32]) -> Vec<f32> {
    let mut out = query.to_vec();
    normalize_row_in_place(&mut out);
    out
}

fn brute_force_top_k_indices(
    flat: &[f32],
    dim: usize,
    query: &[f32],
    limit: usize,
) -> HashSet<usize> {
    let q = normalize_query(query);
    top_k_flat_similarity(&q, flat, dim, limit, Some(MIN_SIMILARITY))
        .into_iter()
        .map(|(idx, _)| idx)
        .collect()
}

/// CE-003: IVF search with all-cluster probing must return the same top-k indices as brute force.
#[test]
fn ivf_search_matches_brute_force_top_k_indices_ce003() {
    let dim = 32usize;
    let vector_count = 512usize;
    let limit = 24usize;
    let flat = normalized_flat_vectors(vector_count, dim, 0xCE_003_u64);
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    assert!(index.validate_member_indices(vector_count));

    let query_indices = [0usize, 17, 137, 299, 400, 511];
    for &qi in &query_indices {
        let query = &flat[qi * dim..(qi + 1) * dim];
        let brute = brute_force_top_k_indices(&flat, dim, query, limit);
        let ivf: HashSet<usize> = index
            .search_flat(&flat, dim, query, limit)
            .into_iter()
            .map(|(idx, _)| idx)
            .collect();
        assert_eq!(
            ivf, brute,
            "IVF top-k index set must match brute-force top_k_flat_similarity (query chunk {qi})"
        );
    }
}
