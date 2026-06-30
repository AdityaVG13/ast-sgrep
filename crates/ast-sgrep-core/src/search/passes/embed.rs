use ast_sgrep_embed::{embed_query, EmbedPreference, SemanticChunkRow};

use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::semantic_ann::rank_chunk_indices;
use crate::store::IndexStore;
use crate::Result;
use crate::search::hits::embed_hit;
use crate::search::types::{SearchHit, SearchOptions};

const EMBED_HIT_LIMIT: usize = 50;

pub fn embed_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() || !options.use_embed {
        return Ok(Vec::new());
    }

    let query = parsed.terms.join(" ");
    let chunks = store.all_semantic_chunks(options.lang_filter.as_deref())?;

    if chunks.is_empty() {
        return embed_legacy_hits(store, options, &query);
    }

    let query_vec = embed_query_vector(store, options, &query, chunks.first().map(|c| c.5.len()))?;
    let indices = rank_chunk_indices(
        store,
        &query_vec,
        &chunks,
        EMBED_HIT_LIMIT,
        options.ann_threshold,
    )?;
    Ok(chunk_indices_to_hits(&chunks, indices))
}

struct EmbedContext {
    backend: Option<String>,
    dim: usize,
    preference: EmbedPreference,
}

fn embed_context(
    store: &IndexStore,
    options: &SearchOptions,
    stored_dim: Option<usize>,
) -> EmbedContext {
    EmbedContext {
        backend: store.get_meta("embed_backend").unwrap_or(None),
        dim: stored_dim.unwrap_or(ast_sgrep_embed::default_semantic_dim()),
        preference: options.embed_preference(),
    }
}

fn embed_query_vector(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
    stored_dim: Option<usize>,
) -> Result<Vec<f32>> {
    let ctx = embed_context(store, options, stored_dim);
    Ok(embed_query(
        query,
        ctx.backend.as_deref(),
        ctx.dim,
        ctx.preference,
    )
    .vector)
}

fn chunk_indices_to_hits(
    chunks: &[SemanticChunkRow],
    indices: Vec<(usize, f32)>,
) -> Vec<SearchHit> {
    indices
        .into_iter()
        .map(|(idx, sim)| {
            let (file, line_start, line_end, symbol, excerpt, _) = &chunks[idx];
            embed_hit(
                file.clone(),
                *line_start,
                *line_end,
                Some(symbol.clone()),
                SCORE_EMBED * f64::from(sim),
                excerpt.clone(),
            )
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
    let ranked = ast_sgrep_embed::rank_chunks_by_vector(&query_vec, &chunks, EMBED_HIT_LIMIT);

    Ok(ranked
        .into_iter()
        .map(|(sim, file, line_start, line_end, symbol, excerpt)| {
            embed_hit(
                file,
                line_start,
                line_end,
                (!symbol.is_empty()).then_some(symbol),
                SCORE_EMBED * f64::from(sim),
                excerpt,
            )
        })
        .collect())
}
