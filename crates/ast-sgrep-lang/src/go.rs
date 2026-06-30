use crate::extract::{
    add_named_symbol, field_child, node_text, parse_and_extract, walk_tree, NodeHandlers,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct GoParser;

impl LanguageParser for GoParser {
    fn language(&self) -> Language {
        Language::Go
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_go::LANGUAGE.into(), source, |tree, src| {
            let handlers = NodeHandlers::new(|ext, node, source| {
                match node.kind() {
                    "function_declaration" => {
                        add_named_symbol(ext, node, source, SymbolKind::Function);
                    }
                    "method_declaration" => {
                        add_named_symbol(ext, node, source, SymbolKind::Method);
                    }
                    "call_expression" => {
                        if let Some(func) = field_child(node, "function") {
                            ext.add_call(node, source, &func);
                        }
                    }
                    "import_spec" => {
                        if let Some(path_node) = field_child(node, "path") {
                            if let Some(path) = node_text(&path_node, source) {
                                ext.add_import(node, source, path.trim_matches('"'));
                            }
                        } else {
                            let mut cursor = node.walk();
                            for child in node.children(&mut cursor) {
                                if child.kind() == "interpreted_string_literal" {
                                    if let Some(path) = node_text(&child, source) {
                                        ext.add_import(node, source, path.trim_matches('"'));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            });
            walk_tree(tree, src, &handlers)
        })
    }
}
