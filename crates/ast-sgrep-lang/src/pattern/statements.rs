//! Per-language statement-root lanes.

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
    let tree = parse_source(Language::CSharp, source).ok()?;
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

// --- D (140A-F4): the java synchronized nested-block + METHOD faces. The
// 139 bare-meta lane keeps the flat faces (D_sync_plain_ctl AGREE n2); the
// lanes here take what it refuses. ---

/// The BLOCK slots of a synchronized face. Resource: `Ok(meta)` binds the
/// resource's inner text in the meta's namespace; `Err(literal)` byte-
/// matches. Body: a bare meta binds the ONE statement's text; a nested
/// `synchronized (…) { … }` recurses.
#[derive(Debug, Clone)]
pub(crate) struct JaSyncMetaBlock {
    resource: Result<LaneMeta, String>,
    body: JaSyncBlockBody,
}

#[derive(Debug, Clone)]
pub(crate) enum JaSyncBlockBody {
    Meta(LaneMeta),
    Nested(Box<JaSyncMetaBlock>),
}

#[derive(Debug, Clone)]
pub(crate) struct JaSyncMethodTemplate {
    /// Pattern modifiers IN ORDER (the head `synchronized` included). The
    /// walk hops ordinary keyword modifiers positionally; an annotation
    /// beyond the DEMANDED prefix blocks passage (the 137 java modifier
    /// discipline, D_sync_static / D_sync_static_pat).
    modifiers: Vec<String>,
    /// The PATTERN-side leading annotation run — each demanded annotation
    /// must match a candidate annotation in order (an absent one refuses).
    annotations: Vec<String>,
    name: LaneMeta,
    body: JaSyncMethodBody,
}

#[derive(Debug, Clone)]
pub(crate) enum JaSyncMethodBody {
    /// `{ $B }` — exactly one statement, B = its text.
    Meta(LaneMeta),
    /// `{ synchronized (…) { … } }` — the body IS a synchronized block.
    SyncBlock(JaSyncMetaBlock),
}

pub(crate) const JA_SYNC_METHOD_MODIFIERS: &[&str] = &[
    "public",
    "protected",
    "private",
    "static",
    "final",
    "abstract",
    "strictfp",
    "default",
    "native",
    "transient",
    "volatile",
    "synchronized",
];

/// Strip the leading modifier-keyword run; returns the modifiers in order
/// and whether the run CARRIES `synchronized` anywhere — `synchronized
/// static …` is the method face too (the in-order demand refuses the
/// static-first candidate).
pub(crate) fn ja_strip_modifier_run(pattern: &str) -> (Vec<String>, bool, &str) {
    let mut modifiers = Vec::new();
    let mut rest = pattern;
    loop {
        let mut consumed = false;
        for keyword in JA_SYNC_METHOD_MODIFIERS {
            if let Some(after) = rest.strip_prefix(keyword) {
                if after.starts_with(char::is_whitespace) {
                    modifiers.push((*keyword).to_string());
                    rest = after.trim_start();
                    consumed = true;
                    break;
                }
            }
        }
        if !consumed {
            break;
        }
    }
    let head_sync = modifiers.iter().any(|m| m == "synchronized");
    (modifiers, head_sync, rest)
}

/// Strip the PATTERN's leading annotation run — bare `@Name` tokens
/// (identifier, no argument list) separated by whitespace/newlines. Each
/// demanded annotation must match a candidate annotation IN ORDER; a pattern
/// annotation with no candidate annotation refuses.
pub(crate) fn ja_strip_annotation_run(pattern: &str) -> (Vec<String>, &str) {
    let mut out = Vec::new();
    let mut rest = pattern;
    while let Some(after) = rest.strip_prefix('@') {
        let name_end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let name = &after[..name_end];
        if name.is_empty() || !is_pattern_ident(name) {
            break;
        }
        out.push(format!("@{name}"));
        rest = after[name_end..].trim_start();
    }
    (out, rest)
}

pub(crate) fn ja_strip_ret(rest: &str) -> Option<(&str, &str)> {
    let rest = rest.trim_start();
    let ret_end = rest.find(char::is_whitespace)?;
    let ret = &rest[..ret_end];
    if ret.is_empty() || ret.starts_with('$') || !is_pattern_ident(ret) {
        return None;
    }
    Some((ret, &rest[ret_end..]))
}

pub(crate) fn ja_name_then_parens(rest: &str) -> Option<(LaneMeta, &str)> {
    let rest = rest.trim_start();
    let name_end = rest
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(rest.len());
    let name = lane_meta(&rest[..name_end])?;
    let rest = rest[name_end..].trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    if !inner[..close].trim().is_empty() {
        return None;
    }
    Some((name, &inner[close + 1..]))
}

pub(crate) fn ja_method_body(rest: &str) -> Option<JaSyncMethodBody> {
    let rest = rest.trim_start();
    let brace_inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(brace_inner)?;
    if !brace_inner[close + 1..].trim().is_empty() {
        return None;
    }
    let section = brace_inner[..close].trim();
    if let Some(meta) = lane_meta(section) {
        return Some(JaSyncMethodBody::Meta(meta));
    }
    let after = section.strip_prefix("synchronized")?;
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    let block = ja_sync_block_slots(after.trim_start())?;
    Some(JaSyncMethodBody::SyncBlock(block))
}

pub(crate) fn ja_sync_block_nested_template(pattern: &str) -> Option<JaSyncMetaBlock> {
    let p = pattern.trim();
    let rest = p.strip_prefix("synchronized")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    ja_sync_block_slots(rest.trim_start())
}

pub(crate) fn ja_sync_block_slots(rest: &str) -> Option<JaSyncMetaBlock> {
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    let resource_section = inner[..close].trim();
    let resource = if resource_section.is_empty() {
        return None;
    } else if let Some(meta) = lane_meta(resource_section) {
        Ok(meta)
    } else if capture_name(resource_section).is_none()
        && !resource_section.contains('$')
        && !resource_section.contains('(')
        && !resource_section.contains('{')
    {
        Err(resource_section.to_string())
    } else {
        return None;
    };
    let rest = inner[close + 1..].trim_start();
    let brace_inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(brace_inner)?;
    if !brace_inner[close + 1..].trim().is_empty() {
        return None;
    }
    let body_section = brace_inner[..close].trim();
    let body = if let Some(meta) = lane_meta(body_section) {
        JaSyncBlockBody::Meta(meta)
    } else {
        let after = body_section.strip_prefix("synchronized")?;
        if !after.starts_with(char::is_whitespace) {
            return None;
        }
        // BOUNDEDNESS-OF-RECORD (141B-F3): PATTERN-controlled recursion —
        // one frame per nested `synchronized` brace group in the QUERY
        // string, unbounded by design (same posture as the
        // csharp_statement_template Nested recursion).
        JaSyncBlockBody::Nested(Box::new(ja_sync_block_slots(after.trim_start())?))
    };
    Some(JaSyncMetaBlock { resource, body })
}

pub(crate) fn ja_sync_method_template(pattern: &str) -> Option<JaSyncMethodTemplate> {
    let p = pattern.trim();
    let (annotations, p) = ja_strip_annotation_run(p);
    let (modifiers, head_sync, rest) = ja_strip_modifier_run(p);
    let (ret, rest) = ja_strip_ret(rest)?;
    let _ = ret;
    let (name, rest) = ja_name_then_parens(rest)?;
    let body = ja_method_body(rest)?;
    if head_sync {
        // `synchronized`-carrying METHOD face:
        // `[@Anno…] [modifiers] ret $M() { body }`.
        return Some(JaSyncMethodTemplate {
            modifiers,
            annotations,
            name,
            body,
        });
    }
    // Method-BODY face: `ret $M() { synchronized (…) { … } }` — the body
    // MUST parse as a synchronized block; `ret $M() { $B }` without the
    // synchronized head is a different, unprobed face.
    match body {
        JaSyncMethodBody::SyncBlock(_) => Some(JaSyncMethodTemplate {
            modifiers,
            annotations,
            name,
            body,
        }),
        JaSyncMethodBody::Meta(_) => None,
    }
}

pub(crate) fn match_java_sync_block_nested(
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let block = ja_sync_block_nested_template(pattern)?;
    let tree = parse_source(Language::Java, source).ok()?;
    let mut out = Vec::new();
    walk_java_sync_block_nested(tree.root_node(), source, pattern, &block, &mut out);
    Some(out)
}

pub(crate) fn walk_java_sync_block_nested(
    node: Node,
    source: &str,
    pattern: &str,
    block: &JaSyncMetaBlock,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "synchronized_statement" && node.is_named() && !is_in_comment_or_string(&node)
    {
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
        }
        if java_sync_block_bind(&node, source, block, &mut captures).is_some() {
            push_match_with_captures(&node, source, pattern, captures, out);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java_sync_block_nested(child, source, pattern, block, out);
    }
}

pub(crate) fn match_java_sync_method(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = ja_sync_method_template(pattern)?;
    let tree = parse_source(Language::Java, source).ok()?;
    let mut out = Vec::new();
    walk_kind(
        tree.root_node(),
        "method_declaration",
        true,
        &mut out,
        |n| java_sync_method_match(n, source, pattern, &template),
    );
    Some(out)
}

pub(crate) fn java_sync_block_bind(
    node: &Node,
    source: &str,
    block: &JaSyncMetaBlock,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let resource = children
        .iter()
        .copied()
        .find(|c| c.kind() == "parenthesized_expression")?;
    let resource_inner = resource.named_child(0)?;
    let resource_text = node_text(&resource_inner, source)?;
    match &block.resource {
        Ok(meta) => meta.bind(captures, resource_text.trim())?,
        Err(literal) => {
            if resource_text.trim() != literal {
                return None;
            }
        }
    }
    let body = children.iter().copied().find(|c| c.kind() == "block")?;
    let mut body_cursor = body.walk();
    let stmts: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    match &block.body {
        JaSyncBlockBody::Meta(meta) => {
            let text = node_text(only, source)?;
            meta.bind(captures, text)?;
        }
        JaSyncBlockBody::Nested(nested) => {
            if only.kind() != "synchronized_statement" {
                return None;
            }
            java_sync_block_bind(only, source, nested, captures)?;
        }
    }
    Some(())
}

pub(crate) fn java_sync_method_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JaSyncMethodTemplate,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    // The candidate's modifier tokens: `Some(text)` for keyword modifiers,
    // `None` for annotations (which BLOCK the hop-scan's passage). A
    // modifier-less candidate may carry NO modifiers node at all — the
    // scan then refuses any pattern modifier and passes a modifier-less
    // pattern.
    let modifiers_node = children.iter().copied().find(|c| c.kind() == "modifiers");
    let mut tokens: Vec<Option<String>> = Vec::new();
    let mut candidate_annotations: Vec<String> = Vec::new();
    if let Some(modifiers_node) = modifiers_node {
        let mut modifier_cursor = modifiers_node.walk();
        for child in modifiers_node.children(&mut modifier_cursor) {
            if child.is_named() {
                if child.kind().contains("annotation") {
                    tokens.push(None);
                    if let Some(text) = node_text(&child, source) {
                        candidate_annotations.push(text.to_string());
                    }
                }
            } else {
                tokens.push(node_text(&child, source).map(str::to_string));
            }
        }
    }
    // The pattern's leading annotation run must match a PREFIX of the
    // candidate's annotation run, in order (an absent candidate annotation
    // refuses). The consumed prefix is then skipped; any annotation BEYOND
    // the demanded prefix still blocks the modifier hop (the standing law).
    if !template.annotations.is_empty() {
        if candidate_annotations.len() < template.annotations.len() {
            return None;
        }
        for (want, got) in template
            .annotations
            .iter()
            .zip(candidate_annotations.iter())
        {
            if want.trim() != got.trim() {
                return None;
            }
        }
        tokens = tokens
            .into_iter()
            .skip(template.annotations.len())
            .collect();
    }
    let mut cursor_index = 0usize;
    for want in &template.modifiers {
        loop {
            let token = tokens.get(cursor_index)?;
            match token {
                // An annotation must not sit between pattern modifiers.
                None => return None,
                Some(text) if text == want => {
                    cursor_index += 1;
                    break;
                }
                Some(_) => cursor_index += 1,
            }
        }
    }
    let name_node = node.child_by_field_name("name")?;
    let name_text = node_text(&name_node, source)?;
    template.name.bind(&mut captures, name_text.trim())?;
    // The probed method shape carries an EMPTY parameter list.
    let parameters = node.child_by_field_name("parameters")?;
    if parameters.named_child_count() != 0 {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let mut body_cursor = body.walk();
    let stmts: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    match &template.body {
        JaSyncMethodBody::Meta(meta) => {
            let text = node_text(only, source)?;
            meta.bind(&mut captures, text)?;
        }
        JaSyncMethodBody::SyncBlock(block) => {
            if only.kind() != "synchronized_statement" {
                return None;
            }
            java_sync_block_bind(only, source, block, &mut captures)?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

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
                let tree = parse_source(lang, source).ok()?;
                let mut out = Vec::new();
                walk_kind(tree.root_node(), "type_alias", true, &mut out, |n| {
                    kt_typealias_match(n, source, pattern, &template)
                });
                return Some(out);
            }
            let template = kt_for_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_kind(tree.root_node(), "for_statement", true, &mut out, |n| {
                kt_for_match(n, source, pattern, &template)
            });
            Some(out)
        }
        Language::Swift => {
            let template = swift_for_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_kind(tree.root_node(), "for_statement", true, &mut out, |n| {
                swift_for_match(n, source, pattern, &template)
            });
            Some(out)
        }
        Language::Rust => {
            let template = rs_let_else_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_kind(tree.root_node(), "let_declaration", true, &mut out, |n| {
                rs_let_else_match(n, source, pattern, &template)
            });
            Some(out)
        }
        Language::C => {
            let template = c_goto_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
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
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_go_goto_bare(tree.root_node(), source, pattern, &template, &mut out);
            Some(out)
        }
        _ => None,
    }
}
