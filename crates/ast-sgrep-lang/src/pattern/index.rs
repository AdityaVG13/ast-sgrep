//! Pattern-node extraction and declaration index rows.

use super::*;
use crate::extract::{is_ident_kind, is_in_comment_or_string, node_text};
use crate::PatternNode;
use tree_sitter::Node;

/// Deepest AST the extraction walk descends into. Pathological nesting
/// (`let x = ((((…1…))))`) made the walk superquadratic — 2.3 s at depth 500,
/// >90 s at depth 10000 — while real code stays far below this bound.
/// > Subtrees beyond the cap are skipped and reported through
/// > `ExtractionResult::depth_truncated` so the cached pattern lane can
/// > refuse to serve those files as complete instead of failing open.
pub const MAX_EXTRACTION_DEPTH: usize = 256;

pub(crate) fn collect_pattern_nodes(root: Node, source: &str) -> (Vec<PatternNode>, bool) {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut depth_truncated = false;
    collect_node_signatures(root, source, &mut out, &mut seen, 0, &mut depth_truncated);
    (out, depth_truncated)
}

pub(crate) fn collect_node_signatures(
    node: Node,
    source: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
    depth: usize,
    depth_truncated: &mut bool,
) {
    if depth > MAX_EXTRACTION_DEPTH {
        *depth_truncated = true;
        // Decl rows stay budget-exempt — a node past
        // the depth bound still records its own declaration row (prefix +
        // name are direct-child reads) and recursion CONTINUES, so decl-exact
        // cached lanes are complete even for files that breached the budget.
        // ident/call rows past the budget stay unrecorded (incomplete by
        // contract: those shapes keep the walk/refusal paths under
        // truncation). Cost note (P-SEPT14E-2): the continued traversal pays
        // the same registered O(nodes×depth) class the native walk pays; it
        // only lands on files deeper than MAX_EXTRACTION_DEPTH.
        if declaration_prefix(&node, source).is_some() {
            record_node_signatures(&node, source, out, seen);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_node_signatures(child, source, out, seen, depth + 1, depth_truncated);
        }
        return;
    }
    if is_in_comment_or_string(&node) {
        // B1 (Sept 14 wave): comment/string subtrees hold no extractable
        // signatures. Recursing into them only burns depth budget and can
        // false-flag a file whose real code is shallow, so prune here.
        return;
    }
    record_node_signatures(&node, source, out, seen);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_signatures(child, source, out, seen, depth + 1, depth_truncated);
    }
}

pub(crate) fn record_node_signatures(
    node: &Node,
    source: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    if is_ident_kind(node.kind()) {
        if let Some(text) = node_text(node, source) {
            push_pattern_node(*node, source, text, out, seen);
        }
    }
    if let Some(prefix) = declaration_prefix(node, source) {
        push_pattern_node(*node, source, &format!("kind:{}", node.kind()), out, seen);
        if let Some(name) = declaration_index_name(node, source) {
            push_pattern_node(*node, source, &format!("{prefix} {name}"), out, seen);
            push_pattern_node(*node, source, &format!("decl:{prefix}:{name}"), out, seen);
        }
    }
    if !is_call_kind(node.kind()) {
        return;
    }
    push_pattern_node(*node, source, &format!("kind:{}", node.kind()), out, seen);
    let Some(callee) = call_target(node, source) else {
        return;
    };
    push_pattern_node(*node, source, &format!("call:{callee}"), out, seen);
    if let Some(name) = callee.rsplit(['.', ':']).find(|p| !p.is_empty()) {
        push_pattern_node(*node, source, &format!("call-name:{name}"), out, seen);
    }
}

/// Map a tree-sitter declaration node to its indexed `decl:` / display prefix.
///
/// Most kinds are table-driven; `class_declaration` inspects Swift
/// `declaration_kind` / Kotlin keyword tokens so singleton forms stay exact.
pub fn declaration_prefix(node: &Node, source: &str) -> Option<&'static str> {
    let kind = node.kind();
    if kind == "class_declaration" {
        return class_declaration_prefix(node, source);
    }
    // MoonBit `fn` / `fn Type::method` reuse C/Python's `function_definition`
    // kind. The table maps that kind to `def`; a `function_identifier` child
    // is the MoonBit spelling, whose indexed rows must be `decl:fn:`.
    if kind == "function_definition" {
        let mut cursor = node.walk();
        if node
            .named_children(&mut cursor)
            .any(|child| child.kind() == "function_identifier")
        {
            return Some("fn");
        }
        return Some("def");
    }
    DECL_KIND_PREFIXES
        .iter()
        .find_map(|&(node_kind, prefix)| (node_kind == kind).then_some(prefix))
}

/// Declaration name for `decl:{prefix}:{name}` rows.
///
/// Field `name` covers most grammars. Dart nests the name under a signature
/// wrapper; C typedefs keep it on `declarator`; MoonBit names are the first
/// positional identifier-like child (`function_identifier` last-wins so
/// `Type::method` stores the method). Do not walk bodies — that would pick
/// an identifier from the function/struct body instead of the declarator.
pub(crate) fn declaration_index_name(node: &Node, source: &str) -> Option<String> {
    if let Some(text) = node
        .child_by_field_name("name")
        .and_then(|n| node_text(&n, source))
    {
        return Some(text.to_string());
    }
    const DART_NESTED_NAME: &[&str] = &[
        "function_declaration",
        "external_function_declaration",
        "local_function_declaration",
        "getter_declaration",
        "setter_declaration",
        "external_getter_declaration",
        "external_setter_declaration",
        "method_declaration",
    ];
    if DART_NESTED_NAME.contains(&node.kind()) {
        return crate::extract::signature_name(node, source);
    }
    if node.kind() == "type_definition" {
        if let Some(name) = crate::extract::declarator_name(node, source) {
            return Some(name);
        }
    }
    const POSITIONAL_NAME_CHILDREN: &[&str] = &[
        "function_identifier",
        "identifier",
        "lowercase_identifier",
        "uppercase_identifier",
        "type_identifier",
    ];
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if POSITIONAL_NAME_CHILDREN.contains(&child.kind()) {
            return crate::extract::last_identifier_under(&child, source);
        }
    }
    None
}

pub(crate) fn class_declaration_prefix(node: &Node, source: &str) -> Option<&'static str> {
    match node
        .child_by_field_name("declaration_kind")
        .and_then(|kind| node_text(&kind, source))
    {
        Some("struct" | "actor") => Some("struct"),
        Some("enum") => Some("enum"),
        Some("extension") => Some("type"),
        // No Swift declaration_kind (or unrecognised) — Kotlin reuses class_declaration.
        _ => match kotlin_class_keyword(node, source) {
            "interface" => Some("interface"),
            "enum" => Some("enum"),
            _ => Some("class"),
        },
    }
}

/// AST node kind → short declaration prefix used in `decl:{prefix}:{name}` signatures.
pub const DECL_KIND_PREFIXES: &[(&str, &str)] = &[
    ("function_item", "fn"),
    ("struct_item", "struct"),
    ("struct_declaration", "struct"),
    ("struct_specifier", "struct"),
    ("struct_definition", "struct"),
    ("tuple_struct_definition", "struct"),
    ("function_definition", "def"),
    ("function_declaration", "function"),
    ("protocol_function_declaration", "function"),
    ("method_definition", "function"),
    ("method_declaration", "function"),
    ("method", "function"),
    ("singleton_method", "function"),
    ("local_function_statement", "function"),
    ("local_function_declaration", "function"),
    ("getter_declaration", "function"),
    ("setter_declaration", "function"),
    ("external_function_declaration", "function"),
    ("external_getter_declaration", "function"),
    ("external_setter_declaration", "function"),
    ("named_lambda_expression", "fn"),
    ("impl_definition", "fn"),
    ("class_definition", "class"),
    ("class", "class"),
    ("record_declaration", "class"),
    ("class_specifier", "class"),
    ("trait_item", "interface"),
    ("interface_declaration", "interface"),
    ("protocol_declaration", "interface"),
    ("mixin_declaration", "interface"),
    ("trait_definition", "interface"),
    ("enum_item", "enum"),
    ("enum_declaration", "enum"),
    ("enum_specifier", "enum"),
    ("enum_definition", "enum"),
    ("extenum_definition", "enum"),
    ("error_type_definition", "enum"),
    ("type_definition", "type"),
    ("extension_declaration", "type"),
    ("extension_type_declaration", "type"),
];
