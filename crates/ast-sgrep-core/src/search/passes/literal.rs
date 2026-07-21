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
        let ok = if let Some(ref nl) = needle_lower {
            has_literal_match(&content.to_lowercase(), nl, word_mode)
        } else {
            has_literal_match(&content, needle, word_mode)
        };
        if !ok {
            continue;
        }
        let excerpt_text = excerpt_opt(file_map.as_ref(), &path, line_no, &content, options);
        hits.push(asgrep_line_hit(path, language, line_no, excerpt_text, 1.0));
    }
    Ok(hits)
}
fn literal_sql(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
) -> Result<Vec<SearchHit>> {
    let (op, pattern) = if options.case_insensitive {
        ("LIKE", format!("%{needle}%"))
    } else {
        ("GLOB", format!("*{needle}*"))
    };
    let limit = options.limit.max(100);
    let sql = format!(
        "SELECT f.path, f.language, l.line_no, l.content
         FROM lines l JOIN files f ON f.id = l.file_id WHERE l.content {op} ?1 ORDER BY f.path, l.line_no LIMIT ?2"
    );
    let mut stmt = store.connection().prepare_cached(&sql)?;
    let rows = stmt.query_map(params![pattern, limit as i64], map_line_row)?;
    let word_mode = parsed.mode == QueryMode::Word;
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
        if word_mode {
            let ok = if let Some(ref nl) = needle_lower {
                has_literal_match(&content.to_lowercase(), nl, true)
            } else {
                has_literal_match(&content, needle, true)
            };
            if !ok {
                continue;
            }
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
fn has_literal_match(haystack: &str, needle: &str, word_mode: bool) -> bool {
    if !word_mode {
        return haystack.contains(needle);
    }
    haystack
        .match_indices(needle)
        .any(|(pos, _)| is_word_boundary(haystack, pos, needle.len()))
}
