use crate::extract::{
    add_named_symbol, field_child, parse_ts_language,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_ts_language(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), source, |ext, node, source| {
            handle_ts_js_node(ext, node, source);
        })
    }
}

pub struct JavaScriptParser;

impl LanguageParser for JavaScriptParser {
    fn language(&self) -> Language {
        Language::JavaScript
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_ts_language(tree_sitter_javascript::LANGUAGE.into(), source, |ext, node, source| {
            handle_ts_js_node(ext, node, source);
        })
    }
}

fn handle_ts_js_node(
    ext: &mut crate::extract::Extractor,
    node: &tree_sitter::Node,
    source: &str,
) {
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
                if let Some(path) = crate::extract::node_text(&source_node, source) {
                    let cleaned = path.trim_matches(|c| c == '"' || c == '\'' || c == '`');
                    ext.add_import(node, source, cleaned);
                }
            }
        }
        _ => {}
    }
}
