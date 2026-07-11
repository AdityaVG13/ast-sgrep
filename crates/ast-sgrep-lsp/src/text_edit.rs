use crate::types::{Position, TextDocumentContentChangeEvent};
pub fn utf16_char_to_byte(line: &str, utf16_offset: u32) -> usize {
    let mut utf16 = 0u32;
    for (byte_idx, ch) in line.char_indices() {
        let units = ch.len_utf16() as u32;
        if utf16_offset < utf16 + units { return byte_idx; }
        utf16 += units;
    }
    line.len()
}
pub fn apply_text_edit(content: &str, change: &TextDocumentContentChangeEvent) -> String {
    let Some(range) = &change.range else { return change.text.clone(); };
    let start = lsp_position_to_byte_offset(content, &range.start);
    let end = change.range_length.map_or_else(
        || lsp_position_to_byte_offset(content, &range.end),
        |len| utf16_span_to_byte_end(content, &range.start, len),
    );
    if start > end || end > content.len() { return content.to_string(); }
    let mut out = String::with_capacity(content.len().saturating_add(change.text.len()));
    out.push_str(&content[..start]);
    out.push_str(&change.text);
    out.push_str(&content[end..]);
    out
}
fn utf16_span_to_byte_end(content: &str, start: &Position, utf16_len: u32) -> usize {
    let start_byte = lsp_position_to_byte_offset(content, start);
    let mut utf16 = 0u32;
    for (byte_idx, ch) in content[start_byte..].char_indices() {
        utf16 += ch.len_utf16() as u32;
        if utf16 >= utf16_len { return start_byte + byte_idx + ch.len_utf8(); }
    }
    content.len()
}
fn lsp_position_to_byte_offset(content: &str, pos: &Position) -> usize {
    let mut offset = 0usize;
    for (line_no, line) in content.split_inclusive('\n').enumerate() {
        if line_no as u32 == pos.line {
            let body = line.strip_suffix('\n').unwrap_or(line);
            return offset + utf16_char_to_byte(body, pos.character);
        }
        offset += line.len();
    }
    content.len()
}
pub fn extract_identifier_at(line: &str, byte_offset: usize) -> Option<String> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let idx = identifier_char_index(&chars, byte_offset)?;
    let (start_byte, end_byte) = identifier_byte_span(line, &chars, idx);
    let ident = line.get(start_byte..end_byte)?.trim();
    (!ident.is_empty()).then(|| ident.to_string())
}
fn identifier_char_index(chars: &[(usize, char)], byte_offset: usize) -> Option<usize> {
    let mut idx = chars
        .iter()
        .position(|(offset, _)| *offset >= byte_offset)
        .unwrap_or_else(|| chars.len().saturating_sub(1));
    if !is_ident_char(chars[idx].1) && idx > 0 {
        idx -= 1;
    }
    is_ident_char(chars[idx].1).then_some(idx)
}
fn identifier_byte_span(line: &str, chars: &[(usize, char)], idx: usize) -> (usize, usize) {
    let mut lo = idx;
    let mut hi = idx;
    while lo > 0 && is_ident_char(chars[lo - 1].1) {
        lo -= 1;
    }
    while hi + 1 < chars.len() && is_ident_char(chars[hi + 1].1) {
        hi += 1;
    }
    (
        chars[lo].0,
        chars.get(hi + 1).map(|(o, _)| *o).unwrap_or(line.len()),
    )
}
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
