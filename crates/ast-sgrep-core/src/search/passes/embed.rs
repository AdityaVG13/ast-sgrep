use crate::intent::{classify, QueryIntent};
use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::search::field_weight::{field_weights, rescore_similarity, EmbedFieldScores};
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::semantic_ann::{flatten_vectors_for_search, rank_chunk_indices_flat};
use crate::semantic_chunk::SemanticFieldVectors;
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::{embed_query, SemanticChunkRow};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
const EMBED_HIT_LIMIT: usize = 50;
pub struct EmbedContext {
    pub chunks: Arc<Vec<SemanticChunkRow>>,
    pub flat_vectors: Arc<Vec<f32>>,
}

pub(crate) struct SemanticCache {
    lang_filter: Option<String>,
    max_id: i64,
    index_data_version: i64,
    semantic_data_version: i64,
    embed_backend: String,
    /// SQLite `PRAGMA data_version` at load time (br-yp1). Bumps on EVERY
    /// committed database write — including foreign raw-SQL mutations through
    /// a separate connection that move none of the local counters above — so
    /// a cached chunk set can never outlive an external writer's commit.
    /// `None` means the pragma was unreadable and the context was NOT cached.
    data_version: Option<i64>,
    chunks: Arc<Vec<SemanticChunkRow>>,
    flat_vectors: Arc<Vec<f32>>,
}

fn lock_clear_on_poison<T>(mutex: &Mutex<T>, clear: impl FnOnce(&mut T)) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            mutex.clear_poison();
            let mut guard = PoisonError::into_inner(poisoned);
            clear(&mut guard);
            guard
        }
    }
}

pub(crate) fn load_semantic_context(
    store: &IndexStore,
    options: &SearchOptions,
    cache: &Mutex<Option<SemanticCache>>,
) -> Result<Option<EmbedContext>> {
    if !options.use_embed {
        return Ok(None);
    }
    let lang_filter = options.lang_filter.clone();
    let max_id = store.semantic_chunk_max_id()?.unwrap_or(0);
    let index_data_version = store.index_data_version()?;
    let semantic_data_version = store.semantic_data_version()?;
    // br-yp1: the local counters above miss foreign raw-SQL mutations. SQLite's
    // PRAGMA data_version bumps on every committed write by ANY connection; an
    // unreadable pragma fails closed (no caching) rather than pinning a value.
    let data_version = store
        .connection()
        .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
        .ok();
    let embed_backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    {
        let guard = lock_clear_on_poison(cache, |slot| {
            *slot = None;
        });
        if let Some(c) = guard.as_ref() {
            if c.lang_filter == lang_filter
                && c.max_id == max_id
                && c.index_data_version == index_data_version
                && c.semantic_data_version == semantic_data_version
                && c.embed_backend == embed_backend
                && c.data_version.is_some()
                && c.data_version == data_version
            {
                return Ok(Some(EmbedContext {
                    chunks: Arc::clone(&c.chunks),
                    flat_vectors: Arc::clone(&c.flat_vectors),
                }));
            }
        }
    }
    let chunks = store.all_semantic_chunks(lang_filter.as_deref())?;
    if chunks.is_empty() {
        return Ok(None);
    }
    let flat_vectors = flatten_vectors_for_search(&chunks, chunks[0].5.len())?;
    let entry = SemanticCache {
        lang_filter,
        max_id,
        index_data_version,
        semantic_data_version,
        embed_backend,
        data_version,
        chunks: Arc::new(chunks),
        flat_vectors: Arc::new(flat_vectors),
    };
    let ctx = EmbedContext {
        chunks: Arc::clone(&entry.chunks),
        flat_vectors: Arc::clone(&entry.flat_vectors),
    };
    *lock_clear_on_poison(cache, |slot| {
        *slot = None;
    }) = Some(entry);
    Ok(Some(ctx))
}

pub(crate) fn run_embed_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    cache: &Mutex<Option<SemanticCache>>,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    if let Some(hits) =
        embed_pass_lazy_ivf_with_rescoring(store, options, parsed, use_field_rescoring)?
    {
        return Ok(hits);
    }
    match load_semantic_context(store, options, cache)? {
        Some(ctx) => embed_pass_with_context_and_rescoring(
            store,
            options,
            parsed,
            Some(ctx),
            use_field_rescoring,
        ),
        None => Ok(vec![]),
    }
}

pub fn embed_pass_lazy_ivf(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Option<Vec<SearchHit>>> {
    embed_pass_lazy_ivf_with_rescoring(store, options, parsed, true)
}

pub(crate) fn embed_pass_lazy_ivf_with_rescoring(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    use_field_rescoring: bool,
) -> Result<Option<Vec<SearchHit>>> {
    if parsed.terms.is_empty() || !options.use_embed {
        return Ok(Some(Vec::new()));
    }
    if options.lang_filter.is_some() {
        return Ok(None);
    }
    let (ids, dim) = cached_semantic_chunk_ids(store)?;
    let count = ids.len();
    let max_id = ids.last().copied().unwrap_or(0);
    if !crate::semantic_ann::should_use_ann(count, options.ann_threshold) || dim == 0 {
        return Ok(None);
    }
    let backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    let fingerprint = crate::semantic_ivf::compute_ann_fingerprint(
        count,
        max_id,
        dim,
        Some(&backend),
        store.index_data_version()?,
    );
    let path = crate::semantic_ivf::semantic_ivf_path(store.db_path());
    let Some(ivf) = crate::semantic_ivf::load_semantic_ivf_index(&path, fingerprint)? else {
        return Ok(None);
    };
    if ivf.chunk_count() != count || ivf.dim != dim {
        return Ok(None);
    }
    let query = parsed.terms.join(" ");
    let query_vec = embed_query_vector(store, options, &query, Some(dim))?;
    let intent = classify(parsed);
    let hit_limit = EMBED_HIT_LIMIT.max(options.limit);
    let pool = field_rescore_pool(hit_limit);
    // Door B: rank from the mmap'd concat payload, then SQLite-fetch only the
    // top-N survivors. Door C: field blobs are selected only for those ids
    // and only for intent-weighted columns.
    let ranked_payload = match ivf.search(&query_vec, pool, options.ann_probes) {
        Some(ranked) if !ranked.is_empty() => ranked,
        Some(_) => {
            return if ivf
                .candidate_indices(&query_vec, options.ann_probes)
                .is_empty()
            {
                Ok(None)
            } else {
                Ok(Some(Vec::new()))
            };
        }
        None => {
            let candidate_indices = ivf.candidate_indices(&query_vec, options.ann_probes);
            if candidate_indices.is_empty() {
                return Ok(None);
            }
            let candidate_ids: Vec<i64> = candidate_indices
                .iter()
                .filter_map(|&idx| ids.get(idx).copied())
                .collect();
            if candidate_ids.len() != candidate_indices.len() {
                return Ok(None);
            }
            let Some(chunks) = rows_in_id_order_with_vectors(store, &candidate_ids)? else {
                return Ok(None);
            };
            return Ok(Some(embed_hits_from_concat_rank(
                store,
                &chunks,
                &candidate_ids,
                &query_vec,
                intent,
                hit_limit,
                use_field_rescoring,
            )?));
        }
    };
    let candidate_ids: Vec<i64> = ranked_payload
        .iter()
        .filter_map(|(idx, _)| ids.get(*idx).copied())
        .collect();
    if candidate_ids.len() != ranked_payload.len() {
        return Ok(None);
    }
    let Some(chunks) = rows_in_id_order(store, &candidate_ids)? else {
        return Ok(None);
    };
    let ranked: Vec<(usize, f32)> = ranked_payload
        .iter()
        .enumerate()
        .map(|(i, (_, score))| (i, *score))
        .collect();
    let fields = fields_for_ids(store, &candidate_ids, use_field_rescoring, intent)?;
    Ok(Some(embed_hits_rescored(
        &chunks,
        ranked,
        &query_vec,
        &fields,
        intent,
        hit_limit,
    )))
}

/// Hybrid cascade: score only IVF mmap rows whose path is in `allowed_files`.
/// Avoids SQLite-fetching concat blobs for every survivor file (~29 ms at 54k).
fn embed_pass_lazy_ivf_for_files(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
    use_field_rescoring: bool,
) -> Result<Option<Vec<SearchHit>>> {
    if parsed.terms.is_empty() || !options.use_embed || allowed_files.is_empty() {
        return Ok(Some(Vec::new()));
    }
    if options.lang_filter.is_some() {
        return Ok(None);
    }
    let (ids, paths, dim) = cached_semantic_chunk_index(store)?;
    let count = ids.len();
    if paths.len() != count {
        return Ok(None);
    }
    let max_id = ids.last().copied().unwrap_or(0);
    if !crate::semantic_ann::should_use_ann(count, options.ann_threshold) || dim == 0 {
        return Ok(None);
    }
    let backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    let fingerprint = crate::semantic_ivf::compute_ann_fingerprint(
        count,
        max_id,
        dim,
        Some(&backend),
        store.index_data_version()?,
    );
    let path = crate::semantic_ivf::semantic_ivf_path(store.db_path());
    let Some(ivf) = crate::semantic_ivf::load_semantic_ivf_index(&path, fingerprint)? else {
        return Ok(None);
    };
    if ivf.chunk_count() != count || ivf.dim != dim {
        return Ok(None);
    }
    let members: Vec<usize> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| allowed_files.contains(p.as_str()))
        .map(|(i, _)| i)
        .collect();
    if members.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let query = parsed.terms.join(" ");
    let query_vec = embed_query_vector(store, options, &query, Some(dim))?;
    let intent = classify(parsed);
    let hit_limit = EMBED_HIT_LIMIT.max(options.limit);
    let pool = field_rescore_pool(hit_limit);
    let Some(ranked_payload) = ivf.search_members(&query_vec, &members, pool) else {
        return Ok(None);
    };
    if ranked_payload.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let candidate_ids: Vec<i64> = ranked_payload
        .iter()
        .filter_map(|(idx, _)| ids.get(*idx).copied())
        .collect();
    if candidate_ids.len() != ranked_payload.len() {
        return Ok(None);
    }
    let Some(chunks) = rows_in_id_order(store, &candidate_ids)? else {
        return Ok(None);
    };
    let ranked: Vec<(usize, f32)> = ranked_payload
        .iter()
        .enumerate()
        .map(|(i, (_, score))| (i, *score))
        .collect();
    let fields = fields_for_ids(store, &candidate_ids, use_field_rescoring, intent)?;
    Ok(Some(embed_hits_rescored(
        &chunks,
        ranked,
        &query_vec,
        &fields,
        intent,
        hit_limit,
    )))
}

pub fn embed_pass_for_files(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    embed_pass_for_files_with_rescoring(store, options, parsed, allowed_files, true)
}

pub(crate) fn embed_pass_for_files_with_rescoring(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || !options.use_embed || allowed_files.is_empty() {
        return Ok(Vec::new());
    }
    // gauntlet-r4 (E1): when both persistent semantic sources are globally
    // empty, the three per-file fetch loops below provably return nothing for
    // ANY allowed_files set — skip them. Output-identical by construction:
    // with zero chunks and zero embeddings every loop contributes no rows and
    // `survivors.is_empty()` returns Ok(Vec::new()) anyway; this only skips
    // the work of proving it one point-query at a time. Non-empty stores pay
    // one microsecond-scale EXISTS probe per query.
    if store.semantic_sources_empty()? {
        return Ok(Vec::new());
    }
    if let Some(hits) = embed_pass_lazy_ivf_for_files(
        store,
        options,
        parsed,
        allowed_files,
        use_field_rescoring,
    )? {
        return Ok(hits);
    }
    let query = parsed.terms.join(" ");
    let intent = classify(parsed);
    let hit_limit = EMBED_HIT_LIMIT.max(options.limit);
    let mut survivors = Vec::new();
    let mut survivor_ids = Vec::new();
    for (id, row) in store.semantic_chunks_for_files(allowed_files, options.lang_filter.as_deref())?
    {
        survivor_ids.push(id);
        survivors.push(row);
    }
    let modern_files = survivors
        .iter()
        .map(|chunk| chunk.0.clone())
        .collect::<HashSet<_>>();
    let legacy_only_files = allowed_files
        .difference(&modern_files)
        .cloned()
        .collect::<HashSet<_>>();
    let legacy =
        store.legacy_embeddings_for_files(&legacy_only_files, options.lang_filter.as_deref())?;
    survivor_ids.extend(std::iter::repeat_n(0, legacy.len()));
    survivors.extend(legacy);
    if survivors.is_empty() {
        return Ok(Vec::new());
    }
    let query_vec = embed_query_vector(
        store,
        options,
        &query,
        survivors.first().map(|chunk| chunk.5.len()),
    )?;
    embed_hits_from_concat_rank(
        store,
        &survivors,
        &survivor_ids,
        &query_vec,
        intent,
        hit_limit,
        use_field_rescoring,
    )
}

pub fn embed_pass_with_context(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    ctx: Option<EmbedContext>,
) -> Result<Vec<SearchHit>> {
    embed_pass_with_context_and_rescoring(store, options, parsed, ctx, true)
}

pub(crate) fn embed_pass_with_context_and_rescoring(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    ctx: Option<EmbedContext>,
    use_field_rescoring: bool,
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
    let ann_threshold = if options.lang_filter.is_some() {
        Some(usize::MAX)
    } else {
        options.ann_threshold
    };
    let indices =
        rank_chunk_indices_flat(store, &query_vec, chunks, flat, chunks.len(), ann_threshold)?;
    let intent = classify(parsed);
    let hit_limit = EMBED_HIT_LIMIT.max(options.limit);
    let ids = store.semantic_chunk_ids(options.lang_filter.as_deref())?;
    // Same JOIN + ORDER BY sc.id as all_semantic_chunks. Length mismatch
    // means skip rescoring rather than pairing the wrong field vectors.
    if ids.len() != chunks.len() {
        return Ok(embed_hits_rescored(
            chunks,
            indices,
            &query_vec,
            &[],
            intent,
            hit_limit,
        ));
    }
    Ok(embed_hits_from_pre_rank(
        store,
        chunks,
        &ids,
        indices,
        &query_vec,
        intent,
        hit_limit,
        use_field_rescoring,
    )?)
}
/// Process-wide query embedding cache (query|backend|model|dim|pref → vector).
/// Poison fails closed: clear the map before reuse (sxjc / pass11).
static QUERY_EMBED_CACHE: OnceLock<Mutex<HashMap<String, Vec<f32>>>> = OnceLock::new();
const QUERY_EMBED_CACHE_CAP: usize = 64;

struct ChunkIdMemo {
    db: String,
    index_data_version: i64,
    semantic_data_version: i64,
    ids: Arc<Vec<i64>>,
    paths: Arc<Vec<String>>,
    dim: usize,
}

/// `SELECT id FROM semantic_chunks ORDER BY id` is ~12 ms at 54k rows. The IVF
/// payload order is that same id list, so cache it per store generation.
static CHUNK_ID_CACHE: OnceLock<Mutex<Option<ChunkIdMemo>>> = OnceLock::new();

fn chunk_id_cache() -> &'static Mutex<Option<ChunkIdMemo>> {
    CHUNK_ID_CACHE.get_or_init(|| Mutex::new(None))
}

fn cached_semantic_chunk_ids(store: &IndexStore) -> Result<(Arc<Vec<i64>>, usize)> {
    let (ids, _paths, dim) = cached_semantic_chunk_index(store)?;
    Ok((ids, dim))
}

fn cached_semantic_chunk_index(
    store: &IndexStore,
) -> Result<(Arc<Vec<i64>>, Arc<Vec<String>>, usize)> {
    let index_data_version = store.index_data_version()?;
    let semantic_data_version = store.semantic_data_version()?;
    let db = store.db_path().to_string_lossy().into_owned();
    {
        let guard = lock_clear_on_poison(chunk_id_cache(), |slot| {
            *slot = None;
        });
        if let Some(memo) = guard.as_ref() {
            if memo.db == db
                && memo.index_data_version == index_data_version
                && memo.semantic_data_version == semantic_data_version
            {
                return Ok((Arc::clone(&memo.ids), Arc::clone(&memo.paths), memo.dim));
            }
        }
    }
    let pairs = store.semantic_chunk_ids_and_paths()?;
    let mut ids = Vec::with_capacity(pairs.len());
    let mut paths = Vec::with_capacity(pairs.len());
    for (id, path) in pairs {
        ids.push(id);
        paths.push(path);
    }
    let ids = Arc::new(ids);
    let paths = Arc::new(paths);
    let dim = store.semantic_primary_dim()?;
    *lock_clear_on_poison(chunk_id_cache(), |slot| {
        *slot = None;
    }) = Some(ChunkIdMemo {
        db,
        index_data_version,
        semantic_data_version,
        ids: Arc::clone(&ids),
        paths: Arc::clone(&paths),
        dim,
    });
    Ok((ids, paths, dim))
}

fn query_embed_cache() -> &'static Mutex<HashMap<String, Vec<f32>>> {
    QUERY_EMBED_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn embed_query_vector(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
    stored_dim: Option<usize>,
) -> Result<Vec<f32>> {
    let stored_backend = store.get_meta("embed_backend")?;
    let stored_model = store.get_meta("embed_model")?;
    let dim = stored_dim.unwrap_or(ast_sgrep_embed::default_semantic_dim());
    // An unversioned embed_backend="semantic" store must not serve results —
    // only a full rewrite (index_all) may promote the layout.
    if store.needs_legacy_semantic_rewrite()? {
        return Err(crate::StoreError::Other(
            "index advertises an unversioned semantic backend; run `asgrep reindex` to rewrite every chunk before semantic search"
                .into(),
        ));
    }
    if let Some(backend_name) = stored_backend.as_deref() {
        let backend = ast_sgrep_embed::EmbedBackendKind::parse(backend_name).ok_or_else(|| {
            crate::StoreError::Other(format!("unknown stored embedding backend {backend_name:?}"))
        })?;
        let active_model = ast_sgrep_embed::configured_backend_model_id(backend, dim);
        if stored_model != active_model {
            return Err(crate::StoreError::Other(format!(
                "stored embedding model {:?} does not match active model {:?}; reindex with: asgrep reindex",
                stored_model.as_deref().unwrap_or("unknown"),
                active_model.as_deref().unwrap_or("unavailable")
            )));
        }
    }
    let cache_key = format!(
        "{}|{}|{}|{}|{:?}",
        query,
        stored_backend.as_deref().unwrap_or(""),
        stored_model.as_deref().unwrap_or(""),
        dim,
        options.embed_preference()
    );
    {
        let guard = lock_clear_on_poison(query_embed_cache(), |map| map.clear());
        if let Some(v) = guard.get(&cache_key) {
            return Ok(v.clone());
        }
    }
    let _span = crate::perf_profile::Span::start(
        "embed_query",
        "semantic",
        "hashed/neural query embed (cache miss)",
    );
    let vector = embed_query(
        query,
        stored_backend.as_deref(),
        dim,
        options.embed_preference(),
    )
    .map_err(crate::StoreError::Other)?
    .vector;
    {
        let mut guard = lock_clear_on_poison(query_embed_cache(), |map| map.clear());
        if guard.len() < QUERY_EMBED_CACHE_CAP {
            guard.insert(cache_key, vector.clone());
        }
    }
    Ok(vector)
}
fn field_rescore_pool(hit_limit: usize) -> usize {
    // IVF mmap already ranked candidates. Fetching 8× hit_limit (400 at the
    // default embed cap of 50) from SQLite was the unique-query floor.
    // Field-rescore among the concat top-N; N = returned hit cap, min 64.
    hit_limit.max(64)
}

fn rows_in_id_order_with_vectors(
    store: &IndexStore,
    ids: &[i64],
) -> Result<Option<Vec<SemanticChunkRow>>> {
    assemble_rows_in_id_order(store.semantic_chunks_by_ids(ids)?, ids)
}

fn rows_in_id_order(
    store: &IndexStore,
    ids: &[i64],
) -> Result<Option<Vec<SemanticChunkRow>>> {
    let _span = crate::perf_profile::Span::start(
        "semantic_hit_fetch",
        "semantic",
        "sqlite metadata for IVF survivors (no concat blob)",
    );
    assemble_rows_in_id_order(store.semantic_chunk_hits_by_ids(ids)?, ids)
}

fn assemble_rows_in_id_order(
    fetched: Vec<(i64, SemanticChunkRow)>,
    ids: &[i64],
) -> Result<Option<Vec<SemanticChunkRow>>> {
    let mut rows: HashMap<i64, SemanticChunkRow> = fetched.into_iter().collect();
    let mut chunks = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(row) = rows.remove(id) else {
            return Ok(None);
        };
        chunks.push(row);
    }
    Ok(Some(chunks))
}

fn fields_for_ids(
    store: &IndexStore,
    ids: &[i64],
    use_field_rescoring: bool,
    intent: QueryIntent,
) -> Result<Vec<SemanticFieldVectors>> {
    let mask = field_weights(intent).mask();
    if !use_field_rescoring || !mask.any() || ids.is_empty() {
        return Ok(Vec::new());
    }
    let field_map = store.semantic_field_vectors_by_ids(ids, mask)?;
    Ok(ids
        .iter()
        .map(|id| field_map.get(id).cloned().unwrap_or_default())
        .collect())
}

fn embed_hits_from_pre_rank(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    ids: &[i64],
    ranked: Vec<(usize, f32)>,
    query_vec: &[f32],
    intent: QueryIntent,
    hit_limit: usize,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    let pool = field_rescore_pool(hit_limit);
    let taken: Vec<(usize, f32)> = ranked.into_iter().take(pool).collect();
    let pool_chunks: Vec<SemanticChunkRow> = taken
        .iter()
        .map(|(idx, _)| chunks[*idx].clone())
        .collect();
    let pool_ids: Vec<i64> = taken.iter().map(|(idx, _)| ids[*idx]).collect();
    let remapped: Vec<(usize, f32)> = taken
        .into_iter()
        .enumerate()
        .map(|(i, (_, score))| (i, score))
        .collect();
    let fields = fields_for_ids(store, &pool_ids, use_field_rescoring, intent)?;
    Ok(embed_hits_rescored(
        &pool_chunks,
        remapped,
        query_vec,
        &fields,
        intent,
        hit_limit,
    ))
}

fn embed_hits_from_concat_rank(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    ids: &[i64],
    query_vec: &[f32],
    intent: QueryIntent,
    hit_limit: usize,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    let ranked =
        ast_sgrep_embed::rank_chunk_indices_by_vector(query_vec, chunks, chunks.len());
    let pool = field_rescore_pool(hit_limit);
    let taken: Vec<(usize, f32)> = ranked.into_iter().take(pool).collect();
    let pool_chunks: Vec<SemanticChunkRow> = taken
        .iter()
        .map(|(idx, _)| chunks[*idx].clone())
        .collect();
    let pool_ids: Vec<i64> = taken.iter().map(|(idx, _)| ids[*idx]).collect();
    let remapped: Vec<(usize, f32)> = taken
        .into_iter()
        .enumerate()
        .map(|(i, (_, score))| (i, score))
        .collect();
    let fields = fields_for_ids(store, &pool_ids, use_field_rescoring, intent)?;
    Ok(embed_hits_rescored(
        &pool_chunks,
        remapped,
        query_vec,
        &fields,
        intent,
        hit_limit,
    ))
}

fn embed_hits_rescored(
    chunks: &[SemanticChunkRow],
    ranked: Vec<(usize, f32)>,
    query_vec: &[f32],
    fields: &[SemanticFieldVectors],
    intent: QueryIntent,
    hit_limit: usize,
) -> Vec<SearchHit> {
    let mut notes = vec![None; chunks.len()];
    let ranked = ranked
        .into_iter()
        .map(|(idx, primary)| {
            if let Some(field) = fields.get(idx) {
                let (score, note) = rescore_similarity(primary, query_vec, field, intent);
                if idx < notes.len() {
                    notes[idx] = note;
                }
                (idx, score)
            } else {
                (idx, primary)
            }
        })
        .collect();
    embed_similarity_hits(chunks, ranked, &notes, hit_limit)
}

fn embed_similarity_hits(
    chunks: &[SemanticChunkRow],
    ranked: Vec<(usize, f32)>,
    field_notes: &[Option<EmbedFieldScores>],
    hit_limit: usize,
) -> Vec<SearchHit> {
    #[derive(Debug)]
    struct ParentMatch {
        best_index: usize,
        best_similarity: f32,
        children: Vec<(f32, String)>,
    }

    let mut parents = HashMap::<(String, u32, u32, String), ParentMatch>::new();
    for (index, similarity) in ranked {
        let Some((file, line_start, line_end, symbol, excerpt, _)) = chunks.get(index) else {
            continue;
        };
        let parent = parents
            .entry((file.clone(), *line_start, *line_end, symbol.clone()))
            .or_insert_with(|| ParentMatch {
                best_index: index,
                best_similarity: similarity,
                children: Vec::new(),
            });
        if similarity > parent.best_similarity {
            parent.best_index = index;
            parent.best_similarity = similarity;
        }
        if !parent.children.iter().any(|(_, child)| child == excerpt) {
            parent.children.push((similarity, excerpt.clone()));
        }
    }
    let mut parents = parents.into_values().collect::<Vec<_>>();
    parents.sort_by(|left, right| {
        right
            .best_similarity
            .total_cmp(&left.best_similarity)
            .then_with(|| chunks[left.best_index].0.cmp(&chunks[right.best_index].0))
            .then_with(|| chunks[left.best_index].1.cmp(&chunks[right.best_index].1))
            .then_with(|| chunks[left.best_index].2.cmp(&chunks[right.best_index].2))
            .then_with(|| chunks[left.best_index].3.cmp(&chunks[right.best_index].3))
    });
    parents.truncate(hit_limit);
    parents
        .into_iter()
        .map(|mut parent| {
            parent.children.sort_by(|left, right| {
                right
                    .0
                    .total_cmp(&left.0)
                    .then_with(|| left.1.cmp(&right.1))
            });
            parent.children.truncate(3);
            let (file, line_start, line_end, symbol, _, _) = &chunks[parent.best_index];
            let mut hit = SearchHit::span(SpanHitInput {
                kind: HitKind::Embed,
                file: file.clone(),
                line_start: *line_start,
                line_end: *line_end,
                score: SCORE_EMBED * f64::from(parent.best_similarity),
                excerpt: parent
                    .children
                    .into_iter()
                    .map(|(_, excerpt)| excerpt)
                    .collect::<Vec<_>>()
                    .join("\n...\n"),
                symbol: (!symbol.is_empty()).then_some(symbol.clone()),
                language: None,
            });
            hit.embed_fields = field_notes.get(parent.best_index).cloned().flatten();
            hit
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
        ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &chunks, chunks.len()),
        &[],
        EMBED_HIT_LIMIT.max(options.limit),
    ))
}
