//! S8 declaration-lane bind/match fns (isomorphic split of roots142.rs).

use super::*;
use crate::extract::node_text;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Shared slot binding: a name field (LaneName) and an optional second
/// field (LaneName against the field node's text).
pub(crate) fn root142_field_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_field: &str,
    second_field: Option<&str>,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let (name, second) = match template {
        Root142Template::RsExternCrate { name } => (name, None),
        Root142Template::GoType { name, ty } => (name, Some(ty)),
        _ => return None,
    };
    let _ = second_field;
    // Modifier doctrine: a visibility/modifier child the pattern lacks
    // breaks the reference child alignment.
    if has_child_kind(node, "visibility_modifier") || has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name(name_field)?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    if let Some(ty) = second {
        let ty_node = node.child_by_field_name("type")?;
        ty.bind_or_match(&mut captures, node_text(&ty_node, source)?.trim())?;
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// `module <Name>\n  $B\nend` — the name field binds N; the body field
/// (the `body_statement`) binds B as the WHOLE trimmed middle (E3/E3b).
pub(crate) fn root142_rb_module_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::RbModule { name, body } = template else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    body.bind(&mut captures, node_text(&body_node, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// The `{ $B }` block law: the body field's named non-trivia children are
/// SINGLE-EXACT — exactly one member binds its text (E2/K1/K3); 0/≥2
/// refuse. The modifier doctrine applies to the enclosing node.
pub(crate) fn root142_block_member_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_field: &str,
    body_field: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::RsMod { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "visibility_modifier") || has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name(name_field)?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name(body_field)?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `declare module <name> { $B }` — the ambient_declaration wraps a module
/// node (name field carries the QUOTES, E10); the statement block is
/// single-statement-exact (E10b).
pub(crate) fn root142_ts_module_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::TsDeclareModule { name, body } = template else {
        return None;
    };
    let module_node = node.named_child(0)?;
    // `declare module "x"` / `declare module x` / `namespace` — the
    // ambient child is the ts `module` or `internal_module` node (both
    // carry name!/body? fields).
    if module_node.kind() != "module" && module_node.kind() != "internal_module" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = module_node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = module_node.child_by_field_name("body")?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `declare const <X>: <T>;` — the lexical_declaration's kind token must be
/// `const` (H23); X = the declarator name, T = the annotation text minus
/// the `:` (E11: T='number').
pub(crate) fn root142_ts_const_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::TsDeclareConst { name, ty } = template else {
        return None;
    };
    let decl = node.named_child(0)?;
    if decl.kind() != "lexical_declaration" {
        return None;
    }
    let mut cursor = decl.walk();
    let kind_tok = decl
        .child_by_field_name("kind")
        .or_else(|| decl.children(&mut cursor).find(|c| !c.is_named()))?;
    if node_text(&kind_tok, source)?.trim() != "const" {
        return None;
    }
    let declarators = named_non_trivia_children(&decl);
    let declarator = declarators.first()?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = declarator.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let ty_node = declarator.child_by_field_name("type")?;
    let ty_text = node_text(&ty_node, source)?.trim();
    let ty_text = ty_text.strip_prefix(':').unwrap_or(ty_text).trim();
    ty.bind_or_match(&mut captures, ty_text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `async def <N>($$P):\n    $$B` — the function_definition must carry the
/// `async` token (sync faces keep their existing routes); P is
/// PARAM-EXACT; B = the whole body block text trimmed (E8b).
pub(crate) fn root142_py_async_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::PyAsyncDef { name, param, body } = template else {
        return None;
    };
    let mut cursor = node.walk();
    let is_async = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && node_text(&c, source).is_some_and(|t| t == "async"));
    if !is_async {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let params_node = node.child_by_field_name("parameters")?;
    let params = named_non_trivia_children(&params_node);
    let [only] = params.as_slice() else {
        return None;
    };
    param.bind(&mut captures, node_text(only, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    body.bind(&mut captures, node_text(&body_node, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `enum <name> { <body> }` — members are the enum_constant children plus
/// the declarations inside `enum_body_declarations`; the body slot is
/// SINGLE-EXACT (K3/G3), the literal slot byte-matches (H3), and the EMPTY
/// slot binds 0-member candidates (K4). Modifier candidates refuse (G3).
pub(crate) fn root142_ja_enum_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::JaEnum { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifiers") {
        return None;
    }
    let body_node = node.child_by_field_name("body")?;
    let mut members: Vec<Node> = named_non_trivia_children(&body_node)
        .into_iter()
        .flat_map(|child| {
            if child.kind() == "enum_body_declarations" {
                named_non_trivia_children(&child)
            } else {
                vec![child]
            }
        })
        .collect();
    if matches!(body, JaEnumBody::Meta(_)) {
        // The `{`/`}` around an EMPTY member list can surface as an
        // anonymous-token-only body; nothing to filter — 0 members is 0.
        members.retain(|m| m.kind() != "enum_body_declarations");
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    match body {
        JaEnumBody::Empty => {
            if !members.is_empty() {
                return None;
            }
        }
        JaEnumBody::Meta(meta) => {
            let [only] = members.as_slice() else {
                return None;
            };
            meta.bind(&mut captures, node_text(only, source)?.trim())?;
        }
        JaEnumBody::Literal(literal) => {
            let [only] = members.as_slice() else {
                return None;
            };
            if node_text(only, source)?.trim() != literal {
                return None;
            }
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// `record <name>(<P>);[ { $B }]` — the parameter slot is PARAM-EXACT
/// (K2/K7), the optional body single-member-exact (J6); modifier
/// candidates refuse (G4).
pub(crate) fn root142_cs_record_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::CsRecord { name, param, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    // NOTE: the cs grammar gives record_declaration NO parameter_list
    // FIELD (only name/body are fields) — the list is a fieldless child.
    let params_node = named_non_trivia_children(node)
        .into_iter()
        .find(|c| c.kind() == "parameter_list")?;
    let params: Vec<Node> = named_non_trivia_children(&params_node)
        .into_iter()
        .filter(|c| c.kind() == "parameter")
        .collect();
    let [only] = params.as_slice() else {
        return None;
    };
    param.bind(&mut captures, node_text(only, source)?.trim())?;
    let body_node = node.child_by_field_name("body");
    match (body, body_node) {
        (None, None) => {}
        (None, Some(_)) => return None,
        (Some(_), None) => return None,
        (Some(slot), Some(body_node)) => {
            let members = named_non_trivia_children(&body_node);
            let [only] = members.as_slice() else {
                return None;
            };
            slot.bind(&mut captures, node_text(only, source)?.trim())?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// `struct <name> { $B }` — the body is SINGLE- MEMBER-EXACT (K1); empty
/// bodies refuse (J9); modifier candidates refuse (G6).
pub(crate) fn root142_cs_struct_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::CsStruct { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}
