//! Universal (`$$NAME`) walk over every tree-sitter node.

use super::*;
use crate::extract::{byte_to_line, node_text};
use std::collections::BTreeMap;
use tree_sitter::Node;

/// `$$NAME` / `$$_` universal matching — visit EVERY node (anonymous
/// tokens included, no comment/string skip: pinned comment, docstring, and
/// string rows) and push each via the shared byte-range dedup.
/// A named metavariable binds the node text through `bind_capture`, so the
/// reserved `MATCH` key keeps its overwrite semantics.
pub(crate) fn walk_universal(
    node: Node,
    source: &str,
    pattern: &str,
    name: Option<&str>,
    out: &mut Vec<PatternMatch>,
) {
    // Hash-set dedup — the previous linear scan over `out` made the
    // universal walk quadratic. The set only gates the push; emission order
    // is unchanged.
    let mut seen = std::collections::HashSet::new();
    // The dedup alone left the walk quadratic — `node_lines` re-counts `\n`
    // from byte 0 twice per node, and the excerpt fallback re-scanned the
    // whole source per long node. Pre-order start bytes are strictly
    // increasing, so a forward (byte,line) cursor amortizes the
    // start-line computation to O(source) across the whole walk; the end
    // line is a local scan over the node's own span. Both produce exactly
    // the values `node_lines` would.
    let mut cursor = (0usize, 1u32);
    walk_universal_inner(node, source, pattern, name, &mut seen, &mut cursor, out);
}

/// Forward line cursor: `line_at` is the 1-based line number of `byte_at`.
/// Only valid while bytes are visited in non-decreasing order (pre-order).
pub(crate) type LineCursor = (usize, u32);

pub(crate) fn walk_universal_inner(
    node: Node,
    source: &str,
    pattern: &str,
    name: Option<&str>,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    line_cursor: &mut LineCursor,
    out: &mut Vec<PatternMatch>,
) {
    let byte_start = node.start_byte();
    let byte_end = node.end_byte();
    // Advance the monotonic cursor to this node's start (pre-order guarantees
    // byte_start >= cursor byte).
    let from = line_cursor.0.min(byte_start);
    let newlines = source.as_bytes()[from..byte_start]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32;
    let line_start = if byte_start >= line_cursor.0 {
        line_cursor.1 + newlines
    } else {
        byte_to_line(source, byte_start)
    };
    line_cursor.0 = byte_start;
    line_cursor.1 = line_start;
    // End line: newlines inside the node's own span (local, O(span)).
    let line_end = line_start
        + source.as_bytes()[byte_start..byte_end.min(source.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32;
    if seen.insert((byte_start, byte_end)) {
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
            if let Some(name) = name {
                bind_capture(&mut captures, name, text);
            }
        }
        out.push(PatternMatch {
            line_start,
            line_end,
            byte_start,
            byte_end,
            excerpt: excerpt_for_node(&node, source, pattern),
            captures,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_universal_inner(child, source, pattern, name, seen, line_cursor, out);
    }
}
