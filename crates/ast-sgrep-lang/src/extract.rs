use crate::{CallSite, ExtractionResult, ImportSite, Language, SymbolDef, SymbolKind};
use std::cell::RefCell;
use std::collections::HashMap;
use tree_sitter::{Node, Parser, Tree};

thread_local! {
    /// One tree-sitter Parser per language (creating parsers is relatively expensive).
    static TS_PARSERS: RefCell<HashMap<Language, Parser>> = RefCell::new(HashMap::new());
}

/// Parse `source` and run `extract` on the tree. When `lang_key` is set, reuses a thread-local parser for that language.
pub fn parse_and_extract_for(
    lang_key: Option<Language>,
    language: tree_sitter::Language,
    source: &str,
    extract: impl FnOnce(&Tree, &str) -> ExtractionResult,
) -> anyhow::Result<ExtractionResult> {
    let tree = parse_tree(lang_key, language, source)?;
    let mut result = extract(&tree, source);
    if result.pattern_nodes.is_empty() {
        result.pattern_nodes = crate::pattern::collect_pattern_nodes(tree.root_node(), source);
    }
    Ok(result)
}

fn parse_tree(
    lang_key: Option<Language>,
    language: tree_sitter::Language,
    source: &str,
) -> anyhow::Result<Tree> {
    if let Some(key) = lang_key {
        return TS_PARSERS.with(|cell| {
            let mut map = cell.borrow_mut();
            use std::collections::hash_map::Entry;
            let parser = match map.entry(key) {
                Entry::Occupied(o) => o.into_mut(),
                Entry::Vacant(v) => {
                    let mut p = Parser::new();
                    p.set_language(&language)
                        .map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
                    v.insert(p)
                }
            };
            parser
                .parse(source, None)
                .ok_or_else(|| anyhow::anyhow!("failed to parse source"))
        });
    }
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
    parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse source"))
}
pub fn byte_to_line(source: &str, byte: usize) -> u32 {
    source[..byte.min(source.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count() as u32
        + 1
}
pub fn node_lines(node: &Node, source: &str) -> (u32, u32) {
    (
        byte_to_line(source, node.start_byte()),
        byte_to_line(source, node.end_byte()),
    )
}
pub fn node_text<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    source.get(node.start_byte()..node.end_byte())
}
pub fn last_identifier_in_chain(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" | "property_identifier" => {
            node_text(node, source).map(str::to_string)
        }
        "field_expression"
        | "scoped_identifier"
        | "scoped_type_identifier"
        | "member_expression"
        | "member_access_expression"
        | "selector_expression" => {
            let mut cursor = node.walk();
            let mut last = None;
            for child in node.children(&mut cursor) {
                if let Some(name) = last_identifier_in_chain(&child, source) {
                    last = Some(name);
                }
            }
            last
        }
        _ => {
            let mut cursor = node.walk();
            let mut found = None;
            for c in node.children(&mut cursor) {
                if let Some(name) = last_identifier_in_chain(&c, source) {
                    found = Some(name);
                    break;
                }
            }
            found
        }
    }
}
pub fn is_in_comment_or_string(node: &Node) -> bool {
    let mut current = Some(*node);
    while let Some(n) = current {
        if matches!(
            n.kind(),
            "comment"
                | "line_comment"
                | "block_comment"
                | "string_literal"
                | "raw_string_literal"
                | "string"
                | "template_string"
                | "interpreted_string_literal"
                | "quoted_string_literal"
        ) {
            return true;
        }
        current = n.parent();
    }
    false
}
/// True if any ancestor node has a kind in `kinds`.
pub fn is_inside_any(node: &Node, kinds: &[&str]) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if kinds.iter().any(|&k| n.kind() == k) {
            return true;
        }
        current = n.parent();
    }
    false
}

pub fn add_named_symbol(ext: &mut Extractor, node: &Node, source: &str, kind: SymbolKind) {
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Some(name) = node_text(&name_node, source) {
            ext.add_symbol(node, source, name, kind);
        }
    }
}

pub fn trim_string_literal(raw: &str) -> &str {
    raw.trim().trim_matches(|c| matches!(c, '"' | '\'' | '`'))
}

/// Table-driven extraction action for a tree-sitter node kind.
///
/// Prefer positional variants so language kind-maps stay compact and scannable.
#[derive(Clone, Copy)]
pub enum KindRule {
    /// Named symbol with fixed kind.
    Sym(SymbolKind),
    /// Named symbol: Method if inside any of these parents, else Function.
    MethodIn(&'static [&'static str]),
    /// Symbol kind from `child_by_field_name(field).kind()` cases, else `default`.
    SymByField(
        &'static str,
        &'static [(&'static str, SymbolKind)],
        SymbolKind,
    ),
    /// For each direct child of `child_kind`, add a named symbol of `sym`.
    SymChildren(&'static str, SymbolKind),
    /// If parent kind matches, add named symbol on the parent (arrow → declarator).
    SymParent(&'static str, SymbolKind),
    /// Call site; callee from child field name.
    Call(&'static str),
    /// Call via field; if callee text ∈ import_names, import string under args field.
    CallOrImport(&'static str, &'static [&'static str], &'static str),
    /// Import = identifiers under node joined by separator.
    ImportJoin(&'static str),
    /// Import = quoted string from child field.
    ImportQuoted(&'static str),
    /// Import = quoted string from field, else first child of fallback kind.
    ImportQuotedOrChild(&'static str, &'static str),
    /// Import path: (name_kinds, skip_words, recursive, optional id-join fallback).
    ImportPath(
        &'static [&'static str],
        &'static [&'static str],
        bool,
        Option<&'static str>,
    ),
}

/// Apply the first matching kind rule. Returns true if a rule fired.
pub fn apply_kind_table(
    ext: &mut Extractor,
    node: &Node,
    source: &str,
    table: &[(&str, KindRule)],
) -> bool {
    let kind = node.kind();
    for &(k, rule) in table {
        if k != kind {
            continue;
        }
        apply_kind_rule(ext, node, source, rule);
        return true;
    }
    false
}

fn apply_kind_rule(ext: &mut Extractor, node: &Node, source: &str, rule: KindRule) {
    match rule {
        KindRule::Sym(sk) => add_named_symbol(ext, node, source, sk),
        KindRule::MethodIn(parents) => {
            let sk = if is_inside_any(node, parents) {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            add_named_symbol(ext, node, source, sk);
        }
        KindRule::SymByField(field, cases, default) => {
            let sk = field_child(node, field)
                .and_then(|t| {
                    let tk = t.kind();
                    cases.iter().find(|(k, _)| *k == tk).map(|(_, sk)| *sk)
                })
                .unwrap_or(default);
            add_named_symbol(ext, node, source, sk);
        }
        KindRule::SymChildren(child_kind, sk) => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == child_kind {
                    add_named_symbol(ext, &child, source, sk);
                }
            }
        }
        KindRule::SymParent(parent_kind, sk) => {
            if let Some(parent) = node.parent() {
                if parent.kind() == parent_kind {
                    add_named_symbol(ext, &parent, source, sk);
                }
            }
        }
        KindRule::Call(field) => {
            if let Some(func) = field_child(node, field) {
                ext.add_call(node, source, &func);
            }
        }
        KindRule::CallOrImport(callee_field, import_names, args_field) => {
            let Some(method) = field_child(node, callee_field) else {
                return;
            };
            let Some(name) = node_text(&method, source) else {
                return;
            };
            if import_names.contains(&name) {
                if let Some(args) = field_child(node, args_field) {
                    if let Some(path) = first_string_literal(&args, source) {
                        ext.add_import(node, source, &path);
                    }
                }
            } else {
                ext.add_call(node, source, &method);
            }
        }
        KindRule::ImportJoin(sep) => {
            let ids = collect_identifiers(node, source);
            if !ids.is_empty() {
                ext.add_import(node, source, &ids.join(sep));
            }
        }
        KindRule::ImportQuoted(field) => {
            if let Some(path_node) = field_child(node, field) {
                if let Some(path) = node_text(&path_node, source) {
                    ext.add_import(node, source, trim_string_literal(path));
                }
            }
        }
        KindRule::ImportQuotedOrChild(field, fallback_kind) => {
            if let Some(path_node) = field_child(node, field) {
                if let Some(path) = node_text(&path_node, source) {
                    ext.add_import(node, source, trim_string_literal(path));
                    return;
                }
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == fallback_kind {
                    if let Some(path) = node_text(&child, source) {
                        ext.add_import(node, source, trim_string_literal(path));
                    }
                }
            }
        }
        KindRule::ImportPath(name_kinds, skip, recursive, id_join) => {
            if let Some(path) = path_from_name_children(node, source, name_kinds, skip, recursive) {
                ext.add_import(node, source, &path);
                return;
            }
            if let Some(sep) = id_join {
                let ids: Vec<String> = collect_identifiers(node, source)
                    .into_iter()
                    .filter(|s| !skip.contains(&s.as_str()))
                    .collect();
                if !ids.is_empty() {
                    ext.add_import(node, source, &ids.join(sep));
                }
            }
        }
    }
}

/// First direct (or recursive) child whose kind is in `name_kinds` and text is not in `skip`.
pub fn path_from_name_children(
    node: &Node,
    source: &str,
    name_kinds: &[&str],
    skip: &[&str],
    recursive: bool,
) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if name_kinds.iter().any(|&k| child.kind() == k) {
            if let Some(text) = node_text(&child, source) {
                if !skip.contains(&text) {
                    return Some(text.to_string());
                }
            }
        }
        if recursive {
            if let Some(path) = path_from_name_children(&child, source, name_kinds, skip, true) {
                return Some(path);
            }
        }
    }
    None
}

const STRING_KINDS: &[&str] = &[
    "string",
    "string_content",
    "interpreted_string_literal",
    "bare_string_literal",
];

/// First string-like descendant (quoted literals / string content nodes).
pub fn first_string_literal(node: &Node, source: &str) -> Option<String> {
    if STRING_KINDS.iter().any(|&k| node.kind() == k) {
        return node_text(node, source).map(|raw| trim_string_literal(raw).to_string());
    }
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        if let Some(s) = first_string_literal(&c, source) {
            return Some(s);
        }
    }
    None
}
pub fn enclosing_symbol_name(node: &Node, source: &str) -> Option<String> {
    let mut current = node.parent();
    while let Some(n) = current {
        match n.kind() {
            "function_item"
            | "function_declaration"
            | "function_definition"
            | "method_declaration"
            | "method_definition"
            | "method" => {
                if let Some(name_node) = n.child_by_field_name("name") {
                    return node_text(&name_node, source).map(str::to_string);
                }
            }
            "arrow_function" | "function_expression" => {
                if let Some(name_node) = n.child_by_field_name("name") {
                    return node_text(&name_node, source).map(str::to_string);
                }
                if let Some(parent) = n.parent() {
                    if parent.kind() == "variable_declarator" {
                        if let Some(name_node) = parent.child_by_field_name("name") {
                            return node_text(&name_node, source).map(str::to_string);
                        }
                    }
                }
            }
            _ => {}
        }
        current = n.parent();
    }
    None
}
pub struct Extractor {
    pub symbols: Vec<SymbolDef>,
    pub calls: Vec<CallSite>,
    pub imports: Vec<ImportSite>,
}
impl Extractor {
    pub fn new() -> Self {
        Self {
            symbols: vec![],
            calls: vec![],
            imports: vec![],
        }
    }

    pub fn into_result(self) -> ExtractionResult {
        ExtractionResult {
            symbols: self.symbols,
            calls: self.calls,
            imports: self.imports,
            pattern_nodes: vec![],
        }
    }

    pub fn add_symbol(&mut self, node: &Node, source: &str, name: &str, kind: SymbolKind) {
        let (line_start, line_end) = node_lines(node, source);
        self.symbols.push(SymbolDef {
            name: name.to_string(),
            kind,
            line_start,
            line_end,
            byte_start: node.start_byte(),
            byte_end: node.end_byte(),
        });
    }

    pub fn add_call(&mut self, node: &Node, source: &str, callee_node: &Node) {
        if is_in_comment_or_string(node) {
            return;
        }
        let Some(callee) = last_identifier_in_chain(callee_node, source) else {
            return;
        };
        self.calls.push(CallSite {
            caller: enclosing_symbol_name(node, source).unwrap_or_else(|| "<module>".into()),
            callee,
            line: byte_to_line(source, node.start_byte()),
            byte_start: node.start_byte(),
            byte_end: node.end_byte(),
        });
    }

    pub fn add_import(&mut self, node: &Node, source: &str, module: &str) {
        self.imports.push(ImportSite {
            module_path: module.to_string(),
            line: byte_to_line(source, node.start_byte()),
        });
    }
}
impl Default for Extractor {
    fn default() -> Self {
        Self::new()
    }
}
pub fn collect_identifiers(node: &Node, source: &str) -> Vec<String> {
    let mut ids = Vec::new();
    collect_identifiers_rec(node, source, &mut ids);
    ids
}
fn collect_identifiers_rec(node: &Node, source: &str, ids: &mut Vec<String>) {
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "property_identifier" | "package_identifier"
    ) {
        if let Some(text) = node_text(node, source) {
            ids.push(text.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !matches!(child.kind(), "comment" | "line_comment" | "block_comment") {
            collect_identifiers_rec(&child, source, ids);
        }
    }
}
pub fn field_child<'a>(node: &'a Node, name: &str) -> Option<Node<'a>> {
    node.child_by_field_name(name)
}
pub fn parse_ts_language_for(
    lang_key: Option<Language>,
    language: tree_sitter::Language,
    source: &str,
    mut on_node: impl FnMut(&mut Extractor, &Node, &str),
) -> anyhow::Result<ExtractionResult> {
    parse_and_extract_for(lang_key, language, source, |tree, src| {
        let mut extractor = Extractor::new();
        walk_mut(&mut extractor, &tree.root_node(), src, &mut on_node);
        extractor.into_result()
    })
}
fn walk_mut(
    ext: &mut Extractor,
    node: &Node,
    source: &str,
    on_node: &mut impl FnMut(&mut Extractor, &Node, &str),
) {
    on_node(ext, node, source);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_mut(ext, &child, source, on_node);
    }
}
