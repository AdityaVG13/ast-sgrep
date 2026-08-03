use crate::query::ParsedQuery;
use crate::rank::score_lexical_rrf;
use crate::search::passes::bmh::{asgrep_line_hit, map_line_row};
use crate::search::types::matches_lang;
use crate::search::types::{SearchHit, SearchOptions};
use crate::store::IndexStore;
use crate::Result;
use rusqlite::params;
use std::collections::HashMap;
type LineRanks = HashMap<(String, u32), Vec<usize>>;
type LineMeta = HashMap<(String, u32), (Option<String>, String)>;

/// Floor for the lexical candidate pool. User `options.limit` raises this when larger (s7jw.1).
pub const LEXICAL_POOL_FLOOR: usize = 100;

pub fn lexical_pool_limit(options: &SearchOptions) -> usize {
    options.limit.max(LEXICAL_POOL_FLOOR)
}

pub fn lexical_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    if parsed.terms.is_empty() {
        return Ok(Vec::new());
    }
    if options.use_tantivy {
        if let Some(sidecar) = crate::tantivy_index::TantivySidecar::open_existing_for_search(
            &options.root,
            options.index_path.as_deref(),
        )? {
            let hits = lexical_from_sidecar(options, parsed, &sidecar)?;
            // s7jw.2: never empty-succeed on auto/sidecar path when SQL FTS still has hits.
            if !hits.is_empty() {
                return Ok(hits);
            }
        }
    }
    lexical_from_fts(store, options, parsed)
}
fn lexical_from_sidecar(
    options: &SearchOptions,
    parsed: &ParsedQuery,
    sidecar: &crate::tantivy_index::TantivySidecar,
) -> Result<Vec<SearchHit>> {
    let mut line_ranks = LineRanks::new();
    let mut line_meta = LineMeta::new();
    let limit = lexical_pool_limit(options);
    for (file, line_no, content, language, rank) in sidecar.search(&parsed.terms, limit)? {
        accumulate(
            options,
            &mut line_ranks,
            &mut line_meta,
            file,
            line_no,
            language,
            content,
            rank,
        );
    }
    Ok(hits_from_ranks(line_ranks, line_meta))
}
fn lexical_from_fts(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let mut line_ranks = LineRanks::new();
    let mut line_meta = LineMeta::new();
    let fts_query = crate::fts::escape_fts_query(&parsed.terms);
    let limit = lexical_pool_limit(options);
    // Apply lang filter in SQL before LIMIT so path order cannot drop matching langs (iva9.5 sibling).
    let (sql, lang_bind): (&str, Option<&str>) = match options.lang_filter.as_deref() {
        Some(lang) => (
            "SELECT f.path, f.language, l.line_no, l.content
         FROM lines_fts JOIN files f ON f.id = lines_fts.file_id JOIN lines l ON l.file_id = lines_fts.file_id AND l.line_no = lines_fts.line_no WHERE lines_fts MATCH ?1 AND f.language = ?3 ORDER BY bm25(lines_fts), f.path, l.line_no LIMIT ?2",
            Some(lang),
        ),
        None => (
            "SELECT f.path, f.language, l.line_no, l.content
         FROM lines_fts JOIN files f ON f.id = lines_fts.file_id JOIN lines l ON l.file_id = lines_fts.file_id AND l.line_no = lines_fts.line_no WHERE lines_fts MATCH ?1 ORDER BY bm25(lines_fts), f.path, l.line_no LIMIT ?2",
            None,
        ),
    };
    let mut stmt = store.connection().prepare_cached(sql)?;
    let rows = match lang_bind {
        Some(lang) => stmt.query_map(params![fts_query, limit as i64, lang], map_line_row)?,
        None => stmt.query_map(params![fts_query, limit as i64], map_line_row)?,
    };
    for (rank, row) in rows.enumerate() {
        let (path, language, line_no, content) = row?;
        accumulate(
            options,
            &mut line_ranks,
            &mut line_meta,
            path,
            line_no,
            language,
            content,
            rank,
        );
    }
    Ok(hits_from_ranks(line_ranks, line_meta))
}
#[allow(clippy::too_many_arguments)]
fn accumulate(
    options: &SearchOptions,
    line_ranks: &mut LineRanks,
    line_meta: &mut LineMeta,
    path: String,
    line_no: u32,
    language: Option<String>,
    content: String,
    rank: usize,
) {
    if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
        return;
    }
    let key = (path, line_no);
    line_ranks.entry(key.clone()).or_default().push(rank);
    line_meta.insert(key, (language, content));
}
fn hits_from_ranks(line_ranks: LineRanks, mut line_meta: LineMeta) -> Vec<SearchHit> {
    line_ranks
        .into_iter()
        .map(|((path, line_no), ranks)| {
            let (language, content) = line_meta
                .remove(&(path.clone(), line_no))
                .unwrap_or((None, String::new()));
            asgrep_line_hit(path, language, line_no, content, score_lexical_rrf(&ranks))
        })
        .collect()
}
