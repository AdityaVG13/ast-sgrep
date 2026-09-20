//! Java synchronized and PHP namespace block lanes.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

// ===========================================================================
// Dedicated statement-head lanes for the java `synchronized (R) { $B }`
// META-body face and the php `namespace [N] { $B }` BLOCK face — spellings
// whose bare-meta body the general template cannot substitute (a
// placeholder alone inside a block is a parse ERROR).
//
//   * java `synchronized ($X) { $B }`: binds the synchronized BLOCK — `$X`
//     = the resource's inner text, `$B` = the single body statement text;
//     0- or 2+-statement bodies refuse; CONCRETE-body spellings keep
//     their general-lane route — this lane admits ONLY the bare-meta body.
//   * php `namespace $N { $B }` / `namespace { $B }`: `$N` = the namespace
//     name (`App`; the GLOBAL form binds no name and refuses a named
//     candidate), `$B` = the single member text; 0- or 2+-member bodies
//     refuse. The `namespace $N;` STATEMENT form keeps its general lane.
// ===========================================================================

/// The JAVA class member-count classifier — the shared [`classify_native`]
/// refuses `class` Exactly bodies (the bare-colon empty-suite contract
/// rides that refusal for every language), so the java-scoped face
/// classifies here and rides the SAME Class member-count machinery through
/// a constructed kind (run_queries: the hopping scan, heritage/trivia
/// refusals, and the member-count body filter). Law: `class $N { $B }`
/// binds the single-member java class, refuses empty/multi-member bodies
/// and heritage clauses.
pub(crate) fn classify_java_class_member_count(pattern: &str) -> Option<NativeKind> {
    let p = pattern.trim();
    let (declaration, _) = strip_declaration_modifiers(p);
    let rest = declaration.strip_prefix("class ")?;
    let head = rest
        .split(|c: char| c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim();
    if head.is_empty() {
        return None;
    }
    let name = match head.strip_prefix('$') {
        // `$$`/`$$$`-prefixed heads answer like the reference — admit with no
        // direct name bind; the generic declaration-head capture path binds
        // it.
        Some(_) if capture_name(head).is_some() => None,
        Some(_) => return None,
        None if is_pattern_ident(head) => Some(head.to_string()),
        None => return None,
    };
    let tail = rest[head.len()..].trim();
    let body = parse_body_template(tail)?;
    if !matches!(body, Some(BodyTemplate::Exactly(_))) {
        return None;
    }
    Some(NativeKind::Class {
        keyword: "class",
        name,
        body,
    })
}

/// The java synchronized META-body template: `synchronized`, whitespace,
/// balanced parens (any non-empty resource section), balanced braces, a
/// BARE-meta body, nothing after. Concrete bodies stay on the general lane.
pub(crate) fn ja_synchronized_meta_template(pattern: &str) -> Option<String> {
    let p = pattern.trim();
    let rest = p.strip_prefix("synchronized")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    let resource = inner[..close].trim();
    let rest = inner[close + 1..].trim_start();
    let inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(inner)?;
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    if resource.is_empty() {
        return None;
    }
    capture_name(inner[..close].trim()).map(str::to_string)
}

pub(crate) fn match_java_synchronized_meta(
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let body_name = ja_synchronized_meta_template(pattern)?;
    let tree = parse_source(Language::Java, source).ok()?;
    let mut out = Vec::new();
    walk_java_synchronized_meta(tree.root_node(), source, pattern, &body_name, &mut out);
    Some(out)
}

pub(crate) fn walk_java_synchronized_meta(
    node: Node,
    source: &str,
    pattern: &str,
    body_name: &str,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "synchronized_statement" && node.is_named() && !is_in_comment_or_string(&node)
    {
        if let Some(hit) = java_synchronized_meta_match(&node, source, pattern, body_name) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java_synchronized_meta(child, source, pattern, body_name, out);
    }
}

pub(crate) fn java_synchronized_meta_match(
    node: &Node,
    source: &str,
    pattern: &str,
    body_name: &str,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    // The resource is the parenthesized expression child; the reference
    // binds its INNER text (X=`lock`).
    let resource = children
        .iter()
        .copied()
        .find(|c| c.kind() == "parenthesized_expression")?;
    let resource_inner = resource.named_child(0)?;
    let resource_text = node_text(&resource_inner, source)?;
    // The resource section of the PATTERN binds like the general lane: a
    // meta takes the inner text; any other spelling byte-matches.
    if let Some(meta) = ja_synchronized_resource_meta_name(pattern) {
        meta.bind(&mut captures, resource_text.trim())?;
    } else {
        let want = ja_synchronized_resource_literal_text(pattern)?;
        if resource_text.trim() != want {
            return None;
        }
    }
    // The body: the block child; `$B` binds a ONE-statement body's
    // statement text (0/2+-statement candidates refuse).
    let body = children.iter().copied().find(|c| c.kind() == "block")?;
    let mut body_cursor = body.walk();
    let stmts: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    let text = node_text(only, source)?;
    bind_capture(&mut captures, body_name, text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// The resource section between the balanced parens of a synchronized
/// META-body pattern.
pub(crate) fn ja_synchronized_resource_section(pattern: &str) -> Option<&str> {
    let p = pattern.trim();
    let rest = p.strip_prefix("synchronized")?;
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    Some(inner[..close].trim())
}

pub(crate) fn ja_synchronized_resource_meta_name(pattern: &str) -> Option<LaneMeta> {
    // The resource meta keeps its namespace — `$$X` binds single, `$$$X`
    // binds the MULTI namespace.
    ja_synchronized_resource_section(pattern).and_then(lane_meta)
}

pub(crate) fn ja_synchronized_resource_literal_text(pattern: &str) -> Option<String> {
    let section = ja_synchronized_resource_section(pattern)?;
    if section.is_empty() || capture_name(section).is_some() {
        return None;
    }
    Some(section.to_string())
}

/// The php braced-namespace template: `namespace [NAME] { $B }` — the name
/// is a bare canonical meta (any namespace: `$$N` single, `$$$N` multi),
/// a literal name byte-matched, or ABSENT (the global block form); the body
/// is any canonical meta — `$$B` follows the ONE-member `$B` law and `$$$B`
/// binds the MULTI list at any member count. The `namespace $N;` statement
/// form keeps its own route.
pub(crate) struct PhpNamespaceBlockTemplate {
    name: Option<LaneName>,
    body: PhpNamespaceBody,
}

/// The body slot's MIXED exact-order face — literal prefix statements
/// followed by ONE trailing single-member meta (`const A = 1; $$B`). The
/// reference BINDS the exact-order face and binds nothing on the
/// mixed-ORDER, zero-trailing, two-trailing, and prefix-mismatch faces.
#[derive(Debug, Clone)]
pub(crate) enum PhpNamespaceBody {
    Meta(LaneMeta),
    Mixed { prefix: String, tail: LaneMeta },
}

/// Split `const A = 1; $$B` into the literal prefix statement run and the
/// ONE trailing single-member meta (last whitespace-delimited word). The
/// `$$$B` multi tail keeps its uncovered route and a prefix not ending in
/// `;` is not a statement run.
pub(crate) fn php_mixed_namespace_body(section: &str) -> Option<(String, LaneMeta)> {
    let s = section.trim();
    let tail_start = s.rfind(char::is_whitespace)? + 1;
    let tail = &s[tail_start..];
    if tail.is_empty() {
        return None;
    }
    let meta = lane_meta(tail)?;
    if meta.multi {
        return None;
    }
    let prefix = s[..tail_start].trim_end();
    if prefix.is_empty() || !prefix.ends_with(';') {
        return None;
    }
    Some((prefix.to_string(), meta))
}

pub(crate) fn php_namespace_block_template(pattern: &str) -> Option<PhpNamespaceBlockTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("namespace")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let (name, brace_section) = if let Some(inner) = rest.strip_prefix('{') {
        (None, inner)
    } else {
        let brace_at = rest.find('{')?;
        let name_section = rest[..brace_at].trim();
        if name_section.is_empty() {
            return None;
        }
        let name = if let Some(meta) = lane_meta(name_section) {
            LaneName::Meta(meta)
        } else if !name_section.contains('$')
            && !name_section.starts_with(|c: char| c.is_ascii_digit())
            && name_section
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\\')
        {
            LaneName::Literal(name_section.to_string())
        } else {
            return None;
        };
        (Some(name), &rest[brace_at + 1..])
    };
    let close = balanced_brace_close(brace_section)?;
    if !brace_section[close + 1..].trim().is_empty() {
        return None;
    }
    let body_section = brace_section[..close].trim();
    // The `$$`-body RefusedEmpty reading is REFUTED — the reference BINDS
    // `$$B` under the ONE-member `$B` law (any single member) and `$$$B`
    // under the MULTI list law (ANY member count, 0 → the empty list;
    // refuse only at ≥2 for the single namespace). The parse therefore
    // treats every canonical meta body alike; the member-count law lives
    // in the walk. The MIXED exact-order face joins the slot (see
    // [`PhpNamespaceBody`]).
    let body = if let Some(meta) = lane_meta(body_section) {
        PhpNamespaceBody::Meta(meta)
    } else if let Some((prefix, tail)) = php_mixed_namespace_body(body_section) {
        PhpNamespaceBody::Mixed { prefix, tail }
    } else {
        return None;
    };
    Some(PhpNamespaceBlockTemplate { name, body })
}

pub(crate) fn match_php_namespace_block(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = php_namespace_block_template(pattern)?;
    let tree = parse_source(Language::Php, source).ok()?;
    let mut out = Vec::new();
    walk_php_namespace_block(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

pub(crate) fn walk_php_namespace_block(
    node: Node,
    source: &str,
    pattern: &str,
    template: &PhpNamespaceBlockTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "namespace_definition" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = php_namespace_block_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_namespace_block(child, source, pattern, template, out);
    }
}

pub(crate) fn php_namespace_block_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &PhpNamespaceBlockTemplate,
) -> Option<PatternMatch> {
    let name_node = node.child_by_field_name("name");
    match (&template.name, &name_node) {
        // The global pattern refuses a named candidate and vice versa
        // (`namespace { $B }` × `namespace App { … }` answers empty).
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(want), Some(name_node)) = (&template.name, name_node) {
        let name_text = node_text(&name_node, source)?;
        want.bind_or_match(&mut captures, name_text.trim())?;
    }
    // The body: the compound_statement child. The member-count law follows
    // the meta's namespace — `$B`/`$$B` (single) bind a ONE-member body's
    // member text (0/2+-member candidates refuse); `$$$B` (multi) binds EVERY
    // member at ANY count, 0 members → the empty list.
    let body = node.child_by_field_name("body")?;
    let mut body_cursor = body.walk();
    let members: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let body_meta = match (&template.body, &members[..]) {
        (PhpNamespaceBody::Meta(body_meta), _) => body_meta,
        (PhpNamespaceBody::Mixed { prefix, tail }, members) => {
            // The candidate's members must be the prefix statement run EXACTLY
            // (statement-wise, whitespace-normalized, comment-stripped)
            // followed by exactly ONE trailing member — the mixed-ORDER,
            // zero-trailing, two-trailing, and prefix-mismatch faces all
            // refuse.
            let doc = format!("<?php\nnamespace __Px {{ {prefix} }}\n");
            let tpl_tree = parse_source(Language::Php, &doc).ok()?;
            if tpl_tree.root_node().has_error() {
                return None;
            }
            let mut root_cursor = tpl_tree.root_node().walk();
            let prefix_members: Vec<Node> = tpl_tree
                .root_node()
                .children(&mut root_cursor)
                .find(|n| n.kind() == "namespace_definition")
                .and_then(|ns| ns.child_by_field_name("body"))
                .map(|body| {
                    let mut c = body.walk();
                    body.children(&mut c)
                        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
                        .collect()
                })
                .unwrap_or_default();
            if prefix_members.is_empty() || members.len() != prefix_members.len() + 1 {
                return None;
            }
            let normalize = |node: &Node, src: &str| -> Option<String> {
                Some(
                    strip_comment_spans(node_text(node, src)?)
                        .split_whitespace()
                        .collect(),
                )
            };
            for (p_stmt, c_stmt) in prefix_members.iter().zip(members.iter()) {
                if normalize(p_stmt, &doc) != normalize(c_stmt, source) {
                    return None;
                }
            }
            let tail_text = node_text(&members[members.len() - 1], source)?.trim();
            tail.bind(&mut captures, tail_text)?;
            return Some(hit_for_node(node, source, pattern, captures));
        }
    };
    if body_meta.multi {
        // The subject's capture map is text-valued: the MULTI encoding of
        // record joins the member texts with '\n' (the reference emits a
        // JSON array — an encoding difference of record; count/span parity
        // holds).
        let mut texts = Vec::new();
        for member in &members {
            texts.push(node_text(member, source)?.trim().to_string());
        }
        body_meta.bind(&mut captures, &texts.join("\n"))?;
    } else {
        let [only] = members.as_slice() else {
            return None;
        };
        let text = node_text(only, source)?;
        body_meta.bind(&mut captures, text)?;
    }
    Some(hit_for_node(node, source, pattern, captures))
}
