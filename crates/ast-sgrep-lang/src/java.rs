use crate::extract::{
    add_named_symbol, field_child, is_inside_kind, node_text, parse_ts_language,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct JavaParser;

impl LanguageParser for JavaParser {
    fn language(&self) -> Language {
        Language::Java
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_ts_language(tree_sitter_java::LANGUAGE.into(), source, |ext, node, source| {
                match node.kind() {
                    "method_declaration" | "constructor_declaration" => {
                        let kind = if is_inside_kind(node, "class_declaration") {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Function
                        };
                        add_named_symbol(ext, node, source, kind);
                    }
                    "field_declaration" => {
                        let mut cursor = node.walk();
                        for child in node.children(&mut cursor) {
                            if child.kind() == "variable_declarator" {
                                add_named_symbol(ext, &child, source, SymbolKind::Method);
                            }
                        }
                    }
                    "method_invocation" => {
                        if let Some(name_node) = field_child(node, "name") {
                            ext.add_call(node, source, &name_node);
                        }
                    }
                    "import_declaration" => {
                        if let Some(path) = java_import_path(node, source) {
                            ext.add_import(node, source, &path);
                        }
                    }
                    _ => {}
                }
        })
    }
}

fn java_import_path(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "scoped_identifier" || child.kind() == "identifier" {
            if let Some(text) = node_text(&child, source) {
                if text != "import" && text != "static" {
                    return Some(text.to_string());
                }
            }
        }
        if let Some(path) = java_import_path(&child, source) {
            return Some(path);
        }
    }
    let ids: Vec<String> = crate::extract::collect_identifiers(node, source)
        .into_iter()
        .filter(|s| s != "import" && s != "static")
        .collect();
    if ids.is_empty() {
        None
    } else {
        Some(ids.join("."))
    }
}
