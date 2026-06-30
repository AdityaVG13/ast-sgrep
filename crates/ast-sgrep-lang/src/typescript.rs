use crate::extract::{
    add_named_symbol, field_child, node_text, parse_and_extract, walk_tree, NodeHandlers,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), source, |tree, src| {
            extract_ts_js(tree, src)
        })
    }
}

pub struct JavaScriptParser;

impl LanguageParser for JavaScriptParser {
    fn language(&self) -> Language {
        Language::JavaScript
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_javascript::LANGUAGE.into(), source, |tree, src| {
            extract_ts_js(tree, src)
        })
    }
}

fn extract_ts_js(tree: &tree_sitter::Tree, src: &str) -> ExtractionResult {
    let handlers = NodeHandlers::new(|ext, node, source| {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                add_named_symbol(ext, node, source, SymbolKind::Function);
            }
            "method_definition" => {
                add_named_symbol(ext, node, source, SymbolKind::Method);
            }
            "function_expression" | "arrow_function" => {
                if let Some(parent) = node.parent() {
                    if parent.kind() == "variable_declarator" {
                        add_named_symbol(ext, &parent, source, SymbolKind::Function);
                    }
                }
            }
            "call_expression" => {
                if let Some(func) = field_child(node, "function") {
                    ext.add_call(node, source, &func);
                }
            }
            "import_statement" => {
                if let Some(source_node) = field_child(node, "source") {
                    if let Some(path) = node_text(&source_node, source) {
                        let cleaned = path.trim_matches(|c| c == '"' || c == '\'' || c == '`');
                        ext.add_import(node, source, cleaned);
                    }
                }
            }
            _ => {}
        }
    });
    walk_tree(tree, src, &handlers)
}
