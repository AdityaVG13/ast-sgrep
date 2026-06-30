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

/// Score indexed rows by cosine similarity to a query vector.
pub fn cosine_scores_for<'a>(
    query_vec: &[f32],
    rows: impl Iterator<Item = (usize, &'a [f32])>,
) -> Vec<(usize, f32)> {
    rows.filter(|(_, emb)| emb.len() == query_vec.len())
        .map(|(idx, emb)| (idx, cosine_similarity(query_vec, emb)))
        .collect()
}

/// Sort by similarity descending, take `limit`, optionally filter by minimum score.
pub fn top_by_similarity(
    mut scored: Vec<(usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .filter(|(_, sim)| min_similarity.is_none_or(|min| *sim > min))
        .collect()
}
