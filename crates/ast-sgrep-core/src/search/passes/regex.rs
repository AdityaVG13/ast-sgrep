use crate::query::ParsedQuery;
use crate::search::passes::bmh::{build_excerpt_with_context, build_file_lines_map};
use crate::search::types::matches_lang;
use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::IndexStore;
use crate::{Result, StoreError};
use regex::Regex;
use std::sync::Arc;
use std::thread;
pub fn regex_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
) -> Result<Vec<SearchHit>> {
    let pattern = match parsed.target.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(Vec::new()),
    };
    let re = if options.case_insensitive {
        Regex::new(&format!("(?i){pattern}"))
    } else {
        Regex::new(pattern)
    }
    .map_err(|e| StoreError::Other(format!("invalid regex: {e}")))?;
    let lines = if let Some(literal) = required_literal(pattern) {
        trigram_regex_candidates(store, &literal)?
    } else {
        store.all_indexed_lines()?
    };
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(lines.len());
    let chunk_size = lines.len().div_ceil(num_threads).max(1);
    let file_map = if options.context_before > 0 || options.context_after > 0 {
        Some(Arc::new(build_file_lines_map(&store.all_indexed_lines()?)))
    } else {
        None
    };
    let (context_before, context_after) = (options.context_before, options.context_after);
    let lang_filter = options.lang_filter.clone();
    let re = Arc::new(re);
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for chunk in lines.chunks(chunk_size) {
            let re = Arc::clone(&re);
            let file_map = file_map.clone();
            let lang_filter = lang_filter.clone();
            handles.push(scope.spawn(move || {
                scan_regex_chunk(
                    chunk,
                    &re,
                    &lang_filter,
                    context_before,
                    context_after,
                    file_map.as_deref(),
                )
            }));
        }
        Ok(handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect())
    })
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
fn trigram_regex_candidates(
    store: &IndexStore,
    literal: &str,
) -> Result<Vec<crate::store::IndexedLineRow>> {
    let query = crate::fts::escape_fts_term(literal);
    let mut stmt = store.connection().prepare_cached( "SELECT f.path, l.line_no, l.content, f.language
         FROM lines_trigram JOIN lines l ON l.rowid = lines_trigram.rowid JOIN files f ON f.id = l.file_id WHERE lines_trigram MATCH ?1 ORDER BY f.path, l.line_no",
    )?;
    let rows = stmt.query_map([query], |row| {
        Ok((
            Arc::<str>::from(row.get::<_, String>(0)?),
            row.get(1)?,
            row.get(2)?,
            row.get::<_, Option<String>>(3)?.map(Arc::from),
        ))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}
fn scan_regex_chunk(
    chunk: &[crate::store::IndexedLineRow],
    re: &Regex,
    lang_filter: &Option<String>,
    context_before: usize,
    context_after: usize,
    file_map: Option<&crate::search::passes::bmh::FileLinesMap>,
) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for (rank, (path, line_no, content, language)) in chunk.iter().enumerate() {
        if !matches_lang(language.as_deref(), lang_filter.as_deref()) || !re.is_match(content) {
            continue;
        }
        let excerpt = file_map.map_or_else(
            || content.to_string(),
            |fm| {
                build_excerpt_with_context(
                    fm,
                    path,
                    *line_no,
                    content,
                    context_before,
                    context_after,
                )
            },
        );
        hits.push(SearchHit::span(SpanHitInput {
            kind: HitKind::Asgrep,
            file: path.to_string(),
            line_start: *line_no,
            line_end: *line_no,
            score: 1.0 / (1.0 + rank as f64 * 0.01),
            excerpt,
            symbol: None,
            language: language.as_deref().map(str::to_owned),
        }));
    }
    hits
}
