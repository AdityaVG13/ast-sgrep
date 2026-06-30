//! Shared vector math for embedding backends.

/// Dot product of two equal-length unit-ish embedding vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Minimum similarity for semantic chunk hits.
pub const MIN_SIMILARITY: f32 = 0.08;
