use crate::query::{ParsedQuery, QueryMode};
use crate::search::passes::bmh::{
    asgrep_line_hit, build_file_lines_map, excerpt_opt, is_word_boundary, map_line_row,
    needs_context, BMH_LINE_THRESHOLD,
};
use crate::search::types::matches_lang;
use crate::search::types::{SearchHit, SearchOptions};
use crate::store::IndexStore;
use crate::Result;
use rusqlite::params;
pub fn literal_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let needle = match parsed.target.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(Vec::new()),
    };
    if store.indexed_line_count_at_least(BMH_LINE_THRESHOLD)? && needle.chars().count() >= 3 {
        literal_trigram(store, options, parsed, needle)
    } else {
        literal_sql(store, options, parsed, needle)
    }
}
fn literal_trigram(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
) -> Result<Vec<SearchHit>> {
    let query = crate::fts::escape_fts_term(needle);
    let file_map = if needs_context(options) {
        Some(build_file_lines_map(&store.all_indexed_lines()?))
    } else {
        None
    };
    let mut stmt = store.connection().prepare_cached(
        "SELECT f.path, f.language, l.line_no, l.content
         FROM lines_trigram JOIN lines l ON l.rowid = lines_trigram.rowid JOIN files f ON f.id = l.file_id WHERE lines_trigram MATCH ?1 ORDER BY f.path, l.line_no",
    )?;
    let rows = stmt.query_map(params![query], map_line_row)?;
    let needle_lower = options.case_insensitive.then(|| needle.to_lowercase());
    let word_mode = parsed.mode == QueryMode::Word;
    let mut hits = Vec::new();
    for row in rows {
        let (path, language, line_no, content) = row?;
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        if !content_matches_literal(&content, needle, needle_lower.as_deref(), word_mode) {
            continue;
        }
        let excerpt_text = excerpt_opt(file_map.as_ref(), &path, line_no, &content, options);
        hits.push(asgrep_line_hit(path, language, line_no, excerpt_text, 1.0));
        if hits.len() >= options.limit.max(100) {
            break;
        }
    }
    Ok(hits)
}
/// SQL templates for literal line scan: [case_insensitive][has_lang].
/// Case-insensitive → LIKE ESCAPE; case-sensitive → GLOB (no ESCAPE). Bead ast-sgrep-c2j5.
/// Lang filter in SQL before ORDER/LIMIT so a lang page cannot go empty (iva9.5).
const LITERAL_SQL: [[&str; 2]; 2] = [
    // case_sensitive (GLOB)
    [
        "SELECT f.path, f.language, l.line_no, l.content
         FROM lines l JOIN files f ON f.id = l.file_id WHERE l.content GLOB ?1 ORDER BY f.path, l.line_no LIMIT ?2",
        "SELECT f.path, f.language, l.line_no, l.content
         FROM lines l JOIN files f ON f.id = l.file_id WHERE l.content GLOB ?1 AND f.language = ?3 ORDER BY f.path, l.line_no LIMIT ?2",
    ],
    // case_insensitive (LIKE ESCAPE)
    [
        "SELECT f.path, f.language, l.line_no, l.content
         FROM lines l JOIN files f ON f.id = l.file_id WHERE l.content LIKE ?1 ESCAPE '\\' ORDER BY f.path, l.line_no LIMIT ?2",
        "SELECT f.path, f.language, l.line_no, l.content
         FROM lines l JOIN files f ON f.id = l.file_id WHERE l.content LIKE ?1 ESCAPE '\\' AND f.language = ?3 ORDER BY f.path, l.line_no LIMIT ?2",
    ],
];

#[inline]
fn literal_sql_template(case_insensitive: bool, has_lang: bool) -> &'static str {
    LITERAL_SQL[usize::from(case_insensitive)][usize::from(has_lang)]
}

fn literal_sql(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
) -> Result<Vec<SearchHit>> {
    // Escape metacharacters so the needle is matched literally.
    let limit = options.limit.max(100);
    let lang = options.lang_filter.as_deref();
    let pattern = if options.case_insensitive {
        format!("%{}%", crate::store::sql::escape_like_term(needle))
    } else {
        format!("*{}*", crate::store::sql::escape_glob_literal(needle))
    };
    let sql = literal_sql_template(options.case_insensitive, lang.is_some());
    let mut stmt = store.connection().prepare_cached(sql)?;
    let rows = match lang {
        Some(lang) => stmt.query_map(params![pattern, limit as i64, lang], map_line_row)?,
        None => stmt.query_map(params![pattern, limit as i64], map_line_row)?,
    };
    let word_mode = parsed.mode == QueryMode::Word;
    // SQL already matched the literal; word_mode only needs a boundary postfilter.
    let needle_lower = (word_mode && options.case_insensitive).then(|| needle.to_lowercase());
    let file_map = if needs_context(options) {
        Some(build_file_lines_map(&store.all_indexed_lines()?))
    } else {
        None
    };
    let mut hits = Vec::new();
    for (rank, row) in rows.enumerate() {
        let (path, language, line_no, content) = row?;
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        if word_mode
            && !content_matches_literal(&content, needle, needle_lower.as_deref(), true)
        {
            continue;
        }
        hits.push(asgrep_line_hit(
            path.clone(),
            language,
            line_no,
            excerpt_opt(file_map.as_ref(), &path, line_no, &content, options),
            1.0 / (1.0 + rank as f64 * 0.01),
        ));
    }
    Ok(hits)
}

/// Shared case-fold + word/substring gate used by both trigram and SQL residual paths.
/// Collapses the duplicated `if let Some(needle_lower)` decision tree (pass 8).
fn content_matches_literal(
    content: &str,
    needle: &str,
    needle_lower: Option<&str>,
    word_mode: bool,
) -> bool {
    match needle_lower {
        Some(nl) => has_literal_match(&content.to_lowercase(), nl, word_mode),
        None => has_literal_match(content, needle, word_mode),
    }
}

fn has_literal_match(haystack: &str, needle: &str, word_mode: bool) -> bool {
    if !word_mode {
        return haystack.contains(needle);
    }
    haystack
        .match_indices(needle)
        .any(|(pos, _)| is_word_boundary(haystack, pos, needle.len()))
}
