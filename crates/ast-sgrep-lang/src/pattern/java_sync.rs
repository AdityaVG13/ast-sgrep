//! Java `synchronized` nested-block + METHOD faces (D cluster, split from statements.rs).
//!
//! The 139 bare-meta lane keeps the flat faces (D_sync_plain_ctl AGREE n2);
//! the lanes here take what it refuses.

use super::*;
use crate::extract::node_text;
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

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
    return_type: String,
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
    let (name, rest) = ja_name_then_parens(rest)?;
    let body = ja_method_body(rest)?;
    if head_sync {
        // `synchronized`-carrying METHOD face:
        // `[@Anno…] [modifiers] ret $M() { body }`.
        return Some(JaSyncMethodTemplate {
            modifiers,
            annotations,
            return_type: ret.to_string(),
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
            return_type: ret.to_string(),
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
    let tree = parse_source_silent(Language::Java, source)?;
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
    walk_kind(node, "synchronized_statement", true, out, |n| {
        java_sync_block_nested_match(n, source, pattern, block)
    });
}

/// Single-candidate check for [`walk_java_sync_block_nested`].
fn java_sync_block_nested_match(
    node: &Node,
    source: &str,
    pattern: &str,
    block: &JaSyncMetaBlock,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    java_sync_block_bind(node, source, block, &mut captures)?;
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) fn match_java_sync_method(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = ja_sync_method_template(pattern)?;
    let tree = parse_source_silent(Language::Java, source)?;
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
    let mut resource_cursor = resource.walk();
    let resource_inner = resource
        .named_children(&mut resource_cursor)
        .find(|child| !is_trivia_kind(child.kind()))?;
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
    let return_type = node.child_by_field_name("type")?;
    if node_text(&return_type, source)?.trim() != template.return_type {
        return None;
    }
    let name_node = node.child_by_field_name("name")?;
    let name_text = node_text(&name_node, source)?;
    template.name.bind(&mut captures, name_text.trim())?;
    // The probed method shape carries an EMPTY parameter list.
    let parameters = node.child_by_field_name("parameters")?;
    let mut parameter_cursor = parameters.walk();
    if parameters
        .named_children(&mut parameter_cursor)
        .any(|child| !is_trivia_kind(child.kind()))
    {
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
