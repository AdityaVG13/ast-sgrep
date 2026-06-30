use crate::extract::{
    add_named_symbol, field_child, is_inside_kind, parse_ts_language,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct RustParser;

impl LanguageParser for RustParser {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_ts_language(tree_sitter_rust::LANGUAGE.into(), source, |ext, node, source| {
            match node.kind() {
                "function_item" => {
                    let kind = if is_inside_kind(node, "impl_item") {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    add_named_symbol(ext, node, source, kind);
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
        })
    }
}
