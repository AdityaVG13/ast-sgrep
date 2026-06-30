use crate::extract::{field_child, node_text, parse_and_extract, walk_tree, NodeHandlers};
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
                        if let Some(name_node) = field_child(node, "name") {
                            if let Some(name) = node_text(&name_node, source) {
                                ext.add_symbol(node, source, name, SymbolKind::Function);
                            }
                        }
                    }
                    "method_declaration" => {
                        if let Some(name_node) = field_child(node, "name") {
                            if let Some(name) = node_text(&name_node, source) {
                                ext.add_symbol(node, source, name, SymbolKind::Method);
                            }
                        }
                    }
                    "call_expression" => {
                        if let Some(func) = field_child(node, "function") {
                            ext.add_call(node, source, &func);
                        }
                    }
                    "import_declaration" => {
                        if let Some(path_node) = node.child_by_field_name("path") {
                            if let Some(path) = node_text(&path_node, source) {
                                let cleaned = path.trim_matches('"');
                                ext.add_import(node, source, cleaned);
                            }
                        } else {
                            let ids = crate::extract::collect_identifiers(node, source);
                            if !ids.is_empty() {
                                ext.add_import(node, source, &ids.join("/"));
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
