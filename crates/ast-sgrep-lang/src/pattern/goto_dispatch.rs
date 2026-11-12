//! C `goto` + Go `goto` tail lanes and the statement-root hub (F cluster, split from statements.rs).
//!
//! Covers the `c_goto` pair, the go import-semi accepted-empty face, the
//! go goto-bare pair, and `match_statement_root_140`; the kt/swift/rs
//! templates stay in statements.rs — no overlap.

use super::*;
use crate::extract::node_text;
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

pub(crate) struct CGotoTemplate {
    label: LaneMeta,
}

pub(crate) fn c_goto_template(pattern: &str) -> Option<CGotoTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("goto")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let label = lane_meta(rest.trim_start().strip_suffix(';')?.trim())?;
    Some(CGotoTemplate { label })
}

pub(crate) fn c_goto_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &CGotoTemplate,
) -> Option<PatternMatch> {
    let mut cursor = node.walk();
    let label = node
        .children(&mut cursor)
        .find(|c| c.kind() == "statement_identifier")?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    template
        .label
        .bind(&mut captures, node_text(&label, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// Go's `$`-carrying `;`-ful import spelling is ACCEPTED-EMPTY —
/// census-answerable, the walk's empty IS the agreement (the `goto $L;` twin
/// rides [`sg_goto_semi_pattern`]).
pub(crate) fn sg_go_import_semi_pattern(pattern: &str) -> bool {
    let p = pattern.trim();
    let Some(rest) = p.strip_prefix("import") else {
        return false;
    };
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let Some(target) = rest.trim_start().strip_suffix(';') else {
        return false;
    };
    let target = target.trim();
    !target.is_empty() && !target.contains(char::is_whitespace) && capture_name(target).is_some()
}

/// The go SEMI-LESS `goto <label>` spelling — the reference binds every go
/// goto_statement's label (tail comments irrelevant) where the `;`-ful twin
/// is the registered accepted-empty envelope and the C grammar is the
/// opposite (its semi-ful form binds). `$`-carrying labels only: the
/// concrete `goto end` spelling keeps the literal lane's faces.
pub(crate) struct GoGotoBareTemplate {
    label: LaneMeta,
}

pub(crate) fn go_goto_bare_template(pattern: &str) -> Option<GoGotoBareTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("goto")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let label = rest.trim_start();
    if label.is_empty() || label.contains(char::is_whitespace) || label.ends_with(';') {
        return None;
    }
    Some(GoGotoBareTemplate {
        label: lane_meta(label)?,
    })
}

pub(crate) fn walk_go_goto_bare(
    node: Node,
    source: &str,
    pattern: &str,
    template: &GoGotoBareTemplate,
    out: &mut Vec<PatternMatch>,
) {
    walk_kind(node, "goto_statement", true, out, |n| {
        go_goto_bare_match(n, source, pattern, template)
    });
}

/// Single-candidate check for [`walk_go_goto_bare`]: the label child binds
/// through the lane meta; anything else is not this face.
fn go_goto_bare_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &GoGotoBareTemplate,
) -> Option<PatternMatch> {
    let mut cursor = node.walk();
    let label = node.children(&mut cursor).find(|c| c.is_named())?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    template
        .label
        .bind(&mut captures, node_text(&label, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// The remaining statement-root lanes, language-scoped exactly as covered
/// (c `goto $L;` BINDS while go answers the same spelling empty).
pub(crate) fn match_statement_root_140(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    match lang {
        Language::Kotlin => {
            if let Some(template) = kt_typealias_template(pattern) {
                let tree = parse_source_silent(lang, source)?;
                let mut out = Vec::new();
                walk_kind(tree.root_node(), "type_alias", true, &mut out, |n| {
                    kt_typealias_match(n, source, pattern, &template)
                });
                return Some(out);
            }
            let template = kt_for_template(pattern)?;
            let tree = parse_source_silent(lang, source)?;
            let mut out = Vec::new();
            walk_kind(tree.root_node(), "for_statement", true, &mut out, |n| {
                kt_for_match(n, source, pattern, &template)
            });
            Some(out)
        }
        Language::Swift => {
            let template = swift_for_template(pattern)?;
            let tree = parse_source_silent(lang, source)?;
            let mut out = Vec::new();
            walk_kind(tree.root_node(), "for_statement", true, &mut out, |n| {
                swift_for_match(n, source, pattern, &template)
            });
            Some(out)
        }
        Language::Rust => {
            let template = rs_let_else_template(pattern)?;
            let tree = parse_source_silent(lang, source)?;
            let mut out = Vec::new();
            walk_kind(tree.root_node(), "let_declaration", true, &mut out, |n| {
                rs_let_else_match(n, source, pattern, &template)
            });
            Some(out)
        }
        Language::C => {
            let template = c_goto_template(pattern)?;
            let tree = parse_source_silent(lang, source)?;
            let mut out = Vec::new();
            walk_kind(tree.root_node(), "goto_statement", true, &mut out, |n| {
                c_goto_match(n, source, pattern, &template)
            });
            Some(out)
        }
        Language::Go => {
            // The semi-less `goto $L` family binds per site; the `;`-ful twin
            // keeps its accepted-empty envelope via sg_goto_semi_pattern and
            // never reaches here.
            let template = go_goto_bare_template(pattern)?;
            let tree = parse_source_silent(lang, source)?;
            let mut out = Vec::new();
            walk_go_goto_bare(tree.root_node(), source, pattern, &template, &mut out);
            Some(out)
        }
        _ => None,
    }
}
