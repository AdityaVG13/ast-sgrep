//! Build enriched symbol chunks for semantic indexing.

use ast_sgrep_embed::expand_concepts;

use crate::store::{CallerRow, SymbolRow};

/// Input for one symbol-level semantic chunk.
#[derive(Debug, Clone)]
pub struct SemanticChunkInput {
    pub symbol_name: String,
    pub kind: String,
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
    pub callers: Vec<String>,
    pub callees: Vec<String>,
}

/// Build semantic chunk inputs from extracted symbols, callers, and file lines.
pub fn build_semantic_chunks(
    symbols: &[SymbolRow],
    callers: &[CallerRow],
    lines: &[(u32, String)],
) -> Vec<SemanticChunkInput> {
    let mut out = Vec::new();
    for sym in symbols {
        let excerpt = excerpt_for_span(lines, sym.line_start, sym.line_end);
        if excerpt.trim().is_empty() {
            continue;
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

        out.push(SemanticChunkInput {
            symbol_name: sym.name.clone(),
            kind: sym.kind.clone(),
            line_start: sym.line_start,
            line_end: sym.line_end,
            excerpt,
            callers: caller_names,
            callees: callee_names,
        });
    }
    out
}

/// Render chunk text with structural context and concept expansion.
pub fn render_chunk_text(chunk: &SemanticChunkInput) -> String {
    let callers = if chunk.callers.is_empty() {
        String::new()
    } else {
        format!(" called_by: {}", chunk.callers.join(" "))
    };
    let callees = if chunk.callees.is_empty() {
        String::new()
    } else {
        format!(" calls: {}", chunk.callees.join(" "))
    };
    let raw = format!(
        "symbol: {} kind: {}{}{} excerpt: {}",
        chunk.symbol_name, chunk.kind, callers, callees, chunk.excerpt
    );
    expand_concepts(&raw)
}

fn excerpt_for_span(lines: &[(u32, String)], line_start: u32, line_end: u32) -> String {
    lines
        .iter()
        .filter(|(no, _)| *no >= line_start && *no <= line_end)
        .map(|(_, content)| content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{CallerRow, SymbolRow};

    #[test]
    fn chunk_includes_call_graph_context() {
        let symbols = vec![SymbolRow {
            name: "auth_refresh".into(),
            kind: "function".into(),
            line_start: 19,
            line_end: 22,
            byte_start: 0,
            byte_end: 0,
        }];
        let callers = vec![
            CallerRow {
                caller: "main".into(),
                callee: "auth_refresh".into(),
                line_no: 5,
                byte_start: 0,
                byte_end: 0,
            },
            CallerRow {
                caller: "auth_refresh".into(),
                callee: "fetch_token".into(),
                line_no: 20,
                byte_start: 0,
                byte_end: 0,
            },
        ];
        let lines = vec![
            (19, "fn auth_refresh() {".into()),
            (20, "    fetch_token();".into()),
            (21, "}".into()),
        ];
        let chunks = build_semantic_chunks(&symbols, &callers, &lines);
        assert_eq!(chunks.len(), 1);
        let text = render_chunk_text(&chunks[0]);
        assert!(text.contains("auth_refresh"));
        assert!(text.contains("fetch_token"));
        assert!(text.contains("main"));
    }
}
