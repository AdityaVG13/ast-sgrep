use crate::store::{CallerRow, SymbolRow};
use ast_sgrep_embed::expand_concepts;
use ast_sgrep_lang::PatternNode;
use std::collections::HashSet;

const MAX_CHILD_CHUNKS_PER_PARENT: usize = 2;
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
    build_semantic_chunks_with_patterns(symbols, callers, &[], lines, language)
}

pub fn build_semantic_chunks_with_patterns(
    symbols: &[SymbolRow],
    callers: &[CallerRow],
    pattern_nodes: &[PatternNode],
    lines: &[(u32, String)],
    language: Option<&str>,
) -> Vec<SemanticChunkInput> {
    let mut chunks = Vec::new();
    let parents = symbols
        .iter()
        .filter(|symbol| symbol.kind == "function" || symbol.kind == "method")
        .collect::<Vec<_>>();
    for (parent_index, sym) in parents.iter().enumerate() {
        let parent_excerpt = excerpt_for_span(lines, sym.line_start, sym.line_end);
        let mut excerpts = select_child_excerpts(
            pattern_nodes
                .iter()
                .filter(|node| containing_parent_index(node, &parents) == Some(parent_index)),
            (sym.line_start, sym.line_end),
            &parent_excerpt,
        );
        if excerpts.is_empty() {
            if parent_excerpt.trim().is_empty() {
                continue;
            }
            excerpts.push(parent_excerpt);
        }

        let mut caller_names = callers
            .iter()
            .filter(|caller| caller.callee == sym.name)
            .map(|caller| caller.caller.clone())
            .collect::<Vec<_>>();
        caller_names.sort();
        caller_names.dedup();
        let mut callee_names = callers
            .iter()
            .filter(|caller| caller.caller == sym.name)
            .map(|caller| caller.callee.clone())
            .collect::<Vec<_>>();
        callee_names.sort();
        callee_names.dedup();
        let doc = doc_comment_above(lines, sym.line_start, language);
        let scope = enclosing_scope(symbols, sym);
        for excerpt in excerpts {
            chunks.push(SemanticChunkInput {
                symbol_name: sym.name.clone(),
                kind: sym.kind.clone(),
                line_start: sym.line_start,
                line_end: sym.line_end,
                excerpt,
                callers: caller_names.clone(),
                callees: callee_names.clone(),
                doc: doc.clone(),
                scope: scope.clone(),
            });
        }
    }

    let file_start = lines
        .first()
        .map(|(line, _)| *line)
        .or_else(|| pattern_nodes.iter().map(|node| node.line_start).min());
    let file_end = lines
        .last()
        .map(|(line, _)| *line)
        .or_else(|| pattern_nodes.iter().map(|node| node.line_end).max());
    if let (Some(file_start), Some(file_end)) = (file_start, file_end) {
        let excerpts = select_child_excerpts(
            pattern_nodes
                .iter()
                .filter(|node| containing_parent_index(node, &parents).is_none()),
            (file_start, file_end),
            "",
        );
        for excerpt in excerpts {
            chunks.push(SemanticChunkInput {
                symbol_name: String::new(),
                kind: "file".into(),
                line_start: file_start,
                line_end: file_end,
                excerpt,
                callers: Vec::new(),
                callees: Vec::new(),
                doc: String::new(),
                scope: String::new(),
            });
        }
    }
    chunks
}

fn containing_parent_index(node: &PatternNode, parents: &[&SymbolRow]) -> Option<usize> {
    parents
        .iter()
        .enumerate()
        .filter(|(_, parent)| {
            node.line_start >= parent.line_start && node.line_end <= parent.line_end
        })
        .min_by_key(|(index, parent)| {
            (
                parent.line_end.saturating_sub(parent.line_start),
                parent.byte_end.saturating_sub(parent.byte_start),
                *index,
            )
        })
        .map(|(index, _)| index)
}

fn select_child_excerpts<'a>(
    nodes: impl Iterator<Item = &'a PatternNode>,
    parent_span: (u32, u32),
    parent_excerpt: &str,
) -> Vec<String> {
    let mut candidates = nodes
        .filter(|node| {
            !node.excerpt.trim().is_empty()
                && ((node.line_start, node.line_end) != parent_span
                    || node.excerpt.trim() != parent_excerpt.trim())
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        child_priority(&left.signature)
            .cmp(&child_priority(&right.signature))
            .then_with(|| {
                left.line_end
                    .saturating_sub(left.line_start)
                    .cmp(&right.line_end.saturating_sub(right.line_start))
            })
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.line_end.cmp(&right.line_end))
            .then_with(|| left.excerpt.cmp(&right.excerpt))
    });
    let mut seen = HashSet::new();
    candidates.retain(|node| seen.insert(node.excerpt.trim().to_owned()));
    candidates.truncate(MAX_CHILD_CHUNKS_PER_PARENT);
    candidates.sort_by(|left, right| {
        left.line_start
            .cmp(&right.line_start)
            .then_with(|| left.line_end.cmp(&right.line_end))
            .then_with(|| left.excerpt.cmp(&right.excerpt))
    });
    candidates
        .into_iter()
        .map(|node| node.excerpt.trim().to_owned())
        .collect()
}

fn child_priority(signature: &str) -> u8 {
    if signature.starts_with("call:") || signature.starts_with("decl:") {
        0
    } else if signature.starts_with("kind:") {
        1
    } else {
        2
    }
}

/// Per-field embed texts stored beside the concatenated chunk vector (7d5x.2.2).
/// Empty strings are not embedded (NULL in SQLite). Query weighting is 7d5x.3.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChunkFieldTexts {
    pub name: String,
    pub docs: String,
    pub body: String,
    pub graph: String,
}

pub fn chunk_field_texts(chunk: &SemanticChunkInput) -> ChunkFieldTexts {
    let name = if chunk.symbol_name.is_empty() && chunk.kind.is_empty() && chunk.scope.is_empty() {
        String::new()
    } else {
        let mut raw = format!("symbol: {} kind: {}", chunk.symbol_name, chunk.kind);
        if !chunk.scope.is_empty() {
            raw.push_str(&format!(" scope: {}", chunk.scope));
        }
        expand_concepts(&raw)
    };
    let docs = if chunk.doc.is_empty() {
        String::new()
    } else {
        expand_concepts(&format!("doc: {}", chunk.doc))
    };
    let body = if chunk.excerpt.trim().is_empty() {
        String::new()
    } else {
        expand_concepts(&format!("excerpt: {}", chunk.excerpt))
    };
    let mut graph = String::new();
    if !chunk.callers.is_empty() {
        graph.push_str(&format!("called_by: {}", chunk.callers.join(" ")));
    }
    if !chunk.callees.is_empty() {
        if !graph.is_empty() {
            graph.push(' ');
        }
        graph.push_str(&format!("calls: {}", chunk.callees.join(" ")));
    }
    let graph = if graph.is_empty() {
        String::new()
    } else {
        expand_concepts(&graph)
    };
    ChunkFieldTexts {
        name,
        docs,
        body,
        graph,
    }
}

/// Persisted per-field embedding blobs (NULL when the field text was empty).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticFieldVectors {
    pub name: Option<Vec<u8>>,
    pub docs: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
    pub graph: Option<Vec<u8>>,
}

pub fn render_chunk_text(chunk: &SemanticChunkInput) -> String {
    // Body first (7d5x.1): metadata used to precede the excerpt, so a long
    // graph/doc prefix was what survived when embedders truncated.
    let mut raw = format!("excerpt: {}", chunk.excerpt);
    raw.push_str(&format!(
        " symbol: {} kind: {}",
        chunk.symbol_name, chunk.kind
    ));
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
/// `#` is only a comment marker for hash-comment languages (python/ruby).
/// Rust attributes (`#[derive]`) and JS/TS private fields (`#foo`) must not
/// be treated as docs (bead ast-sgrep-pwfm).
fn comment_markers_for(language: Option<&str>) -> &'static [&'static str] {
    match language {
        Some("python") | Some("ruby") => &["#"],
        Some("php") => &["#", "//", "/**", "/*", "*/", "*"],
        Some("rust") | Some("typescript") | Some("javascript") | Some("java") | Some("go")
        | Some("csharp") | Some("c") | Some("cpp") | Some("kotlin") | Some("swift") => {
            &["///", "//!", "//", "/**", "/*", "*/", "*"]
        }
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
#[path = "../../../tests/unit/core/semantic_chunk.rs"]
mod tests;
