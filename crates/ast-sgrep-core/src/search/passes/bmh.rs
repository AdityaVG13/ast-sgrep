use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::IndexStore;
use crate::{Result, StoreError};
use std::time::{Duration, Instant};
pub(crate) const BMH_LINE_THRESHOLD: usize = 1000;
/// SQL line projection: path, language, line_no, content.
pub(crate) type LineSqlRow = (String, Option<String>, u32, String);
pub(crate) fn needs_context(options: &SearchOptions) -> bool {
    options.context_before > 0 || options.context_after > 0
}
pub(crate) fn retained_limit(options: &SearchOptions) -> usize {
    if options.use_rerank {
        options.limit.max(options.rerank_top_k)
    } else {
        options.limit
    }
}
pub(crate) fn map_line_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LineSqlRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}
pub(crate) fn asgrep_line_hit(
    path: String,
    language: Option<String>,
    line_no: u32,
    excerpt: String,
    score: f64,
) -> SearchHit {
    SearchHit::span(SpanHitInput {
        kind: HitKind::Asgrep,
        file: path,
        line_start: line_no,
        line_end: line_no,
        score,
        excerpt,
        symbol: None,
        language,
    })
}
pub(crate) fn attach_context(
    store: &IndexStore,
    options: &SearchOptions,
    hits: &mut [SearchHit],
) -> Result<()> {
    if !needs_context(options) {
        return Ok(());
    }
    for hit in hits {
        let before = u32::try_from(options.context_before).unwrap_or(u32::MAX);
        let after = u32::try_from(options.context_after).unwrap_or(u32::MAX);
        let start = hit.line_start.saturating_sub(before);
        let end = hit.line_end.saturating_add(after);
        let excerpt = store.indexed_excerpt_in_range(&hit.file, start, end)?;
        if !excerpt.is_empty() {
            hit.excerpt = excerpt;
        }
    }
    Ok(())
}
pub(crate) fn attach_regex_context(
    store: &IndexStore,
    options: &SearchOptions,
    hits: &mut [SearchHit],
    deadline: Instant,
    budget: Duration,
) -> Result<()> {
    if !needs_context(options) {
        return Ok(());
    }
    for hit in hits {
        check_regex_deadline(deadline, budget)?;
        let before = u32::try_from(options.context_before).unwrap_or(u32::MAX);
        let after = u32::try_from(options.context_after).unwrap_or(u32::MAX);
        let start = hit.line_start.saturating_sub(before);
        let end = hit.line_end.saturating_add(after);
        let excerpt = store.indexed_excerpt_in_range(&hit.file, start, end)?;
        check_regex_deadline(deadline, budget)?;
        if !excerpt.is_empty() {
            hit.excerpt = excerpt;
        }
    }
    Ok(())
}
fn check_regex_deadline(deadline: Instant, budget: Duration) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(StoreError::Other(format!(
            "regex search exceeded wall-clock budget of {}ms (ASGREP_REGEX_BUDGET_MS); partial results discarded",
            budget.as_millis()
        )));
    }
    Ok(())
}
pub(crate) fn is_word_boundary(s: &str, pos: usize, len: usize) -> bool {
    let before_ok = pos == 0
        || s[..pos]
            .chars()
            .last()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let after_ok = pos + len >= s.len()
        || s[pos + len..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
    before_ok && after_ok
}
