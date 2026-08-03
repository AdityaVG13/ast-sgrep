use crate::semantic_ivf::{
    compute_ann_fingerprint, invalidate_semantic_ivf, load_semantic_ivf,
    load_semantic_ivf_unchecked, save_semantic_ivf, semantic_ivf_path, PersistedSemanticIvf,
};
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::{
    cosine_similarity, normalize_vec, normalize_vec_in_place, top_k_flat_similarity,
    top_k_similarity, SemanticChunkRow, MIN_SIMILARITY, PARALLEL_CHUNK_THRESHOLD,
};
use rayon::prelude::*;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};
pub const DEFAULT_ANN_THRESHOLD: usize = 2_000;
/// Measured minimum satisfying recall@10 >= 0.99 at 2,048 and 10,000 vectors.
pub const DEFAULT_ADAPTIVE_PROBE_PERCENT: usize = 90;
#[derive(Debug, Clone)]
pub struct SemanticAnnIndex {
    centroids: Vec<Vec<f32>>,
    clusters: Vec<Vec<usize>>,
}
impl SemanticAnnIndex {
    pub fn build_from_flat(vectors: &[f32], dim: usize) -> Self {
        let n = vectors.len().checked_div(dim).unwrap_or(0);
        if n == 0 || dim == 0 {
            return Self {
                centroids: vec![],
                clusters: vec![],
            };
        }
        let normalized = normalize_flat(vectors, dim);
        let row_vecs: Vec<Vec<f32>> = (0..n)
            .map(|i| normalized[i * dim..(i + 1) * dim].to_vec())
            .collect();
        let (centroids, assignments) =
            kmeans(&row_vecs, ((n as f64).sqrt() as usize).clamp(16, 256), 12);
        let mut clusters = vec![Vec::new(); centroids.len()];
        for (idx, &c) in assignments.iter().enumerate() {
            clusters[c].push(idx);
        }
        Self {
            centroids,
            clusters,
        }
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
    pub fn read_clusters_from<R: Read>(
        reader: &mut R,
        k: usize,
        dim: usize,
    ) -> std::io::Result<Self> {
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
        Ok(Self {
            centroids,
            clusters,
        })
    }
    pub fn read_clusters_bounded(
        bytes: &[u8],
        k: usize,
        dim: usize,
        chunk_count: usize,
    ) -> std::io::Result<Self> {
        let mut offset = 0usize;
        if take_u32(bytes, &mut offset)? as usize != k {
            return Err(invalid_ivf_index());
        }
        let centroid_values = k.checked_mul(dim).ok_or_else(invalid_ivf_index)?;
        let mut centroids = Vec::with_capacity(k);
        for _ in 0..k {
            let mut centroid = Vec::with_capacity(dim);
            for _ in 0..dim {
                centroid.push(f32::from_bits(take_u32(bytes, &mut offset)?));
            }
            centroids.push(centroid);
        }
        if centroids.iter().map(Vec::len).sum::<usize>() != centroid_values
            || take_u32(bytes, &mut offset)? as usize != k
        {
            return Err(invalid_ivf_index());
        }
        let mut clusters = Vec::with_capacity(k);
        let mut seen = vec![false; chunk_count];
        let mut members = 0usize;
        for _ in 0..k {
            let length = take_u32(bytes, &mut offset)? as usize;
            members = members
                .checked_add(length)
                .filter(|total| *total <= chunk_count)
                .ok_or_else(invalid_ivf_index)?;
            if offset
                .checked_add(length.checked_mul(4).ok_or_else(invalid_ivf_index)?)
                .is_none_or(|end| end > bytes.len())
            {
                return Err(invalid_ivf_index());
            }
            let mut cluster = Vec::with_capacity(length);
            for _ in 0..length {
                let index = take_u32(bytes, &mut offset)? as usize;
                let slot = seen.get_mut(index).ok_or_else(invalid_ivf_index)?;
                if *slot {
                    return Err(invalid_ivf_index());
                }
                *slot = true;
                cluster.push(index);
            }
            clusters.push(cluster);
        }
        if members != chunk_count
            || offset != bytes.len()
            || seen.into_iter().any(|present| !present)
        {
            return Err(invalid_ivf_index());
        }
        Ok(Self {
            centroids,
            clusters,
        })
    }

    pub fn heap_bytes(&self) -> usize {
        self.centroids
            .iter()
            .map(|centroid| centroid.capacity() * std::mem::size_of::<f32>())
            .sum::<usize>()
            .saturating_add(
                self.clusters
                    .iter()
                    .map(|cluster| cluster.capacity() * std::mem::size_of::<usize>())
                    .sum::<usize>(),
            )
    }

    pub fn validate_partition(&self, chunk_count: usize) -> bool {
        let mut seen = vec![false; chunk_count];
        for &index in self.clusters.iter().flatten() {
            let Some(slot) = seen.get_mut(index) else {
                return false;
            };
            if *slot {
                return false;
            }
            *slot = true;
        }
        seen.into_iter().all(|present| present)
    }

    /// `probes`: None/0 = at most 90% of populated clusters; ≥ n_clusters = exact.
    pub fn candidate_indices(&self, query: &[f32], probes: Option<usize>) -> Vec<usize> {
        if self.centroids.is_empty() {
            return vec![];
        }
        let q = normalize_vec(query);
        let mut scores: Vec<(usize, f32)> = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, cosine_similarity(&q, c)))
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores.retain(|(id, _)| self.clusters.get(*id).is_some_and(|c| !c.is_empty()));
        let populated = scores.len();
        if populated == 0 {
            return vec![];
        }
        let take = match probes {
            None | Some(0) if populated > 1 => populated
                .saturating_mul(DEFAULT_ADAPTIVE_PROBE_PERCENT)
                .div_euclid(100)
                .clamp(1, populated - 1),
            None | Some(0) => 1,
            Some(p) => p.max(1).min(populated),
        };
        let mut members = Vec::new();
        for (id, _) in scores.into_iter().take(take) {
            members.extend_from_slice(&self.clusters[id]);
        }
        members.sort_unstable();
        members
    }
    pub fn search_flat(
        &self,
        flat: &[f32],
        dim: usize,
        query: &[f32],
        limit: usize,
    ) -> Vec<(usize, f32)> {
        self.search_flat_with_probes(flat, dim, query, limit, None)
    }
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
        if n < DEFAULT_ANN_THRESHOLD || self.centroids.is_empty() {
            return brute_force_flat(flat, dim, query, limit);
        }
        let q = normalize_vec(query);
        score_members(&q, flat, dim, n, &self.candidate_indices(&q, probes), limit)
    }
    pub fn reassign_all(&mut self, flat: &[f32], dim: usize) {
        if flat.is_empty() || dim == 0 {
            return;
        }
        *self = Self::build_from_flat(flat, dim);
    }
}
pub fn flatten_vectors_for_search(chunks: &[SemanticChunkRow], dim: usize) -> Result<Vec<f32>> {
    if dim == 0 {
        return if chunks.is_empty() {
            Ok(vec![])
        } else {
            Err(crate::StoreError::Other(
                "semantic embedding dimension is 0 (corrupt store or unset backend; reindex)"
                    .into(),
            ))
        };
    }
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.5.len() != dim {
            return Err(crate::StoreError::Other(format!(
                "semantic chunk {} has dimension {} but expected {} (mixed backends or corrupted store; reindex with --force-reindex)", i, chunk.5.len(), dim
            )));
        }
    }
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
    Ok(flat)
}
fn write_u32<W: Write>(w: &mut W, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn invalid_ivf_index() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid semantic IVF cluster index",
    )
}

fn take_u32(bytes: &[u8], offset: &mut usize) -> std::io::Result<u32> {
    let end = offset.checked_add(4).ok_or_else(invalid_ivf_index)?;
    let raw: [u8; 4] = bytes
        .get(*offset..end)
        .ok_or_else(invalid_ivf_index)?
        .try_into()
        .map_err(|_| invalid_ivf_index())?;
    *offset = end;
    Ok(u32::from_le_bytes(raw))
}

fn read_u32<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_f32<R: Read>(r: &mut R) -> std::io::Result<f32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(f32::from_le_bytes(b))
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
        return vec![];
    }
    let score = |idx: &usize| -> Option<(usize, f32)> {
        if *idx >= n {
            return None;
        }
        let start = idx * dim;
        (start + dim <= flat.len())
            .then(|| cosine_similarity(query, &flat[start..start + dim]))
            .filter(|&sim| sim > MIN_SIMILARITY)
            .map(|sim| (*idx, sim))
    };
    if members.len() < PARALLEL_CHUNK_THRESHOLD {
        top_k_similarity(members.iter().filter_map(score), limit, None)
    } else {
        top_k_similarity(
            members.par_iter().filter_map(score).collect::<Vec<_>>(),
            limit,
            None,
        )
    }
}
fn normalize_flat(vectors: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vectors.to_vec();
    for i in 0..vectors.len() / dim {
        normalize_vec_in_place(&mut out[i * dim..(i + 1) * dim]);
    }
    out
}
fn brute_force_flat(flat: &[f32], dim: usize, query: &[f32], limit: usize) -> Vec<(usize, f32)> {
    top_k_flat_similarity(
        &normalize_vec(query),
        flat,
        dim,
        limit,
        Some(MIN_SIMILARITY),
    )
}
fn nearest_centroid(vector: &[f32], centroids: &[Vec<f32>]) -> usize {
    centroids
        .iter()
        .enumerate()
        .map(|(ci, c)| (ci, cosine_similarity(vector, c)))
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
                    let nearest_sim = c
                        .iter()
                        .map(|cent| cosine_similarity(v, cent))
                        .fold(f32::NEG_INFINITY, f32::max);
                    (i, 1.0 - nearest_sim)
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
static SESSION_CACHE: Mutex<Vec<(String, SessionCache)>> = Mutex::new(Vec::new());

fn lock_session_cache() -> std::sync::MutexGuard<'static, Vec<(String, SessionCache)>> {
    match SESSION_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            SESSION_CACHE.clear_poison();
            let mut guard = PoisonError::into_inner(poisoned);
            guard.clear();
            guard
        }
    }
}

pub fn mark_semantic_ivf_stale(store: &IndexStore) {
    if store
        .get_meta("semantic_ivf_stale")
        .ok()
        .flatten()
        .as_deref()
        != Some("1")
    {
        let _ = store.set_meta("semantic_ivf_stale", "1");
    }
    lock_session_cache().clear();
    let _ = invalidate_semantic_ivf(store.db_path());
}
fn ann_session_key(store: &IndexStore, chunks: &[SemanticChunkRow]) -> Result<([u8; 32], String)> {
    let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
    let max_id = store.semantic_chunk_max_id()?.unwrap_or(0);
    let backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    Ok((
        compute_ann_fingerprint(
            chunks.len(),
            max_id,
            dim,
            Some(&backend),
            store.index_data_version()?,
        ),
        store.db_path().to_string_lossy().into_owned(),
    ))
}
fn cache_session(db_key: &str, fingerprint: [u8; 32], ivf: Arc<PersistedSemanticIvf>) {
    let mut cache = lock_session_cache();
    if let Some(pos) = cache.iter().position(|(key, _)| key == db_key) {
        cache.remove(pos);
    }
    if cache.len() == 4 {
        cache.remove(0);
    }
    cache.push((db_key.to_string(), SessionCache { fingerprint, ivf }));
}
pub fn load_or_build_semantic_ivf(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    override_threshold: Option<usize>,
) -> Result<Option<Arc<PersistedSemanticIvf>>> {
    let dim = chunks.first().map(|c| c.5.len()).unwrap_or(0);
    if chunks.is_empty() || dim == 0 || !should_use_ann(chunks.len(), override_threshold) {
        return Ok(None);
    }
    let (fingerprint, db_key) = ann_session_key(store, chunks)?;
    let ivf_path = semantic_ivf_path(store.db_path());
    match load_semantic_ivf(&ivf_path, fingerprint) {
        Ok(Some(ivf)) => {
            let _ = store.set_meta("semantic_ivf_stale", "0");
            let ivf = Arc::new(ivf);
            cache_session(&db_key, fingerprint, Arc::clone(&ivf));
            return Ok(Some(ivf));
        }
        Ok(None) => {}
        Err(error) => return Err(error),
    }
    let flat = flatten_vectors_for_search(chunks, dim)?;
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let published = save_semantic_ivf(&ivf_path, fingerprint, dim, &flat, &index)?;
    let _ = store.set_meta("semantic_ivf_stale", if published { "0" } else { "1" });
    let ivf = Arc::new(PersistedSemanticIvf::from_owned(
        fingerprint,
        dim,
        flat,
        index,
    ));
    cache_session(&db_key, fingerprint, Arc::clone(&ivf));
    Ok(Some(ivf))
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
        let mut cache = lock_session_cache();
        if let Some(pos) = cache.iter().position(|(key, _)| key == &db_key) {
            let (key, cached) = cache.remove(pos);
            if cached.fingerprint == fingerprint {
                let ivf = Arc::clone(&cached.ivf);
                cache.push((key, cached));
                return Ok(Some(ivf));
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
        return Ok(vec![]);
    }
    let dim = chunks[0].5.len();
    if let Some(ivf) = cached_semantic_ivf(store, chunks, override_threshold)? {
        return Ok(ivf.search(query_vec, limit));
    }
    Ok(match flat {
        Some(f) => brute_force_flat(f, dim, query_vec, limit),
        None => brute_force_flat(
            &flatten_vectors_for_search(chunks, dim)?,
            dim,
            query_vec,
            limit,
        ),
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
    let Some(first) = chunks.first().filter(|c| !c.5.is_empty()) else {
        return Ok(());
    };
    let dim = first.5.len();
    if reassign_stale_ivf_partition(store, chunks, dim)? {
        return Ok(());
    }
    let _ = load_or_build_semantic_ivf(store, chunks, override_threshold)?;
    Ok(())
}

/// When the IVF sidecar is marked stale but topology still matches, reassign members
/// in place instead of a full rebuild.
fn reassign_stale_ivf_partition(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    dim: usize,
) -> Result<bool> {
    if store.get_meta("semantic_ivf_stale")?.as_deref() != Some("1") {
        return Ok(false);
    }
    let Some(ivf) = load_semantic_ivf_unchecked(&semantic_ivf_path(store.db_path()))? else {
        return Ok(false);
    };
    if ivf.chunk_count() != chunks.len() || ivf.dim != dim {
        return Ok(false);
    }
    let vectors = flatten_vectors_for_search(chunks, dim)?;
    let mut index = ivf.index.clone();
    drop(ivf);
    index.reassign_all(&vectors, dim);
    let (fingerprint, db_key) = ann_session_key(store, chunks)?;
    let published = save_semantic_ivf(
        &semantic_ivf_path(store.db_path()),
        fingerprint,
        dim,
        &vectors,
        &index,
    )?;
    let rebuilt = PersistedSemanticIvf::from_owned(fingerprint, dim, vectors, index);
    cache_session(&db_key, fingerprint, Arc::new(rebuilt));
    let _ = store.set_meta("semantic_ivf_stale", if published { "0" } else { "1" });
    Ok(true)
}

#[cfg(test)]
mod flatten_bounds_tests {
    use super::flatten_vectors_for_search;
    use ast_sgrep_embed::SemanticChunkRow;

    #[test]
    fn flatten_rejects_zero_dim_with_chunks() {
        let chunks: Vec<SemanticChunkRow> = vec![(
            "a.rs".into(),
            1u32,
            1u32,
            "sym".into(),
            "x".into(),
            vec![],
        )];
        let err = flatten_vectors_for_search(&chunks, 0).expect_err("dim=0 must fail");
        assert!(
            err.to_string().contains("dimension is 0"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn flatten_allows_empty_chunks_with_zero_dim() {
        let out = flatten_vectors_for_search(&[], 0).expect("empty ok");
        assert!(out.is_empty());
    }
}

