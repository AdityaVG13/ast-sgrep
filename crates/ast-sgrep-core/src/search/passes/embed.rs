use super::embed_fetch::{
    fields_for_ids, rows_and_fields_in_id_order, rows_in_id_order_with_vectors,
};
use super::embed_query::{embed_query_vector, embed_query_vector_from_meta};
use super::super::fusion::{FileCsr, NameIntern};
use crate::intent::{classify, QueryIntent};
use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::search::field_weight::{
    field_rank_union_keys, field_rank_union_mode, rescore_similarity,
    EmbedFieldScores, FieldRankUnionMode,
};
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::semantic_ann::{flatten_vectors_for_search, rank_chunk_indices_flat};
use crate::semantic_chunk::SemanticFieldVectors;
use crate::store::IndexStore;
use crate::Result;
use ast_sgrep_embed::{
    cosine_similarity, normalize_vec, top_k_similarity, SemanticChunkRow, MIN_SIMILARITY,
};
use std::collections::HashSet;
use std::sync::Arc;
const EMBED_HIT_LIMIT: usize = 50;
/// Exhaustive concat rank is the IVF-miss fallback, not the serve default.
/// Cap keeps a held snapshot from scoring an unbounded mmap.
const SNAPSHOT_EXHAUSTIVE_CHUNK_CAP: usize = 65_536;

/// Single home for the embed hit-limit stanza (F5): floor 50, honour larger
/// retained limits. Every embed pass derives its cap here instead of
/// repeating `EMBED_HIT_LIMIT.max(options.retained_limit())`.
pub fn embed_hit_limit(options: &SearchOptions) -> usize {
    EMBED_HIT_LIMIT.max(options.retained_limit())
}

pub struct EmbedContext {
    pub chunks: Arc<Vec<SemanticChunkRow>>,
    pub packed_f16: Arc<Vec<u16>>,
    pub flat_vectors: Arc<Vec<f32>>,
    pub ids: Arc<Vec<i64>>,
    pub embed_backend: String,
    pub embed_model: Option<String>,
    pub dim: usize,
    chunks_by_file: Arc<FileCsr>,
    bundle: Option<Arc<crate::store::read_bundle::ReadBundle>>,
    view: Option<Arc<crate::store::bundle_semantic::SemanticView>>,
    rank_idx: Option<Arc<Vec<usize>>>,
}

fn embed_ctx_len(ctx: &EmbedContext) -> usize {
    if let Some(idx) = ctx.rank_idx.as_ref() {
        idx.len()
    } else if let Some(view) = ctx.view.as_ref() {
        view.count
    } else {
        ctx.chunks.len()
    }
}

fn empty_chunk_row() -> SemanticChunkRow {
    SemanticChunkRow {
        path: String::new(),
        line_start: 0,
        line_end: 0,
        symbol: String::new(),
        text: String::new(),
        embedding: Vec::new(),
    }
}

fn chunk_at(ctx: &EmbedContext, i: usize) -> SemanticChunkRow {
    if let (Some(bundle), Some(view)) = (&ctx.bundle, &ctx.view) {
        view.row(bundle, i).unwrap_or_else(empty_chunk_row)
    } else {
        ctx.chunks.get(i).cloned().unwrap_or_else(empty_chunk_row)
    }
}

fn chunks_by_file(chunks: &[SemanticChunkRow]) -> Arc<FileCsr> {
    Arc::new(FileCsr::build(
        chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| (chunk.path.as_str(), i as u32))
            .collect(),
    ))
}

fn embed_context_from_parts(
    chunks: Arc<Vec<SemanticChunkRow>>,
    packed_f16: Arc<Vec<u16>>,
    flat_vectors: Arc<Vec<f32>>,
    ids: Arc<Vec<i64>>,
    embed_backend: String,
    embed_model: Option<String>,
    dim: usize,
) -> EmbedContext {
    let chunks_by_file = chunks_by_file(&chunks);
    EmbedContext {
        chunks,
        packed_f16,
        flat_vectors,
        ids,
        embed_backend,
        embed_model,
        dim,
        chunks_by_file,
        bundle: None,
        view: None,
        rank_idx: None,
    }
}

fn lang_ok(chunk_path_lang: Option<&str>, want: Option<&str>) -> bool {
    want.is_none_or(|want| {
        chunk_path_lang.is_some_and(|have| {
            ast_sgrep_lang::Language::normalize_id(have)
                == ast_sgrep_lang::Language::normalize_id(want)
        })
    })
}

fn packed_row_cosine_bytes(query_unit: &[f32], bytes: &[u8], dim: usize, i: usize) -> f32 {
    let base = i * dim * 2;
    let need = base + dim * 2;
    if bytes.len() < need {
        return 0.0;
    }
    if dim <= 256 {
        let mut row = [0f32; 256];
        for j in 0..dim {
            let at = base + j * 2;
            let bits = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            row[j] = crate::store::bundle_semantic::f16_to_f32(bits);
        }
        cosine_similarity(query_unit, &row[..dim])
    } else {
        let mut row = vec![0f32; dim];
        for j in 0..dim {
            let at = base + j * 2;
            let bits = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            row[j] = crate::store::bundle_semantic::f16_to_f32(bits);
        }
        cosine_similarity(query_unit, &row)
    }
}

fn embed_context_from_bundle(
    bundle: Arc<crate::store::read_bundle::ReadBundle>,
    view: Arc<crate::store::bundle_semantic::SemanticView>,
    embed_backend: String,
    embed_model: Option<String>,
    lang_filter: Option<&str>,
) -> Option<EmbedContext> {
    if view.count == 0 || view.dim == 0 {
        return None;
    }
    let mut pairs = Vec::with_capacity(view.count);
    let mut rank_idx = Vec::new();
    for i in 0..view.count {
        let file = view.file_idx[i] as usize;
        if !lang_ok(bundle.language(file), lang_filter) {
            continue;
        }
        pairs.push((bundle.path(file), i as u32));
        if lang_filter.is_some() {
            rank_idx.push(i);
        }
    }
    if pairs.is_empty() {
        return None;
    }
    Some(EmbedContext {
        chunks: Arc::new(Vec::new()),
        packed_f16: Arc::new(Vec::new()),
        flat_vectors: Arc::new(Vec::new()),
        ids: Arc::clone(&view.ids),
        embed_backend,
        embed_model,
        dim: view.dim,
        chunks_by_file: Arc::new(FileCsr::build(pairs)),
        bundle: Some(bundle),
        view: Some(view),
        rank_idx: lang_filter.and(Some(Arc::new(rank_idx))),
    })
}

fn packed_row_cosine(query_unit: &[f32], packed: &[u16], dim: usize, i: usize) -> f32 {
    let base = i * dim;
    if dim <= 256 {
        let mut row = [0f32; 256];
        for j in 0..dim {
            row[j] = crate::store::bundle_semantic::f16_to_f32(packed[base + j]);
        }
        cosine_similarity(query_unit, &row[..dim])
    } else {
        let mut row = vec![0f32; dim];
        for j in 0..dim {
            row[j] = crate::store::bundle_semantic::f16_to_f32(packed[base + j]);
        }
        cosine_similarity(query_unit, &row)
    }
}

pub(crate) fn load_semantic_context(
    store: &IndexStore,
    options: &SearchOptions,
) -> Result<Option<EmbedContext>> {
    if !options.use_embed {
        return Ok(None);
    }
    let embed_backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    let embed_model = store.get_meta("embed_model")?;
    let lang_filter = options.lang_filter.as_deref();
    if let Some(bundle) = store.read_bundle(None)? {
        if let Some(view) = bundle.semantic_view() {
            return Ok(embed_context_from_bundle(
                bundle,
                view,
                embed_backend,
                embed_model,
                lang_filter,
            ));
        }
    }
    let chunks = store.all_semantic_chunks(lang_filter)?;
    if chunks.is_empty() {
        return Ok(None);
    }
    let ids = store.semantic_chunk_ids(lang_filter)?;
    let ids = if ids.len() == chunks.len() {
        ids
    } else {
        Vec::new()
    };
    let dim = chunks[0].embedding.len();
    let flat_vectors = flatten_vectors_for_search(&chunks, dim)?;
    Ok(Some(embed_context_from_parts(
        Arc::new(chunks),
        Arc::new(Vec::new()),
        Arc::new(flat_vectors),
        Arc::new(ids),
        embed_backend,
        embed_model,
        dim,
    )))
}

/// Unconstrained embed. Cold CLI already uses IVF above the ANN threshold.
/// Serve holds a read snapshot; exhaustive mmap rank up to
/// [`SNAPSHOT_EXHAUSTIVE_CHUNK_CAP`] used to run first (self-corpus 19_985
/// chunks). IVF first matches the CLI/CoSQA path. Snapshot exhaustive stays
/// the fallback when IVF does not admit (below threshold, missing sidecar,
/// lang filter). Field rescoring still fetches the rescore-pool blobs.
pub(crate) fn run_embed_pass_cached(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    trust_snapshot: bool,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    let field = use_field_rescoring;
    if let Some(hits) = embed_pass_lazy_ivf_with_rescoring(store, options, parsed, field)? {
        return Ok(hits);
    }
    if trust_snapshot {
        match load_semantic_context(store, options)? {
            Some(ctx)
                if embed_ctx_len(&ctx) > 0
                    && embed_ctx_len(&ctx) <= SNAPSHOT_EXHAUSTIVE_CHUNK_CAP =>
            {
                return embed_pass_with_context_and_rescoring(
                    store,
                    options,
                    parsed,
                    Some(ctx),
                    field,
                );
            }
            _ => {}
        }
    }
    match load_semantic_context(store, options)? {
        Some(ctx) => {
            embed_pass_with_context_and_rescoring(store, options, parsed, Some(ctx), field)
        }
        None => Ok(vec![]),
    }
}

fn semantic_ann_shape(store: &IndexStore) -> Result<(usize, i64, usize)> {
    if let Some(bundle) = store.read_bundle(None)? {
        if let Some(view) = bundle.semantic_view() {
            let max_id = view
                .id_order
                .last()
                .copied()
                .map(|i| view.ids[i as usize])
                .unwrap_or(0);
            return Ok((view.count, max_id, view.dim));
        }
    }
    let (count, max_id) = store.semantic_chunk_count_and_max_id()?;
    Ok((count, max_id, store.semantic_primary_dim()?))
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
    // Fingerprint inputs from the packed semantic view when present (count,
    // max id, dim). SQL COUNT(*)+MAX(id) remains the control arm.
    let (count, max_id, dim) = semantic_ann_shape(store)?;
    if !crate::semantic_ann::should_use_ann(count, options.ann_threshold) {
        return Ok(None);
    }
    if dim == 0 {
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
    let Some(ivf) = store.cached_lazy_ivf(fingerprint)? else {
        return Ok(None);
    };
    if ivf.chunk_count() != count || ivf.dim != dim {
        return Ok(None);
    }
    // v3 sidecars carry the id column in the mmap; v2 sidecars resolve ids
    // from the cached SQL chunk index exactly as before (same ORDER BY id,
    // so every position resolves to the same row).
    let sql_ids;
    let ids: &[i64] = match ivf.ids() {
        Some(mapped) => mapped,
        None => {
            let _span = crate::perf_profile::Span::start(
                "semantic_ids_fallback",
                "semantic",
                "v2 sidecar: SQL chunk-index id resolution",
            );
            let (ids, _paths, _path_order, dim_sql, fp_sql) = cached_semantic_chunk_index(store)?;
            if dim_sql != dim || fp_sql != fingerprint || ids.len() != count {
                return Ok(None);
            }
            sql_ids = ids;
            &sql_ids
        }
    };
    let query = parsed.terms.join(" ");
    let query_vec = embed_query_vector(store, &query, Some(dim))?;
    let intent = classify(parsed);
    let hit_limit = embed_hit_limit(options);
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
                field_rescore_pool(hit_limit),
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
    let Some((chunks, fields)) =
        rows_and_fields_in_id_order(store, &candidate_ids, use_field_rescoring, intent)?
    else {
        return Ok(None);
    };
    let ranked: Vec<(usize, f32)> = ranked_payload
        .iter()
        .enumerate()
        .map(|(i, (_, score))| (i, *score))
        .collect();
    Ok(Some(embed_hits_rescored(
        &chunks, ranked, &query_vec, &fields, intent, hit_limit,
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
    let (ids, paths, path_order, dim, fingerprint) = cached_semantic_chunk_index(store)?;
    let count = ids.len();
    if paths.len() != count || path_order.len() != count {
        return Ok(None);
    }
    if !crate::semantic_ann::should_use_ann(count, options.ann_threshold) || dim == 0 {
        return Ok(None);
    }
    let Some(ivf) = store.cached_lazy_ivf(fingerprint)? else {
        return Ok(None);
    };
    if ivf.chunk_count() != count || ivf.dim != dim {
        return Ok(None);
    }
    let members = {
        let _span = crate::perf_profile::Span::start(
            "semantic_member_filter",
            "semantic",
            "allowed_files -> IVF row indices",
        );
        member_indices_for_files(&paths, &path_order, allowed_files)
    };
    if members.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let query = parsed.terms.join(" ");
    let query_vec = embed_query_vector(store, &query, Some(dim))?;
    let intent = classify(parsed);
    let hit_limit = embed_hit_limit(options);
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
    let Some((chunks, fields)) =
        rows_and_fields_in_id_order(store, &candidate_ids, use_field_rescoring, intent)?
    else {
        return Ok(None);
    };
    let ranked: Vec<(usize, f32)> = ranked_payload
        .iter()
        .enumerate()
        .map(|(i, (_, score))| (i, *score))
        .collect();
    Ok(Some(embed_hits_rescored(
        &chunks, ranked, &query_vec, &fields, intent, hit_limit,
    )))
}

/// Hybrid cascade embed: score mmap/SQL vectors filtered to cascade files.
pub(crate) fn embed_pass_for_files_cached(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
    trust_snapshot: bool,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || !options.use_embed || allowed_files.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(hits) = try_warmed_file_embed(
        store,
        options,
        parsed,
        allowed_files,
        trust_snapshot,
        use_field_rescoring,
    )? {
        return Ok(hits);
    }
    if let Some(hits) =
        embed_pass_lazy_ivf_for_files(store, options, parsed, allowed_files, use_field_rescoring)?
    {
        return Ok(hits);
    }
    // gauntlet-r4 (E1): IVF miss only. Skip per-file fallback loops when both
    // semantic sources are globally empty. The IVF success path never needed
    // this EXISTS probe.
    if store.semantic_sources_empty()? {
        return Ok(Vec::new());
    }
    let query = parsed.terms.join(" ");
    let intent = classify(parsed);
    let hit_limit = embed_hit_limit(options);
    let mut survivors = Vec::new();
    let mut survivor_ids = Vec::new();
    for (id, row) in
        store.semantic_chunks_for_files(allowed_files, options.lang_filter.as_deref())?
    {
        survivor_ids.push(id);
        survivors.push(row);
    }
    let modern_files = survivors
        .iter()
        .map(|chunk| chunk.path.clone())
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
    let query_vec =
        embed_query_vector(store, &query, survivors.first().map(|chunk| chunk.embedding.len()))?;
    embed_hits_from_concat_rank(
        store,
        &survivors,
        &survivor_ids,
        &query_vec,
        intent,
        hit_limit,
        field_rescore_pool(hit_limit),
        use_field_rescoring,
    )
}

fn try_warmed_file_embed(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
    trust_snapshot: bool,
    use_field_rescoring: bool,
) -> Result<Option<Vec<SearchHit>>> {
    let Some(ctx) = load_semantic_context(store, options)? else {
        return Ok(None);
    };
    if embed_ctx_len(&ctx) == 0 {
        return Ok(None);
    }
    if ctx.view.is_none() && (ctx.ids.len() != ctx.chunks.len() || ctx.chunks.is_empty()) {
        return Ok(None);
    }
    if let Some(view) = ctx.view.as_ref() {
        if ctx.ids.len() != view.count {
            return Ok(None);
        }
    }
    let query = parsed.terms.join(" ");
    let intent = classify(parsed);
    let hit_limit = embed_hit_limit(options);
    let dim = ctx.dim;
    let n = embed_ctx_len(&ctx);
    let mmap_bytes = ctx
        .view
        .as_ref()
        .zip(ctx.bundle.as_ref())
        .and_then(|(view, bundle)| view.f16_bytes(bundle));
    let use_mmap = dim > 0 && mmap_bytes.is_some();
    let use_packed = dim > 0 && ctx.packed_f16.len() == n * dim;
    let use_flat = dim > 0 && ctx.flat_vectors.len() == n * dim;
    if use_mmap || use_packed || use_flat {
        let allowed_idx: Vec<usize> = allowed_files
            .iter()
            .flat_map(|path| ctx.chunks_by_file.get(path).iter().copied())
            .map(|i| i as usize)
            .collect();
        if allowed_idx.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let query_vec = embed_query_vector_from_meta(
            &query,
            Some(dim),
            Some(ctx.embed_backend.as_str()),
            ctx.embed_model.as_deref(),
        )?;
        let query_unit = normalize_vec(&query_vec);
        let pool = if use_field_rescoring {
            field_rescore_pool(hit_limit)
        } else {
            hit_limit
        };
        let ranked = top_k_similarity(
            allowed_idx.into_iter().map(|i| {
                let sim = if let Some(bytes) = mmap_bytes {
                    packed_row_cosine_bytes(&query_unit, bytes, dim, i)
                } else if use_packed {
                    packed_row_cosine(&query_unit, &ctx.packed_f16, dim, i)
                } else {
                    let start = i * dim;
                    cosine_similarity(&query_unit, &ctx.flat_vectors[start..start + dim])
                };
                (i, sim)
            }),
            pool.max(1),
            Some(MIN_SIMILARITY),
        );
        return Ok(Some(embed_hits_from_pre_rank(
            store,
            |i| chunk_at(&ctx, i),
            &ctx.ids,
            ranked,
            &query_vec,
            intent,
            hit_limit,
            use_field_rescoring,
        )?));
    }
    let mut survivors = Vec::new();
    let mut survivor_ids = Vec::new();
    let mut present = HashSet::new();
    for (chunk, id) in ctx.chunks.iter().zip(ctx.ids.iter()) {
        if allowed_files.contains(&chunk.path) {
            present.insert(chunk.path.as_str());
            survivors.push(chunk.clone());
            survivor_ids.push(*id);
        }
    }
    let missing: HashSet<String> = allowed_files
        .iter()
        .filter(|file| !present.contains(file.as_str()))
        .cloned()
        .collect();
    // Snapshot-held Code Mode already loaded every modern chunk at warm.
    // Skip the legacy-embeddings SQL for files with no modern vectors.
    if !trust_snapshot && !missing.is_empty() {
        let legacy = store.legacy_embeddings_for_files(&missing, options.lang_filter.as_deref())?;
        survivor_ids.extend(std::iter::repeat_n(0, legacy.len()));
        survivors.extend(legacy);
    }
    if survivors.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let query_vec = embed_query_vector_from_meta(
        &query,
        survivors.first().map(|chunk| chunk.embedding.len()),
        Some(ctx.embed_backend.as_str()),
        ctx.embed_model.as_deref(),
    )?;
    Ok(Some(embed_hits_from_concat_rank(
        store,
        &survivors,
        &survivor_ids,
        &query_vec,
        intent,
        hit_limit,
        field_rescore_pool(hit_limit),
        use_field_rescoring,
    )?))
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
    if let Some(ctx) = ctx.as_ref() {
        if let (Some(bundle), Some(view)) = (&ctx.bundle, &ctx.view) {
            if view.count > 0 && ctx.ids.len() == view.count {
                if let Some(bytes) = view.f16_bytes(bundle) {
                    let query_vec = embed_query_vector_from_meta(
                        &query,
                        Some(ctx.dim),
                        Some(ctx.embed_backend.as_str()),
                        ctx.embed_model.as_deref(),
                    )?;
                    let ranked = crate::semantic_ann::brute_force_flat_f16_bytes(
                        bytes,
                        view.dim,
                        view.count,
                        &query_vec,
                        view.count,
                        ctx.rank_idx.as_ref().map(|idx| idx.as_slice()),
                    );
                    let intent = classify(parsed);
                    let hit_limit = embed_hit_limit(options);
                    return embed_hits_from_pre_rank(
                        store,
                        |i| chunk_at(ctx, i),
                        &ctx.ids,
                        ranked,
                        &query_vec,
                        intent,
                        hit_limit,
                        use_field_rescoring,
                    );
                }
            }
        }
    }
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
    let flat = ctx
        .as_ref()
        .map(|c| c.flat_vectors.as_slice())
        .filter(|f| !f.is_empty());
    let packed = ctx
        .as_ref()
        .map(|c| c.packed_f16.as_slice())
        .filter(|p| !p.is_empty());
    let query_vec = match ctx.as_ref() {
        Some(ctx) => embed_query_vector_from_meta(
            &query,
            Some(ctx.dim),
            Some(ctx.embed_backend.as_str()),
            ctx.embed_model.as_deref(),
        )?,
        None => embed_query_vector(store, &query, chunks.first().map(|c| c.embedding.len()))?,
    };
    let ann_threshold = if options.lang_filter.is_some()
        || (ctx.is_some() && chunks.len() <= SNAPSHOT_EXHAUSTIVE_CHUNK_CAP)
    {
        // Lang-filter and held-snapshot RAM: exact rank under the exhaustive
        // cap, never IVF (independent of field rescoring).
        Some(usize::MAX)
    } else {
        options.ann_threshold
    };
    let indices = rank_chunk_indices_flat(
        store,
        &query_vec,
        chunks,
        flat,
        packed,
        chunks.len(),
        ann_threshold,
    )?;
    let intent = classify(parsed);
    let hit_limit = embed_hit_limit(options);
    let fetched_ids;
    let ids: &[i64] = match ctx.as_ref() {
        Some(ctx) if ctx.ids.len() == chunks.len() => ctx.ids.as_slice(),
        _ => {
            fetched_ids = store.semantic_chunk_ids(options.lang_filter.as_deref())?;
            // Same JOIN + ORDER BY sc.id as all_semantic_chunks. Length mismatch
            // means skip rescoring rather than pairing the wrong field vectors.
            if fetched_ids.len() != chunks.len() {
                return Ok(embed_hits_rescored(
                    chunks,
                    indices,
                    &query_vec,
                    &[],
                    intent,
                    hit_limit,
                ));
            }
            &fetched_ids
        }
    };
    embed_hits_from_pre_rank(
        store,
        |i| chunks[i].clone(),
        ids,
        indices,
        &query_vec,
        intent,
        hit_limit,
        use_field_rescoring,
    )
}
/// IVF payload order is `semantic_chunks.id` order. Bundle view is the
/// snapshot; SQL is the identity control arm. No process-wide memo.

pub fn member_indices_for_files(
    paths: &[String],
    path_order: &[u32],
    allowed_files: &HashSet<String>,
) -> Vec<usize> {
    let mut members = Vec::new();
    for file in allowed_files {
        let found =
            path_order.binary_search_by(|&idx| paths[idx as usize].as_str().cmp(file.as_str()));
        let mut i = match found {
            Ok(hit) => hit,
            Err(_) => continue,
        };
        while i > 0 && paths[path_order[i - 1] as usize] == *file {
            i -= 1;
        }
        while i < path_order.len() && paths[path_order[i] as usize] == *file {
            members.push(path_order[i] as usize);
            i += 1;
        }
    }
    members.sort_unstable();
    members
}

/// Cached chunk index: (ids, paths, path_order, dim, fingerprint).
type CachedChunkIndex = (
    Arc<Vec<i64>>,
    Arc<Vec<String>>,
    Arc<Vec<u32>>,
    usize,
    [u8; 32],
);

fn cached_semantic_chunk_index(store: &IndexStore) -> Result<CachedChunkIndex> {
    if crate::store::read_bundle::bundle_enabled() {
        if let Some(bundle) = store.read_bundle(None)? {
            if let Some(view) = bundle.semantic_view() {
            let index_data_version = store.index_data_version().unwrap_or(0);
            let backend = store
                .get_meta("embed_backend")?
                .unwrap_or_else(|| "semantic".into());
            let fingerprint = crate::semantic_ivf::compute_ann_fingerprint(
                view.ids.len(),
                view.ids.last().copied().unwrap_or(0),
                view.dim,
                Some(&backend),
                index_data_version,
            );
            let paths = Arc::new(
                (0..view.count)
                    .map(|i| bundle.path(view.file_idx[i] as usize).to_owned())
                    .collect::<Vec<_>>(),
            );
            return Ok((
                Arc::clone(&view.ids),
                paths,
                Arc::clone(&view.path_order),
                view.dim,
                fingerprint,
            ));
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
    let mut path_order: Vec<u32> = (0..paths.len() as u32).collect();
    path_order.sort_by(|&a, &b| paths[a as usize].cmp(&paths[b as usize]));
    let path_order = Arc::new(path_order);
    let dim = store.semantic_primary_dim()?;
    let backend = store
        .get_meta("embed_backend")?
        .unwrap_or_else(|| "semantic".into());
    let fingerprint = crate::semantic_ivf::compute_ann_fingerprint(
        ids.len(),
        ids.last().copied().unwrap_or(0),
        dim,
        Some(&backend),
        store.index_data_version().unwrap_or(0),
    );
    Ok((ids, paths, path_order, dim, fingerprint))
}

fn field_rescore_pool(hit_limit: usize) -> usize {
    // IVF mmap already ranked candidates. Fetching 8× hit_limit (400 at the
    // default embed cap of 50) from SQLite was the unique-query floor.
    // Field-rescore among the concat top-N; N must reach past the zone
    // where a diluted joint primary hides a chunk whose name/docs field
    // matches strongly (measured: a whole-body primary at rank ~108 of
    // 18.8k whose name field ranks ~29 pure). 192 keeps the masked
    // by-ids fetch bounded while covering that rescue zone.
    (hit_limit * 3).max(192)
}


// Stable rank-fanout shape: rank context is threaded, not bundled; bundling
// would churn every rescoring call for no behavior gain.
#[allow(clippy::too_many_arguments)]
fn embed_hits_from_pre_rank(
    store: &IndexStore,
    chunk_at: impl Fn(usize) -> SemanticChunkRow,
    ids: &[i64],
    ranked: Vec<(usize, f32)>,
    query_vec: &[f32],
    intent: QueryIntent,
    hit_limit: usize,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    let pool = if use_field_rescoring {
        field_rescore_pool(hit_limit)
    } else {
        hit_limit
    };
    let taken: Vec<(usize, f32)> = ranked.into_iter().take(pool.max(1)).collect();
    let pool_chunks: Vec<SemanticChunkRow> =
        taken.iter().map(|(idx, _)| chunk_at(*idx)).collect();
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

// Stable rank-fanout shape: rank context is threaded, not bundled; bundling
// would churn every rescoring call for no behavior gain.
#[allow(clippy::too_many_arguments)]
fn embed_hits_from_concat_rank(
    store: &IndexStore,
    chunks: &[SemanticChunkRow],
    ids: &[i64],
    query_vec: &[f32],
    intent: QueryIntent,
    hit_limit: usize,
    rescore_pool: usize,
    use_field_rescoring: bool,
) -> Result<Vec<SearchHit>> {
    let ranked = ast_sgrep_embed::rank_chunk_indices_by_vector(query_vec, chunks, chunks.len());
    let taken: Vec<(usize, f32)> = ranked.into_iter().take(rescore_pool.max(1)).collect();
    let pool_chunks: Vec<SemanticChunkRow> =
        taken.iter().map(|(idx, _)| chunks[*idx].clone()).collect();
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
        best_rank: (u32, f32),
        children: Vec<(f32, usize)>,
    }

    let union_mode = field_rank_union_mode();
    let files: Vec<&str> = chunks.iter().map(|c| c.path.as_str()).collect();
    let union_keys = if union_mode == FieldRankUnionMode::Off {
        (0..ranked.len() as u32)
            .zip(ranked.iter().map(|(_, score)| *score))
            .collect()
    } else {
        field_rank_union_keys(&ranked, &files, field_notes, union_mode)
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct ParentKey {
        file: u32,
        line_start: u32,
        line_end: u32,
        symbol: u32,
    }

    let mut intern = NameIntern::new(ranked.len());
    let mut parent_keys: Vec<ParentKey> = Vec::new();
    let mut parents: Vec<ParentMatch> = Vec::new();
    for (pos, (index, similarity)) in ranked.into_iter().enumerate() {
        let Some(chunk) = chunks.get(index) else {
            continue;
        };
        let key = union_keys
            .get(pos)
            .copied()
            .unwrap_or((pos as u32, similarity));
        let parent_key = ParentKey {
            file: intern.intern(&chunk.path),
            line_start: chunk.line_start,
            line_end: chunk.line_end,
            symbol: intern.intern(&chunk.symbol),
        };
        let parent = if let Some(slot) = parent_keys.iter().position(|seen| *seen == parent_key) {
            &mut parents[slot]
        } else {
            parent_keys.push(parent_key);
            parents.push(ParentMatch {
                best_index: index,
                best_similarity: similarity,
                best_rank: key,
                children: Vec::new(),
            });
            let slot = parents.len() - 1;
            &mut parents[slot]
        };
        if similarity > parent.best_similarity {
            parent.best_index = index;
            parent.best_similarity = similarity;
        }
        if key < parent.best_rank {
            parent.best_rank = key;
        }
        if !parent
            .children
            .iter()
            .any(|&(_, idx)| chunks.get(idx).is_some_and(|row| row.text == chunk.text))
        {
            parent.children.push((similarity, index));
        }
    }
    // Channel order = (best list rank, winning-list cosine, rescored
    // similarity), then the deterministic span tie-breaks. The carried
    // score stays the rescored similarity; only order changes.
    parents.sort_by(|left, right| {
        left.best_rank
            .0
            .cmp(&right.best_rank.0)
            .then_with(|| right.best_rank.1.total_cmp(&left.best_rank.1))
            .then_with(|| right.best_similarity.total_cmp(&left.best_similarity))
            .then_with(|| chunks[left.best_index].path.cmp(&chunks[right.best_index].path))
            .then_with(|| chunks[left.best_index].line_start.cmp(&chunks[right.best_index].line_start))
            .then_with(|| chunks[left.best_index].line_end.cmp(&chunks[right.best_index].line_end))
            .then_with(|| chunks[left.best_index].symbol.cmp(&chunks[right.best_index].symbol))
    });
    parents.truncate(hit_limit);
    parents
        .into_iter()
        .map(|mut parent| {
            parent.children.sort_by(|left, right| {
                right
                    .0
                    .total_cmp(&left.0)
                    .then_with(|| chunks[left.1].text.cmp(&chunks[right.1].text))
            });
            parent.children.truncate(3);
            let best = &chunks[parent.best_index];
            let mut hit = SearchHit::span(SpanHitInput {
                kind: HitKind::Embed,
                file: best.path.clone(),
                line_start: best.line_start,
                line_end: best.line_end,
                score: SCORE_EMBED * f64::from(parent.best_similarity),
                excerpt: parent
                    .children
                    .into_iter()
                    .map(|(_, idx)| chunks[idx].text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n...\n"),
                symbol: (!best.symbol.is_empty()).then_some(best.symbol.clone()),
                language: None,
                byte_span: None,
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
    let query_vec = embed_query_vector(store, query, chunks.first().map(|c| c.embedding.len()))?;
    Ok(embed_similarity_hits(
        &chunks,
        ast_sgrep_embed::rank_chunk_indices_by_vector(&query_vec, &chunks, chunks.len()),
        &[],
        embed_hit_limit(options),
    ))
}
