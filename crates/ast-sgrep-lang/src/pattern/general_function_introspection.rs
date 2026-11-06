//! Function-introspection helpers for the general lane.

use super::*;
use crate::extract::node_text;
use tree_sitter::Node;

/// Grammar fields that carry an optional declared return type. Only optional
/// presence fields belong here: grammars whose return-type-ish child is
/// mandatory (java/c/cpp `type`) must stay matchable by keyword-agnostic
/// templates — the strictness is about presence agreement, not keyword
/// translation.
pub(crate) const RETURN_TYPE_FIELDS: &[&str] = &["return_type", "result", "returns"];

/// The declared return-type text of a function-like node, if any grammar
/// return field is present (br-hs1). Shared by the section matcher and the
/// capture binder.
pub(crate) fn function_return_type_text<'a>(node: &Node<'a>, source: &'a str) -> Option<&'a str> {
    // The ts/php `return_type` field node spans the separator too
    // (`: number` — type_annotation includes the colon); rust/go field
    // nodes carry the bare type. Strip a leading `:` so concrete
    // comparisons see the type text alone.
    RETURN_TYPE_FIELDS
        .iter()
        .find_map(|field| node.child_by_field_name(field))
        .and_then(|n| node_text(&n, source))
        .map(|s| s.trim_start_matches(':').trim())
}

pub(crate) fn function_body_matches(node: &Node, template: &BodyTemplate) -> bool {
    let Some(body) = function_body_node(node) else {
        return false;
    };
    match template {
        BodyTemplate::Any => true,
        BodyTemplate::Exactly(want) => count_statements(body) == *want,
    }
}

/// The body block of a function-like match node. Falls back to scanning named
/// children (and their `body` fields, for `const f = () => {...}` declarators)
/// when the grammar has no `body` field.
pub(crate) fn function_body_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    // The ts type_alias_declaration spells its member-count body as the
    // `value` field (the object_type) — the `type-alias` Class face counts
    // its members there.
    if node.kind() == "type_alias_declaration" {
        if let Some(value) = node.child_by_field_name("value") {
            return Some(value);
        }
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'a>> = node.named_children(&mut cursor).collect();
    children
        .iter()
        .find(|child| BLOCK_KINDS.contains(&child.kind()))
        .copied()
        .or_else(|| {
            children
                .iter()
                .find_map(|child| child.child_by_field_name("body"))
        })
}

/// Count named non-comment statements in a body, descending through
/// statement-free wrapper nodes (`function_body` → `statements` → …).
pub(crate) fn count_statements(body: Node) -> usize {
    let mut container = body;
    while STMT_WRAPPER_KINDS.contains(&container.kind()) {
        let mut cursor = container.walk();
        let named: Vec<Node> = container
            .named_children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            .collect();
        match named.as_slice() {
            [only] if BLOCK_KINDS.contains(&only.kind()) => container = *only,
            _ => break,
        }
    }
    let mut cursor = container.walk();
    container
        .named_children(&mut cursor)
        .filter(|child| !is_trivia_kind(child.kind()))
        .count()
}
