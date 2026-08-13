use crate::query::ParsedQuery;
use crate::rank::score_lexical_rrf;
use crate::search::passes::bmh::{asgrep_line_hit, map_line_row};
use crate::search::types::matches_lang;
use crate::search::types::{SearchHit, SearchOptions};
use crate::store::IndexStore;
use crate::Result;
use rusqlite::params;
use std::collections::HashMap;
type LineMatches = HashMap<(String, u32), (Vec<usize>, Option<String>, String)>;

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
        let sidecar = crate::tantivy_index::TantivySidecar::open_for_index(
            &options.root,
            options.index_path.as_deref(),
        )?;
        if sidecar.exists() && sidecar.is_fresh(store.index_data_version()?)? {
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
    let mut matches = LineMatches::new();
    let limit = lexical_pool_limit(options);
    for (file, line_no, content, language, rank) in sidecar.search(&parsed.terms, limit)? {
        accumulate(
            options,
            &mut matches,
            (file, language, line_no, content),
            rank,
        );
    }
    Ok(hits_from_matches(matches))
}
fn lexical_from_fts(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let mut matches = LineMatches::new();
    let fts_query = crate::fts::escape_fts_query(&parsed.terms);
    let limit = lexical_pool_limit(options);
    // Apply lang filter in SQL before LIMIT so path order cannot drop matching langs (iva9.5 sibling).
    // vvpk: pick the analyzer that matches the query. `lines_fts` is porter
    // stemmed, which is right for prose and wrong for identifiers -- it folds
    // `indexing` into `index` and splits `refresh_token`. `lines_code_fts` is
    // unstemmed with `_` as a token character.
    // Routing to only one field can miss a query that needs the other analyzer,
    // so try the matching field first and retain the other as a fallback.
    let (primary, fallback) = if query_is_code_like(parsed) {
        ("lines_code_fts", "lines_fts")
    } else {
        ("lines_fts", "lines_code_fts")
    };
    lexical_from_field(store, options, primary, &fts_query, limit, &mut matches)?;
    // Fallback, not union: always querying both can crowd primary candidates
    // out of the bounded pool. Use the second analyzer only when the primary
    // one did not fill it.
    if matches.len() < limit {
        lexical_from_field(store, options, fallback, &fts_query, limit, &mut matches)?;
    }
    Ok(hits_from_matches(matches))
}

/// Run the lexical query against one analyzer field (vvpk).
fn lexical_from_field(
    store: &IndexStore,
    options: &SearchOptions,
    field: &str,
    fts_query: &str,
    limit: usize,
    matches: &mut LineMatches,
) -> Result<()> {
    let fts_query = fts_query.to_string();
    let (sql, lang_bind): (String, Option<&str>) = match options.lang_filter.as_deref() {
        Some(lang) => (
            format!(
                "SELECT f.path, f.language, l.line_no, l.content
         FROM {field} JOIN files f ON f.id = {field}.file_id JOIN lines l ON l.file_id = {field}.file_id AND l.line_no = {field}.line_no WHERE {field} MATCH ?1 AND f.language = ?3 ORDER BY bm25({field}), f.path, l.line_no LIMIT ?2"
            ),
            Some(lang),
        ),
        None => (
            format!(
                "SELECT f.path, f.language, l.line_no, l.content
         FROM {field} JOIN files f ON f.id = {field}.file_id JOIN lines l ON l.file_id = {field}.file_id AND l.line_no = {field}.line_no WHERE {field} MATCH ?1 ORDER BY bm25({field}), f.path, l.line_no LIMIT ?2"
            ),
            None,
        ),
    };
    let sql = sql.as_str();
    let mut stmt = store.connection().prepare_cached(sql)?;
    let rows = match lang_bind {
        Some(lang) => stmt.query_map(params![fts_query, limit as i64, lang], map_line_row)?,
        None => stmt.query_map(params![fts_query, limit as i64], map_line_row)?,
    };
    for (rank, row) in rows.enumerate() {
        accumulate(options, matches, row?, rank);
    }
    Ok(())
}
fn accumulate(
    options: &SearchOptions,
    matches: &mut LineMatches,
    (path, language, line_no, content): (String, Option<String>, u32, String),
    rank: usize,
) {
    if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
        return;
    }
    matches
        .entry((path, line_no))
        .or_insert_with(|| (Vec::new(), language, content))
        .0
        .push(rank);
}
fn hits_from_matches(matches: LineMatches) -> Vec<SearchHit> {
    matches
        .into_iter()
        .map(|((path, line_no), (ranks, language, content))| {
            asgrep_line_hit(path, language, line_no, content, score_lexical_rrf(&ranks))
        })
        .collect()
}

/// Does this query look like code rather than prose (vvpk)?
///
/// Identifier shapes must not be stemmed: `refresh_token`, `HTTPStatus`, and
/// `Store::open` mean exactly themselves. Natural-language questions benefit
/// from stemming, so they keep the porter field.
///
/// Deliberately conservative: anything that is not clearly identifier-shaped
/// stays on the prose analyzer, because wrongly skipping stemming on prose
/// costs recall.
pub(crate) fn query_is_code_like(parsed: &ParsedQuery) -> bool {
    let raw = parsed.raw.trim();
    if raw.is_empty() {
        return false;
    }
    // Explicit code-ish modes always use the code field.
    if !matches!(parsed.mode, crate::query::QueryMode::Hybrid) {
        return true;
    }
    let words: Vec<&str> = raw.split_whitespace().collect();
    // A multi-word natural-language question is prose.
    if words.len() > 3 {
        return false;
    }
    words.iter().any(|word| {
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':');
        if trimmed.len() < 2 {
            return false;
        }
        let has_underscore = trimmed.contains('_');
        let has_path = trimmed.contains("::") || trimmed.contains('.');
        // camelCase / PascalCase: an interior uppercase after a lowercase.
        let camel = trimmed
            .chars()
            .zip(trimmed.chars().skip(1))
            .any(|(a, b)| a.is_lowercase() && b.is_uppercase());
        let shouty = trimmed.chars().filter(|c| c.is_uppercase()).count() >= 2
            && trimmed.chars().any(|c| c.is_lowercase());
        has_underscore || has_path || camel || shouty
    })
}
