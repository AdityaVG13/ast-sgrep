use crate::query::ParsedQuery;
use crate::rank::score_lexical_rrf;
use crate::search::passes::bmh::asgrep_line_hit;
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
///
// Join-free hot path: the FTS table itself stores `file_id`, `line_no`, and
// `content` columns, so ranking + projection need no per-row joins. Only the
// ≤limit surviving rows resolve `(file_id → path, language)` via one bounded
// IN-list lookup — same output, a fraction of the join cost on large corpora.
fn lexical_from_field(
    store: &IndexStore,
    options: &SearchOptions,
    field: &str,
    fts_query: &str,
    limit: usize,
    matches: &mut LineMatches,
) -> Result<()> {
    let fts_query = fts_query.to_string();
    // Lang filter in SQL before ORDER/LIMIT so a lang page cannot go empty (iva9.5).
    let (sql, lang_bind): (String, Option<&str>) = match options.lang_filter.as_deref() {
        Some(lang) => (
            format!(
                "SELECT t.file_id, t.line_no, t.content \
                 FROM {field} t WHERE t MATCH ?1 AND t.file_id IN \
                   (SELECT id FROM files WHERE language = ?3) \
                 ORDER BY bm25({field}) LIMIT ?2"
            ),
            Some(lang),
        ),
        None => (
            format!(
                "SELECT t.file_id, t.line_no, t.content \
                 FROM {field} t WHERE t MATCH ?1 ORDER BY bm25({field}) LIMIT ?2"
            ),
            None,
        ),
    };
    let sql = sql.as_str();
    let mut stmt = store.connection().prepare_cached(sql)?;
    let rows: Vec<(i64, u32, String)> = match lang_bind {
        Some(lang) => {
            let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
            stmt.query_map(params![fts_query, limit as i64, lang], map)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        }
        None => {
            let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
            stmt.query_map(params![fts_query, limit as i64], map)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        }
    };
    if rows.is_empty() {
        return Ok(());
    }
    // Resolve identities for exactly the file_ids that survived ranking.
    let ids = {
        let mut seen = std::collections::HashSet::new();
        rows.iter()
            .map(|(id, _, _)| *id)
            .filter(|id| seen.insert(*id))
            .collect::<Vec<_>>()
    };
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let id_sql = format!("SELECT id, path, language FROM files WHERE id IN ({placeholders})");
    let mut ident_stmt = store.connection().prepare_cached(&id_sql)?;
    let bind: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let mut id_rows = ident_stmt.query(bind.as_slice())?;
    let mut idents: std::collections::HashMap<i64, (String, Option<String>)> =
        std::collections::HashMap::with_capacity(ids.len());
    while let Some(row) = id_rows.next()? {
        let id: i64 = row.get(0)?;
        let path: String = row.get(1)?;
        let language: Option<String> = row.get(2)?;
        idents.insert(id, (path, language));
    }
    drop(id_rows);
    drop(ident_stmt);
    for (rank, (file_id, line_no, content)) in rows.into_iter().enumerate() {
        // A file deleted between the two statements yields no identity; skip it.
        let Some((path, language)) = idents.get(&file_id) else {
            continue;
        };
        accumulate(
            options,
            matches,
            (path.clone(), language.clone(), line_no, content),
            rank,
        );
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
