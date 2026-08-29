use crate::query::{ParsedQuery, QueryMode};
use crate::search::passes::bmh::{
    asgrep_line_hit, attach_context, is_word_boundary, map_line_row, retained_limit,
    BMH_LINE_THRESHOLD,
};
use crate::search::types::matches_lang;
use crate::search::types::{SearchHit, SearchOptions};
use crate::store::trigram_df::TrigramShortcut;
use crate::store::IndexStore;
use crate::Result;
use memchr::memchr2;
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
    if let Some(corpus) = store.line_corpus()? {
        return scan_line_corpus(&corpus, store, options, parsed, needle);
    }
    if store.indexed_line_count_at_least(BMH_LINE_THRESHOLD)? && needle.chars().count() >= 3 {
        literal_trigram(store, options, parsed, needle)
    } else {
        literal_sql(store, options, parsed, needle)
    }
}

fn scan_line_corpus(
    corpus: &crate::store::line_corpus::LineCorpus,
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
) -> Result<Vec<SearchHit>> {
    let word_mode = parsed.mode == QueryMode::Word;
    let cap = options.limit.max(100);
    let lang = options.lang_filter.as_deref();
    let needle_lower = options.case_insensitive.then(|| needle.to_lowercase());
    let rows = if !options.case_insensitive {
        corpus.scan_cs(
            needle,
            word_mode,
            lang,
            cap,
            matches_lang,
            |content, pos, len| is_word_boundary(content, pos, len),
        )
    } else {
        corpus.scan_lines(lang, cap, matches_lang, |content| {
            content_matches_literal(content, needle, needle_lower.as_deref(), word_mode)
        })
    };
    let mut hits = Vec::with_capacity(rows.len());
    for (rank, row) in rows.into_iter().enumerate() {
        hits.push(asgrep_line_hit(
            row.path.to_string(),
            row.language.map(str::to_string),
            row.line_no,
            row.content.to_string(),
            1.0 / (1.0 + rank as f64 * 0.01),
        ));
    }
    hits.truncate(retained_limit(options));
    attach_context(store, options, &mut hits)?;
    Ok(hits)
}

fn literal_trigram(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
) -> Result<Vec<SearchHit>> {
    // Rarest-trigram df shortcut (br-umh): when trustworthy document-frequency
    // data shows one needle trigram to be rare, MATCH only that trigram instead
    // of making FTS5 intersect every trigram of the phrase. Safety: only
    // trigrams derived from the needle are candidates, so any candidate's
    // posting list is a superset of true matches, and content_matches_literal
    // reverify restores exactness — poisoned dfs can change speed, not output.
    if let TrigramShortcut::Match(terms) = store.trigram_df().scan_shortcut(store, needle) {
        let query = terms
            .iter()
            .map(|tri| crate::fts::escape_fts_term(tri))
            .collect::<Vec<_>>()
            .join(" AND ");
        return scan_trigram_matches(store, options, parsed, needle, &query);
    }
    let query = crate::fts::escape_fts_term(needle);
    scan_trigram_matches(store, options, parsed, needle, &query)
}

fn scan_trigram_matches(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
    query: &str,
) -> Result<Vec<SearchHit>> {
    // No ORDER BY here: a TEMP B-TREE sort would materialize the whole trigram
    // doclist before the first row, defeating the lazy budget break below.
    // Candidates stream in posting order, the loop stops at the retained
    // budget, and ordering by (path, line_no) is restored in Rust over the
    // small candidate set — identical output for under-budget queries.
    //
    // gauntlet-r13 (T1): non-word reverify is GLOB (case-sensitive) or
    // LIKE ESCAPE (ASCII case-insensitive, same predicate as literal_sql).
    // Pushed into SQL so rejected postings never pay valueToText + Rust
    // reverify. Word mode and non-ASCII CI keep the Rust verify.
    let word_mode = parsed.mode == QueryMode::Word;
    let sql_like = options.case_insensitive && !word_mode && needle.is_ascii();
    let sql_glob = !options.case_insensitive && !word_mode;
    let sql = if sql_like {
        "SELECT f.path, f.language, l.line_no, l.content \
         FROM lines_trigram JOIN lines l ON l.rowid = lines_trigram.rowid JOIN files f ON f.id = l.file_id \
         WHERE lines_trigram MATCH ?1 AND l.content LIKE ?2 ESCAPE '\\' LIMIT ?3"
    } else if sql_glob {
        "SELECT f.path, f.language, l.line_no, l.content \
         FROM lines_trigram JOIN lines l ON l.rowid = lines_trigram.rowid JOIN files f ON f.id = l.file_id \
         WHERE lines_trigram MATCH ?1 AND l.content GLOB ?2 LIMIT ?3"
    } else {
        "SELECT f.path, f.language, l.line_no, l.content \
         FROM lines_trigram JOIN lines l ON l.rowid = lines_trigram.rowid JOIN files f ON f.id = l.file_id \
         WHERE lines_trigram MATCH ?1"
    };
    let _tri_span = crate::perf_profile::Span::start(
        "literal_trigram_scan",
        "search",
        "trigram doclist walk + join",
    );
    let mut stmt = store.connection().prepare_cached(sql)?;
    let glob_pattern = format!("*{}*", crate::store::sql::escape_glob_literal(needle));
    let like_pattern = format!("%{}%", crate::store::sql::escape_like_term(needle));
    let needle_lower = options.case_insensitive.then(|| needle.to_lowercase());
    let cap = options.limit.max(100) as i64;
    // Lang-filtered scans must not SQL-LIMIT: skipped languages consume posting
    // slots in Rust. Unique hybrid has no lang filter, so LIMIT equals the
    // previous lazy break (posting order, first `cap` LIKE/GLOB rows).
    let sql_cap = if options.lang_filter.is_some() { i64::MAX } else { cap };
    let mut hits = Vec::new();
    if sql_like {
        let rows = stmt.query_map(params![query, like_pattern, sql_cap], map_line_row)?;
        for row in rows {
            let (path, language, line_no, content) = row?;
            if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
                continue;
            }
            hits.push(asgrep_line_hit(path, language, line_no, content, 1.0));
            if hits.len() >= options.limit.max(100) {
                break;
            }
        }
    } else if sql_glob {
        let rows = stmt.query_map(params![query, glob_pattern, sql_cap], map_line_row)?;
        for row in rows {
            let (path, language, line_no, content) = row?;
            if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
                continue;
            }
            hits.push(asgrep_line_hit(path, language, line_no, content, 1.0));
            if hits.len() >= options.limit.max(100) {
                break;
            }
        }
    } else {
        let rows = stmt.query_map(params![query], map_line_row)?;
        for row in rows {
            let (path, language, line_no, content) = row?;
            if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
                continue;
            }
            if !content_matches_literal(&content, needle, needle_lower.as_deref(), word_mode) {
                continue;
            }
            hits.push(asgrep_line_hit(path, language, line_no, content, 1.0));
            if hits.len() >= options.limit.max(100) {
                break;
            }
        }
    }
    drop(_tri_span);
    drop(stmt);
    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line_start.cmp(&b.line_start)));
    hits.truncate(retained_limit(options));
    attach_context(store, options, &mut hits)?;
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
    // Word mode post-filters rows for whole-word boundaries AFTER the SQL
    // window; substring-only rows consume window slots, so over-fetch by a
    // bounded multiple to give the filter candidates. Still capped so a huge
    // corpus cannot turn this into an unbounded scan.
    let sql_limit = if parsed.mode == QueryMode::Word {
        limit.saturating_mul(16)
    } else {
        limit
    };
    let lang = options.lang_filter.as_deref();
    let pattern = if options.case_insensitive {
        format!("%{}%", crate::store::sql::escape_like_term(needle))
    } else {
        format!("*{}*", crate::store::sql::escape_glob_literal(needle))
    };
    let sql = literal_sql_template(options.case_insensitive, lang.is_some());
    let mut stmt = store.connection().prepare_cached(sql)?;
    let rows = match lang {
        Some(lang) => stmt.query_map(params![pattern, sql_limit as i64, lang], map_line_row)?,
        None => stmt.query_map(params![pattern, sql_limit as i64], map_line_row)?,
    };
    let word_mode = parsed.mode == QueryMode::Word;
    // SQL already matched the literal; word_mode only needs a boundary postfilter.
    let needle_lower = (word_mode && options.case_insensitive).then(|| needle.to_lowercase());
    let mut hits = Vec::new();
    for (rank, row) in rows.enumerate() {
        let (path, language, line_no, content) = row?;
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        if word_mode && !content_matches_literal(&content, needle, needle_lower.as_deref(), true) {
            continue;
        }
        hits.push(asgrep_line_hit(
            path.clone(),
            language,
            line_no,
            content,
            1.0 / (1.0 + rank as f64 * 0.01),
        ));
    }
    drop(stmt);
    hits.truncate(retained_limit(options));
    attach_context(store, options, &mut hits)?;
    Ok(hits)
}

/// Shared case-fold + word/substring gate used by both trigram and SQL residual paths.
/// ASCII needles skip the per-line `to_lowercase()` allocation (unique-hybrid prefilter).
fn content_matches_literal(
    content: &str,
    needle: &str,
    needle_lower: Option<&str>,
    word_mode: bool,
) -> bool {
    match needle_lower {
        Some(nl) if needle.is_ascii() && content.is_ascii() => {
            has_ascii_ci_match(content.as_bytes(), nl.as_bytes(), word_mode)
        }
        Some(nl) => has_literal_match(&content.to_lowercase(), nl, word_mode),
        None => has_literal_match(content, needle, word_mode),
    }
}

fn has_ascii_ci_match(haystack: &[u8], needle: &[u8], word_mode: bool) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    let first_lo = needle[0].to_ascii_lowercase();
    let first_up = needle[0].to_ascii_uppercase();
    let mut from = 0;
    while from + needle.len() <= haystack.len() {
        let Some(off) = memchr2(first_lo, first_up, &haystack[from..]) else {
            return false;
        };
        let pos = from + off;
        if pos + needle.len() > haystack.len() {
            return false;
        }
        if haystack[pos..pos + needle.len()].eq_ignore_ascii_case(needle)
            && (!word_mode || ascii_word_boundary(haystack, pos, needle.len()))
        {
            return true;
        }
        from = pos + 1;
    }
    false
}

fn ascii_word_boundary(haystack: &[u8], pos: usize, needle_len: usize) -> bool {
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let left_ok = pos == 0 || !is_word(haystack[pos - 1]);
    let end = pos + needle_len;
    let right_ok = end == haystack.len() || !is_word(haystack[end]);
    left_ok && right_ok
}

fn has_literal_match(haystack: &str, needle: &str, word_mode: bool) -> bool {
    if !word_mode {
        return haystack.contains(needle);
    }
    haystack
        .match_indices(needle)
        .any(|(pos, _)| is_word_boundary(haystack, pos, needle.len()))
}

#[cfg(test)]
mod ascii_ci_tests {
    use super::content_matches_literal;

    fn agree(content: &str, needle: &str, word: bool) {
        let lower = needle.to_lowercase();
        let ascii = content_matches_literal(content, needle, Some(&lower), word);
        let unicode = super::has_literal_match(&content.to_lowercase(), &lower, word);
        assert_eq!(ascii, unicode, "content={content:?} needle={needle:?} word={word}");
    }

    #[test]
    fn ascii_ci_matches_unicode_lowercase_on_ascii_inputs() {
        for content in ["Encode payload", "encode payload", "ENCODE", "x_encode_y", "en"] {
            for needle in ["encode", "Encode", "payload"] {
                agree(content, needle, false);
                agree(content, needle, true);
            }
        }
    }
}
