use crate::store::{CallerRow, SymbolRow};
use ast_sgrep_embed::expand_concepts;
use ast_sgrep_lang::PatternNode;
use std::collections::HashSet;

const MAX_CHILD_CHUNKS_PER_PARENT: usize = 32;
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
/// `#` is only a comment marker for hash-comment languages (python/ruby).
/// Rust attributes (`#[derive]`) and JS/TS private fields (`#foo`) must not
/// be treated as docs (bead ast-sgrep-pwfm).
fn comment_markers_for(language: Option<&str>) -> &'static [&'static str] {
    match language {
        Some("python") | Some("ruby") => &["#"],
        Some("rust")
        | Some("typescript")
        | Some("javascript")
        | Some("java")
        | Some("go")
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

    fn function(line_start: u32, line_end: u32) -> SymbolRow {
        SymbolRow {
            name: "renew_account".into(),
            kind: "function".into(),
            line_start,
            line_end,
            byte_start: 0,
            byte_end: 100,
        }
    }

    #[test]
    fn maps_distinct_ast_children_back_to_the_parent_symbol() {
        let symbol = function(2, 8);
        let nodes = vec![
            PatternNode {
                signature: "decl:fn:renew_account".into(),
                line_start: 2,
                line_end: 8,
                excerpt: "whole parent".into(),
            },
            PatternNode {
                signature: "call:charge".into(),
                line_start: 4,
                line_end: 4,
                excerpt: "charge(subscription)".into(),
            },
            PatternNode {
                signature: "identifier".into(),
                line_start: 4,
                line_end: 4,
                excerpt: "charge".into(),
            },
            PatternNode {
                signature: "call:notify".into(),
                line_start: 6,
                line_end: 6,
                excerpt: "notify_customer()".into(),
            },
        ];
        let lines = [(2, "whole parent".into())];
        let chunks = build_semantic_chunks(&[symbol], &[], &nodes, &lines, None);
        assert_eq!(chunks.len(), 3);
        assert!(chunks
            .iter()
            .all(|chunk| (chunk.line_start, chunk.line_end) == (2, 8)));
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.excerpt.as_str())
                .collect::<Vec<_>>(),
            vec!["charge", "charge(subscription)", "notify_customer()"]
        );
    }

    #[test]
    fn assigns_nested_nodes_only_to_the_nearest_parent() {
        let mut outer = function(1, 10);
        outer.name = "outer".into();
        outer.byte_end = 200;
        let mut inner = function(3, 5);
        inner.name = "inner".into();
        inner.byte_start = 40;
        inner.byte_end = 80;
        let lines = (1..=10)
            .map(|line| (line, format!("line {line}")))
            .collect::<Vec<_>>();
        let nodes = [PatternNode {
            signature: "call:inside".into(),
            line_start: 4,
            line_end: 4,
            excerpt: "inside_call()".into(),
        }];
        let chunks = build_semantic_chunks(&[outer, inner], &[], &nodes, &lines, None);
        let owners = chunks
            .iter()
            .filter(|chunk| chunk.excerpt == "inside_call()")
            .map(|chunk| chunk.symbol_name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(owners, vec!["inner"]);
    }

    #[test]
    fn keeps_a_child_from_a_one_line_parent() {
        let lines = [(1, "fn renew_account() { charge() }".to_string())];
        let nodes = [
            PatternNode {
                signature: "decl:fn:renew_account".into(),
                line_start: 1,
                line_end: 1,
                excerpt: lines[0].1.clone(),
            },
            PatternNode {
                signature: "call:charge".into(),
                line_start: 1,
                line_end: 1,
                excerpt: "charge()".into(),
            },
        ];
        let chunks = build_semantic_chunks(&[function(1, 1)], &[], &nodes, &lines, None);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].excerpt, "charge()");
        assert_eq!((chunks[0].line_start, chunks[0].line_end), (1, 1));
    }

    #[test]
    fn maps_top_level_nodes_to_a_file_parent() {
        let lines = [
            (1, "const TIMEOUT: u64 = 30;".into()),
            (2, "type UserId = String;".into()),
        ];
        let nodes = [PatternNode {
            signature: "constant:TIMEOUT".into(),
            line_start: 1,
            line_end: 1,
            excerpt: "const TIMEOUT: u64 = 30;".into(),
        }];
        let chunks = build_semantic_chunks(&[], &[], &nodes, &lines, None);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, "file");
        assert!(chunks[0].symbol_name.is_empty());
        assert_eq!((chunks[0].line_start, chunks[0].line_end), (1, 2));
    }

    #[test]
    fn bounds_children_and_falls_back_to_the_parent_excerpt() {
        let nodes = (2..=50)
            .map(|line| PatternNode {
                signature: format!("identifier:{line}"),
                line_start: line,
                line_end: line,
                excerpt: format!("child_{line}"),
            })
            .collect::<Vec<_>>();
        let chunks = build_semantic_chunks(&[function(1, 60)], &[], &nodes, &[], None);
        assert_eq!(chunks.len(), MAX_CHILD_CHUNKS_PER_PARENT);

        let lines = [(1, "fn renew_account() {}".into())];
        let fallback = build_semantic_chunks(&[function(1, 1)], &[], &[], &lines, None);
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].excerpt, "fn renew_account() {}");
    }

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
        let lines = [
            (1u32, "#[derive(Debug)]".into()),
            (2, "fn foo() {}".into()),
        ];
        let chunks = build_semantic_chunks(&symbols, &[], &[], &lines, Some("rust"));
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
        let lines = [
            (1u32, "/// does a thing".into()),
            (2, "fn foo() {}".into()),
        ];
        let chunks = build_semantic_chunks(&symbols, &[], &[], &lines, Some("rust"));
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
        let lines = [
            (1u32, "  #foo = 1;".into()),
            (2, "  method() {}".into()),
        ];
        let chunks = build_semantic_chunks(&symbols, &[], &[], &lines, Some("typescript"));
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
        let lines = [
            (1u32, "# helper".into()),
            (2, "def foo():".into()),
        ];
        let chunks = build_semantic_chunks(&symbols, &[], &[], &lines, Some("python"));
        assert_eq!(chunks[0].doc, "helper");
    }
}
