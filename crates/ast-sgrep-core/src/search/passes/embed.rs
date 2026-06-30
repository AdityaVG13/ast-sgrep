use ast_sgrep_embed::{embed_query, EmbedPreference};

use crate::query::ParsedQuery;
use crate::rank::SCORE_EMBED;
use crate::semantic_ann::rank_chunk_indices;
use crate::store::IndexStore;
use crate::Result;
use crate::search::types::{HitKind, SearchHit, SearchOptions};

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
        return Ok(fallback_line_embeddings(store, options, parsed)?);
    }

    let stored_dim = chunks
        .first()
        .map(|c| c.5.len())
        .unwrap_or(ast_sgrep_embed::default_semantic_dim());
    let embed_backend = store.get_meta("embed_backend").unwrap_or(None);
    let preference = search_embed_preference(options);

    let query_result = embed_query(
        &query,
        embed_backend.as_deref(),
        stored_dim,
        preference,
    );

    let indices = rank_chunk_indices(
        store,
        &query_result.vector,
        &chunks,
        50,
        options.ann_threshold,
    )?;

    Ok(indices
        .into_iter()
        .map(|(idx, sim)| {
            let (file, line_start, line_end, symbol, excerpt, _) = &chunks[idx];
            SearchHit {
                kind: HitKind::Embed,
                file: file.clone(),
                line_start: *line_start,
                line_end: *line_end,
                symbol: Some(symbol.clone()),
                caller: None,
                callee: None,
                language: None,
                score: SCORE_EMBED * f64::from(sim),
                excerpt: excerpt.clone(),
            }
        })
        .collect())
}

fn search_embed_preference(options: &SearchOptions) -> EmbedPreference {
    if options.use_cloud_embed {
        EmbedPreference::Cloud
    } else if options.use_ollama_embed {
        EmbedPreference::Ollama
    } else if options.use_semantic_only {
        EmbedPreference::Semantic
    } else {
        EmbedPreference::Auto
    }
}

/// Legacy per-line embeddings for indexes built before semantic chunks.
fn fallback_line_embeddings(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let query = parsed.terms.join(" ");
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

    let mut lines: Vec<(String, u32, u32, String, String, Vec<f32>)> = Vec::new();
    if let Some(ref lang) = options.lang_filter {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![lang])?;
        while let Some(row) = rows.next()? {
            push_legacy_row(&mut lines, row)?;
        }
    } else {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            push_legacy_row(&mut lines, row)?;
        }
    }

    if lines.is_empty() {
        return Ok(Vec::new());
    }

    let stored_dim = lines.first().map(|l| l.5.len()).unwrap_or(0);
    let embed_backend = store.get_meta("embed_backend").unwrap_or(None);
    let preference = search_embed_preference(options);
    let chunk_rows: Vec<ast_sgrep_embed::SemanticChunkRow> = lines
        .iter()
        .map(|(file, line_no, line_end, symbol, content, vec)| {
            (
                file.clone(),
                *line_no,
                *line_end,
                symbol.clone(),
                content.clone(),
                vec.clone(),
            )
        })
        .collect();

    let ranked = ast_sgrep_embed::rank_semantic_chunks(
        &query,
        &chunk_rows,
        50,
        embed_backend.as_deref(),
        stored_dim,
        preference,
    );

    Ok(ranked
        .into_iter()
        .map(|(sim, file, line_start, line_end, symbol, excerpt)| SearchHit {
            kind: HitKind::Embed,
            file,
            line_start,
            line_end,
            symbol: if symbol.is_empty() { None } else { Some(symbol) },
            caller: None,
            callee: None,
            language: None,
            score: SCORE_EMBED * f64::from(sim),
            excerpt,
        })
        .collect())
}

fn push_legacy_row(
    lines: &mut Vec<(String, u32, u32, String, String, Vec<f32>)>,
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<()> {
    let file: String = row.get(0)?;
    let line_no: u32 = row.get(1)?;
    let content: String = row.get(2)?;
    let symbol: Option<String> = row.get(3)?;
    let vector: Vec<u8> = row.get(4)?;
    lines.push((
        file,
        line_no,
        line_no,
        symbol.unwrap_or_default(),
        content,
        ast_sgrep_embed::embed_from_bytes(&vector),
    ));
    Ok(())
}
