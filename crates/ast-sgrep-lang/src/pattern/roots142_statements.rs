//! S7 statement-sibling match fns + riders (isomorphic split of roots142.rs).

use super::*;
use crate::extract::node_text;
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

pub(crate) fn named_non_trivia_children<'tree>(node: &Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect()
}

pub(crate) fn has_child_kind<'tree>(node: &Node<'tree>, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == kind);
    found
}

pub(crate) fn root146_named_children<'tree>(node: &Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()) && !child.is_extra())
        .collect()
}

/// cs `goto_statement` — the label is the first named child; the candidate
/// gap (comment/U+2028 junk) is transparent (the parse recovers the junk
/// outside the label).
pub(crate) fn root146_goto_match(
    node: &Node,
    source: &str,
    pattern: &str,
    label: &LaneMeta,
) -> Option<PatternMatch> {
    let label_node = root146_named_children(node).first().copied()?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(&label_node, source)?;
    label.bind(&mut captures, text.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// cs `new <T>(<A>)[;]` at the expression_statement root — exactly ONE
/// argument; the nested face refuses naturally (no object-creation child).
pub(crate) fn root146_cs_new_match(
    node: &Node,
    source: &str,
    pattern: &str,
    ty: &LaneName,
    arg: &LaneMeta,
) -> Option<PatternMatch> {
    let children = root146_named_children(node);
    let [creation] = children.as_slice() else {
        return None;
    };
    if creation.kind() != "object_creation_expression" {
        return None;
    }
    let creation_children = root146_named_children(creation);
    let (type_node, arg_list) = match creation_children.as_slice() {
        [type_node, arg_list] if arg_list.kind() == "argument_list" => (type_node, arg_list),
        _ => return None,
    };
    let args = root146_named_children(arg_list);
    let [only_arg] = args.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let type_text = node_text(type_node, source)?;
    ty.bind_or_match(&mut captures, type_text.trim())?;
    let arg_text = node_text(only_arg, source)?;
    arg.bind(&mut captures, arg_text.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// cs switch_statement — the subject slot binds between the parens and the
/// body must carry EXACTLY ONE switch_section (empty/multi refuse).
/// Kind-exact: the switch_expression row never crosses.
pub(crate) fn root146_cs_switch_match(
    node: &Node,
    source: &str,
    pattern: &str,
    subject: &Option<LaneName>,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let body_node = named.last()?;
    if body_node.kind() != "switch_body" {
        return None;
    }
    let subject_node = if named.len() == 2 {
        Some(&named[0])
    } else {
        None
    };
    match (subject, subject_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let sections = root146_named_children(body_node);
    let [section] = sections.as_slice() else {
        return None;
    };
    if section.kind() != "switch_section" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(slot), Some(subject_node)) = (subject, subject_node) {
        let text = node_text(subject_node, source)?;
        slot.bind_or_match(&mut captures, text.trim())?;
    }
    let text = node_text(section, source)?;
    body.bind(&mut captures, text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// php declaration kinds — the ONE-member meta law and the Mixed
/// exact-order face, modeled on the registered namespace lane; the Mixed-doc
/// wrapper uses the SAME declaration kind so member-kind alignment holds
/// (class property/const prefixes).
pub(crate) fn root146_php_decl_match(
    node: &Node,
    source: &str,
    pattern: &str,
    kind: PhpDeclKind,
    name: &LaneName,
    body: &PhpNamespaceBody,
) -> Option<PatternMatch> {
    let name_node = node.child_by_field_name("name")?;
    let body_node = node.child_by_field_name("body")?;
    let members = root146_named_children(&body_node);
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_text = node_text(&name_node, source)?;
    name.bind_or_match(&mut captures, name_text.trim())?;
    match body {
        PhpNamespaceBody::Meta(body_meta) => {
            let [only] = members.as_slice() else {
                return None;
            };
            let text = node_text(only, source)?;
            body_meta.bind(&mut captures, text)?;
        }
        PhpNamespaceBody::Mixed { prefix, tail } => {
            let doc = format!("<?php\n{kw} __Px {{ {prefix} }}\n", kw = kind.keyword());
            let tpl_tree = parse_source_silent(Language::Php, &doc)?;
            if tpl_tree.root_node().has_error() {
                return None;
            }
            let prefix_members: Vec<Node> = tpl_tree
                .root_node()
                .children(&mut tpl_tree.root_node().walk())
                .find(|n| n.kind() == kind.root_kind())
                .and_then(|decl| decl.child_by_field_name("body"))
                .map(|body| root146_named_children(&body))
                .unwrap_or_default();
            if prefix_members.is_empty() || members.len() != prefix_members.len() + 1 {
                return None;
            }
            for (p_stmt, c_stmt) in prefix_members.iter().zip(members.iter()) {
                if !literal_structural_eq(*p_stmt, &doc, *c_stmt, source, false) {
                    return None;
                }
            }
            let tail_text = node_text(&members[members.len() - 1], source)?.trim();
            tail.bind(&mut captures, tail_text)?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// go expression_switch_statement — optional subject slot (demarcated: a
/// global pattern refuses a subject-bearing candidate and vice versa) and
/// EXACTLY ONE case clause (2-clause/empty refuse). The clause text binds
/// RAW (the capture keeps the trailing newline). Type-switch candidates are
/// a distinct kind.
pub(crate) fn root146_go_switch_match(
    node: &Node,
    source: &str,
    pattern: &str,
    subject: &Option<LaneName>,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let is_clause = |n: &Node| n.kind().ends_with("_case") || n.kind() == "default_case";
    // tree-sitter-go wraps the clauses in a `switch_body` child; the
    // subject-less face must not mistake that wrapper for the subject.
    let body_wrapper = named.iter().find(|n| n.kind() == "switch_body").copied();
    let clauses: Vec<Node> = match &body_wrapper {
        Some(wrapper) => root146_named_children(wrapper)
            .into_iter()
            .filter(|n| is_clause(n))
            .collect(),
        None => named.iter().filter(|n| is_clause(n)).copied().collect(),
    };
    let subject_node = named
        .iter()
        .find(|n| !is_clause(n) && n.kind() != "switch_body");
    match (subject, subject_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let [clause] = clauses.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(slot), Some(subject_node)) = (subject, subject_node) {
        let text = node_text(subject_node, source)?;
        slot.bind_or_match(&mut captures, text.trim())?;
    }
    let text = node_text(clause, source)?;
    body.bind(&mut captures, text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// go if_statement — C binds the raw condition bytes between the `if`
/// keyword and the body; B binds the WHOLE body inner text and refuses the
/// empty body.
pub(crate) fn root146_go_if_match(
    node: &Node,
    source: &str,
    pattern: &str,
    cond: &LaneName,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let if_kw = children.first()?;
    if if_kw.kind() != "if" {
        return None;
    }
    let block = children
        .iter()
        .find(|child| child.is_named() && child.kind() == "block")?;
    let cond_node = children.iter().find(|child| child.is_named())?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let cond_text = source[if_kw.end_byte()..block.start_byte()].trim();
    if cond_text.is_empty() {
        return None;
    }
    let _ = cond_node;
    cond.bind_or_match(&mut captures, cond_text)?;
    let inner = &source[block.start_byte() + 1..block.end_byte().saturating_sub(1)];
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    body.bind(&mut captures, inner)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// go package_clause — the name binds through a comment gap (the comment
/// is an extra outside the identifier).
pub(crate) fn root146_go_package_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [name_node] = named.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(name_node, source)?;
    name.bind(&mut captures, text.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// go SEMI-LESS import_declaration — the single path or the WHOLE group
/// text binds (X raw).
pub(crate) fn root146_go_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    target: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [spec] = named.as_slice() else {
        return None;
    };
    if spec.kind() != "import_spec" && spec.kind() != "import_spec_list" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(spec, source)?;
    target.bind(&mut captures, text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// js if_statement arms — ONE-statement bodies; the else-if tail binds the
/// INNER if's head only and the walk's own recursion emits per level; the
/// plain-else and no-else tails demarcate.
pub(crate) fn root146_js_if_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JsIfTemplate,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [cond_node, body_node, tail_node] = named.as_slice() else {
        let [cond_node, body_node] = named.as_slice() else {
            return None;
        };
        return root146_js_if_bind(node, source, pattern, template, cond_node, body_node, None);
    };
    root146_js_if_bind(
        node,
        source,
        pattern,
        template,
        cond_node,
        body_node,
        Some(tail_node),
    )
}

pub(crate) fn root146_js_if_bind(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JsIfTemplate,
    cond_node: &Node,
    body_node: &Node,
    tail_node: Option<&Node>,
) -> Option<PatternMatch> {
    if cond_node.kind() != "parenthesized_expression" || body_node.kind() != "statement_block" {
        return None;
    }
    let cond_inner = root146_named_children(cond_node);
    let [cond_expr] = cond_inner.as_slice() else {
        return None;
    };
    let body_stmts = root146_named_children(body_node);
    let [body_stmt] = body_stmts.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let cond_text = node_text(cond_expr, source)?;
    template.cond.bind(&mut captures, cond_text.trim())?;
    let body_text = node_text(body_stmt, source)?;
    template.body.bind(&mut captures, body_text)?;
    match &template.tail {
        JsTail::None => {
            if tail_node.is_some() {
                return None;
            }
        }
        JsTail::Block(meta) => {
            let tail_node = tail_node?;
            if tail_node.kind() != "else_clause" {
                return None;
            }
            let else_body = root146_named_children(tail_node);
            let [else_block] = else_body.as_slice() else {
                return None;
            };
            if else_block.kind() != "statement_block" {
                return None;
            }
            let stmts = root146_named_children(else_block);
            let [stmt] = stmts.as_slice() else {
                return None;
            };
            let text = node_text(stmt, source)?;
            meta.bind(&mut captures, text)?;
        }
        JsTail::ElseIf { cond, body } => {
            let tail_node = tail_node?;
            if tail_node.kind() != "else_clause" {
                return None;
            }
            let inner = root146_named_children(tail_node);
            let [inner_if] = inner.as_slice() else {
                return None;
            };
            if inner_if.kind() != "if_statement" {
                return None;
            }
            let inner_named = root146_named_children(inner_if);
            // The inner if may itself carry a FURTHER else-if tail: only
            // cond+body feed this level's C/D binds; the walk emits the
            // deeper levels at their own if nodes.
            if inner_named.len() < 2 || inner_named.len() > 3 {
                return None;
            }
            let (inner_cond, inner_body) = (&inner_named[0], &inner_named[1]);
            if inner_cond.kind() != "parenthesized_expression"
                || inner_body.kind() != "statement_block"
            {
                return None;
            }
            let inner_cond_expr = root146_named_children(inner_cond);
            let [inner_cond_expr] = inner_cond_expr.as_slice() else {
                return None;
            };
            let inner_stmts = root146_named_children(inner_body);
            let [inner_stmt] = inner_stmts.as_slice() else {
                return None;
            };
            let cond_text = node_text(inner_cond_expr, source)?;
            cond.bind(&mut captures, cond_text.trim())?;
            let body_text = node_text(inner_stmt, source)?;
            body.bind(&mut captures, body_text)?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}
