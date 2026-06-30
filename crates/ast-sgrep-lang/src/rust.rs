use crate::extract::{field_child, node_text, parse_and_extract, walk_tree, NodeHandlers};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct RustParser;

impl LanguageParser for RustParser {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_rust::LANGUAGE.into(), source, |tree, src| {
            let handlers = NodeHandlers::new(|ext, node, source| {
                match node.kind() {
                    "function_item" => {
                        if let Some(name_node) = field_child(node, "name") {
                            if let Some(name) = node_text(&name_node, source) {
                                let kind = if is_inside_impl(node) {
                                    SymbolKind::Method
                                } else {
                                    SymbolKind::Function
                                };
                                ext.add_symbol(node, source, name, kind);
                            }
                        }
                    }
                    "call_expression" => {
                        if let Some(func) = field_child(node, "function") {
                            ext.add_call(node, source, &func);
                        }
                    }
                    "use_declaration" | "extern_crate_declaration" => {
                        let ids = crate::extract::collect_identifiers(node, source);
                        if !ids.is_empty() {
                            ext.add_import(node, source, &ids.join("::"));
                        }
                    }
                    _ => {}
                }
            });
            walk_tree(tree, src, &handlers)
        })
    }
}

fn is_inside_impl(node: &tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "impl_item" {
            return true;
        }
        current = n.parent();
    }
    false
}
