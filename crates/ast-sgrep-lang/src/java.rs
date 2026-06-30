use crate::extract::{field_child, node_text, parse_and_extract, walk_tree, NodeHandlers};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct JavaParser;

impl LanguageParser for JavaParser {
    fn language(&self) -> Language {
        Language::Java
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_java::LANGUAGE.into(), source, |tree, src| {
            let handlers = NodeHandlers::new(|ext, node, source| {
                match node.kind() {
                    "method_declaration" => {
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
                    "method_invocation" => {
                        if let Some(name_node) = field_child(node, "name") {
                            ext.add_call(node, source, &name_node);
                        }
                    }
                    "import_declaration" => {
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
        if n.kind() == "class_declaration" {
            return true;
        }
        current = n.parent();
    }
    false
}
