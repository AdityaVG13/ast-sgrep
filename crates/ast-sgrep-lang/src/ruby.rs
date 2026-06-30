use crate::extract::{
    field_child, node_text, parse_and_extract, walk_tree, NodeHandlers,
};
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
                            if let Some(method_name) = node_text(&method, source) {
                                match method_name {
                                    "require" | "require_relative" | "load" => {
                                        if let Some(path) = ruby_string_argument(node, source) {
                                            ext.add_import(node, source, &path);
                                        }
                                    }
                                    _ => ext.add_call(node, source, &method),
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

fn ruby_string_argument(call_node: &tree_sitter::Node, source: &str) -> Option<String> {
    let args = field_child(call_node, "arguments")?;
    extract_ruby_string(&args, source)
}

fn extract_ruby_string(node: &tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" | "string_content" | "interpreted_string_literal" | "bare_string_literal" => {
            node_text(node, source).map(clean_ruby_string)
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(s) = extract_ruby_string(&child, source) {
                    return Some(s);
                }
            }
            None
        }
    }
}

fn clean_ruby_string(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_require_and_require_relative() {
        let src = r#"
require "json"
require_relative "./app"
load "boot.rb"

def main
  process_request("x")
end
"#;
        let result = RubyParser.parse(src).unwrap();
        assert!(result.imports.iter().any(|i| i.module_path == "json"));
        assert!(
            result
                .imports
                .iter()
                .any(|i| i.module_path.contains("app"))
        );
        assert!(result.imports.iter().any(|i| i.module_path == "boot.rb"));
    }
}
