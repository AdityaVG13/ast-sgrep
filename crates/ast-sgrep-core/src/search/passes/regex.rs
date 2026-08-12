use crate::query::ParsedQuery;
use crate::search::passes::bmh::{asgrep_line_hit, attach_regex_context, retained_limit};
use crate::search::types::matches_lang;
use crate::search::types::{SearchHit, SearchOptions};
use crate::store::IndexStore;
use crate::{Result, StoreError};
use regex::Regex;
use std::collections::HashMap;
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
        .map(super::super::compile_glob)
        .transpose()
        .map_err(StoreError::Other)?;
    let budget = regex_budget();
    let deadline = regex_deadline(Instant::now(), budget)?;
    let mut hits = if let Some(literal) = trigram_literal.as_deref() {
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
    hits.truncate(retained_limit(options));
    attach_regex_context(store, options, &mut hits, deadline, budget)?;
    Ok(hits)
}
fn required_literal(pattern: &str) -> Option<String> {
    if pattern.contains("(?") {
        return None;
    }
    let mut runs = Vec::new();
    let mut run = String::new();
    let mut escaped = false;
    let mut in_class = false;
    let chars: Vec<char> = pattern.chars().collect();
    for (index, &ch) in chars.iter().enumerate() {
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
                in_class = true;
                if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            }
            ']' => in_class = false,
            '|' | '?' | '*' if !in_class => return None,
            '{' if !in_class
                && chars[index..]
                    .iter()
                    .take(3)
                    .collect::<String>()
                    .starts_with("{0") =>
            {
                return None;
            }
            _ if !in_class && (ch.is_ascii_alphanumeric() || ch == '_') => run.push(ch),
            _ if !in_class && !run.is_empty() => runs.push(std::mem::take(&mut run)),
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

fn scan_regex_rows(
    rows: impl Iterator<Item = rusqlite::Result<crate::store::IndexedLineRow>>,
    re: &Regex,
    file_filter: Option<&Regex>,
    options: &SearchOptions,
    deadline: Instant,
    budget: Duration,
) -> Result<Vec<SearchHit>> {
    const PREFERRED_PER_FILE: usize = 3;
    let candidate_limit = options.limit.max(100);
    let mut preferred = Vec::new();
    let mut overflow = Vec::new();
    let mut per_file = HashMap::<Arc<str>, usize>::new();
    for (rank, row) in rows.enumerate() {
        // Between-line deadline check (56w1.3): a zero budget must fail
        // immediately instead of scanning everything.
        if Instant::now() >= deadline {
            return Err(StoreError::Other(format!(
                "regex search exceeded wall-clock budget of {}ms (ASGREP_REGEX_BUDGET_MS); partial results discarded",
                budget.as_millis()
            )));
        }
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
            1.0 / (1.0 + rank as f64 * 0.01),
        );
        let count = per_file.entry(path).or_default();
        if *count < PREFERRED_PER_FILE {
            *count += 1;
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

#[cfg(test)]
#[path = "../../../../../tests/unit/core/search__passes__regex.rs"]
mod tests;
