use crate::extract::{field_child, node_text, parse_and_extract, walk_tree, NodeHandlers};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct RubyParser;

impl LanguageParser for RubyParser {
    fn language(&self) -> Language {
        Language::Ruby
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_and_extract(tree_sitter_ruby::LANGUAGE.into(), source, |tree, src| {
            let handlers = NodeHandlers::new(|ext, node, source| {
                match node.kind() {
                    "method" => {
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
                        if let Some(method) = field_child(node, "method") {
                            ext.add_call(node, source, &method);
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
        if n.kind() == "class" {
            return true;
        }
        current = n.parent();
    }
    false
}
