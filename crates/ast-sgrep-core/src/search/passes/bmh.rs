use crate::search::types::{HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::store::IndexedLineRow;
use std::collections::{BTreeMap, HashMap};
pub(crate) const BMH_LINE_THRESHOLD: usize = 1000;
pub(crate) type FileLinesMap = HashMap<String, BTreeMap<u32, String>>;
/// SQL line projection: path, language, line_no, content.
pub(crate) type LineSqlRow = (String, Option<String>, u32, String);
pub(crate) fn build_file_lines_map(lines: &[IndexedLineRow]) -> FileLinesMap {
    let mut map = FileLinesMap::new();
    for (path, line_no, content, _) in lines {
        map.entry(path.to_string())
            .or_default()
            .insert(*line_no, content.clone());
    }
    map
}
pub(crate) fn needs_context(options: &SearchOptions) -> bool {
    options.context_before > 0 || options.context_after > 0
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
pub(crate) fn build_excerpt_with_context(
    file_map: &FileLinesMap,
    path: &str,
    line_no: u32,
    content: &str,
    before: usize,
    after: usize,
) -> String {
    if before == 0 && after == 0 {
        return content.to_string();
    }
    let Some(file_lines) = file_map.get(path) else {
        return content.to_string();
    };
    let start = line_no.saturating_sub(before as u32);
    let end = line_no + after as u32;
    (start..=end)
        .filter_map(|ln| {
            if ln == line_no {
                Some(content.to_string())
            } else {
                file_lines.get(&ln).cloned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub(crate) fn excerpt_opt(
    file_map: Option<&FileLinesMap>,
    path: &str,
    line_no: u32,
    content: &str,
    options: &SearchOptions,
) -> String {
    match file_map {
        Some(fm) => build_excerpt_with_context(
            fm,
            path,
            line_no,
            content,
            options.context_before,
            options.context_after,
        ),
        None => content.to_string(),
    }
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
