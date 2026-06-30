use crate::extract::{field_child, node_text, parse_and_extract, walk_tree, NodeHandlers};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct PythonParser;

impl LanguageParser for PythonParser {
    fn language(&self) -> Language {
        Language::Python
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_python::LANGUAGE.into(), source, |tree, src| {
            let handlers = NodeHandlers::new(|ext, node, source| {
                match node.kind() {
                    "function_definition" => {
                        if let Some(name_node) = field_child(node, "name") {
                            if let Some(name) = node_text(&name_node, source) {
                                let kind = if is_inside_class(node) {
                                    SymbolKind::Method
                                } else {
                                    SymbolKind::Function
                                };
                                ext.add_symbol(node, source, name, kind);
                            }
                        }
                    }
                    "call" => {
                        if let Some(func) = field_child(node, "function") {
                            ext.add_call(node, source, &func);
                        }
                    }
                    "import_statement" | "import_from_statement" => {
                        let ids = crate::extract::collect_identifiers(node, source);
                        if !ids.is_empty() {
                            ext.add_import(node, source, &ids.join("."));
                        }
                    }
                    _ => {}
                }
            });
            walk_tree(tree, src, &handlers)
        })
    }
}

fn is_inside_class(node: &tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "class_definition" {
            return true;
        }
        current = n.parent();
    }
    false
}
