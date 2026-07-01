use std::sync::Arc;

use ast_sgrep_embed::{embed_query, SemanticChunkRow};

use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::semantic_ann::rank_chunk_indices_flat;
use crate::store::IndexStore;
use crate::Result;
use crate::search::types::{HitKind, SearchHit, SearchOptions};

const EMBED_HIT_LIMIT: usize = 50;

pub struct EmbedContext {
    pub chunks: Arc<Vec<SemanticChunkRow>>,
    pub flat_vectors: Arc<Vec<f32>>,
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
    Ok(chunk_indices_to_hits(chunks, indices))
}

fn embed_query_vector(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
    stored_dim: Option<usize>,
) -> Result<Vec<f32>> {
    Ok(embed_query(
        query,
        store.get_meta("embed_backend").unwrap_or(None).as_deref(),
        stored_dim.unwrap_or(ast_sgrep_embed::default_semantic_dim()),
        options.embed_preference(),
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
            SearchHit::span(
                HitKind::Embed,
                file.clone(),
                *line_start,
                *line_end,
                SCORE_EMBED * f64::from(sim),
                excerpt.clone(),
                Some(symbol.clone()),
                None,
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
            SearchHit::span(
                HitKind::Embed,
                file,
                line_start,
                line_end,
                SCORE_EMBED * f64::from(sim),
                excerpt,
                (!symbol.is_empty()).then_some(symbol),
                None,
            )
        })
        .collect())
}
