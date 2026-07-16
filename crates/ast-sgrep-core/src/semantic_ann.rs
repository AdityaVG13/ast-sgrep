use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use rayon::prelude::*;
use ast_sgrep_embed::{
    dot_similarity, top_k_flat_similarity, top_k_similarity, SemanticChunkRow, MIN_SIMILARITY,
    PARALLEL_CHUNK_THRESHOLD,
};
use crate::semantic_ivf::{
    compute_ann_fingerprint, invalidate_semantic_ivf, load_semantic_ivf,
    load_semantic_ivf_unchecked, save_semantic_ivf, semantic_ivf_path, PersistedSemanticIvf,
};
use crate::store::IndexStore;
use crate::Result;
pub const DEFAULT_ANN_THRESHOLD: usize = 2_000;
#[derive(Debug, Clone)]
pub struct SemanticAnnIndex {
    centroids: Vec<Vec<f32>>,
    clusters: Vec<Vec<usize>>,
}
impl SemanticAnnIndex {
    pub fn build_from_flat(vectors: &[f32], dim: usize) -> Self {
        let n = vectors.len().checked_div(dim).unwrap_or(0);
        if n == 0 || dim == 0 { return Self { centroids: vec![], clusters: vec![] }; }
        let normalized = normalize_flat(vectors, dim);
        let row_vecs: Vec<Vec<f32>> = (0..n).map(|i| normalized[i * dim..(i + 1) * dim].to_vec()).collect();
        let (centroids, assignments) = kmeans(&row_vecs, ((n as f64).sqrt() as usize).clamp(16, 256), 12);
        let mut clusters = vec![Vec::new(); centroids.len()];
        for (idx, &c) in assignments.iter().enumerate() {
            clusters[c].push(idx);
        }
        Self { centroids, clusters }
    }

    pub fn write_to<W: Write>(&self, writer: &mut W, dim: usize) -> std::io::Result<()> {
        write_u32(writer, self.centroids.len() as u32)?;
        for c in &self.centroids {
            for &v in c {
                writer.write_all(&v.to_le_bytes())?;
            }
            for _ in c.len()..dim {
                writer.write_all(&0.0f32.to_le_bytes())?;
            }
        }
        write_u32(writer, self.clusters.len() as u32)?;
        for cluster in &self.clusters {
            write_u32(writer, cluster.len() as u32)?;
            for &idx in cluster {
                write_u32(writer, idx as u32)?;
            }
        }
        Ok(())
    }

    pub fn read_clusters_from<R: Read>(reader: &mut R, k: usize, dim: usize) -> std::io::Result<Self> {
        let mut centroids = Vec::with_capacity(k);
        for _ in 0..k {
            let mut c = vec![0.0f32; dim];
            for v in &mut c {
                *v = read_f32(reader)?;
            }
            centroids.push(c);
        }
        let cluster_count = read_u32(reader)? as usize;
        let mut clusters = Vec::with_capacity(cluster_count);
        for _ in 0..cluster_count {
            let len = read_u32(reader)? as usize;
            let mut members = Vec::with_capacity(len);
            for _ in 0..len {
                members.push(read_u32(reader)? as usize);
            }
            clusters.push(members);
        }
        Ok(Self { centroids, clusters })
    }

    pub fn validate_member_indices(&self, chunk_count: usize) -> bool {
        self.clusters.iter().all(|c| c.iter().all(|&i| i < chunk_count))
    }

    /// `probes`: None/0 = adaptive √k clusters; ≥ n_clusters = all (exact).
    pub fn candidate_indices(&self, query: &[f32], probes: Option<usize>) -> Vec<usize> {
        if self.centroids.is_empty() {
            return vec![];
        }
        let q = normalize_vec(query);
        let mut scores: Vec<(usize, f32)> = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, dot_similarity(&q, c)))
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let k = self.centroids.len();
        let take = match probes {
            None | Some(0) => ((k as f64).sqrt() as usize).clamp(1, k),
            Some(p) if p >= k => k,
            Some(p) => p.max(1).min(k),
        };
        let mut members = Vec::new();
        for (id, _) in scores.into_iter().take(take) {
            if let Some(c) = self.clusters.get(id) {
                members.extend_from_slice(c);
            }
        }
        members
    }

    /// Adaptive probe count (`√k` clusters). Prefer
    /// [`Self::search_flat_with_probes`] when callers need exact coverage.
    pub fn search_flat(&self, flat: &[f32], dim: usize, query: &[f32], limit: usize) -> Vec<(usize, f32)> {
        self.search_flat_with_probes(flat, dim, query, limit, None)
    }

    /// `probes`: `None`/`Some(0)` = adaptive √k; `Some(p)` with `p >= k` probes every
    /// cluster (exact over the partitioned set, same top-k as brute force when the
    /// partition is complete).
    pub fn search_flat_with_probes(
        &self,
        flat: &[f32],
        dim: usize,
        query: &[f32],
        limit: usize,
        probes: Option<usize>,
    ) -> Vec<(usize, f32)> {
        let n = flat.len().checked_div(dim).unwrap_or(0);
        if n == 0 {
            return vec![];
        }
        if self.centroids.is_empty() {
            return brute_force_flat(flat, dim, query, limit);
        }
        let q = normalize_vec(query);
        score_members(&q, flat, dim, n, &self.candidate_indices(&q, probes), limit)
    }

    pub fn reassign_all(&mut self, flat: &[f32], dim: usize) {
        let n = flat.len().checked_div(dim).unwrap_or(0);
        if n == 0 || self.centroids.is_empty() {
            return;
        }
        self.clusters = vec![Vec::new(); self.centroids.len()];
        for idx in 0..n {
            let start = idx * dim;
            if start + dim > flat.len() {
                break;
            }
            let vec = &flat[start..start + dim];
            let best = nearest_centroid(vec, &self.centroids);
            if best < self.clusters.len() {
                self.clusters[best].push(idx);
            }
        }
    }
}
pub fn flatten_vectors_for_search(chunks: &[SemanticChunkRow], dim: usize) -> Result<Vec<f32>> {
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.5.len() != dim {
            return Err(crate::StoreError::Other(format!(
                "semantic chunk {} has dimension {} but expected {} (mixed backends or corrupted store; reindex with --force-reindex)",
                i, chunk.5.len(), dim
            )));
        }
    }
    let mut flat = vec![0.0f32; chunks.len() * dim];
    if chunks.len() >= PARALLEL_CHUNK_THRESHOLD {
        flat.par_chunks_mut(dim).zip(chunks.par_iter()).for_each(|(row, chunk)| {
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
    Ok(flat)
}
fn write_u32<W: Write>(w: &mut W, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn read_u32<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_f32<R: Read>(r: &mut R) -> std::io::Result<f32> {
    Ok(f32::from_le_bytes({
        let mut b = [0u8; 4];
        r.read_exact(&mut b)?;
        b
    }))
}
fn score_members(
    query: &[f32],
    flat: &[f32],
    dim: usize,
    n: usize,
    members: &[usize],
    limit: usize,
) -> Vec<(usize, f32)> {
    if members.is_empty() { return vec![]; }
    let score = |idx: &usize| -> Option<(usize, f32)> {
        if *idx >= n { return None; }
        let start = idx * dim;
        (start + dim <= flat.len())
            .then(|| dot_similarity(query, &flat[start..start + dim]))
            .filter(|&sim| sim >= MIN_SIMILARITY)
            .map(|sim| (*idx, sim))
    };
    if members.len() < PARALLEL_CHUNK_THRESHOLD {
        top_k_similarity(members.iter().filter_map(score), limit, None)
    } else {
        top_k_similarity(members.par_iter().filter_map(score).collect::<Vec<_>>(), limit, None)
    }
}
fn normalize_flat(vectors: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vectors.to_vec();
    for i in 0..vectors.len() / dim {
        normalize_vec_in_place(&mut out[i * dim..(i + 1) * dim]);
    }
    out
}
fn normalize_vec_in_place(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec {
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
    top_k_flat_similarity(&normalize_vec(query), flat, dim, limit, Some(MIN_SIMILARITY))
}
fn nearest_centroid(vector: &[f32], centroids: &[Vec<f32>]) -> usize {
    centroids
        .iter()
        .enumerate()
        .map(|(ci, c)| (ci, dot_similarity(vector, c)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(ci, _)| ci)
        .unwrap_or(0)
}
fn kmeans(vectors: &[Vec<f32>], k: usize, max_iters: usize) -> (Vec<Vec<f32>>, Vec<usize>) {
    let k = k.min(vectors.len()).max(1);
    let dim = vectors[0].len();
    let mut centroids = {
        let mut c = vec![vectors[0].clone()];
        while c.len() < k {
            let best = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let min_sim = c.iter().map(|cent| dot_similarity(v, cent)).fold(f32::INFINITY, f32::min);
                    (i, 1.0 - min_sim)
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            c.push(vectors[best].clone());
        }
        c
    };
    let mut assignments = vec![0usize; vectors.len()];
    for _ in 0..max_iters {
        let mut changed = false;
        for (i, v) in vectors.iter().enumerate() {
            let best = nearest_centroid(v, &centroids);
            changed |= assignments[i] != best;
            assignments[i] = best;
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
        centroids = sums
            .iter()
            .zip(counts.iter())
            .zip(centroids.iter())
            .map(|((sum, &count), prev)| {
                if count == 0 {
                    prev.clone()
                } else {
                    normalize_vec(&sum.iter().map(|v| v / count as f32).collect::<Vec<_>>())
                }
            })
            .collect();
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
pub fn clear_semantic_ivf_session_cache() {
    *SESSION_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
pub fn mark_semantic_ivf_stale(store: &IndexStore) {
    let _ = store.set_meta("semantic_ivf_stale", "1");
    clear_semantic_ivf_session_cache();
}
fn ann_session_key(store: &IndexStore, chunks: &[SemanticChunkRow]) -> Result<([u8; 32], String)> {
    let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
    let max_id = store.semantic_chunk_max_id()?.unwrap_or(0);
    let backend = store.get_meta("embed_backend")?.unwrap_or_else(|| "semantic".into());
    Ok((
        compute_ann_fingerprint(chunks.len(), max_id, dim, Some(&backend)),
        store.db_path().to_string_lossy().into_owned(),
    ))
}
fn cache_session(db_key: &str, fingerprint: [u8; 32], ivf: &PersistedSemanticIvf) {
    *SESSION_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((
        db_key.to_string(),
        SessionCache { fingerprint, ivf: Arc::new(ivf.clone()) },
    ));
}
pub fn load_or_build_semantic_ivf(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    override_threshold: Option<usize>,
) -> Result<Option<Arc<PersistedSemanticIvf>>> {
    let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
    if chunks.is_empty() || dim == 0 || !should_use_ann(chunks.len(), override_threshold) { return Ok(None); }
    let (fingerprint, db_key) = ann_session_key(store, chunks)?;
    let ivf_path = semantic_ivf_path(store.db_path());
    if let Ok(Some(ivf)) = load_semantic_ivf(&ivf_path, fingerprint) {
        cache_session(&db_key, fingerprint, &ivf);
        return Ok(Some(Arc::new(ivf)));
    }
    let flat = flatten_vectors_for_search(chunks, dim)?;
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    save_semantic_ivf(&ivf_path, fingerprint, dim, &flat, &index)?;
    let ivf = PersistedSemanticIvf { fingerprint, dim, vectors: flat, index };
    cache_session(&db_key, fingerprint, &ivf);
    Ok(Some(Arc::new(ivf)))
}
pub fn cached_semantic_ivf(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    override_threshold: Option<usize>,
) -> Result<Option<Arc<PersistedSemanticIvf>>> {
    if !should_use_ann(chunks.len(), override_threshold) { return Ok(None); }
    let (fingerprint, db_key) = ann_session_key(store, chunks)?;
    {
        let guard = SESSION_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((key, cached)) = guard.as_ref() {
            if key == &db_key && cached.fingerprint == fingerprint { return Ok(Some(Arc::clone(&cached.ivf))); }
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
    if chunks.is_empty() { return Ok(vec![]); }
    let dim = chunks[0].5.len();
    if let Some(ivf) = cached_semantic_ivf(store, chunks, override_threshold)? { return Ok(ivf.search(query_vec, limit)); }
    Ok(match flat {
        Some(f) => brute_force_flat(f, dim, query_vec, limit),
        None => brute_force_flat(&flatten_vectors_for_search(chunks, dim)?, dim, query_vec, limit),
    })
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
    if chunks.first().is_none_or(|c| c.5.is_empty()) { return Ok(()); }
    let dim = chunks[0].5.len();
    if store.get_meta("semantic_ivf_stale")?.as_deref() == Some("1") {
        if let Some(mut ivf) = load_semantic_ivf_unchecked(&semantic_ivf_path(store.db_path()))? {
            if ivf.chunk_count() == chunks.len() && ivf.dim == dim {
                ivf.vectors = flatten_vectors_for_search(chunks, dim)?;
                ivf.index.reassign_all(&ivf.vectors, dim);
                let (fingerprint, db_key) = ann_session_key(store, chunks)?;
                ivf.fingerprint = fingerprint;
                save_semantic_ivf(&semantic_ivf_path(store.db_path()), fingerprint, dim, &ivf.vectors, &ivf.index)?;
                cache_session(&db_key, fingerprint, &ivf);
                let _ = store.set_meta("semantic_ivf_stale", "0");
                return Ok(());
            }
        }
    }
    let _ = load_or_build_semantic_ivf(store, chunks, override_threshold)?;
    let _ = store.set_meta("semantic_ivf_stale", "0");
    Ok(())
}
