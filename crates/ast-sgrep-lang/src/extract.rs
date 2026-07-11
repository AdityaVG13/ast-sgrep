use tree_sitter::{Node, Parser, Tree};
use crate::{CallSite, ExtractionResult, ImportSite, SymbolDef, SymbolKind};
pub fn parse_and_extract(
    language: tree_sitter::Language,
    source: &str,
    extract: impl FnOnce(&Tree, &str) -> ExtractionResult,
) -> anyhow::Result<ExtractionResult> {
    let mut parser = Parser::new();
    parser.set_language(&language).map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
    let tree = parser.parse(source, None).ok_or_else(|| anyhow::anyhow!("failed to parse source"))?;
    let mut result = extract(&tree, source);
    result.pattern_nodes = crate::pattern::collect_pattern_nodes(tree.root_node(), source);
    Ok(result)
}
pub fn byte_to_line(source: &str, byte: usize) -> u32 {
    source[..byte.min(source.len())].bytes().filter(|&b| b == b'\n').count() as u32 + 1
}
pub fn node_lines(node: &Node, source: &str) -> (u32, u32) {
    (byte_to_line(source, node.start_byte()), byte_to_line(source, node.end_byte()))
}
pub fn node_text<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    source.get(node.start_byte()..node.end_byte())
}
pub fn last_identifier_in_chain(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" | "property_identifier" => {
            node_text(node, source).map(str::to_string)
        }
        "field_expression" | "scoped_identifier" | "scoped_type_identifier" | "member_expression"
        | "member_access_expression" | "selector_expression" => {
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
            "comment" | "line_comment" | "block_comment" | "string_literal" | "raw_string_literal"
                | "string" | "template_string" | "interpreted_string_literal" | "quoted_string_literal"
        ) { return true; }
        current = n.parent();
    }
    false
}
pub fn is_inside_kind(node: &Node, kind: &str) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == kind { return true; }
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
pub fn enclosing_symbol_name(node: &Node, source: &str) -> Option<String> {
    let mut current = node.parent();
    while let Some(n) = current {
        match n.kind() {
            "function_item" | "function_declaration" | "function_definition"
            | "method_declaration" | "method_definition" | "method" => {
                if let Some(name_node) = n.child_by_field_name("name") { return node_text(&name_node, source).map(str::to_string); }
            }
            "arrow_function" | "function_expression" => {
                if let Some(name_node) = n.child_by_field_name("name") { return node_text(&name_node, source).map(str::to_string); }
                if let Some(parent) = n.parent() {
                    if parent.kind() == "variable_declarator" {
                        if let Some(name_node) = parent.child_by_field_name("name") { return node_text(&name_node, source).map(str::to_string); }
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
        Self { symbols: vec![], calls: vec![], imports: vec![] }
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
    if matches!(node.kind(), "identifier" | "type_identifier" | "property_identifier" | "package_identifier") {
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
pub fn parse_ts_language(
    language: tree_sitter::Language,
    source: &str,
    mut on_node: impl FnMut(&mut Extractor, &Node, &str),
) -> anyhow::Result<ExtractionResult> {
    parse_and_extract(language, source, |tree, src| {
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
