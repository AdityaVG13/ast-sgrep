use tree_sitter::{Node, Parser, Tree};

use crate::{CallSite, ExtractionResult, ImportSite, SymbolDef, SymbolKind};

/// Parse source with the given tree-sitter language and run extraction.
pub fn parse_and_extract(
    language: tree_sitter::Language,
    source: &str,
    extract: impl FnOnce(&Tree, &str) -> ExtractionResult,
) -> anyhow::Result<ExtractionResult> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse source"))?;
    Ok(extract(&tree, source))
}

/// Convert byte offset to 1-based line number.
pub fn byte_to_line(source: &str, byte: usize) -> u32 {
    source[..byte.min(source.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count() as u32
        + 1
}

/// Get 1-based line range for a node.
pub fn node_lines(node: &Node, source: &str) -> (u32, u32) {
    (
        byte_to_line(source, node.start_byte()),
        byte_to_line(source, node.end_byte()),
    )
}

/// Extract text for a node from source.
pub fn node_text<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    let start = node.start_byte();
    let end = node.end_byte();
    source.get(start..end)
}

/// Get the last identifier in a field/scoped expression chain.
pub fn last_identifier_in_chain(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" | "property_identifier" => {
            node_text(node, source).map(|s| s.to_string())
        }
        "field_expression" | "scoped_identifier" | "scoped_type_identifier" | "member_expression"
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
            for child in node.children(&mut cursor) {
                if let Some(name) = last_identifier_in_chain(&child, source) {
                    return Some(name);
                }
            }
            None
        }
    }
}

/// Check if a node is inside a comment or string literal.
pub fn is_in_comment_or_string(node: &Node) -> bool {
    let mut current = Some(*node);
    while let Some(n) = current {
        match n.kind() {
            "comment"
            | "line_comment"
            | "block_comment"
            | "string_literal"
            | "raw_string_literal"
            | "string"
            | "template_string"
            | "interpreted_string_literal"
            | "quoted_string_literal" => return true,
            _ => {}
        }
        current = n.parent();
    }
    false
}

/// Find the enclosing function/method name for a node.
pub fn enclosing_symbol_name(node: &Node, source: &str) -> Option<String> {
    let mut current = node.parent();
    while let Some(n) = current {
        match n.kind() {
            "function_item"
            | "function_declaration"
            | "function_definition"
            |             "method_declaration"
            | "method_definition"
            | "method" => {
                if let Some(name_node) = n.child_by_field_name("name") {
                    return node_text(&name_node, source).map(|s| s.to_string());
                }
            }
            _ => {}
        }
        current = n.parent();
    }
    None
}

/// Walk the tree and collect symbols, calls, and imports.
pub struct Extractor {
    pub symbols: Vec<SymbolDef>,
    pub calls: Vec<CallSite>,
    pub imports: Vec<ImportSite>,
}

impl Extractor {
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            calls: Vec::new(),
            imports: Vec::new(),
        }
    }

    pub fn into_result(self) -> ExtractionResult {
        ExtractionResult {
            symbols: self.symbols,
            calls: self.calls,
            imports: self.imports,
        }
    }

    pub fn add_symbol(
        &mut self,
        node: &Node,
        source: &str,
        name: &str,
        kind: SymbolKind,
    ) {
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
        let callee = match last_identifier_in_chain(callee_node, source) {
            Some(c) => c,
            None => return,
        };
        let caller = enclosing_symbol_name(node, source).unwrap_or_else(|| "<module>".to_string());
        let line = byte_to_line(source, node.start_byte());
        self.calls.push(CallSite {
            caller,
            callee,
            line,
            byte_start: node.start_byte(),
            byte_end: node.end_byte(),
        });
    }

    pub fn add_import(&mut self, node: &Node, source: &str, module: &str) {
        let line = byte_to_line(source, node.start_byte());
        self.imports.push(ImportSite {
            module_path: module.to_string(),
            line,
        });
    }

    pub fn walk_node(&mut self, node: &Node, source: &str, handlers: &NodeHandlers) {
        handlers.handle(self, node, source);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_node(&child, source, handlers);
        }
    }
}

impl Default for Extractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-language node handling callbacks.
pub struct NodeHandlers {
    pub on_node: Box<dyn Fn(&mut Extractor, &Node, &str)>,
}

impl NodeHandlers {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&mut Extractor, &Node, &str) + 'static,
    {
        Self {
            on_node: Box::new(f),
        }
    }
}

impl NodeHandlers {
    pub fn handle(&self, extractor: &mut Extractor, node: &Node, source: &str) {
        (self.on_node)(extractor, node, source);
    }
}

/// Collect all identifier descendant names from a node (for use/import paths).
pub fn collect_identifiers(node: &Node, source: &str) -> Vec<String> {
    let mut ids = Vec::new();
    collect_identifiers_rec(node, source, &mut ids);
    ids
}

fn collect_identifiers_rec(node: &Node, source: &str, ids: &mut Vec<String>) {
    if node.kind() == "identifier"
        || node.kind() == "type_identifier"
        || node.kind() == "property_identifier"
        || node.kind() == "package_identifier"
    {
        if let Some(text) = node_text(node, source) {
            ids.push(text.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "comment" && child.kind() != "line_comment" && child.kind() != "block_comment"
        {
            collect_identifiers_rec(&child, source, ids);
        }
    }
}

/// Get child node by field name.
pub fn field_child<'a>(node: &'a Node, name: &str) -> Option<Node<'a>> {
    node.child_by_field_name(name)
}

/// Walk tree with a simple visitor and return extraction results.
pub fn walk_tree(tree: &Tree, source: &str, handlers: &NodeHandlers) -> ExtractionResult {
    let mut extractor = Extractor::new();
    extractor.walk_node(&tree.root_node(), source, handlers);
    extractor.into_result()
}
