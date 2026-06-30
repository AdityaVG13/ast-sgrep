use crate::extract::{
    add_named_symbol, field_child, is_inside_kind, node_text, parse_ts_language,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct RubyParser;

impl LanguageParser for RubyParser {
    fn language(&self) -> Language {
        Language::Ruby
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_ts_language(tree_sitter_ruby::LANGUAGE.into(), source, |ext, node, source| {
            match node.kind() {
                    "method" => {
                        let kind = if is_inside_kind(node, "class") {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Function
                        };
                        add_named_symbol(ext, node, source, kind);
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
        })
    }
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
