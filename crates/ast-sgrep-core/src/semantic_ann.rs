use crate::semantic_ivf::{
    compute_ann_fingerprint, invalidate_semantic_ivf, load_semantic_ivf,
    load_semantic_ivf_unchecked, save_semantic_ivf_with_publication, semantic_ivf_path,
    PersistedSemanticIvf,
};
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, normalize_vec, normalize_vec_in_place,
    top_k_flat_similarity, top_k_similarity, SemanticChunkRow, MIN_SIMILARITY,
    PARALLEL_CHUNK_THRESHOLD,
};
use rayon::prelude::*;
use std::io::Write;
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
        write_usize_u32(writer, self.centroids.len())?;
        for c in &self.centroids {
            for &v in c {
                writer.write_all(&v.to_le_bytes())?;
            }
            for _ in c.len()..dim {
                writer.write_all(&0.0f32.to_le_bytes())?;
            }
        }
        write_usize_u32(writer, self.clusters.len())?;
        for cluster in &self.clusters {
            write_usize_u32(writer, cluster.len())?;
            for &idx in cluster {
                write_usize_u32(writer, idx)?;
            }
        }
        Ok(())
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

    pub fn centroids(&self) -> &[Vec<f32>] {
        &self.centroids
    }

    pub fn centroid_count(&self) -> usize {
        self.centroids.len()
    }

    /// `probes`: None/0 = at most 90% of populated clusters (capped at sqrt(k) in 16..=48 once n>10_000); ≥ n_clusters = exact.
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
            None | Some(0) if populated > 1 => {
                let pct = populated
                    .saturating_mul(DEFAULT_ADAPTIVE_PROBE_PERCENT)
                    .div_euclid(100)
                    .clamp(1, populated - 1);
                // 90% at 54k is nearly exhaustive (~49k candidates). Keep the
                // published 2048/10000 recall gate, but bound nprobe once the
                // corpus is larger than that fixture.
                let n = self.clusters.iter().map(Vec::len).sum::<usize>();
                if n > 10_000 {
                    let bounded = ((populated as f64).sqrt() as usize).clamp(16, 48);
                    pct.min(bounded).clamp(1, populated - 1)
                } else {
                    pct
                }
            }
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
    /// Keep existing centroids and rebuild cluster membership for `flat`.
    ///
    /// Delta reindex uses this so a chunk-count change does not pay full k-means.
    /// Returns false when this index cannot reassign (empty or dim-mismatched
    /// centroids); the caller should fall through to `build_from_flat`.
    pub fn reassign_all(&mut self, flat: &[f32], dim: usize) -> bool {
        let _span = crate::perf_profile::Span::start(
            "semantic_ivf_reassign",
            "semantic",
            "SemanticAnnIndex::reassign_all (keep centroids)",
        );
        let n = flat.len().checked_div(dim).unwrap_or(0);
        if n == 0 || dim == 0 || self.centroids.is_empty() {
            return false;
        }
        if self.centroids.iter().any(|centroid| centroid.len() != dim) {
            return false;
        }
        let mut owned = flat.to_vec();
        for i in 0..n {
            normalize_vec_in_place(&mut owned[i * dim..(i + 1) * dim]);
        }
        let assignments: Vec<usize> = (0..n)
            .into_par_iter()
            .map(|i| nearest_centroid(flat_row(&owned, dim, i), &self.centroids))
            .collect();
        let mut clusters = vec![Vec::new(); self.centroids.len()];
        for (idx, &cluster) in assignments.iter().enumerate() {
            clusters[cluster].push(idx);
        }
        self.clusters = clusters;
        true
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
    // Reject before allocating or validating rows so hostile dim×len cannot OOM.
    let total = chunks.len().checked_mul(dim).ok_or_else(|| {
        crate::StoreError::Other(format!(
            "semantic flatten overflow: {} chunks × dim {} exceeds addressable size",
            chunks.len(),
            dim
        ))
    })?;
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.5.len() != dim {
            return Err(crate::StoreError::Other(format!(
                "semantic chunk {} has dimension {} but expected {} (mixed backends or corrupted store; reindex with --force-reindex)", i, chunk.5.len(), dim
            )));
        }
    }
    let mut flat = vec![0.0f32; total];
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
/// IVF on-disk fields are little-endian u32. Fail closed instead of truncating
/// (pb2w). Format itself is unchanged.
fn write_usize_u32<W: Write>(w: &mut W, value: usize) -> std::io::Result<()> {
    let value = u32::try_from(value).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "semantic IVF exceeds the u32 on-disk format",
        )
    })?;
    write_u32(w, value)
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
            // IVF payload and `search_flat_with_probes` query are L2-normalized,
            // so cosine == dot. One SIMD dot beats three-norm cosine on the
            // few-thousand-member probe set.
            .then(|| dot_similarity(query, &flat[start..start + dim]))
            .map(|sim| (*idx, sim))
    };
    // Sequential on purpose: a few thousand SIMD dots are cheaper than a
    // rayon wakeup on the 1–2 ms semantic-only budget.
    top_k_similarity(
        members.iter().filter_map(score),
        limit,
        Some(MIN_SIMILARITY),
    )
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
    // Rows and centroids are L2-normalized during build and after updates, so
    // cosine equals the plain inner product. Keep the same max and lowest-index
    // tie-break as the previous cosine path.
    centroids
        .iter()
        .enumerate()
        .map(|(ci, c)| (ci, dot_similarity(vector, c)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(ci, _)| ci)
        .unwrap_or(0)
}
/// Deterministic k-means over a row-major flat matrix (`n * dim` floats).
fn kmeans(flat: &[f32], dim: usize, k: usize, max_iters: usize) -> (Vec<Vec<f32>>, Vec<usize>) {
    let n = flat.len().checked_div(dim).unwrap_or(0);
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
        let changed = next.iter().zip(assignments.iter()).any(|(a, b)| a != b);
        assignments = next;
        if !changed {
            break;
        }
        // Serial reduction in ascending row order so f32 sums match the serial
        // algorithm bit-for-bit (parallel float reduce is forbidden).
        let mut sums = vec![vec![0.0f32; dim]; k];
        let mut counts = vec![0usize; k];
        for (i, &c) in assignments.iter().enumerate() {
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
                    let mut mean = sum.clone();
                    for value in &mut mean {
                        *value /= count as f32;
                    }
                    normalize_vec_in_place(&mut mean);
                    mean
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
/// Keeps the on-disk sidecar so a later delta rebuild can reassign members to
/// existing centroids. Search still ignores the file on fingerprint mismatch.
/// Call [`drop_semantic_ivf`] when the centroid set itself must die (full wipe
/// or embedding-identity rewrite).
pub fn mark_semantic_ivf_stale(store: &IndexStore) -> Result<()> {
    if store.get_meta("semantic_ivf_stale")?.as_deref() != Some("1") {
        store.set_meta("semantic_ivf_stale", "1")?;
    }
    lock_session_cache().clear();
    Ok(())
}

/// Drop the IVF sidecar and mark it stale. Used on semantic wipes so the next
/// rebuild cannot reassign onto a centroid set that no longer matches the store.
pub fn drop_semantic_ivf(store: &IndexStore) -> Result<()> {
    mark_semantic_ivf_stale(store)?;
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
            // Search holds a read snapshot. Do not attempt a read-to-write
            // upgrade here: a concurrent index commit would make it fail with
            // SQLITE_BUSY_SNAPSHOT. The fingerprint remains authoritative.
            if store.connection().is_autocommit() {
                store.set_meta("semantic_ivf_stale", "0")?;
            }
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
    if store.connection().is_autocommit() {
        store.set_meta("semantic_ivf_stale", if published { "0" } else { "1" })?;
    }
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
    if chunks.is_empty() || limit == 0 {
        return Ok(vec![]);
    }
    let dim = chunks[0].5.len();
    // Zero-dim embeddings are corrupt/unset; never divide or rank them.
    if dim == 0 {
        return Err(crate::StoreError::Other(
            "semantic embedding dimension is 0 (corrupt store or unset backend; reindex)".into(),
        ));
    }
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

/// When the IVF sidecar is marked stale, reassign every current vector to the
/// persisted centroids and rewrite postings. Chunk-count drift is expected on
/// real edits; only dim mismatch, a missing sidecar, or empty centroids fall
/// through to full k-means.
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
    if ivf.dim != dim || ivf.index.centroid_count() == 0 {
        return Ok(false);
    }
    let vectors = flatten_vectors_for_search(chunks, dim)?;
    let mut index = ivf.index.clone();
    drop(ivf);
    if !index.reassign_all(&vectors, dim) {
        return Ok(false);
    }
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
