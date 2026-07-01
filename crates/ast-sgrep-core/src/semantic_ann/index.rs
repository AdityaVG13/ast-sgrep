//! IVF index build, persistence helpers, and approximate search.

use std::io::{Read, Write};

use rayon::prelude::*;

use ast_sgrep_embed::{cosine_similarity, top_k_similarity, MIN_SIMILARITY, PARALLEL_CHUNK_THRESHOLD};

use super::vector::{brute_force_flat, normalize_flat, normalize_vec, rows_from_flat};

const DEFAULT_NPROBE: usize = 8;

#[derive(Debug, Clone)]
pub struct SemanticAnnIndex {
    centroids: Vec<Vec<f32>>,
    clusters: Vec<Vec<usize>>,
}

impl SemanticAnnIndex {
    pub fn build_from_flat(vectors: &[f32], dim: usize) -> Self {
        let n = if dim == 0 { 0 } else { vectors.len() / dim };
        if n == 0 || dim == 0 {
            return Self::empty();
        }
        let normalized = normalize_flat(vectors, dim);
        let k = cluster_count(n);
        let row_vecs = rows_from_flat(&normalized, dim);
        let (centroids, assignments) = kmeans(&row_vecs, k, 12);
        let mut clusters = vec![Vec::new(); centroids.len()];
        for (idx, &cluster) in assignments.iter().enumerate() {
            clusters[cluster].push(idx);
        }
        Self { centroids, clusters }
    }

    fn empty() -> Self {
        Self {
            centroids: Vec::new(),
            clusters: Vec::new(),
        }
    }

    pub fn write_to<W: Write>(&self, writer: &mut W, dim: usize) -> std::io::Result<()> {
        let k = self.centroids.len() as u32;
        writer.write_all(&k.to_le_bytes())?;
        for c in &self.centroids {
            for &v in c {
                writer.write_all(&v.to_le_bytes())?;
            }
            if c.len() < dim {
                for _ in c.len()..dim {
                    writer.write_all(&0.0f32.to_le_bytes())?;
                }
            }
        }
        let cluster_count = self.clusters.len() as u32;
        writer.write_all(&cluster_count.to_le_bytes())?;
        for cluster in &self.clusters {
            let len = cluster.len() as u32;
            writer.write_all(&len.to_le_bytes())?;
            for &idx in cluster {
                writer.write_all(&(idx as u32).to_le_bytes())?;
            }
        }
        Ok(())
    }

    pub fn read_clusters_from<R: Read>(reader: &mut R, k: usize, dim: usize) -> std::io::Result<Self> {
        let mut centroids = Vec::with_capacity(k);
        for _ in 0..k {
            let mut c = vec![0.0f32; dim];
            for v in &mut c {
                let mut buf = [0u8; 4];
                reader.read_exact(&mut buf)?;
                *v = f32::from_le_bytes(buf);
            }
            centroids.push(c);
        }
        let mut cc_buf = [0u8; 4];
        reader.read_exact(&mut cc_buf)?;
        let cluster_count = u32::from_le_bytes(cc_buf) as usize;
        let mut clusters = Vec::with_capacity(cluster_count);
        for _ in 0..cluster_count {
            let mut len_buf = [0u8; 4];
            reader.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut members = Vec::with_capacity(len);
            for _ in 0..len {
                let mut idx_buf = [0u8; 4];
                reader.read_exact(&mut idx_buf)?;
                members.push(u32::from_le_bytes(idx_buf) as usize);
            }
            clusters.push(members);
        }
        Ok(Self { centroids, clusters })
    }

    pub fn validate_member_indices(&self, chunk_count: usize) -> bool {
        self.clusters
            .iter()
            .all(|cluster| cluster.iter().all(|&idx| idx < chunk_count))
    }

    pub fn search_flat(
        &self,
        flat: &[f32],
        dim: usize,
        query: &[f32],
        limit: usize,
    ) -> Vec<(usize, f32)> {
        let n = if dim == 0 { 0 } else { flat.len() / dim };
        if n == 0 {
            return Vec::new();
        }
        if self.centroids.is_empty() {
            return brute_force_flat(flat, dim, query, limit);
        }
        let q = normalize_vec(query);
        let nprobe = DEFAULT_NPROBE
            .max(self.centroids.len() / 4)
            .min(self.centroids.len());
        let mut centroid_scores: Vec<(usize, f32)> = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, cosine_similarity(&q, c)))
            .collect();
        centroid_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let members = probed_members(&centroid_scores, nprobe, &self.clusters);
        score_members(&q, flat, dim, n, &members, limit)
    }
}

fn probed_members(
    centroid_scores: &[(usize, f32)],
    nprobe: usize,
    clusters: &[Vec<usize>],
) -> Vec<usize> {
    let mut members = Vec::new();
    for (cluster_id, _) in centroid_scores.iter().take(nprobe) {
        if *cluster_id < clusters.len() {
            members.extend_from_slice(&clusters[*cluster_id]);
        }
    }
    members
}

fn score_members(
    query: &[f32],
    flat: &[f32],
    dim: usize,
    n: usize,
    members: &[usize],
    limit: usize,
) -> Vec<(usize, f32)> {
    if members.is_empty() {
        return Vec::new();
    }
    let score = |idx: &usize| -> Option<(usize, f32)> {
        if *idx >= n {
            return None;
        }
        let start = idx * dim;
        if start + dim > flat.len() {
            return None;
        }
        let sim = cosine_similarity(query, &flat[start..start + dim]);
        (sim > MIN_SIMILARITY).then_some((*idx, sim))
    };

    if members.len() < PARALLEL_CHUNK_THRESHOLD {
        return top_k_similarity(members.iter().filter_map(score), limit, None);
    }

    let scored: Vec<(usize, f32)> = if members.len() >= PARALLEL_CHUNK_THRESHOLD {
        members.par_iter().filter_map(score).collect()
    } else {
        members.iter().filter_map(score).collect()
    };
    top_k_similarity(scored, limit, None)
}

fn cluster_count(n: usize) -> usize {
    let sqrt = (n as f64).sqrt() as usize;
    sqrt.clamp(16, 256)
}

fn kmeans(vectors: &[Vec<f32>], k: usize, max_iters: usize) -> (Vec<Vec<f32>>, Vec<usize>) {
    let n = vectors.len();
    let k = k.min(n).max(1);
    let dim = vectors[0].len();

    let mut centroids: Vec<Vec<f32>> = Vec::with_capacity(k);
    centroids.push(vectors[0].clone());
    while centroids.len() < k {
        let mut best_idx = 0usize;
        let mut best_dist = -1.0f32;
        for (i, v) in vectors.iter().enumerate() {
            let min_sim = centroids
                .iter()
                .map(|c| cosine_similarity(v, c))
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
                .map(|(ci, c)| (ci, cosine_similarity(v, c)))
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
                centroids[ci] = normalize_vec(
                    &sums[ci]
                        .iter()
                        .map(|s| s / counts[ci] as f32)
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
    (centroids, assignments)
}
