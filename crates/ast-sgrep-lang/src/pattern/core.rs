//! Hit types and the shared node-walk helper.

use super::*;
use crate::extract::{is_in_comment_or_string, node_lines};
use std::collections::BTreeMap;
use tree_sitter::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
    pub excerpt: String,
    /// Metavariable bindings without the leading `$`. `MATCH` is always the
    /// complete matched node and is available to rewrite templates.
    pub captures: BTreeMap<String, String>,
}

/// Build the [`PatternMatch`] for a matched node: line span and excerpt derive
/// from the node, byte span is the node span, captures pass through untouched
/// (callers insert `MATCH` and metavariable bindings beforehand).
pub(crate) fn hit_for_node(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: BTreeMap<String, String>,
) -> PatternMatch {
    let (line_start, line_end) = node_lines(node, source);
    PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt: excerpt_for_node(node, source, pattern),
        captures,
    }
}

/// Pre-order walk pushing one hit per `kind` node accepted by `check`.
/// `named_only` preserves each lane's original gate; `check` takes `&node`
/// and lanes bind source, pattern, and template through the closure
/// capture. Traversal order matches the per-lane walkers this replaces:
/// test the node, then recurse over children in order.
pub(crate) fn walk_kind<F>(
    node: Node,
    kind: &str,
    named_only: bool,
    out: &mut Vec<PatternMatch>,
    check: F,
) where
    F: Fn(&Node) -> Option<PatternMatch> + Copy,
{
    if node.kind() == kind && (!named_only || node.is_named()) && !is_in_comment_or_string(&node) {
        if let Some(hit) = check(&node) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kind(child, kind, named_only, out, check);
    }
}

/// Declaration / type keyword prefixes used by native classification and prefilters.
///
/// `true` means class-like (`Class`); `false` means function-like (`Function`).
pub const DECL_PATTERN_PREFIXES: &[(&str, bool)] = &[
    ("fn ", false),
    ("def ", false),
    ("function ", false),
    ("func ", false),
    ("class ", true),
    ("struct ", true),
    ("interface ", true),
    ("type ", true),
];
