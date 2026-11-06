//! C# checked/unchecked EXPRESSION root lane (C cluster, split from statements.rs).
//!
//! Covers the `checked_expression` root (`checked ($X)` / `unchecked ($X)`
//! as an expression); the `checked_statement` head lane lives in
//! cs_statements.rs — no overlap.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

// --- C (140A-F3): the csharp checked/unchecked EXPRESSION root lane. ---

pub(crate) struct CsExpressionTemplate {
    keyword: &'static str,
    operand: String,
}

pub(crate) fn cs_expression_template(pattern: &str) -> Option<CsExpressionTemplate> {
    let p = pattern.trim();
    let (keyword, rest) = if let Some(rest) = p.strip_prefix("checked") {
        ("checked", rest)
    } else {
        let rest = p.strip_prefix("unchecked")?;
        ("unchecked", rest)
    };
    if !rest.starts_with('(') {
        return None;
    }
    let inner = &rest[1..];
    let close = balanced_paren_close(inner)?;
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    let operand = capture_name(inner[..close].trim())?.to_string();
    Some(CsExpressionTemplate { keyword, operand })
}

pub(crate) fn match_csharp_expression_root(
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let template = cs_expression_template(pattern)?;
    let tree = parse_source_silent(Language::CSharp, source)?;
    let mut out = Vec::new();
    walk_csharp_expression_root(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

pub(crate) fn walk_csharp_expression_root(
    node: Node,
    source: &str,
    pattern: &str,
    template: &CsExpressionTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "checked_expression" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = csharp_expression_root_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_csharp_expression_root(child, source, pattern, template, out);
    }
}

pub(crate) fn csharp_expression_root_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &CsExpressionTemplate,
) -> Option<PatternMatch> {
    // checked/unchecked SHARE the checked_expression kind — the keyword
    // token must agree token-exactly (the 137 statement-lane discipline).
    let mut cursor = node.walk();
    let keyword = node
        .children(&mut cursor)
        .find(|c| !c.is_named() && !is_trivia_kind(c.kind()))
        .and_then(|c| node_text(&c, source))?;
    if keyword != template.keyword {
        return None;
    }
    let mut cursor = node.walk();
    let expr = node
        .children(&mut cursor)
        .find(|c| c.is_named() && !is_trivia_kind(c.kind()))?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(&expr, source)?;
    bind_capture(&mut captures, &template.operand, text.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}
