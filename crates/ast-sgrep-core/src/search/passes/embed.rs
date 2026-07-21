use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::semantic_ann::rank_chunk_indices_flat;
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::{embed_query, SemanticChunkRow};
use std::collections::HashMap;
use std::sync::Arc;
const EMBED_HIT_LIMIT: usize = 50;
pub struct EmbedContext {
    pub chunks: Arc<Vec<SemanticChunkRow>>,
    pub flat_vectors: Arc<Vec<f32>>,
}
pub fn embed_pass_lazy_ivf(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Option<Vec<SearchHit>>> {
    if parsed.terms.is_empty() || !options.use_embed {
        return Ok(Some(Vec::new()));
    }
    let stats = store.semantic_chunk_stats(options.lang_filter.as_deref())?;
    if !crate::semantic_ann::should_use_ann(stats.count, options.ann_threshold) || stats.dim == 0 {
        return Ok(None);
    }
    let backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    let fingerprint = crate::semantic_ivf::compute_ann_fingerprint(
        stats.count,
        stats.max_id,
        stats.dim,
        Some(&backend),
    );
    let path = crate::semantic_ivf::semantic_ivf_path(store.db_path());
    let Some(ivf) = crate::semantic_ivf::load_semantic_ivf_index(&path, fingerprint)? else {
        return Ok(None);
    };
    if ivf.chunk_count() != stats.count || ivf.dim != stats.dim {
        return Ok(None);
    }
    let query = parsed.terms.join(" ");
    let query_vec = embed_query_vector(store, options, &query, Some(stats.dim))?;
    let candidate_indices = ivf.candidate_indices(&query_vec, options.ann_probes);
    if candidate_indices.is_empty() {
        return Ok(None);
    }
    let ids = store.semantic_chunk_ids(options.lang_filter.as_deref())?;
    if ids.len() != stats.count {
        return Ok(None);
    }
    let candidate_ids: Vec<i64> = candidate_indices
        .iter()
        .filter_map(|&idx| ids.get(idx).copied())
        .collect();
    if candidate_ids.len() != candidate_indices.len() {
        return Ok(None);
    }
    let mut rows: HashMap<i64, SemanticChunkRow> = store
        .semantic_chunks_by_ids(&candidate_ids)?
        .into_iter()
        .collect();
    let mut chunks = Vec::with_capacity(candidate_ids.len());
    for id in candidate_ids {
        let Some(row) = rows.remove(&id) else {
            return Ok(None);
        };
        chunks.push(row);
    }
    let ranked =
        ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &chunks, EMBED_HIT_LIMIT);
    Ok(Some(embed_similarity_hits(&chunks, ranked)))
}
pub fn embed_pass_with_context(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    ctx: Option<EmbedContext>,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || !options.use_embed {
        return Ok(Vec::new());
    }
    let query = parsed.terms.join(" ");
    let owned;
    let chunks: &[SemanticChunkRow] = match &ctx {
        Some(ctx) => &ctx.chunks,
        None => {
            owned = store.all_semantic_chunks(options.lang_filter.as_deref())?;
            &owned
        }
    };
    if chunks.is_empty() {
        return embed_legacy_hits(store, options, &query);
    }
    let flat = ctx.as_ref().map(|c| c.flat_vectors.as_slice());
    let query_vec = embed_query_vector(store, options, &query, chunks.first().map(|c| c.5.len()))?;
    let indices = rank_chunk_indices_flat(
        store,
        &query_vec,
        chunks,
        flat,
        EMBED_HIT_LIMIT,
        options.ann_threshold,
    )?;
    Ok(embed_similarity_hits(chunks, indices))
}
fn embed_query_vector(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
    stored_dim: Option<usize>,
) -> Result<Vec<f32>> {
    use std::sync::{Mutex, OnceLock};
    static QCACHE: OnceLock<Mutex<HashMap<String, Vec<f32>>>> = OnceLock::new();
    let stored_backend = store.get_meta("embed_backend")?;
    if stored_backend.as_deref() == Some("neural") {
        let stored_model = store.get_meta("embed_model")?;
        let active_model = ast_sgrep_embed::neural_configured_model_id();
        if stored_model.as_deref() != Some(active_model) {
            return Err(crate::StoreError::Other(format!( "stored neural model {:?} does not match active model {active_model}; reindex with: asgrep reindex",
                stored_model.as_deref().unwrap_or("unknown")
            )));
        }
    }
    let dim = stored_dim.unwrap_or(ast_sgrep_embed::default_semantic_dim());
    let cache_key = format!(
        "{}|{}|{}|{:?}",
        query,
        stored_backend.as_deref().unwrap_or(""),
        dim,
        options.embed_preference()
    );
    {
        let cache = QCACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(guard) = cache.lock() {
            if let Some(v) = guard.get(&cache_key) {
                return Ok(v.clone());
            }
        }
    }
    let vector = embed_query(
        query,
        stored_backend.as_deref(),
        dim,
        options.embed_preference(),
    )
    .map_err(crate::StoreError::Other)?
    .vector;
    if let Ok(mut guard) = QCACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        if guard.len() < 64 {
            guard.insert(cache_key, vector.clone());
        }
    }
    Ok(vector)
}
fn embed_similarity_hits(chunks: &[SemanticChunkRow], ranked: Vec<(usize, f32)>) -> Vec<SearchHit> {
    ranked
        .into_iter()
        .map(|(idx, sim)| {
            let (file, line_start, line_end, symbol, excerpt, _) = &chunks[idx];
            SearchHit::span(SpanHitInput {
                kind: HitKind::Embed,
                file: file.clone(),
                line_start: *line_start,
                line_end: *line_end,
                score: SCORE_EMBED * f64::from(sim),
                excerpt: excerpt.clone(),
                symbol: (!symbol.is_empty()).then_some(symbol.clone()),
                language: None,
            })
        })
        .collect()
}
fn embed_legacy_hits(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
) -> Result<Vec<SearchHit>> {
    let chunks = store.all_legacy_embeddings(options.lang_filter.as_deref())?;
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let query_vec = embed_query_vector(store, options, query, chunks.first().map(|c| c.5.len()))?;
    Ok(embed_similarity_hits(
        &chunks,
        ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &chunks, EMBED_HIT_LIMIT),
    ))
}
