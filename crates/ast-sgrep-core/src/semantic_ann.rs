use crate::semantic_ivf::{
    compute_ann_fingerprint, invalidate_semantic_ivf, load_semantic_ivf,
    load_semantic_ivf_unchecked, save_semantic_ivf_with_publication, semantic_ivf_path, PersistedSemanticIvf,
};
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, normalize_vec, normalize_vec_in_place,
    top_k_flat_similarity, top_k_similarity, SemanticChunkRow, MIN_SIMILARITY,
    PARALLEL_CHUNK_THRESHOLD,
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
        let _span = crate::perf_profile::Span::start(
            "semantic_ivf_build",
            "semantic",
            "SemanticAnnIndex::build_from_flat (kmeans)",
        );
        let n = vectors.len().checked_div(dim).unwrap_or(0);
        if n == 0 || dim == 0 {
            return Self {
                centroids: vec![],
                clusters: vec![],
            };
        }
        // T1 lever A (IVF-COPY): one owned flat normalize only. Prior path did
        // `normalize_flat` (full clone) then per-row `.to_vec()` into `Vec<Vec<f32>>`
        // (second full materialize). k-means now reads row slices from this flat buffer.
        // T1-B: parallel per-row assignment + serial row-order centroid reduction →
        // bit-identical centroids/assignments vs the pre-T1 multi-copy serial path.
        let mut flat = vectors.to_vec();
        for i in 0..n {
            normalize_vec_in_place(&mut flat[i * dim..(i + 1) * dim]);
        }
        let (centroids, assignments) =
            kmeans(&flat, dim, ((n as f64).sqrt() as usize).clamp(16, 256), 12);
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

    pub fn validate_member_indices(&self, chunk_count: usize) -> bool {
        self.validate_partition(chunk_count)
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
        // ANN eligibility (n vs ASGREP_ANN_THRESHOLD / options.ann_threshold) is a
        // *build-time* decision (`should_use_ann` / load_or_build). Re-checking
        // DEFAULT_ANN_THRESHOLD here forced every query with n < 2000 onto
        // brute_force even when IVF was intentionally built under a lower
        // override — so the override could never enable ANN search for mid-size
        // corpora. Empty centroids still mean "no IVF"; fall back to exact.
        if self.centroids.is_empty() {
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
    // Gate through top_k_similarity(..., Some(MIN_SIMILARITY)) so IVF member
    // scoring uses the same ULP-stable exclusive predicate as brute_force_flat
    // / top_k_flat_similarity (firi / jiyy.5). A plain `sim > MIN` would admit
    // the first float above MIN while exceeds_threshold rejects it.
    let score = |idx: &usize| -> Option<(usize, f32)> {
        if *idx >= n {
            return None;
        }
        let start = idx * dim;
        (start + dim <= flat.len())
            .then(|| cosine_similarity(query, &flat[start..start + dim]))
            .map(|sim| (*idx, sim))
    };
    if members.len() < PARALLEL_CHUNK_THRESHOLD {
        top_k_similarity(
            members.iter().filter_map(score),
            limit,
            Some(MIN_SIMILARITY),
        )
    } else {
        top_k_similarity(
            members.par_iter().filter_map(score).collect::<Vec<_>>(),
            limit,
            Some(MIN_SIMILARITY),
        )
    }
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
#[inline]
fn flat_row(flat: &[f32], dim: usize, i: usize) -> &[f32] {
    let start = i * dim;
    &flat[start..start + dim]
}
fn nearest_centroid(vector: &[f32], centroids: &[Vec<f32>]) -> usize {
    // T1-R (Pass 9): rows + centroids are L2-normalized (build_from_flat +
    // post-update renorm). Cosine then equals the plain inner product, and
    // `dot_similarity` uses simsimd for dim ≥ 64 (product dim=256). Same max
    // + lowest-index tie-break as the old cosine path; scores differ slightly
    // from full cosine (f64 renorm) so IVF clusters are not bit-identical to
    // pre-T1-R sidecars — see L9_CHANGE.md.
    centroids
        .iter()
        .enumerate()
        .map(|(ci, c)| (ci, dot_similarity(vector, c)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(ci, _)| ci)
        .unwrap_or(0)
}
/// Deterministic k-means over a row-major flat matrix (`n * dim` floats).
/// Farthest-point init from row 0 (serial), parallel per-row assignment
/// (lowest index wins ties), serial mean + renorm update in ascending row
/// order, early exit when assignments stop changing. Similarity for init and
/// assign is [`dot_similarity`] on L2-normalized rows/centroids (T1-R; cosine
/// equals dot for unit vectors). Parallel assign + ordered reduce is
/// bit-identical to fully serial assign/update under the same metric.
/// Operates on slices so callers need not materialize per-row `Vec`s.
fn kmeans(flat: &[f32], dim: usize, k: usize, max_iters: usize) -> (Vec<Vec<f32>>, Vec<usize>) {
    let n = if dim == 0 {
        0
    } else {
        flat.len() / dim
    };
    let k = k.min(n).max(1);
    let mut centroids = {
        let mut c = vec![flat_row(flat, dim, 0).to_vec()];
        while c.len() < k {
            let best = (0..n)
                .map(|i| {
                    let v = flat_row(flat, dim, i);
                    let nearest_sim = c
                        .iter()
                        .map(|cent| dot_similarity(v, cent))
                        .fold(f32::NEG_INFINITY, f32::max);
                    (i, 1.0 - nearest_sim)
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            c.push(flat_row(flat, dim, best).to_vec());
        }
        c
    };
    let mut assignments = vec![0usize; n];
    for _ in 0..max_iters {
        // T1-B: assignment is embarrassingly parallel given fixed centroids.
        // `(0..n).into_par_iter().map(...).collect()` preserves index order, so
        // `next[i]` is the assignment for row `i`. No cross-row data dependence.
        let next: Vec<usize> = (0..n)
            .into_par_iter()
            .map(|i| nearest_centroid(flat_row(flat, dim, i), &centroids))
            .collect();
        // Early exit: OR of per-row deltas (associative). Must match serial:
        // first iteration where all rows stable skips centroid update.
        let changed = next
            .iter()
            .zip(assignments.iter())
            .any(|(a, b)| a != b);
        assignments = next;
        if !changed {
            break;
        }
        // Serial reduction in ascending row order so f32 sums match the serial
        // algorithm bit-for-bit (parallel float reduce is forbidden).
        let mut sums = vec![vec![0.0f32; dim]; k];
        let mut counts = vec![0usize; k];
        for i in 0..n {
            let c = assignments[i];
            counts[c] += 1;
            for (j, val) in flat_row(flat, dim, i).iter().enumerate() {
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
/// Empty / under-filled ANN results must not short-circuit the flat path:
/// sufficient only when at least `min(limit, total)` candidates survived.
pub fn ann_result_is_sufficient(found: usize, total: usize, limit: usize) -> bool {
    found >= limit.min(total)
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

pub fn clear_semantic_ivf_session_cache() {
    lock_session_cache().clear();
}

/// Mark IVF sidecar dirty after semantic-affecting mutations.
///
/// Must not swallow durability failures: if the stale bit cannot be written or
/// the on-disk IVF cannot be invalidated, callers must fail the mutation so the
/// rebuild gate cannot keep serving a prior generation.
pub fn mark_semantic_ivf_stale(store: &IndexStore) -> Result<()> {
    if store.get_meta("semantic_ivf_stale")?.as_deref() != Some("1") {
        store.set_meta("semantic_ivf_stale", "1")?;
    }
    lock_session_cache().clear();
    invalidate_semantic_ivf(store.db_path())?;
    Ok(())
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
            store.set_meta("semantic_ivf_stale", "0")?;
            let ivf = Arc::new(ivf);
            cache_session(&db_key, fingerprint, Arc::clone(&ivf));
            return Ok(Some(ivf));
        }
        Ok(None) => {}
        Err(error) => return Err(error),
    }
    let flat = flatten_vectors_for_search(chunks, dim)?;
    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let published = save_semantic_ivf_with_publication(&ivf_path, fingerprint, dim, &flat, &index)?;
    store.set_meta("semantic_ivf_stale", if published { "0" } else { "1" })?;
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
        invalidate_semantic_ivf(store.db_path())?;
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
    let published = save_semantic_ivf_with_publication(
        &semantic_ivf_path(store.db_path()),
        fingerprint,
        dim,
        &vectors,
        &index,
    )?;
    let rebuilt = PersistedSemanticIvf::from_owned(fingerprint, dim, vectors, index);
    cache_session(&db_key, fingerprint, Arc::new(rebuilt));
    store.set_meta("semantic_ivf_stale", if published { "0" } else { "1" })?;
    Ok(true)
}

#[cfg(test)]
mod min_similarity_gate_tests {
    use super::{score_members, DEFAULT_ANN_THRESHOLD, SemanticAnnIndex};
    use ast_sgrep_embed::{top_k_flat_similarity, top_k_similarity, MIN_SIMILARITY};

    /// IVF member scoring and flat top-k must share the ULP-stable exclusive gate.
    #[test]
    fn score_members_rejects_one_ulp_above_min_like_flat() {
        let min = MIN_SIMILARITY;
        let one = f32::from_bits(min.to_bits() + 1);
        let two = f32::from_bits(min.to_bits() + 2);
        // Direct top_k path (same predicate score_members now uses).
        assert!(
            top_k_similarity([(0, one)], 1, Some(min)).is_empty(),
            "1 ULP above min must be excluded"
        );
        assert_eq!(
            top_k_similarity([(0, two)], 1, Some(min)),
            vec![(0, two)]
        );
        // score_members on a 1-d "flat" of constant rows: cosine(query,row)=row[0]
        // when query=[1] and rows are length-1 (cosine degenerates to sign-aware
        // product / norms). Use dim=2 unit rows for true cosine.
        let dim = 2usize;
        let q = [1.0_f32, 0.0];
        let y_one = (1.0 - one * one).sqrt();
        let y_two = (1.0 - two * two).sqrt();
        let flat = vec![one, y_one, two, y_two];
        let members = vec![0usize, 1usize];
        let hits = score_members(&q, &flat, dim, 2, &members, 2);
        let idxs: Vec<usize> = hits.iter().map(|(i, _)| *i).collect();
        assert!(
            !idxs.contains(&0),
            "score_members must exclude sim=1ulp above MIN, got {hits:?}"
        );
        assert!(
            idxs.contains(&1),
            "score_members must keep sim=2ulp above MIN, got {hits:?}"
        );
        let flat_hits = top_k_flat_similarity(&q, &flat, dim, 2, Some(MIN_SIMILARITY));
        let flat_idxs: Vec<usize> = flat_hits.iter().map(|(i, _)| *i).collect();
        assert_eq!(idxs, flat_idxs);
    }

    #[test]
    fn mid_size_ivf_uses_score_members_not_default_threshold_gate() {
        // Override-class corpus: n well below DEFAULT_ANN_THRESHOLD but IVF
        // was built (as load_or_build would under a lowered ann_threshold).
        // Query path must score via clusters (all probes) not silent brute-only.
        let dim = 4usize;
        let n = 128usize;
        assert!(n < DEFAULT_ANN_THRESHOLD);
        let mut flat = Vec::with_capacity(n * dim);
        let mut state = 0xA11_u64;
        for _ in 0..n {
            let start = flat.len();
            for _ in 0..dim {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                flat.push((((state >> 32) as u32) as f32 / u32::MAX as f32) * 2.0 - 1.0);
            }
            ast_sgrep_embed::normalize_vec_in_place(&mut flat[start..start + dim]);
        }
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let q = &flat[..dim];
        assert!(
            !index.candidate_indices(q, Some(usize::MAX)).is_empty(),
            "built IVF must expose cluster members"
        );
        let ivf = index.search_flat_with_probes(&flat, dim, q, 10, Some(usize::MAX));
        let brute = top_k_flat_similarity(
            &ast_sgrep_embed::normalize_vec(q),
            &flat,
            dim,
            10,
            Some(MIN_SIMILARITY),
        );
        let ivf_idx: Vec<usize> = ivf.iter().map(|(i, _)| *i).collect();
        let brute_idx: Vec<usize> = brute.iter().map(|(i, _)| *i).collect();
        assert_eq!(
            ivf_idx, brute_idx,
            "mid-size IVF (all probes) must match flat; was query still gated on DEFAULT_ANN_THRESHOLD?"
        );
    }

    #[test]
    fn ivf_route_above_threshold_matches_flat_on_ulp_boundary_fixture() {
        // Boundary fixture at default ANN size (production build gate).
        let dim = 2usize;
        let n = DEFAULT_ANN_THRESHOLD;
        let min = MIN_SIMILARITY;
        let one = f32::from_bits(min.to_bits() + 1);
        let two = f32::from_bits(min.to_bits() + 2);
        let y_one = (1.0 - one * one).sqrt();
        let y_two = (1.0 - two * two).sqrt();
        // Fill with low-similarity noise, then plant boundary rows at 0 and 1.
        let mut flat = Vec::with_capacity(n * dim);
        for i in 0..n {
            if i == 0 {
                flat.extend_from_slice(&[one, y_one]);
            } else if i == 1 {
                flat.extend_from_slice(&[two, y_two]);
            } else {
                // Nearly orthogonal to [1,0]
                flat.extend_from_slice(&[0.0, 1.0]);
            }
        }
        let index = SemanticAnnIndex::build_from_flat(&flat, dim);
        let q = [1.0_f32, 0.0];
        let ivf: Vec<usize> = index
            .search_flat_with_probes(&flat, dim, &q, 8, Some(usize::MAX))
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        let brute: Vec<usize> = top_k_flat_similarity(&q, &flat, dim, 8, Some(MIN_SIMILARITY))
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert!(
            !ivf.contains(&0) && !brute.contains(&0),
            "1ulp row must be gated out on both paths: ivf={ivf:?} brute={brute:?}"
        );
        assert!(
            ivf.contains(&1) && brute.contains(&1),
            "2ulp row must pass both paths: ivf={ivf:?} brute={brute:?}"
        );
        assert_eq!(ivf, brute);
    }
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

#[cfg(test)]
mod kmeans_flat_tests {
    use super::SemanticAnnIndex;

    fn synthetic_flat(n: usize, dim: usize) -> Vec<f32> {
        let mut flat = Vec::with_capacity(n * dim);
        for i in 0..n {
            for d in 0..dim {
                flat.push(((i * 17 + d * 3) % 97) as f32 * 0.01 + 0.001);
            }
        }
        flat
    }

    #[test]
    fn build_from_flat_is_deterministic_bit_identical_sidecar() {
        let dim = 8usize;
        let n = 64usize;
        let flat = synthetic_flat(n, dim);
        let a = SemanticAnnIndex::build_from_flat(&flat, dim);
        let b = SemanticAnnIndex::build_from_flat(&flat, dim);
        assert!(a.validate_partition(n));
        assert!(b.validate_partition(n));
        let mut wa = Vec::new();
        let mut wb = Vec::new();
        a.write_to(&mut wa, dim).expect("serialize a");
        b.write_to(&mut wb, dim).expect("serialize b");
        assert_eq!(
            wa, wb,
            "two builds on same input must produce bit-identical IVF payload"
        );
        let q = &flat[..dim];
        assert_eq!(
            a.search_flat(&flat, dim, q, 10),
            b.search_flat(&flat, dim, q, 10)
        );
    }

    #[test]
    fn build_from_flat_empty_and_zero_dim() {
        let empty = SemanticAnnIndex::build_from_flat(&[], 8);
        assert!(empty.candidate_indices(&[1.0; 8], Some(1)).is_empty());
        let zero_dim = SemanticAnnIndex::build_from_flat(&[1.0, 2.0], 0);
        assert!(zero_dim.candidate_indices(&[1.0], Some(1)).is_empty());
    }

    #[test]
    fn kmeans_flat_matches_row_layout_reference() {
        // Reference: same algorithm as pre-T1 `&[Vec<f32>]` k-means, for a small
        // fixed matrix. Asserts flat-slice kmeans produces identical centroids.
        let dim = 4usize;
        let n = 12usize;
        let flat = synthetic_flat(n, dim);
        // Normalize like build_from_flat.
        let mut norm = flat.clone();
        for i in 0..n {
            ast_sgrep_embed::normalize_vec_in_place(&mut norm[i * dim..(i + 1) * dim]);
        }
        let rows: Vec<Vec<f32>> = (0..n)
            .map(|i| norm[i * dim..(i + 1) * dim].to_vec())
            .collect();
        let k = ((n as f64).sqrt() as usize).clamp(16, 256).min(n).max(1);
        let (c_flat, a_flat) = super::kmeans(&norm, dim, k, 12);
        let (c_rows, a_rows) = kmeans_row_reference(&rows, k, 12);
        assert_eq!(a_flat, a_rows);
        assert_eq!(c_flat.len(), c_rows.len());
        for (a, b) in c_flat.iter().zip(c_rows.iter()) {
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(
                    x.to_bits(),
                    y.to_bits(),
                    "centroid float bits must match row-layout reference"
                );
            }
        }
    }

    /// Serial row-layout k-means reference for isomorphism (same metric as
    /// production: `dot_similarity` on normalized rows; T1-R). Fully serial
    /// assignment + serial reduce in row order.
    fn kmeans_row_reference(
        vectors: &[Vec<f32>],
        k: usize,
        max_iters: usize,
    ) -> (Vec<Vec<f32>>, Vec<usize>) {
        use ast_sgrep_embed::{dot_similarity, normalize_vec};
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
                            .map(|cent| dot_similarity(v, cent))
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
                let best = centroids
                    .iter()
                    .enumerate()
                    .map(|(ci, c)| (ci, dot_similarity(v, c)))
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(ci, _)| ci)
                    .unwrap_or(0);
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

    fn assert_kmeans_matches_serial_ref(flat: &[f32], dim: usize, max_iters: usize) {
        let n = flat.len() / dim;
        let mut norm = flat.to_vec();
        for i in 0..n {
            ast_sgrep_embed::normalize_vec_in_place(&mut norm[i * dim..(i + 1) * dim]);
        }
        let rows: Vec<Vec<f32>> = (0..n)
            .map(|i| norm[i * dim..(i + 1) * dim].to_vec())
            .collect();
        let k = ((n as f64).sqrt() as usize).clamp(16, 256).min(n).max(1);
        let (c_ref, a_ref) = kmeans_row_reference(&rows, k, max_iters);
        let (c_par, a_par) = super::kmeans(&norm, dim, k, max_iters);
        assert_eq!(
            a_par, a_ref,
            "assignments must match serial row-layout reference (n={n} dim={dim} k={k})"
        );
        assert_eq!(c_par.len(), c_ref.len());
        for (ci, (a, b)) in c_par.iter().zip(c_ref.iter()).enumerate() {
            assert_eq!(a.len(), b.len());
            for (j, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                assert_eq!(
                    x.to_bits(),
                    y.to_bits(),
                    "centroid[{ci}][{j}] bits must match serial ref (n={n} dim={dim})"
                );
            }
        }
    }

    #[test]
    fn kmeans_parallel_matches_serial_on_synthetics() {
        // Deterministic seeds via synthetic_flat formula; vary n/dim to cover
        // k-clamp paths (k=min(n, clamp(sqrt(n),16,256))).
        for &(n, dim) in &[(12, 4), (32, 8), (64, 16), (100, 8), (256, 4)] {
            let flat = synthetic_flat(n, dim);
            assert_kmeans_matches_serial_ref(&flat, dim, 12);
        }
        // Fixed alternate pattern (still deterministic).
        let dim = 6usize;
        let n = 48usize;
        let mut flat = Vec::with_capacity(n * dim);
        for i in 0..n {
            for d in 0..dim {
                flat.push(((i * 31 + d * 7) % 53) as f32 * 0.02 - 0.1);
            }
        }
        assert_kmeans_matches_serial_ref(&flat, dim, 12);
    }

    #[test]
    fn kmeans_bit_identical_under_1_and_4_rayon_threads() {
        // Local pools via install so thread count is controlled even if the
        // global Rayon pool was already initialized by other tests.
        let dim = 8usize;
        let n = 128usize;
        let flat = synthetic_flat(n, dim);
        let mut norm = flat.clone();
        for i in 0..n {
            ast_sgrep_embed::normalize_vec_in_place(&mut norm[i * dim..(i + 1) * dim]);
        }
        let rows: Vec<Vec<f32>> = (0..n)
            .map(|i| norm[i * dim..(i + 1) * dim].to_vec())
            .collect();
        let k = ((n as f64).sqrt() as usize).clamp(16, 256).min(n).max(1);
        let (c_ref, a_ref) = kmeans_row_reference(&rows, k, 12);

        for threads in [1usize, 4usize] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("build rayon pool");
            let (c_par, a_par) = pool.install(|| super::kmeans(&norm, dim, k, 12));
            assert_eq!(
                a_par, a_ref,
                "assignments must match serial ref at RAYON threads={threads}"
            );
            for (a, b) in c_par.iter().zip(c_ref.iter()) {
                for (x, y) in a.iter().zip(b.iter()) {
                    assert_eq!(
                        x.to_bits(),
                        y.to_bits(),
                        "centroid bits must match at threads={threads}"
                    );
                }
            }
        }
    }
}
