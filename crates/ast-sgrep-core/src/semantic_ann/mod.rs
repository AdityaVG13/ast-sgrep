//! Semantic approximate-nearest-neighbor search (IVF + brute force).

mod index;
mod session;
mod vector;

pub use index::SemanticAnnIndex;
pub use session::{
    ann_threshold, cached_semantic_ivf, load_or_build_semantic_ivf, rank_chunk_indices_flat,
    rebuild_semantic_ivf_sidecar, should_use_ann, DEFAULT_ANN_THRESHOLD,
};
pub use vector::flatten_vectors_for_search;
