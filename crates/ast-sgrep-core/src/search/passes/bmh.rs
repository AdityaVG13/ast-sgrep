use std::collections::{BTreeMap, HashMap}; use crate::store::IndexedLineRow; pub(crate) const BMH_LINE_THRESHOLD: usize = 1000; pub(crate) type FileLinesMap = HashMap<String, BTreeMap<u32, String>>;
pub(crate) fn build_file_lines_map(lines: &[IndexedLineRow]) -> FileLinesMap {
    let mut map = FileLinesMap::new(); for (path, line_no, content, _) in lines {
        map.entry(path.to_string())
            .or_default() .insert(*line_no, content.clone());
    } map
} pub(crate) fn build_excerpt_with_context( file_map: &FileLinesMap, path: &str, line_no: u32, content: &str, before: usize, after: usize,
) -> String {
    if before == 0 && after == 0 { return content.to_string(); } let Some(file_lines) = file_map.get(path) else { return content.to_string(); };
    let start = line_no.saturating_sub(before as u32); let end = line_no + after as u32; (start..=end)
        .filter_map(|ln| {
            if ln == line_no { Some(content.to_string()) } else {
                file_lines.get(&ln).cloned()
            }
        }) .collect::<Vec<_>>() .join("\n")
} pub(crate) fn is_word_boundary(s: &str, pos: usize, len: usize) -> bool {
    let before_ok = pos == 0
        || s[..pos]
            .chars() .last() .is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let after_ok = pos + len >= s.len()
        || s[pos + len..]
            .chars() .next() .is_none_or(|c| !c.is_alphanumeric() && c != '_');
    before_ok && after_ok
}
