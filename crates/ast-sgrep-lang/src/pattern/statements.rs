//! Per-language statement-root lanes.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

// --- F (140A-F6): the remaining statement roots. ---

pub(crate) struct KtTypealiasTemplate {
    name: LaneMeta,
    target: LaneMeta,
}

pub(crate) fn kt_typealias_template(pattern: &str) -> Option<KtTypealiasTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("typealias")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let eq_at = rest.find('=')?;
    let name = lane_meta(rest[..eq_at].trim())?;
    let target = lane_meta(rest[eq_at + 1..].trim())?;
    Some(KtTypealiasTemplate { name, target })
}

pub(crate) fn kt_typealias_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &KtTypealiasTemplate,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    let [name_node, target_node] = children.as_slice() else {
        return None;
    };
    template
        .name
        .bind(&mut captures, node_text(name_node, source)?.trim())?;
    template
        .target
        .bind(&mut captures, node_text(target_node, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) struct KtForTemplate {
    item: LaneMeta,
    collection: LaneMeta,
    body: LaneMeta,
}

pub(crate) fn kt_for_template(pattern: &str) -> Option<KtForTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("for")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    let header = &inner[..close];
    let (left, right) = split_top_level_keyword(header, " in ")?;
    let item = lane_meta(left.trim())?;
    let collection = lane_meta(right.trim())?;
    let rest = inner[close + 1..].trim_start();
    let brace_inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(brace_inner)?;
    if !brace_inner[close + 1..].trim().is_empty() {
        return None;
    }
    let body = lane_meta(brace_inner[..close].trim())?;
    Some(KtForTemplate {
        item,
        collection,
        body,
    })
}

pub(crate) fn kt_for_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &KtForTemplate,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    // kotlin for_statement carries NO field list — the positions are the
    // law: item, collection, then the control-structure body LAST.
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    if children.len() < 3 {
        return None;
    }
    let item = &children[0];
    let collection = &children[1];
    let body = children.last()?;
    let body_text = node_text(body, source)?;
    // The brace-less body refuses.
    let brace_inner = body_text.trim_start().strip_prefix('{')?;
    if !brace_inner.trim_end().ends_with('}') {
        return None;
    }
    let inner = brace_inner.trim_end().strip_suffix('}')?;
    template
        .item
        .bind(&mut captures, node_text(item, source)?.trim())?;
    template
        .collection
        .bind(&mut captures, node_text(collection, source)?.trim())?;
    // B = the block inner trimmed BOTH ends (F_kt_for / R2_kt_for_2stmt).
    template.body.bind(&mut captures, inner.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) struct SwiftForTemplate {
    item: LaneMeta,
    collection: LaneMeta,
    where_clause: Option<LaneMeta>,
    body: LaneMeta,
}

pub(crate) fn swift_for_template(pattern: &str) -> Option<SwiftForTemplate> {
    let mut tokens = pattern.split_whitespace();
    if tokens.next()? != "for" {
        return None;
    }
    let item = lane_meta(tokens.next()?)?;
    if tokens.next()? != "in" {
        return None;
    }
    let collection = lane_meta(tokens.next()?)?;
    let mut where_clause = None;
    match tokens.next()? {
        "where" => {
            where_clause = Some(lane_meta(tokens.next()?)?);
            if tokens.next()? != "{" {
                return None;
            }
        }
        "{" => {}
        _ => return None,
    }
    let body = lane_meta(tokens.next()?)?;
    // The spelling's closing `}` is the last token.
    if tokens.next() != Some("}") || tokens.next().is_some() {
        return None;
    }
    Some(SwiftForTemplate {
        item,
        collection,
        where_clause,
        body,
    })
}

pub(crate) fn swift_for_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &SwiftForTemplate,
) -> Option<PatternMatch> {
    let item = node.child_by_field_name("item")?;
    let collection = node.child_by_field_name("collection")?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    template
        .item
        .bind(&mut captures, node_text(&item, source)?.trim())?;
    template
        .collection
        .bind(&mut captures, node_text(&collection, source)?.trim())?;
    // The `where` clause presence must agree on BOTH sides.
    let mut cursor = node.walk();
    let where_node = node
        .children(&mut cursor)
        .find(|c| c.kind() == "where_clause");
    match (&template.where_clause, &where_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    if let (Some(want), Some(where_node)) = (&template.where_clause, where_node) {
        // The condition is the where_clause's named child EXCLUDING the
        // `where` keyword token the grammar surfaces as a named child
        // (F_sw_for_where W=`x > 1`).
        let mut wcursor = where_node.walk();
        let condition = where_node
            .children(&mut wcursor)
            .filter(|c| c.is_named())
            .find(|c| node_text(c, source) != Some("where"))?;
        want.bind(&mut captures, node_text(&condition, source)?.trim())?;
    }
    // Body: the LAST named child; B = the inner text trim_start ONLY —
    // the reference KEEPS the trailing whitespace (B=`g(x)\n    `).
    // tree-sitter-swift exposes the body as the brace-less statements
    // group whose span IS the reference's B text; a braced child
    // (defensive spelling) unwraps first.
    let mut cursor = node.walk();
    let body = node.children(&mut cursor).filter(|c| c.is_named()).last()?;
    let body_text = node_text(&body, source)?;
    let inner = if let Some(brace_inner) = body_text.trim_start().strip_prefix('{') {
        brace_inner.strip_suffix('}')?.trim_start()
    } else {
        body_text
    };
    template.body.bind(&mut captures, inner)?;
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) struct RsLetElseTemplate {
    pattern_slot: LaneMeta,
    value: LaneMeta,
    body: LaneMeta,
}

pub(crate) fn rs_let_else_template(pattern: &str) -> Option<RsLetElseTemplate> {
    let mut tokens = pattern.split_whitespace();
    if tokens.next()? != "let" {
        return None;
    }
    let pattern_slot = tokens.next()?;
    if tokens.next()? != "=" {
        return None;
    }
    let value = tokens.next()?;
    if tokens.next()? != "else" {
        return None;
    }
    if tokens.next()? != "{" {
        return None;
    }
    let body = tokens.next()?;
    // The spelling ends `};`.
    if tokens.next()? != "};" || tokens.next().is_some() {
        return None;
    }
    Some(RsLetElseTemplate {
        pattern_slot: lane_meta(pattern_slot)?,
        value: lane_meta(value)?,
        body: lane_meta(body)?,
    })
}

pub(crate) fn rs_let_else_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &RsLetElseTemplate,
) -> Option<PatternMatch> {
    let pattern_node = node.child_by_field_name("pattern")?;
    let value = node.child_by_field_name("value")?;
    // A no-else candidate refuses.
    let alternative = node.child_by_field_name("alternative")?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    template
        .pattern_slot
        .bind(&mut captures, node_text(&pattern_node, source)?.trim())?;
    template
        .value
        .bind(&mut captures, node_text(&value, source)?.trim())?;
    // B = the ONE-statement else body's text.
    let mut body_cursor = alternative.walk();
    let stmts: Vec<Node> = alternative
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    template
        .body
        .bind(&mut captures, node_text(only, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

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
    if node.kind() == "goto_statement" && node.is_named() && !is_in_comment_or_string(&node) {
        let mut cursor = node.walk();
        let label = node.children(&mut cursor).find(|c| c.is_named());
        if let Some(hit) = label.and_then(|label| {
            let mut captures = BTreeMap::new();
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            template
                .label
                .bind(&mut captures, node_text(&label, source)?.trim())?;
            Some(hit_for_node(&node, source, pattern, captures))
        }) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_go_goto_bare(child, source, pattern, template, out);
    }
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
