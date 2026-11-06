//! General structural lane: placeholder-in-literal scans.

use super::*;
use crate::extract::node_text;
use std::collections::BTreeMap;
use tree_sitter::Node;

pub(crate) fn pattern_has_placeholder_in_literal(
    node: &Node,
    doc: &str,
    placeholders: &BTreeMap<String, String>,
) -> bool {
    let kind = node.kind();
    // A placeholder inside a string INTERPOLATION is a metavariable hole,
    // not literal text — the reference binds it (`"user-#{$N}"` answers with
    // N = the interpolation content). The interpolation subtree is exempt
    // from the scan; plain string content keeps the literal-metavariable
    // refusal.
    if kind == "interpolation" {
        return false;
    }
    // A raw string literal (`r#"$A"#`) is TEXT to the reference — its
    // expando-replaced pattern still carries the placeholder bytes inside
    // the raw string, and the reference answers accepted-empty on such
    // faces, not a refusal. Exempting the literal AND its string_content
    // child lets the template build; the leaf text comparison then answers
    // match-none exactly like the reference (the placeholder bytes never
    // appear in a real source raw string). Ordinary string literals keep
    // the literal-metavariable refusal.
    let exempt = kind == "raw_string_literal"
        || (kind == "string_content"
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == "raw_string_literal"));
    if !exempt
        && (kind.contains("string") || is_trivia_kind(kind) || kind.contains("regex"))
        && node_text(node, doc).is_some_and(|text| text.contains(GENERAL_MV_PREFIX))
    {
        // The placeholder may reach a string node ONLY through interpolation
        // children (exempted below) — when every placeholder occurrence
        // inside this node sits in an interpolation subtree, the node itself
        // is container syntax, not literal text, and must not refuse. Plain
        // string content keeps the literal-metavariable refusal.
        // EXCEPT the WHOLE-CONTENT meta hole — a `string_content` leaf whose
        // ENTIRE text is a meta token IS a metavariable and binds the
        // candidate content. When every placeholder-bearing leaf of the
        // string node IS exactly a placeholder, the build is admitted;
        // prefix/suffix spellings (`"pre-$A"`) keep the refusal — those
        // leaves read as literal text — and comment/regex nodes never
        // qualify.
        let string_node = kind.contains("string");
        if !placeholders_only_inside_interpolations(node, doc)
            && !(string_node && placeholders_are_whole_string_content(node, doc, placeholders))
        {
            return true;
        }
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| pattern_has_placeholder_in_literal(&child, doc, placeholders));
    found
}

/// True when every leaf of this string node whose text carries the
/// placeholder prefix IS exactly a placeholder key — i.e. the string's
/// content is a bare metavariable hole (`"$A"`), the reference-truth meta
/// binding. Any placeholder that is a PROPER SUBSTRING of a content leaf
/// (`"pre-$A"`, `"$A-$B"`) makes this false and keeps the literal-text
/// refusal.
pub(crate) fn placeholders_are_whole_string_content(
    node: &Node,
    doc: &str,
    placeholders: &BTreeMap<String, String>,
) -> bool {
    let mut whole = true;
    fn scan(node: &Node, doc: &str, placeholders: &BTreeMap<String, String>, whole: &mut bool) {
        if node.kind() == "interpolation" {
            return; // interpolation subtrees are already-admitted meta holes
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        if children.is_empty() {
            if let Some(text) = node_text(node, doc) {
                if text.contains(GENERAL_MV_PREFIX) && !placeholders.contains_key(text) {
                    *whole = false;
                }
            }
            return;
        }
        for child in children {
            scan(&child, doc, placeholders, whole);
        }
    }
    scan(node, doc, placeholders, &mut whole);
    whole
}

/// True when every direct child carrying the placeholder prefix is a string
/// interpolation node (the only string context the reference treats as a
/// metavariable hole).
pub(crate) fn placeholders_only_inside_interpolations(node: &Node, doc: &str) -> bool {
    let mut cursor = node.walk();
    let all_clear = node.children(&mut cursor).all(|child| {
        let carries = node_text(&child, doc).is_some_and(|text| text.contains(GENERAL_MV_PREFIX));
        !carries || child.kind() == "interpolation"
    });
    all_clear
}
