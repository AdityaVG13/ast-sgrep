//! Semantic approximate-nearest-neighbor search (IVF + brute force).

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use rayon::prelude::*;

use ast_sgrep_embed::{
    cosine_similarity, top_k_flat_similarity, top_k_similarity, SemanticChunkRow,
    MIN_SIMILARITY, PARALLEL_CHUNK_THRESHOLD,
};

use crate::semantic_ivf::{
    compute_ann_fingerprint, invalidate_semantic_ivf, load_semantic_ivf, save_semantic_ivf,
    semantic_ivf_path, PersistedSemanticIvf,
};
use crate::store::IndexStore;
use crate::Result;

pub const DEFAULT_ANN_THRESHOLD: usize = 2_000;

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

        let mut members = Vec::new();
        for (cluster_id, _) in centroid_scores.into_iter().take(nprobe) {
            if cluster_id < self.clusters.len() {
                members.extend_from_slice(&self.clusters[cluster_id]);
            }
        }
        score_members(&q, flat, dim, n, &members, limit)
    }
}

pub fn flatten_vectors_for_search(chunks: &[SemanticChunkRow], dim: usize) -> Vec<f32> {
    let mut flat = vec![0.0f32; chunks.len() * dim];
    if chunks.len() >= PARALLEL_CHUNK_THRESHOLD {
        flat.par_chunks_mut(dim)
            .zip(chunks.par_iter())
            .for_each(|(row, chunk)| {
                row.copy_from_slice(&chunk.5);
                normalize_vec_in_place(row);
            });
    } else {
        for (i, chunk) in chunks.iter().enumerate() {
            let start = i * dim;
            flat[start..start + dim].copy_from_slice(&chunk.5);
            normalize_vec_in_place(&mut flat[start..start + dim]);
        }
    }
    flat
}

fn flatten_vectors(chunks: &[SemanticChunkRow], dim: usize) -> Vec<f32> {
    flatten_vectors_for_search(chunks, dim)
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
    let scored: Vec<(usize, f32)> = members.par_iter().filter_map(score).collect();
    top_k_similarity(scored, limit, None)
}

fn normalize_flat(vectors: &[f32], dim: usize) -> Vec<f32> {
    let n = vectors.len() / dim;
    let mut out = vectors.to_vec();
    for i in 0..n {
        let start = i * dim;
        let row = &mut out[start..start + dim];
        normalize_vec_in_place(row);
    }
    out
}

fn rows_from_flat(flat: &[f32], dim: usize) -> Vec<Vec<f32>> {
    let n = flat.len() / dim;
    (0..n)
        .map(|i| {
            let start = i * dim;
            flat[start..start + dim].to_vec()
        })
        .collect()
}

fn cluster_count(n: usize) -> usize {
    let sqrt = (n as f64).sqrt() as usize;
    sqrt.clamp(16, 256)
}

fn normalize_vec_in_place(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

fn normalize_vec(vec: &[f32]) -> Vec<f32> {
    let mut out = vec.to_vec();
    normalize_vec_in_place(&mut out);
    out
}

fn brute_force_flat(flat: &[f32], dim: usize, query: &[f32], limit: usize) -> Vec<(usize, f32)> {
    let q = normalize_vec(query);
    top_k_flat_similarity(&q, flat, dim, limit, Some(MIN_SIMILARITY))
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

pub fn ann_threshold(override_threshold: Option<usize>) -> usize {
    override_threshold.unwrap_or_else(|| {
        std::env::var("ASGREP_ANN_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_ANN_THRESHOLD)
    })
}

pub fn should_use_ann(chunk_count: usize, override_threshold: Option<usize>) -> bool {
    chunk_count >= ann_threshold(override_threshold)
}

struct SessionCache {
    fingerprint: [u8; 32],
    ivf: Arc<PersistedSemanticIvf>,
}

static SESSION_CACHE: Mutex<Option<(String, SessionCache)>> = Mutex::new(None);

pub fn load_or_build_semantic_ivf(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    override_threshold: Option<usize>,
) -> Result<Option<Arc<PersistedSemanticIvf>>> {
    let chunk_count = chunks.len();
    if chunk_count == 0 {
        return Ok(None);
    }
    let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
    if dim == 0 {
        return Ok(None);
    }

    let (fingerprint, db_key) = ann_session_key(store, chunks)?;
    let ivf_path = semantic_ivf_path(store.db_path());

    if should_use_ann(chunk_count, override_threshold) {
        if let Ok(Some(ivf)) = load_semantic_ivf(&ivf_path, fingerprint) {
            cache_session(&db_key, fingerprint, &ivf);
            return Ok(Some(Arc::new(ivf)));
        }
        let flat = flatten_vectors(chunks, dim);
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        save_semantic_ivf(&ivf_path, fingerprint, dim, &flat, &index)?;
        let ivf = PersistedSemanticIvf {
            fingerprint,
            dim,
            vectors: flat,
            index,
        };
        cache_session(&db_key, fingerprint, &ivf);
        return Ok(Some(Arc::new(ivf)));
    }

    Ok(None)
}

pub fn cached_semantic_ivf(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    override_threshold: Option<usize>,
) -> Result<Option<Arc<PersistedSemanticIvf>>> {
    if !should_use_ann(chunks.len(), override_threshold) {
        return Ok(None);
    }
    let (fingerprint, db_key) = ann_session_key(store, chunks)?;

    {
        let guard = SESSION_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((key, cached)) = guard.as_ref() {
            if key == &db_key && cached.fingerprint == fingerprint {
                return Ok(Some(Arc::clone(&cached.ivf)));
            }
        }
    }

    load_or_build_semantic_ivf(store, chunks, override_threshold)
}

pub fn rank_chunk_indices_flat(
    store: &IndexStore,
    query_vec: &[f32],
    chunks: &[SemanticChunkRow],
    flat: Option<&[f32]>,
    limit: usize,
    override_threshold: Option<usize>,
) -> Result<Vec<(usize, f32)>> {
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let dim = chunks[0].5.len();

    if let Some(ivf) = cached_semantic_ivf(store, chunks, override_threshold)? {
        return Ok(ivf.search(query_vec, limit));
    }

    if let Some(flat) = flat {
        return Ok(brute_force_flat(flat, dim, query_vec, limit));
    }
    let owned = flatten_vectors(chunks, dim);
    Ok(brute_force_flat(&owned, dim, query_vec, limit))
}

pub fn rebuild_semantic_ivf_sidecar(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    override_threshold: Option<usize>,
) -> Result<()> {
    if !should_use_ann(chunks.len(), override_threshold) {
        let _ = invalidate_semantic_ivf(store.db_path());
        return Ok(());
    }
    let _ = load_or_build_semantic_ivf(store, chunks, override_threshold)?;
    Ok(())
}

fn ann_session_key(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
) -> Result<([u8; 32], String)> {
    let chunk_count = chunks.len();
    let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
    let max_id = store.semantic_chunk_max_id()?.unwrap_or(0);
    let backend = store.get_meta("embed_backend")?.unwrap_or_else(|| "semantic".to_string());
    Ok((
        compute_ann_fingerprint(chunk_count, max_id, dim, Some(&backend)),
        store.db_path().to_string_lossy().into_owned(),
    ))
}

fn cache_session(db_key: &str, fingerprint: [u8; 32], ivf: &PersistedSemanticIvf) {
    let mut guard = SESSION_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some((
        db_key.to_string(),
        SessionCache {
            fingerprint,
            ivf: Arc::new(ivf.clone()),
        },
    ));
}
