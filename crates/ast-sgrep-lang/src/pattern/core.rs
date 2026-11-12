//! Hit types and the shared node-walk helper.

use super::*;
use crate::extract::{is_in_comment_or_string, node_lines};
use std::collections::{BTreeMap, HashSet};
use tree_sitter::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
    pub excerpt: String,
    /// Single metavariables use bare names; rest captures use `$$$NAME`.
    /// `MATCH` holds the complete node unless an explicit `$MATCH` binds it.
    /// These public bindings are available to rewrite templates.
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

/// The ONE shared pre-order traversal behind every `walk_*` lane walker:
/// test the node with `gate`, build the hit with `check`, then recurse over
/// children in order. `walk_kind` (single-kind lanes) and the multi-kind /
/// gateless lanes are all thin instantiations — no lane keeps its own
/// recurse boilerplate. Traversal order matches the per-lane walkers this
/// replaces: test the node, then recurse over children in order.
pub(crate) fn walk_nodes<G, C>(node: Node, out: &mut Vec<PatternMatch>, gate: G, check: C)
where
    G: Fn(&Node) -> bool + Copy,
    C: Fn(&Node) -> Option<PatternMatch> + Copy,
{
    if gate(&node) {
        if let Some(hit) = check(&node) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_nodes(child, out, gate, check);
    }
}

/// Multi-hit sibling of [`walk_nodes`]: `check` may emit a batch per node
/// (statement lanes whose match builds several rows).
pub(crate) fn walk_nodes_multi<G, C>(node: Node, out: &mut Vec<PatternMatch>, gate: G, check: C)
where
    G: Fn(&Node) -> bool + Copy,
    C: Fn(&Node) -> Option<Vec<PatternMatch>> + Copy,
{
    if gate(&node) {
        if let Some(hits) = check(&node) {
            out.extend(hits);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_nodes_multi(child, out, gate, check);
    }
}

/// Single-kind instantiation of [`walk_nodes`].
/// `named_only` preserves each lane's original gate; `check` takes `&node`
/// and lanes bind source, pattern, and template through the closure
/// capture.
pub(crate) fn walk_kind<F>(
    node: Node,
    kind: &str,
    named_only: bool,
    out: &mut Vec<PatternMatch>,
    check: F,
) where
    F: Fn(&Node) -> Option<PatternMatch> + Copy,
{
    walk_nodes(
        node,
        out,
        |n: &Node| {
            n.kind() == kind && (!named_only || n.is_named()) && !is_in_comment_or_string(n)
        },
        check,
    );
}

/// Single-kind instantiation of [`walk_nodes_multi`].
pub(crate) fn walk_kind_multi<F>(
    node: Node,
    kind: &str,
    named_only: bool,
    out: &mut Vec<PatternMatch>,
    check: F,
) where
    F: Fn(&Node) -> Option<Vec<PatternMatch>> + Copy,
{
    walk_nodes_multi(
        node,
        out,
        |n: &Node| {
            n.kind() == kind && (!named_only || n.is_named()) && !is_in_comment_or_string(n)
        },
        check,
    );
}

/// Dedup-owning instantiation of the shared walk: the byte-range set lives
/// inside the walk (one set per top-level call, same lifetime as the
/// hand-rolled sets this replaces) for lanes whose template can align one
/// node through distinct binds. Insertion order is preserved.
pub(crate) fn walk_kind_seen<F>(
    node: Node,
    kind: &str,
    named_only: bool,
    out: &mut Vec<PatternMatch>,
    check: F,
) where
    F: Fn(&Node) -> Option<PatternMatch>,
{
    fn rec<F>(
        node: Node,
        kind: &str,
        named_only: bool,
        out: &mut Vec<PatternMatch>,
        seen: &mut HashSet<(usize, usize)>,
        check: &F,
    ) where
        F: Fn(&Node) -> Option<PatternMatch>,
    {
        if node.kind() == kind && (!named_only || node.is_named()) && !is_in_comment_or_string(&node)
        {
            if let Some(hit) = check(&node) {
                if seen.insert((node.start_byte(), node.end_byte())) {
                    out.push(hit);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            rec(child, kind, named_only, out, seen, check);
        }
    }
    rec(node, kind, named_only, out, &mut HashSet::new(), &check);
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
