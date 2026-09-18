//! Kind/identifier collectors and await-operand retention.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Drop every py bare-`await` hit whose span sits on the `await` KEYWORD
/// TOKEN of an operand-bearing `await` node — the literal-lane row overlaps
/// exactly that token, never the whole node. Whole-node containment would
/// also drop a genuinely answerable row: a bare `await` token that
/// error-recovers to an identifier INSIDE an operand-bearing await's
/// operand is span-disjoint from the outer keyword token and survives. The
/// bare pattern matches only the identifier-recovered bare keyword subtree,
/// so operand rows never answer; hits outside collected tokens keep the
/// literal-lane bytes.
pub(crate) fn retain_py_await_operand_free(source: &str, hits: &mut Vec<PatternMatch>) {
    if hits.is_empty() {
        return;
    }
    let Ok(tree) = parse_source(Language::Python, source) else {
        return;
    };
    let mut operand_spans: Vec<(usize, usize)> = Vec::new();
    collect_py_operand_await_spans(tree.root_node(), &mut operand_spans);
    if operand_spans.is_empty() {
        return;
    }
    hits.retain(|m| {
        !operand_spans
            .iter()
            .any(|&(start, end)| m.byte_start >= start && m.byte_end <= end)
    });
}

/// The KEYWORD-TOKEN span of each operand-bearing `await` node — the byte
/// range of the anonymous `await` token (the node's first non-named child),
/// not the whole-node span.
pub(crate) fn collect_py_operand_await_spans(node: Node, spans: &mut Vec<(usize, usize)>) {
    if node.kind() == "await" && node.named_child_count() > 0 {
        let mut cursor = node.walk();
        let token = node
            .children(&mut cursor)
            .find(|child| !child.is_named())
            .map(|child| (child.start_byte(), child.end_byte()));
        if let Some(span) = token {
            spans.push(span);
        } else {
            // No anonymous child (defensive): fall back to the node start
            // — still never swallows a sibling-disjoint identifier row.
            spans.push((node.start_byte(), node.start_byte()));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_py_operand_await_spans(child, spans);
    }
}

/// [`collect_kind_matches`] with an optional operand-less candidate filter:
/// when `require_operand_less`, only kind nodes with no non-trivia named
/// children answer — the `;`-ful `return;` face's empty-operand discipline.
pub(crate) fn collect_kind_matches_filtered(
    lang: Language,
    source: &str,
    pattern: &str,
    kinds: &[&str],
    require_operand_less: bool,
) -> Vec<PatternMatch> {
    if !require_operand_less {
        return collect_kind_matches(lang, source, pattern, kinds);
    }
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk_kind_list_operand_less(
        tree.root_node(),
        source,
        pattern,
        kinds,
        &mut seen,
        &mut out,
    );
    out
}

pub(crate) fn walk_kind_list_operand_less(
    node: Node,
    source: &str,
    pattern: &str,
    kinds: &[&str],
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if kinds.contains(&node.kind()) && !is_in_comment_or_string(&node) {
        let mut cursor = node.walk();
        let operand_less = node
            .children(&mut cursor)
            .all(|child| child.kind().contains("comment") || !child.is_named());
        if operand_less {
            walk_kind_list(node, source, pattern, kinds, seen, out);
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kind_list_operand_less(child, source, pattern, kinds, seen, out);
    }
}

/// The reference's bare ruby pattern matches at IDENTIFIER level — every
/// identifier node whose text equals the pattern (the method slot of every
/// raise call, the bare statement included), outside comments and strings.
/// One match per node, byte-range deduped.
pub(crate) fn collect_identifier_matches(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Vec<PatternMatch> {
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk_identifier_matches(tree.root_node(), source, pattern, &mut seen, &mut out);
    out
}

pub(crate) fn walk_identifier_matches(
    node: Node,
    source: &str,
    pattern: &str,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if is_in_comment_or_string(&node) {
        return;
    }
    if identifier_matches(&node, source, pattern) {
        let (byte_start, byte_end) = (node.start_byte(), node.end_byte());
        if seen.insert((byte_start, byte_end)) {
            let mut captures = BTreeMap::new();
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            out.push(hit_for_node(&node, source, pattern, captures));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_identifier_matches(child, source, pattern, seen, out);
    }
}

/// Walk every node of `kinds` (childless or not — the kind IS the
/// constraint) and push one match per node, byte-range deduped.
pub(crate) fn collect_kind_matches(
    lang: Language,
    source: &str,
    pattern: &str,
    kinds: &[&str],
) -> Vec<PatternMatch> {
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk_kind_list(
        tree.root_node(),
        source,
        pattern,
        kinds,
        &mut seen,
        &mut out,
    );
    out
}

pub(crate) fn walk_kind_list(
    node: Node,
    source: &str,
    pattern: &str,
    kinds: &[&str],
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if kinds.contains(&node.kind()) && !is_in_comment_or_string(&node) {
        let (byte_start, byte_end) = (node.start_byte(), node.end_byte());
        if seen.insert((byte_start, byte_end)) {
            let mut captures = BTreeMap::new();
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            out.push(hit_for_node(&node, source, pattern, captures));
        }
        // The reference reports ONE hit per statement of the family:
        // grammars that wrap the statement kind inside another kind of
        // the same set (python `yield_statement` wrapping `yield`) must
        // not answer twice for one site — the outermost node is the
        // statement.
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kind_list(child, source, pattern, kinds, seen, out);
    }
}
