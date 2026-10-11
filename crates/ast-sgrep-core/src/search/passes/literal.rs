use crate::query::{ParsedQuery, QueryMode};
use crate::search::passes::bmh::{
    asgrep_line_hit, attach_context, is_word_boundary, map_borrowed_line_row, map_line_row,
    BMH_LINE_THRESHOLD,
};
use crate::search::types::matches_lang;
use crate::search::types::{SearchHit, SearchOptions};
use crate::store::line_corpus::cascade_prefers_file;
use crate::store::trigram_df::TrigramShortcut;
use crate::store::IndexStore;
use crate::Result;
use memchr::memchr2;
use rusqlite::params;
use std::collections::HashSet;

pub fn literal_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let needle = match parsed.target.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(Vec::new()),
    };
    // Cached-only: a one-shot query must not pay the full `lines`
    // materialization + token-index build (flamegraph 2026-09-20). Warmed
    // sessions (codemode-serve) hold the corpus and keep the memchr scan;
    // everyone else takes the trigram/SQL path below.
    if let Some(corpus) = store.line_corpus_if_cached()? {
        return scan_line_corpus(&corpus, store, options, parsed, needle);
    }
    // Neither SQLite LIKE nor trigram normalization is a superset of Rust's
    // Unicode lowercase matching (including expansions such as İ -> i + dot).
    // MATCH parses a NUL-terminated query; NUL-bearing needles need SQL INSTR.
    if !options.case_insensitive
        && !needle.contains('\0')
        && store.indexed_line_count_at_least(BMH_LINE_THRESHOLD)?
        && needle.chars().count() >= 3
    {
        literal_trigram(store, options, parsed, needle)
    } else {
        literal_sql(store, options, needle, parsed.mode == QueryMode::Word, None)
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
    let cap = options.retained_limit().max(100);
    let lang = options.lang_filter.as_deref();
    let needle_lower = options.case_insensitive.then(|| needle.to_lowercase());
    let rows = if !options.case_insensitive {
        corpus.scan_cs(needle, word_mode, lang, cap, matches_lang, is_word_boundary)
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
    hits.truncate(options.retained_limit());
    attach_context(store, options, &mut hits)?;
    Ok(hits)
}

fn literal_trigram(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
) -> Result<Vec<SearchHit>> {
    // Rarest-trigram df shortcut: when trustworthy document-frequency
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
        // Static (unarmed) picks are blind to this corpus: budget the probe
        // scan and fall back to the phrase MATCH on exhaustion. Armed picks
        // carry exact dfs, so they keep today's unbounded shape.
        let budget = if store.trigram_df().is_armed() {
            None
        } else {
            Some(STATIC_SCAN_BUDGET)
        };
        let (hits, exhausted) =
            scan_trigram_matches(store, options, parsed, needle, &query, budget)?;
        if !exhausted {
            return Ok(hits);
        }
    }
    let query = crate::fts::escape_fts_term(needle);
    let (hits, _) = scan_trigram_matches(store, options, parsed, needle, &query, None)?;
    Ok(hits)
}

/// Posting rows a static-prior probe may examine before it falls back to
/// the phrase MATCH. Good picks complete near the 100-hit cap; a locally
/// flooding pick would otherwise walk thousands of rejects.
const STATIC_SCAN_BUDGET: usize = 400;

/// File discovery counts distinct files, not matching lines. Reuse the literal
/// verifier, but do not let a dense file exhaust the line lane's window before
/// another file reaches the structural stages. Prefilter needles have 3+ chars.
/// Sorting gives each file its first matching line as deterministic evidence.
pub(crate) fn literal_file_pass(
    store: &IndexStore,
    options: &SearchOptions,
    needle: &str,
    already: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    literal_sql(store, options, needle, false, Some(already))
}

fn scan_trigram_matches(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    needle: &str,
    query: &str,
    scan_budget: Option<usize>,
) -> Result<(Vec<SearchHit>, bool)> {
    // No ORDER BY here: a TEMP B-TREE sort would materialize the whole trigram
    // doclist before the first row, defeating the lazy budget break below.
    // Candidates stream in posting order, the loop stops at the retained
    // budget, and ordering by (path, line_no) is restored in Rust over the
    // small candidate set — identical output for under-budget queries.
    //
    // This lane is case-sensitive only. BLOB INSTR checks the full byte span,
    // including NUL bytes that terminate GLOB's comparison, without text-mode
    // codepoint counting. Word boundaries still need the shared Rust predicate.
    let word_mode = parsed.mode == QueryMode::Word;
    let sql_contains = !word_mode;
    let sql = if sql_contains {
        "SELECT f.path, f.language, l.line_no, l.content \
         FROM lines_trigram JOIN lines l ON l.rowid = lines_trigram.rowid JOIN files f ON f.id = l.file_id \
         WHERE lines_trigram MATCH ?1 AND instr(CAST(l.content AS BLOB), CAST(?2 AS BLOB)) > 0 LIMIT ?3"
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
    let cap = options.retained_limit().max(100) as i64;
    // Lang-filtered scans must not SQL-LIMIT: skipped languages consume posting
    // slots in Rust. Unique hybrid has no lang filter, so LIMIT equals the
    // previous lazy break (posting order, first `cap` substring rows).
    let sql_cap = if options.lang_filter.is_some() {
        i64::MAX
    } else {
        cap
    };
    let rows = if sql_contains {
        stmt.query_map(params![query, needle, sql_cap], map_line_row)?
    } else {
        stmt.query_map(params![query], map_line_row)?
    };
    let needs_content_check = !sql_contains;
    let mut hits = Vec::new();
    let mut scanned = 0usize;
    for row in rows {
        let (path, language, line_no, content) = row?;
        scanned += 1;
        if scan_budget.is_some_and(|budget| scanned > budget) {
            return Ok((Vec::new(), true));
        }
        if !matches_lang(language.as_deref(), options.lang_filter.as_deref()) {
            continue;
        }
        if needs_content_check && !content_matches_literal(&content, needle, None, word_mode) {
            continue;
        }
        hits.push(asgrep_line_hit(path, language, line_no, content, 1.0));
        if hits.len() >= options.retained_limit().max(100) {
            break;
        }
    }
    drop(_tri_span);
    drop(stmt);
    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line_start.cmp(&b.line_start)));
    // Same rank-decayed scores as the corpus/SQL literal paths: warm (corpus)
    // and cold (trigram) searches must agree hit-for-hit, and `finish` sorts
    // by score, so flat 1.0s would order ties differently.
    for (rank, hit) in hits.iter_mut().enumerate() {
        hit.score = 1.0 / (1.0 + rank as f64 * 0.01);
    }
    hits.truncate(options.retained_limit());
    attach_context(store, options, &mut hits)?;
    Ok((hits, false))
}
/// Ordered cold candidates share the warm lane's acceptance predicate.
/// The cap counts accepted lines (or distinct files), never substring rejects.
/// CI scans cannot prefilter via LIKE/FTS without losing Unicode matches.
fn literal_sql(
    store: &IndexStore,
    options: &SearchOptions,
    needle: &str,
    word_mode: bool,
    distinct_files: Option<&HashSet<String>>,
) -> Result<Vec<SearchHit>> {
    let limit = if distinct_files.is_some() {
        options.limit
    } else {
        options.retained_limit().max(100)
    };
    if limit == 0 {
        return Ok(Vec::new());
    }
    let lang = options.lang_filter.as_deref();
    let mut excluded = Vec::new();
    if options.case_insensitive {
        if let Some(already) = distinct_files {
            if let Some(frontier) = already.iter().max() {
                // Earlier successful discovery scans fully decoded every path
                // before this frontier in the same read snapshot. The frontier
                // file itself may have stopped at its first matching line.
                excluded.extend(
                    already
                        .iter()
                        .filter(|path| *path < frontier)
                        .map(String::as_str),
                );
            }
        }
    }
    excluded.sort_unstable();
    let excluded_sql = if excluded.is_empty() {
        None
    } else {
        // Bucket statement shapes without changing the excluded set.
        excluded.resize(
            excluded.len().next_power_of_two(),
            *excluded.last().unwrap(),
        );
        Some(format!(
            "SELECT f.path, f.language, l.line_no, l.content \
             FROM lines l JOIN files f ON f.id = l.file_id \
             WHERE f.path NOT IN ({}) ORDER BY f.path, l.line_no",
            vec!["?"; excluded.len()].join(",")
        ))
    };
    let sql = match (options.case_insensitive, lang.is_some()) {
        (true, _) => excluded_sql.as_deref().unwrap_or("SELECT f.path, f.language, l.line_no, l.content
            FROM lines l JOIN files f ON f.id = l.file_id ORDER BY f.path, l.line_no"),
        (false, false) => "SELECT f.path, f.language, l.line_no, l.content
            FROM lines l JOIN files f ON f.id = l.file_id WHERE instr(CAST(l.content AS BLOB), CAST(?1 AS BLOB)) > 0 ORDER BY f.path, l.line_no LIMIT ?2",
        (false, true) => "SELECT f.path, f.language, l.line_no, l.content
            FROM lines l JOIN files f ON f.id = l.file_id WHERE instr(CAST(l.content AS BLOB), CAST(?1 AS BLOB)) > 0 AND f.language = ?3 ORDER BY f.path, l.line_no LIMIT ?2",
    };
    let mut stmt = store.connection().prepare_cached(sql)?;
    // Preserve the SQL bound when every substring candidate is an accepted
    // line. Word/file admission must run before applying any hit cap.
    let sql_limit = if word_mode || distinct_files.is_some() {
        i64::MAX
    } else {
        limit as i64
    };
    let mut rows = match (options.case_insensitive, lang) {
        (true, _) => stmt.query(rusqlite::params_from_iter(excluded.iter().copied()))?,
        (false, lang) => match lang {
            Some(lang) => stmt.query(params![needle, sql_limit, lang])?,
            None => stmt.query(params![needle, sql_limit])?,
        },
    };
    let needle_lower = options.case_insensitive.then(|| needle.to_lowercase());
    let mut hits = Vec::new();
    let mut secondary: Vec<SearchHit> = Vec::new();
    while let Some(row) = rows.next()? {
        // Validate every projected field before duplicate/language rejection,
        // as the owned mapper did; borrow only until evidence is admitted.
        let (path, language, line_no, content) = map_borrowed_line_row(row)?;
        if distinct_files.is_some_and(|already| {
            already.contains(path.as_ref())
                || hits
                    .last()
                    .is_some_and(|hit: &SearchHit| hit.file == path.as_ref())
                || secondary
                    .last()
                    .is_some_and(|hit| hit.file == path.as_ref())
        }) || !matches_lang(language.as_deref(), options.lang_filter.as_deref())
            || ((options.case_insensitive || word_mode)
                && !content_matches_literal(&content, needle, needle_lower.as_deref(), word_mode))
        {
            continue;
        }
        let preferred = distinct_files.is_none() || cascade_prefers_file(&path);
        let score = if distinct_files.is_some() {
            1.0
        } else {
            1.0 / (1.0 + hits.len() as f64 * 0.01)
        };
        let bucket = if preferred { &mut hits } else { &mut secondary };
        if bucket.len() >= limit {
            continue;
        }
        bucket.push(asgrep_line_hit(
            path.into_owned(),
            language.map(|language| language.into_owned()),
            line_no,
            content.into_owned(),
            score,
        ));
        if preferred && hits.len() >= limit {
            break;
        }
    }
    drop(rows);
    drop(stmt);
    if distinct_files.is_some() {
        // Nonpreferred files may be path-early. Hold only a bounded fallback
        // page until preferred admission has exhausted the same ordered scan.
        hits.extend(secondary.into_iter().take(limit.saturating_sub(hits.len())));
    } else {
        hits.truncate(options.retained_limit());
    }
    attach_context(store, options, &mut hits)?;
    Ok(hits)
}

/// Shared case-fold + word/substring gate used by both trigram and SQL residual paths.
/// ASCII needles skip the per-line `to_lowercase()` allocation (unique-hybrid prefilter).
pub fn content_matches_literal(
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

pub fn has_literal_match(haystack: &str, needle: &str, word_mode: bool) -> bool {
    if !word_mode {
        return haystack.contains(needle);
    }
    haystack
        .match_indices(needle)
        .any(|(pos, _)| is_word_boundary(haystack, pos, needle.len()))
}
