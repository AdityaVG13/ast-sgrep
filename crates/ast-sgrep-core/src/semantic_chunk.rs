use crate::store::{CallerRow, SymbolRow};
use ast_sgrep_embed::expand_concepts;
#[derive(Debug, Clone)]
pub struct SemanticChunkInput {
    pub symbol_name: String,
    pub kind: String,
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
    pub callers: Vec<String>,
    pub callees: Vec<String>,
    pub doc: String,
    pub scope: String,
}
pub fn build_semantic_chunks(
    symbols: &[SymbolRow],
    callers: &[CallerRow],
    lines: &[(u32, String)],
    language: Option<&str>,
) -> Vec<SemanticChunkInput> {
    symbols
        .iter()
        .filter(|s| s.kind == "function" || s.kind == "method")
        .filter_map(|sym| {
            let excerpt = excerpt_for_span(lines, sym.line_start, sym.line_end);
            if excerpt.trim().is_empty() {
                return None;
            }
            let mut caller_names: Vec<String> = callers
                .iter()
                .filter(|c| c.callee == sym.name)
                .map(|c| c.caller.clone())
                .collect();
            caller_names.sort();
            caller_names.dedup();
            let mut callee_names: Vec<String> = callers
                .iter()
                .filter(|c| c.caller == sym.name)
                .map(|c| c.callee.clone())
                .collect();
            callee_names.sort();
            callee_names.dedup();
            Some(SemanticChunkInput {
                symbol_name: sym.name.clone(),
                kind: sym.kind.clone(),
                line_start: sym.line_start,
                line_end: sym.line_end,
                excerpt,
                callers: caller_names,
                callees: callee_names,
                doc: doc_comment_above(lines, sym.line_start, language),
                scope: enclosing_scope(symbols, sym),
            })
        })
        .collect()
}
pub fn render_chunk_text(chunk: &SemanticChunkInput) -> String {
    let mut raw = format!("symbol: {} kind: {}", chunk.symbol_name, chunk.kind);
    if !chunk.scope.is_empty() {
        raw.push_str(&format!(" scope: {}", chunk.scope));
    }
    if !chunk.doc.is_empty() {
        raw.push_str(&format!(" doc: {}", chunk.doc));
    }
    if !chunk.callers.is_empty() {
        raw.push_str(&format!(" called_by: {}", chunk.callers.join(" ")));
    }
    if !chunk.callees.is_empty() {
        raw.push_str(&format!(" calls: {}", chunk.callees.join(" ")));
    }
    raw.push_str(&format!(" excerpt: {}", chunk.excerpt));
    expand_concepts(&raw)
}
fn enclosing_scope(symbols: &[SymbolRow], sym: &SymbolRow) -> String {
    symbols
        .iter()
        .filter(|s| {
            matches!(s.kind.as_str(), "class" | "type" | "interface" | "enum")
                && s.byte_start <= sym.byte_start
                && s.byte_end >= sym.byte_end
                && (s.byte_start, s.byte_end) != (sym.byte_start, sym.byte_end)
        })
        .min_by_key(|s| s.byte_end - s.byte_start)
        .map(|s| s.name.clone())
        .unwrap_or_default()
}
const DOC_LOOKBACK_LINES: usize = 8;

/// Line-comment / doc-comment prefixes recognized for `language`.
/// `#` is only a comment marker for hash-comment languages (python/ruby).
/// Rust attributes (`#[derive]`) and JS/TS private fields (`#foo`) must not
/// be treated as docs (bead ast-sgrep-pwfm).
fn comment_markers_for(language: Option<&str>) -> &'static [&'static str] {
    match language {
        Some("python") | Some("ruby") => &["#"],
        Some("rust") | Some("typescript") | Some("javascript") | Some("java") | Some("go")
        | Some("csharp") => &["///", "//!", "//", "/**", "/*", "*/", "*"],
        // Unknown: C-style only — never bare `#`.
        _ => &["///", "//!", "//", "/**", "/*", "*/", "*", "--"],
    }
}

fn doc_comment_above(lines: &[(u32, String)], line_start: u32, language: Option<&str>) -> String {
    let markers = comment_markers_for(language);
    let mut collected = Vec::new();
    let mut expect = line_start.saturating_sub(1);
    for (no, content) in lines.iter().rev() {
        if *no > expect || expect == 0 {
            continue;
        }
        if *no < expect || collected.len() >= DOC_LOOKBACK_LINES {
            break;
        }
        let Some(text) = strip_comment_marker(content, markers) else {
            break;
        };
        collected.push(text);
        expect -= 1;
    }
    collected.reverse();
    collected.join(" ").trim().to_string()
}
fn strip_comment_marker<'a>(line: &'a str, markers: &[&str]) -> Option<&'a str> {
    let trimmed = line.trim();
    markers
        .iter()
        .find_map(|m| trimmed.strip_prefix(m).map(str::trim))
}
fn excerpt_for_span(lines: &[(u32, String)], line_start: u32, line_end: u32) -> String {
    lines
        .iter()
        .filter(|(no, _)| *no >= line_start && *no <= line_end)
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_derive_attribute_is_not_doc_comment() {
        let symbols = [SymbolRow {
            name: "foo".into(),
            kind: "function".into(),
            line_start: 2,
            line_end: 2,
            byte_start: 20,
            byte_end: 40,
        }];
        let lines = [(1u32, "#[derive(Debug)]".into()), (2, "fn foo() {}".into())];
        let chunks = build_semantic_chunks(&symbols, &[], &lines, Some("rust"));
        assert_eq!(chunks.len(), 1);
        assert!(
            chunks[0].doc.is_empty(),
            "#[derive] must not become doc text; got {:?}",
            chunks[0].doc
        );
        let rendered = render_chunk_text(&chunks[0]);
        assert!(
            !rendered.contains("doc:"),
            "rendered chunk must not inject derive as doc; got {rendered}"
        );
    }

    #[test]
    fn rust_line_doc_comments_still_captured() {
        let symbols = [SymbolRow {
            name: "foo".into(),
            kind: "function".into(),
            line_start: 2,
            line_end: 2,
            byte_start: 20,
            byte_end: 40,
        }];
        let lines = [(1u32, "/// does a thing".into()), (2, "fn foo() {}".into())];
        let chunks = build_semantic_chunks(&symbols, &[], &lines, Some("rust"));
        assert_eq!(chunks[0].doc, "does a thing");
    }

    #[test]
    fn typescript_private_field_hash_is_not_doc_comment() {
        let symbols = [SymbolRow {
            name: "method".into(),
            kind: "method".into(),
            line_start: 2,
            line_end: 2,
            byte_start: 20,
            byte_end: 40,
        }];
        let lines = [(1u32, "  #foo = 1;".into()), (2, "  method() {}".into())];
        let chunks = build_semantic_chunks(&symbols, &[], &lines, Some("typescript"));
        assert_eq!(chunks.len(), 1);
        assert!(
            chunks[0].doc.is_empty(),
            "TS private field #foo must not become doc; got {:?}",
            chunks[0].doc
        );
    }

    #[test]
    fn python_hash_comments_still_captured() {
        let symbols = [SymbolRow {
            name: "foo".into(),
            kind: "function".into(),
            line_start: 2,
            line_end: 2,
            byte_start: 20,
            byte_end: 40,
        }];
        let lines = [(1u32, "# helper".into()), (2, "def foo():".into())];
        let chunks = build_semantic_chunks(&symbols, &[], &lines, Some("python"));
        assert_eq!(chunks[0].doc, "helper");
    }
}
