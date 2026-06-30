//! In-memory IVF-ANN index for large semantic chunk corpora (10k+ symbols).
//!
//! Below the threshold, brute-force cosine over all chunks is fast enough.
//! Above it, vectors are clustered (k-means) and search probes the nearest
//! centroids before scoring candidates — sub-linear at scale.

use std::sync::Mutex;

use ast_sgrep_embed::SemanticChunkRow;

/// Default chunk count above which IVF-ANN is used instead of brute force.
pub const DEFAULT_ANN_THRESHOLD: usize = 2_000;

/// Number of centroid clusters to probe per query.
const DEFAULT_NPROBE: usize = 8;

/// Minimum similarity to return (matches embed crate filter).
const MIN_SIMILARITY: f32 = 0.08;

/// IVF index over normalized semantic chunk vectors.
#[derive(Debug, Clone)]
pub struct SemanticAnnIndex {
    centroids: Vec<Vec<f32>>,
    /// For each vector, which centroid it belongs to.
    #[allow(dead_code)]
    assignments: Vec<usize>,
    /// Vectors grouped by cluster for fast probing.
    clusters: Vec<Vec<usize>>,
}

impl SemanticAnnIndex {
    /// Build an IVF index from chunk rows (vectors are L2-normalized in place).
    pub fn build(chunks: &[SemanticChunkRow]) -> Self {
        let n = chunks.len();
        let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
        if n == 0 || dim == 0 {
            return Self {
                centroids: Vec::new(),
                assignments: Vec::new(),
                clusters: Vec::new(),
            };
        }

        let k = cluster_count(n);
        let vectors: Vec<Vec<f32>> = chunks.iter().map(|c| normalize(c.5.clone())).collect();
        let (centroids, assignments) = kmeans(&vectors, k, 12);
        let mut clusters = vec![Vec::new(); centroids.len()];
        for (idx, &cluster) in assignments.iter().enumerate() {
            clusters[cluster].push(idx);
        }

        Self {
            centroids,
            assignments,
            clusters,
        }
    }

    /// Return indices into the original chunk slice, sorted by descending similarity.
    pub fn search(&self, query: &[f32], vectors: &[Vec<f32>], limit: usize) -> Vec<(usize, f32)> {
        if vectors.is_empty() {
            return Vec::new();
        }
        if self.centroids.is_empty() {
            return brute_force(query, vectors, limit);
        }

        let q = normalize(query.to_vec());
        let nprobe = DEFAULT_NPROBE.max(self.centroids.len() / 4).min(self.centroids.len());
        let mut centroid_scores: Vec<(usize, f32)> = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, cosine(&q, c)))
            .collect();
        centroid_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut candidates = Vec::new();
        for (cluster_id, _) in centroid_scores.into_iter().take(nprobe) {
            for &idx in &self.clusters[cluster_id] {
                let sim = cosine(&q, &vectors[idx]);
                if sim > MIN_SIMILARITY {
                    candidates.push((idx, sim));
                }
            }
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(limit);
        candidates
    }
}

fn cluster_count(n: usize) -> usize {
    let sqrt = (n as f64).sqrt() as usize;
    sqrt.clamp(16, 256)
}

fn normalize(mut vec: Vec<f32>) -> Vec<f32> {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
    vec
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }
    a.iter()
        .zip(b.iter())
        .take(len)
        .map(|(x, y)| x * y)
        .sum()
}

fn brute_force(query: &[f32], vectors: &[Vec<f32>], limit: usize) -> Vec<(usize, f32)> {
    let q = normalize(query.to_vec());
    let mut scored: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i, cosine(&q, v)))
        .filter(|(_, sim)| *sim > MIN_SIMILARITY)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// Simplified k-means (cosine distance via normalized vectors).
fn kmeans(vectors: &[Vec<f32>], k: usize, max_iters: usize) -> (Vec<Vec<f32>>, Vec<usize>) {
    let n = vectors.len();
    let k = k.min(n).max(1);
    let dim = vectors[0].len();

    // k-means++ style seeding: spread initial centroids
    let mut centroids: Vec<Vec<f32>> = Vec::with_capacity(k);
    centroids.push(vectors[0].clone());
    while centroids.len() < k {
        let mut best_idx = 0usize;
        let mut best_dist = -1.0f32;
        for (i, v) in vectors.iter().enumerate() {
            let min_sim = centroids
                .iter()
                .map(|c| cosine(v, c))
                .fold(f32::INFINITY, f32::min);
            let dist = 1.0 - min_sim;
            if dist > best_dist {
                best_dist = dist;
                best_idx = i;
            }
        }
        centroids.push(vectors[best_idx].clone());
    }

    let mut assignments = vec![0usize; n];
    for _ in 0..max_iters {
        let mut changed = false;
        for (i, v) in vectors.iter().enumerate() {
            let (best, _) = centroids
                .iter()
                .enumerate()
                .map(|(ci, c)| (ci, cosine(v, c)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or((0, 0.0));
            if assignments[i] != best {
                assignments[i] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }

        let mut sums = vec![vec![0.0f32; dim]; k];
        let mut counts = vec![0usize; k];
        for (i, v) in vectors.iter().enumerate() {
            let c = assignments[i];
            counts[c] += 1;
            for (j, val) in v.iter().enumerate() {
                sums[c][j] += val;
            }
        }
        for ci in 0..k {
            if counts[ci] > 0 {
                centroids[ci] = normalize(
                    sums[ci]
                        .iter()
                        .map(|s| s / counts[ci] as f32)
                        .collect(),
                );
            }
        }
    }

    (centroids, assignments)
}

/// Chunk count at which ANN kicks in (`ASGREP_ANN_THRESHOLD`, default 2000).
pub fn ann_threshold() -> usize {
    std::env::var("ASGREP_ANN_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ANN_THRESHOLD)
}

pub fn should_use_ann(chunk_count: usize) -> bool {
    chunk_count >= ann_threshold()
}

struct CachedAnn {
    chunk_count: usize,
    index: SemanticAnnIndex,
}

static ANN_CACHE: Mutex<Option<(String, CachedAnn)>> = Mutex::new(None);

/// Get or build a cached ANN index for this database path and chunk count.
pub fn cached_ann_index(
    db_key: &str,
    chunk_count: usize,
    chunks: &[SemanticChunkRow],
) -> SemanticAnnIndex {
    if !should_use_ann(chunk_count) {
        return SemanticAnnIndex::build(&[]);
    }

    let mut guard = ANN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((key, cached)) = guard.as_ref() {
        if key == db_key && cached.chunk_count == chunk_count {
            return cached.index.clone();
        }
    }
    let index = SemanticAnnIndex::build(chunks);
    *guard = Some((
        db_key.to_string(),
        CachedAnn {
            chunk_count,
            index: index.clone(),
        },
    ));
    index
}

/// Rank chunks using ANN when over threshold, else return None (caller uses brute force).
pub fn ann_rank_indices(
    db_key: &str,
    query_vec: &[f32],
    chunks: &[SemanticChunkRow],
    limit: usize,
) -> Option<Vec<(usize, f32)>> {
    if !should_use_ann(chunks.len()) {
        return None;
    }
    let vectors: Vec<Vec<f32>> = chunks.iter().map(|c| c.5.clone()).collect();
    let index = cached_ann_index(db_key, chunks.len(), chunks);
    Some(index.search(query_vec, &vectors, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunks(n: usize) -> Vec<SemanticChunkRow> {
        let dim = 256;
        (0..n)
            .map(|i| {
                let mut v = vec![0.0f32; dim];
                v[i % dim] = 1.0;
                v[(i * 13 + 7) % dim] = 0.5;
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                for x in v.iter_mut() {
                    *x /= norm;
                }
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
        let index = SemanticAnnIndex::build(&chunks);
        let query = chunks[42].5.clone();
        let vectors: Vec<Vec<f32>> = chunks.iter().map(|c| c.5.clone()).collect();
        let results = index.search(&query, &vectors, 10);
        assert!(!results.is_empty());
        assert!(
            results.iter().any(|(i, sim)| *i == 42 && *sim > 0.99),
            "ANN should recall the exact match within top results: {:?}",
            results
        );
    }

    #[test]
    fn brute_force_below_threshold() {
        assert!(!should_use_ann(100));
        assert!(should_use_ann(2000));
    }

    #[test]
    fn cache_reuses_index() {
        let chunks = make_chunks(100);
        let idx1 = cached_ann_index("test.db", 100, &chunks);
        let idx2 = cached_ann_index("test.db", 100, &chunks);
        assert_eq!(idx1.centroids.len(), idx2.centroids.len());
    }
}
