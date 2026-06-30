use std::io::{Read, Write};

use ast_sgrep_embed::{cosine_similarity, SemanticChunkRow, MIN_SIMILARITY};

use super::kmeans::kmeans;
use super::vector::{
    brute_force_flat, cluster_count, flatten_vectors, normalize_flat, normalize_vec, rows_from_flat,
};

/// Number of centroid clusters to probe per query.
const DEFAULT_NPROBE: usize = 8;

/// IVF index over normalized semantic chunk vectors.
#[derive(Debug, Clone)]
pub struct SemanticAnnIndex {
    centroids: Vec<Vec<f32>>,
    clusters: Vec<Vec<usize>>,
}

impl SemanticAnnIndex {
    pub fn build(chunks: &[SemanticChunkRow]) -> Self {
        let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
        if chunks.is_empty() || dim == 0 {
            return Self::empty();
        }
        let flat = flatten_vectors(chunks, dim);
        Self::build_from_flat(&flat, dim)
    }

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

        let mut candidates = Vec::new();
        for (cluster_id, _) in centroid_scores.into_iter().take(nprobe) {
            if cluster_id >= self.clusters.len() {
                continue;
            }
            for &idx in &self.clusters[cluster_id] {
                if idx >= n {
                    continue;
                }
                let start = idx * dim;
                if start + dim > flat.len() {
                    continue;
                }
                let row = &flat[start..start + dim];
                let sim = cosine_similarity(&q, row);
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
