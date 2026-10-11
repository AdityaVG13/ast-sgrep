use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::IndexStore;
use crate::{Result, StoreError};
use std::borrow::Cow;
use std::time::{Duration, Instant};
pub(crate) const BMH_LINE_THRESHOLD: usize = 1000;
/// SQL line projection: path, language, line_no, content.
pub(crate) type LineSqlRow<T = String> = (T, Option<T>, u32, T);
pub(crate) fn needs_context(options: &SearchOptions) -> bool {
    options.context_before > 0 || options.context_after > 0
}
pub(crate) fn map_line_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LineSqlRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}
/// Decode the same projection/order without owning rejected rows' text.
pub(crate) fn map_borrowed_line_row<'row>(
    row: &'row rusqlite::Row<'_>,
) -> rusqlite::Result<LineSqlRow<Cow<'row, str>>> {
    let path = borrowed_row_text(row, 0)?;
    let language = match row.get_ref(1)? {
        rusqlite::types::ValueRef::Null => None,
        _ => Some(borrowed_row_text(row, 1)?),
    };
    let line_no = row.get(2)?;
    let content = borrowed_row_text(row, 3)?;
    Ok((path, language, line_no, content))
}
fn borrowed_row_text<'row>(
    row: &'row rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<Cow<'row, str>> {
    match row.get_ref(column)?.as_str() {
        Ok(text) => Ok(Cow::Borrowed(text)),
        // Retain Row::get's column/type/UTF-8 error mapping, not as_str's
        // bare FromSqlError. This also preserves any owned conversion fallback.
        Err(_) => row.get::<_, String>(column).map(Cow::Owned),
    }
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
        byte_span: None,
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
pub(super) fn check_regex_deadline(deadline: Instant, budget: Duration) -> Result<()> {
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
