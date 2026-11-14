use super::super::fusion::NameIntern;
use crate::query::ParsedQuery;
use crate::search::passes::bmh::{asgrep_line_hit, attach_regex_context, check_regex_deadline};
use crate::search::types::matches_lang;
use crate::search::types::{SearchHit, SearchOptions};
use crate::store::IndexStore;
use crate::{Result, StoreError};
use regex::Regex;
use std::sync::Arc;
use std::time::{Duration, Instant};
/// Default wall-clock budget for `regex:` scans (ReDoS / large-corpus hang guard).
pub const DEFAULT_REGEX_BUDGET_MS: u64 = 2_000;
fn regex_budget() -> Duration {
    std::env::var("ASGREP_REGEX_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(DEFAULT_REGEX_BUDGET_MS))
}
fn regex_deadline(start: Instant, budget: Duration) -> Result<Instant> {
    start.checked_add(budget).ok_or_else(|| {
        StoreError::Other(format!(
            "regex wall-clock budget of {}ms is too large",
            budget.as_millis()
        ))
    })
}
pub fn regex_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let pattern = match parsed.target.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(Vec::new()),
    };
    if pattern.chars().count() > crate::limits::MAX_REGEX_PATTERN_CHARS {
        return Err(StoreError::Other(format!(
            "regex pattern exceeds maximum of {} characters",
            crate::limits::MAX_REGEX_PATTERN_CHARS
        )));
    }
    let re = if options.case_insensitive {
        Regex::new(&format!("(?i){pattern}"))
    } else {
        Regex::new(pattern)
    }
    .map_err(|e| StoreError::Other(format!("invalid regex: {e}")))?;
    let trigram_literal = required_literal(pattern);
    let file_filter = options
        .file_filter
        .as_deref()
        .map(super::super::scope::compile_glob)
        .transpose()
        .map_err(StoreError::Other)?;
    let budget = regex_budget();
    let deadline = regex_deadline(Instant::now(), budget)?;
    // Bundle-backed scan first: the per-file trigram filters prune the same
    // way a trigram doclist does, at 2.2MB instead of a full substring index.
    // The SQL arms below stay for indexes without a read bundle.
    let mut hits = if let Some(corpus) = store.line_corpus_for_query()? {
        let candidates = match trigram_literal.as_deref() {
            Some(literal) => corpus.files_may_contain(literal),
            None => (0..corpus.file_count()).collect(),
        };
        scan_regex_corpus(
            &corpus,
            &candidates,
            &re,
            file_filter.as_ref(),
            options,
            deadline,
            budget,
        )?
    } else if let Some(literal) = trigram_literal.as_deref() {
        let query = crate::fts::escape_fts_term(literal);
        let mut stmt = store.connection().prepare_cached(
            "SELECT f.path, l.line_no, l.content, f.language
             FROM lines_trigram
             JOIN lines l ON l.rowid = lines_trigram.rowid
             JOIN files f ON f.id = l.file_id
             WHERE lines_trigram MATCH ?1
             ORDER BY f.path, l.line_no",
        )?;
        let rows = stmt.query_map([query], map_regex_row)?;
        scan_regex_rows(rows, &re, file_filter.as_ref(), options, deadline, budget)?
    } else {
        let mut stmt = store.connection().prepare_cached(
            "SELECT f.path, l.line_no, l.content, f.language
             FROM lines l JOIN files f ON f.id = l.file_id
             ORDER BY f.path, l.line_no",
        )?;
        let rows = stmt.query_map([], map_regex_row)?;
        scan_regex_rows(rows, &re, file_filter.as_ref(), options, deadline, budget)?
    };
    hits.truncate(options.retained_limit());
    attach_regex_context(store, options, &mut hits, deadline, budget)?;
    Ok(hits)
}

/// Regex scan over the resident line corpus.
///
/// `candidates` are ascending file indices (path order), already pruned by the
/// required literal's trigrams. Admission, the per-file preference cap, and
/// the rank-decayed score mirror [`scan_regex_rows`] so the two arms agree on
/// membership and order; only the rank's candidate stream differs, and both
/// streams are monotonic in (path, line_no).
fn scan_regex_corpus(
    corpus: &crate::store::line_corpus::LineCorpus,
    candidates: &[usize],
    re: &Regex,
    file_filter: Option<&Regex>,
    options: &SearchOptions,
    deadline: Instant,
    budget: Duration,
) -> Result<Vec<SearchHit>> {
    const PREFERRED_PER_FILE: usize = 3;
    let candidate_limit = options.retained_limit().max(100);
    let mut preferred = Vec::new();
    let mut overflow = Vec::new();
    let mut intern = NameIntern::new(candidate_limit);
    let mut per_file: Vec<usize> = Vec::new();
    let mut admitted = 0usize;
    corpus.scan_files(candidates, |line| {
        check_regex_deadline(deadline, budget)?;
        if file_filter.is_some_and(|filter| !filter.is_match(line.path))
            || !matches_lang(line.language, options.lang_filter.as_deref())
            || !re.is_match(line.content)
        {
            return Ok(true);
        }
        let hit = asgrep_line_hit(
            line.path.to_string(),
            line.language.map(str::to_owned),
            line.line_no,
            line.content.to_string(),
            regex_rank_score(admitted),
        );
        admitted += 1;
        let id = intern.intern(line.path) as usize;
        if per_file.len() <= id {
            per_file.resize(id + 1, 0);
        }
        if per_file[id] < PREFERRED_PER_FILE {
            per_file[id] += 1;
            preferred.push(hit);
        } else if overflow.len() < candidate_limit {
            overflow.push(hit);
        }
        Ok(preferred.len() < candidate_limit)
    })?;
    preferred.extend(overflow.into_iter().take(candidate_limit - preferred.len()));
    Ok(preferred)
}
fn required_literal(pattern: &str) -> Option<String> {
    if pattern.contains("(?") {
        return None;
    }
    let mut runs = Vec::new();
    let mut run = String::new();
    let mut escaped = false;
    for ch in pattern.chars() {
        if escaped {
            escaped = false;
            if matches!(ch, 'x' | 'u' | 'U' | 'p' | 'P') {
                return None;
            }
            if ch.is_ascii_alphanumeric() {
                if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            } else {
                run.push(ch);
            }
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' => {
                // A character class's members are alternatives, not required
                // text: harvesting them (or their tails) as a trigram literal
                // silently drops regex-matching lines (false negatives).
                // Leading-`]` and `\]` membership make precise scanning
                // nontrivial; bail conservatively instead.
                return None;
            }
            ']' => {}
            '|' | '?' | '*' => return None,
            '{' => {
                // Repetition bounds are syntax, never required source text.
                // Escaped braces are handled above as literal characters.
                return None;
            }
            _ if ch.is_ascii_alphanumeric() || ch == '_' => run.push(ch),
            _ if !run.is_empty() => runs.push(std::mem::take(&mut run)),
            _ => {}
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs.into_iter()
        .filter(|s| s.len() >= 3)
        .max_by_key(String::len)
}
fn map_regex_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::store::IndexedLineRow> {
    Ok((
        Arc::<str>::from(row.get::<_, String>(0)?),
        row.get(1)?,
        row.get(2)?,
        row.get::<_, Option<String>>(3)?.map(Arc::from),
    ))
}

/// Positional decay for a regex hit.
///
/// The rank is the hit's position among *admitted* results, not among scanned
/// candidates. Candidate streams differ by arm (a trigram doclist versus the
/// lines of bloom-admitted files) and by index shape, so a candidate-relative
/// rank made the same query score differently depending on which arm answered
/// it. Ranking admitted hits keeps the score a property of the result set.
fn regex_rank_score(admitted: usize) -> f64 {
    1.0 / (1.0 + admitted as f64 * 0.01)
}

fn scan_regex_rows(
    rows: impl Iterator<Item = rusqlite::Result<crate::store::IndexedLineRow>>,
    re: &Regex,
    file_filter: Option<&Regex>,
    options: &SearchOptions,
    deadline: Instant,
    budget: Duration,
) -> Result<Vec<SearchHit>> {
    const PREFERRED_PER_FILE: usize = 3;
    let candidate_limit = options.retained_limit().max(100);
    let mut preferred = Vec::new();
    let mut overflow = Vec::new();
    let mut intern = NameIntern::new(candidate_limit);
    let mut per_file: Vec<usize> = Vec::new();
    let mut admitted = 0usize;
    for row in rows {
        // Between-line deadline check (56w1.3): a zero budget must fail
        // immediately instead of scanning everything.
        check_regex_deadline(deadline, budget)?;
        let (path, line_no, content, language) = row?;
        if file_filter.is_some_and(|filter| !filter.is_match(&path))
            || !matches_lang(language.as_deref(), options.lang_filter.as_deref())
            || !re.is_match(&content)
        {
            continue;
        }
        let hit = asgrep_line_hit(
            path.to_string(),
            language.as_deref().map(str::to_owned),
            line_no,
            content,
            regex_rank_score(admitted),
        );
        admitted += 1;
        let id = intern.intern(path.as_ref()) as usize;
        if per_file.len() <= id {
            per_file.resize(id + 1, 0);
        }
        if per_file[id] < PREFERRED_PER_FILE {
            per_file[id] += 1;
            preferred.push(hit);
        } else if overflow.len() < candidate_limit {
            overflow.push(hit);
        }
        if preferred.len() >= candidate_limit {
            break;
        }
    }
    preferred.extend(overflow.into_iter().take(candidate_limit - preferred.len()));
    Ok(preferred)
}
