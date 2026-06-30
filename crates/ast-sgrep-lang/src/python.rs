use crate::extract::{
    add_named_symbol, field_child, is_inside_kind, parse_ts_language,
};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};

pub struct PythonParser;

impl LanguageParser for PythonParser {
    fn language(&self) -> Language {
        Language::Python
    }

    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        parse_ts_language(tree_sitter_python::LANGUAGE.into(), source, |ext, node, source| {
            match node.kind() {
                "function_definition" => {
                    let kind = if is_inside_kind(node, "class_definition") {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    add_named_symbol(ext, node, source, kind);
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
        })
    }
}
