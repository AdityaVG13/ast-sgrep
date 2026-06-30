//! UTF-16 text positions and incremental document edits.

use crate::types::{Position, TextDocumentContentChangeEvent};

/// Convert LSP UTF-16 code unit offset to byte offset within a line.
pub fn utf16_char_to_byte(line: &str, utf16_offset: u32) -> usize {
    let mut utf16 = 0u32;
    for (byte_idx, ch) in line.char_indices() {
        let units = ch.len_utf16() as u32;
        if utf16_offset < utf16 + units {
            return byte_idx;
        }
        utf16 += units;
    }
    line.len()
}

pub fn apply_text_edit(content: &str, change: &TextDocumentContentChangeEvent) -> String {
    let range = match &change.range {
        Some(r) => r,
        None => return change.text.clone(),
    };
    let start = lsp_position_to_byte_offset(content, &range.start);
    let end = if let Some(len) = change.range_length {
        utf16_span_to_byte_end(content, &range.start, len)
    } else {
        lsp_position_to_byte_offset(content, &range.end)
    };
    if start > end || end > content.len() {
        return content.to_string();
    }
    let mut out = String::with_capacity(content.len().saturating_add(change.text.len()));
    out.push_str(&content[..start]);
    out.push_str(&change.text);
    out.push_str(&content[end..]);
    out
}

fn utf16_span_to_byte_end(content: &str, start: &Position, utf16_len: u32) -> usize {
    let start_byte = lsp_position_to_byte_offset(content, start);
    let tail = &content[start_byte..];
    let mut utf16 = 0u32;
    for (byte_idx, ch) in tail.char_indices() {
        utf16 += ch.len_utf16() as u32;
        if utf16 >= utf16_len {
            return start_byte + byte_idx + ch.len_utf8();
        }
    }
    content.len()
}

fn lsp_position_to_byte_offset(content: &str, pos: &Position) -> usize {
    let mut line_no = 0u32;
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let line_body = line.strip_suffix('\n').unwrap_or(line);
        if line_no == pos.line {
            return offset + utf16_char_to_byte(line_body, pos.character);
        }
        offset += line.len();
        line_no += 1;
    }
    content.len()
}

pub fn extract_identifier_at(line: &str, byte_offset: usize) -> Option<String> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    if chars.is_empty() {
        return None;
    }
    let mut idx = 0;
    while idx < chars.len() && chars[idx].0 < byte_offset {
        idx += 1;
    }
    if idx >= chars.len() {
        idx = chars.len().saturating_sub(1);
    }
    if !is_ident_char(chars[idx].1) {
        if idx > 0 {
            idx -= 1;
        }
    }
    if !is_ident_char(chars[idx].1) {
        return None;
    }
    let mut lo = idx;
    let mut hi = idx;
    while lo > 0 && is_ident_char(chars[lo - 1].1) {
        lo -= 1;
    }
    while hi + 1 < chars.len() && is_ident_char(chars[hi + 1].1) {
        hi += 1;
    }
    let start_byte = chars[lo].0;
    let end_byte = if hi + 1 < chars.len() {
        chars[hi + 1].0
    } else {
        line.len()
    };
    let ident = line.get(start_byte..end_byte)?.trim();
    if ident.is_empty() {
        None
    } else {
        Some(ident.to_string())
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Position, Range, TextDocumentContentChangeEvent};

    #[test]
    fn utf16_offset_handles_multibyte() {
        let line = "fn café() {}";
        assert!(utf16_char_to_byte(line, 0) < utf16_char_to_byte(line, 5));
    }

    #[test]
    fn utf16_offset_handles_emoji_interior() {
        let line = "🙂abc";
        let emoji_byte = utf16_char_to_byte(line, 0);
        let interior = utf16_char_to_byte(line, 1);
        assert_eq!(emoji_byte, interior);
        assert_eq!(utf16_char_to_byte(line, 2), "🙂".len());
    }

    #[test]
    fn apply_incremental_text_edit() {
        let content = "fn main() {\n    old();\n}\n";
        let change = TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position { line: 1, character: 4 },
                end: Position { line: 1, character: 7 },
            }),
            range_length: None,
            text: "new".to_string(),
        };
        let edited = apply_text_edit(content, &change);
        assert!(edited.contains("new();"));
        assert!(!edited.contains("old();"));
    }

    #[test]
    fn apply_edit_honors_range_length() {
        let content = "fn main() {\n    old();\n}\n";
        let change = TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position { line: 1, character: 4 },
                end: Position { line: 1, character: 4 },
            }),
            range_length: Some(3),
            text: "new".to_string(),
        };
        let edited = apply_text_edit(content, &change);
        assert!(edited.contains("new();"));
    }

    #[test]
    fn extracts_identifier_at_cursor() {
        let line = "    process_request(\"x\");";
        assert_eq!(
            extract_identifier_at(line, 6),
            Some("process_request".to_string())
        );
    }
}
