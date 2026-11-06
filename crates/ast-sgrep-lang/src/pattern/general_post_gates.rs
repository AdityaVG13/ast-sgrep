//! Per-language post-gates for the general structural lane (isomorphic split of general_match.rs).

use super::*;
use crate::extract::node_text;
use tree_sitter::Node;

/// True when the bytes between the leading `for`/`go`/`defer` keyword token
/// and the first non-trivia child are comment-free ASCII whitespace — the
/// law at these junctions, scoped to the three covered keywords.
/// The gated statement node for a general-lane go hit: the hit node itself
/// or a same-span descendant of the three gated kinds.
pub(crate) fn go_gate_target<'tree>(node: &Node<'tree>) -> Option<Node<'tree>> {
    if matches!(
        node.kind(),
        "for_statement" | "go_statement" | "defer_statement"
    ) {
        return Some(*node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() == node.start_byte() && child.end_byte() == node.end_byte() {
            if let Some(found) = go_gate_target(&child) {
                return Some(found);
            }
        }
    }
    None
}

pub(crate) fn go_keyword_body_junction_clean(node: &Node, source: &str) -> bool {
    // The reference refuses the head comment on the CLAUSE forms
    // (range/3-clause — first named child is a `for_clause`) while the
    // while-form's head-leading comment stays transparent
    // (`for /* h */ n > 0 {` binds, A=`n > 0`).
    if node.kind() == "for_statement" {
        let mut cursor = node.walk();
        let first_named = node
            .children(&mut cursor)
            .find(|c| c.is_named() && !is_trivia_kind(c.kind()) && !c.is_extra());
        if !matches!(
            first_named.map(|c| c.kind()),
            Some("for_clause") | Some("range_clause")
        ) {
            return true;
        }
    }
    let mut cursor = node.walk();
    let mut children = node.children(&mut cursor);
    let Some(kw) = children.next() else {
        return true;
    };
    if kw.is_named() || !matches!(kw.kind(), "for" | "go" | "defer") {
        return true;
    }
    let Some(first) = children.find(|child| !is_trivia_kind(child.kind()) && !child.is_extra())
    else {
        return true;
    };
    let gap = &source[kw.end_byte()..first.start_byte()];
    !gap.contains("//") && !gap.contains("/*") && gap.chars().all(|c| c.is_ascii_whitespace())
}

/// The using-var head gate — the head spans from the statement start to the
/// declarator NAME's first byte. The reference refuses comments and
/// outsider-junk ERROR runs there while binding everything after the name
/// and every junk position INSIDE the initializer (junk retained in the
/// capture).
pub(crate) fn cs_using_var_head_clean(node: &Node, source: &str) -> bool {
    // The declarator name is the first `identifier` named descendant
    // (`using var s` -> variable_declarator -> identifier; plain `int x`
    // decls share the shape).
    fn first_identifier<'tree>(node: &Node<'tree>) -> Option<Node<'tree>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() && !is_trivia_kind(child.kind()) && child.kind() == "identifier" {
                return Some(child);
            }
            if let Some(found) = first_identifier(&child) {
                return Some(found);
            }
        }
        None
    }
    let Some(name) = first_identifier(node) else {
        return true;
    };
    let head_end = name.start_byte();
    // The reference refuses comments and outsider-junk ERROR runs ANYWHERE
    // in the keyword->name head — the comment extras attach BELOW the
    // statement node, so the scan is recursive. After the name everything
    // binds, junk retained.
    fn head_scan(node: &Node, source: &str, head_end: usize) -> bool {
        if node.start_byte() >= head_end {
            return true; // pre-order: nothing later can overlap the head
        }
        if node.end_byte() > node.start_byte() && node.start_byte() < head_end {
            if node.is_extra() || node.kind() == "comment" {
                return false;
            }
            if node.kind() == "ERROR" && !cs_subtree_outsider_junk_free(*node, source) {
                return false;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !head_scan(&child, source, head_end) {
                return false;
            }
        }
        true
    }
    head_scan(node, source, head_end)
}

/// True when a trivia child sits directly inside a member link before the
/// `?.` connector — the anonymous token or the named `optional_chain`
/// wrapper — the ts-only refusal position (the junction doctrine's
/// general-lane twin).
pub(crate) fn ts_member_link_receiver_trivia(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (index, child) in children.iter().enumerate() {
        if is_trivia_kind(child.kind()) || child.is_extra() {
            let next = children[index + 1..]
                .iter()
                .find(|next| !is_trivia_kind(next.kind()) && !next.is_extra());
            if next.is_some_and(|next| {
                (next.is_named() && next.kind().contains("optional"))
                    || (!next.is_named() && next.kind() == "?.")
            }) {
                return true;
            }
        }
    }
    false
}

/// True when no ERROR node in the subtree carries a candidate gap-junk char
/// outside the pattern-side cs trivia class (see the [`is_sg_cs_trivia`] /
/// [`is_sg_cs_gap_junk`] seam distinction; the OUTSIDER set is the junk that
/// is neither reference-class trivia nor plain ASCII whitespace —
/// U+2028/U+2029/U+0085/U+1680/…, the spellings the cs parse refuses at token
/// gaps).
pub(crate) fn cs_subtree_outsider_junk_free(node: Node, source: &str) -> bool {
    if node.is_error() {
        if let Some(text) = node_text(&node, source) {
            if text
                .chars()
                .any(|c| is_sg_cs_gap_junk(c) && !is_sg_cs_trivia(c) && !c.is_ascii_whitespace())
            {
                return false;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !cs_subtree_outsider_junk_free(child, source) {
            return false;
        }
    }
    true
}
