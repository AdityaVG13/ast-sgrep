//! Session cache and ranking entry points for semantic ANN search.

use std::sync::{Arc, Mutex};

use ast_sgrep_embed::SemanticChunkRow;

use crate::semantic_ivf::{
    compute_ann_fingerprint, invalidate_semantic_ivf, load_semantic_ivf, save_semantic_ivf,
    semantic_ivf_path, PersistedSemanticIvf,
};
use crate::store::IndexStore;
use crate::Result;

use super::index::SemanticAnnIndex;
use super::vector::{brute_force_flat, flatten_vectors};

pub const DEFAULT_ANN_THRESHOLD: usize = 2_000;

struct SessionCache {
    fingerprint: [u8; 32],
    ivf: Arc<PersistedSemanticIvf>,
}

static SESSION_CACHE: Mutex<Option<(String, SessionCache)>> = Mutex::new(None);

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
