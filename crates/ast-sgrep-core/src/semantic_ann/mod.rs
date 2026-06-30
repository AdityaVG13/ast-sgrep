//! In-memory IVF-ANN index for large semantic chunk corpora.
//!
//! Below the threshold, brute-force cosine over flat vectors is used.
//! Above it, vectors are clustered (k-means) and search probes nearest
//! centroids. Structure is persisted to `.asgrep/semantic.ivf`.

mod ivf;
mod kmeans;
mod session;
mod vector;

pub use ivf::SemanticAnnIndex;
pub use session::{
    ann_threshold, cached_semantic_ivf, load_or_build_semantic_ivf, rank_chunk_indices,
    rebuild_semantic_ivf_sidecar, should_use_ann, DEFAULT_ANN_THRESHOLD,
};

#[cfg(test)]
mod tests {
    use ast_sgrep_embed::SemanticChunkRow;

    use super::ivf::SemanticAnnIndex;
    use super::session::should_use_ann;
    use super::vector::{flatten_vectors, normalize_vec_in_place};

    fn make_chunks(n: usize) -> Vec<SemanticChunkRow> {
        let dim = 256;
        (0..n)
            .map(|i| {
                let mut v = vec![0.0f32; dim];
                v[i % dim] = 1.0;
                v[(i * 13 + 7) % dim] = 0.5;
                normalize_vec_in_place(&mut v);
                (
                    format!("f{i}.rs"),
                    i as u32,
                    i as u32,
                    format!("sym{i}"),
                    format!("excerpt {i}"),
                    v,
                )
            })
            .collect()
    }

    #[test]
    fn ann_finds_high_similarity_neighbor() {
        let chunks = make_chunks(100);
        let dim = 256;
        let flat = flatten_vectors(&chunks, dim);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let query = index.search_flat(&flat, dim, &chunks[42].5, 10);
        assert!(query.iter().any(|(i, sim)| *i == 42 && *sim > 0.99));
    }

    #[test]
    fn brute_force_below_threshold() {
        assert!(!should_use_ann(100, None));
        assert!(should_use_ann(2000, None));
    }
}
