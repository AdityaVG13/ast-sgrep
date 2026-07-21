use ast_sgrep_embed::expand_concepts; use crate::store::{CallerRow, SymbolRow}; #[derive(Debug, Clone)] pub struct SemanticChunkInput { pub symbol_name: String, pub kind: String, pub line_start: u32, pub line_end: u32, pub excerpt: String, pub callers: Vec<String>, pub callees: Vec<String>, pub doc: String, pub scope: String, } pub fn build_semantic_chunks(
    symbols: &[SymbolRow], callers: &[CallerRow], lines: &[(u32, String)], ) -> Vec<SemanticChunkInput> {
    symbols
        .iter() .filter(|s| s.kind == "function" || s.kind == "method") .filter_map(|sym| {
            let excerpt = excerpt_for_span(lines, sym.line_start, sym.line_end); if excerpt.trim().is_empty() { return None; } let mut caller_names: Vec<String> = callers
                .iter() .filter(|c| c.callee == sym.name) .map(|c| c.caller.clone()) .collect();
            caller_names.sort(); caller_names.dedup(); let mut callee_names: Vec<String> = callers
                .iter() .filter(|c| c.caller == sym.name) .map(|c| c.callee.clone()) .collect();
            callee_names.sort(); callee_names.dedup(); Some(SemanticChunkInput {
                symbol_name: sym.name.clone(), kind: sym.kind.clone(), line_start: sym.line_start, line_end: sym.line_end, excerpt,
                callers: caller_names, callees: callee_names, doc: doc_comment_above(lines, sym.line_start), scope: enclosing_scope(symbols, sym), })
        }) .collect()
} pub fn render_chunk_text(chunk: &SemanticChunkInput) -> String {
    let mut raw = format!("symbol: {} kind: {}", chunk.symbol_name, chunk.kind); if !chunk.scope.is_empty() { raw.push_str(&format!(" scope: {}", chunk.scope)); } if !chunk.doc.is_empty() {
        raw.push_str(&format!(" doc: {}", chunk.doc));
    } if !chunk.callers.is_empty() { raw.push_str(&format!(" called_by: {}", chunk.callers.join(" "))); } if !chunk.callees.is_empty() {
        raw.push_str(&format!(" calls: {}", chunk.callees.join(" ")));
    } raw.push_str(&format!(" excerpt: {}", chunk.excerpt)); expand_concepts(&raw)
} fn enclosing_scope(symbols: &[SymbolRow], sym: &SymbolRow) -> String {
    symbols
        .iter() .filter(|s| {
            matches!(s.kind.as_str(), "class" | "type" | "interface" | "enum")
                && s.byte_start <= sym.byte_start && s.byte_end >= sym.byte_end && (s.byte_start, s.byte_end) != (sym.byte_start, sym.byte_end)
        }) .min_by_key(|s| s.byte_end - s.byte_start) .map(|s| s.name.clone()) .unwrap_or_default()
} const DOC_LOOKBACK_LINES: usize = 8; fn doc_comment_above(lines: &[(u32, String)], line_start: u32) -> String {
    let mut collected = Vec::new(); let mut expect = line_start.saturating_sub(1); for (no, content) in lines.iter().rev() {
        if *no > expect || expect == 0 { continue; } if *no < expect || collected.len() >= DOC_LOOKBACK_LINES { break; } let Some(text) = strip_comment_marker(content) else { break; }; collected.push(text); expect -= 1;
    } collected.reverse(); collected.join(" ").trim().to_string()
} fn strip_comment_marker(line: &str) -> Option<&str> {
    let trimmed = line.trim(); ["///", "//!", "//", "/**", "/*", "*/", "*", "#", "--"]
        .into_iter() .find_map(|m| trimmed.strip_prefix(m).map(str::trim))
} fn excerpt_for_span(lines: &[(u32, String)], line_start: u32, line_end: u32) -> String {
    lines
        .iter() .filter(|(no, _)| *no >= line_start && *no <= line_end) .map(|(_, c)| c.as_str()) .collect::<Vec<_>>() .join("\n")
}
