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
        return Ok(fallback_line_embeddings(store, options, &query)?);
    }

    let ctx = embed_context(store, options, chunks.first().map(|c| c.5.len()));
    let query_result = embed_query(
        &query,
        ctx.backend.as_deref(),
        ctx.dim,
        ctx.preference,
    );

    let indices = rank_chunk_indices(
        store,
        &query_result.vector,
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

/// Legacy per-line embeddings for indexes built before semantic chunks.
fn fallback_line_embeddings(
    store: &IndexStore,
    options: &SearchOptions,
    query: &str,
) -> Result<Vec<SearchHit>> {
    let chunk_rows = load_legacy_chunk_rows(store, options)?;
    if chunk_rows.is_empty() {
        return Ok(Vec::new());
    }

    let ctx = embed_context(store, options, chunk_rows.first().map(|c| c.5.len()));
    let query_result = embed_query(
        query,
        ctx.backend.as_deref(),
        ctx.dim,
        ctx.preference,
    );
    let ranked = ast_sgrep_embed::rank_chunks_by_vector(
        &query_result.vector,
        &chunk_rows,
        EMBED_HIT_LIMIT,
    );

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

fn load_legacy_chunk_rows(
    store: &IndexStore,
    options: &SearchOptions,
) -> Result<Vec<SemanticChunkRow>> {
    let conn = store.connection();
    let lang_clause = if options.lang_filter.is_some() {
        " AND f.language = ?1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector
         FROM embeddings e
         JOIN lines l ON l.file_id = e.file_id AND l.line_no = e.line_no
         JOIN files f ON f.id = e.file_id
         LEFT JOIN semantic_chunks sc ON sc.file_id = f.id AND sc.line_start = l.line_no
         WHERE 1=1{lang_clause}
         LIMIT 5000"
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = match options.lang_filter.as_deref() {
        Some(lang) => stmt.query(rusqlite::params![lang])?,
        None => stmt.query([])?,
    };

    let mut chunk_rows = Vec::new();
    while let Some(row) = rows.next()? {
        let file: String = row.get(0)?;
        let line_no: u32 = row.get(1)?;
        let content: String = row.get(2)?;
        let symbol: Option<String> = row.get(3)?;
        let vector: Vec<u8> = row.get(4)?;
        chunk_rows.push((
            file,
            line_no,
            line_no,
            symbol.unwrap_or_default(),
            content,
            ast_sgrep_embed::embed_from_bytes(&vector).unwrap_or_default(),
        ));
    }
    Ok(chunk_rows)
}
