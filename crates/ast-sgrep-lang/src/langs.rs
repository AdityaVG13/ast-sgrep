use crate::extract:: { add_named_symbol, collect_identifiers, field_child, is_inside_kind, node_text, }; use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};
macro_rules! parser {
    ($name:ident, $lang:ident, $ts:expr, $body:expr) => {
        pub struct $name; impl LanguageParser for $name {
            fn language(&self) -> Language { Language::$lang } fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
                crate::extract::parse_ts_language_for( Some(Language::$lang), $ts.into(), source, $body, )
            }
        }
    };
}
parser!(RustParser, Rust, tree_sitter_rust::LANGUAGE, |ext, node, source| match node.kind() {
    "function_item" => add_named_symbol( ext, node, source, if is_inside_kind(node, "impl_item") { SymbolKind::Method } else { SymbolKind::Function },
    ), "struct_item" | "union_item" | "type_item" => add_named_symbol(ext, node, source, SymbolKind::Type),
    "enum_item" => add_named_symbol(ext, node, source, SymbolKind::Enum), "trait_item" => add_named_symbol(ext, node, source, SymbolKind::Interface), "call_expression" => { if let Some(func) = field_child(node, "function") { ext.add_call(node, source, &func); } } "use_declaration" | "extern_crate_declaration" => { let ids = collect_identifiers(node, source); if !ids.is_empty() { ext.add_import(node, source, &ids.join("::")); } } _ => {}
});
parser!(PythonParser, Python, tree_sitter_python::LANGUAGE, |ext, node, source| match node.kind() {
    "function_definition" => add_named_symbol(
        ext, node, source, if is_inside_kind(node, "class_definition") { SymbolKind::Method } else { SymbolKind::Function },
    ), "class_definition" => add_named_symbol(ext, node, source, SymbolKind::Class), "call" => { if let Some(func) = field_child(node, "function") { ext.add_call(node, source, &func); } } "import_statement" | "import_from_statement" => { let ids = collect_identifiers(node, source); if !ids.is_empty() { ext.add_import(node, source, &ids.join(".")); } } _ => {}
});
parser!(GoParser, Go, tree_sitter_go::LANGUAGE, |ext, node, source| match node.kind() {
    "function_declaration" => add_named_symbol(ext, node, source, SymbolKind::Function), "method_declaration" => add_named_symbol(ext, node, source, SymbolKind::Method), "type_spec" => { let kind = match field_child(node, "type").map(|t| t.kind()) { Some("interface_type") => SymbolKind::Interface, _ => SymbolKind::Type, }; add_named_symbol(ext, node, source, kind); } "call_expression" => { if let Some(func) = field_child(node, "function") { ext.add_call(node, source, &func); } } "import_spec" => {
        if let Some(path_node) = field_child(node, "path") { if let Some(path) = node_text(&path_node, source) { ext.add_import(node, source, path.trim_matches('"')); } } else {
            let mut cursor = node.walk(); for child in node.children(&mut cursor) { if child.kind() == "interpreted_string_literal" { if let Some(path) = node_text(&child, source) { ext.add_import(node, source, path.trim_matches('"')); } } }
        }
    } _ => {}
});
parser!(JavaParser, Java, tree_sitter_java::LANGUAGE, |ext, node, source| match node.kind() {
    "method_declaration" | "constructor_declaration" => add_named_symbol(
        ext, node, source, if is_inside_kind(node, "class_declaration") { SymbolKind::Method } else { SymbolKind::Function },
    ), "class_declaration" | "record_declaration" => add_named_symbol(ext, node, source, SymbolKind::Class), "interface_declaration" => add_named_symbol(ext, node, source, SymbolKind::Interface),
    "enum_declaration" => add_named_symbol(ext, node, source, SymbolKind::Enum), "field_declaration" => { let mut cursor = node.walk(); for child in node.children(&mut cursor) { if child.kind() == "variable_declarator" { add_named_symbol(ext, &child, source, SymbolKind::Method); } } } "method_invocation" => { if let Some(name_node) = field_child(node, "name") { ext.add_call(node, source, &name_node); } } "import_declaration" => {
        if let Some(path) = java_import_path(node, source) { ext.add_import(node, source, &path); }
    } _ => {}
}); fn java_import_path(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk(); for child in node.children(&mut cursor) {
        if matches!(child.kind(), "scoped_identifier" | "identifier") { if let Some(text) = node_text(&child, source) { if text != "import" && text != "static" { return Some(text.to_string()); } } } if let Some(path) = java_import_path(&child, source) { return Some(path); }
    } let ids: Vec<String> = collect_identifiers(node, source)
        .into_iter() .filter(|s| s != "import" && s != "static") .collect();
    (!ids.is_empty()).then(|| ids.join("."))
}
parser!(CSharpParser, CSharp, tree_sitter_c_sharp::LANGUAGE, |ext, node, source| match node.kind() {
    "method_declaration" | "constructor_declaration" => add_named_symbol(
        ext, node, source, if inside_type_declaration(node) { SymbolKind::Method } else { SymbolKind::Function },
    ), "local_function_statement" => add_named_symbol(ext, node, source, SymbolKind::Function), "class_declaration" | "record_declaration" => add_named_symbol(ext, node, source, SymbolKind::Class),
    "struct_declaration" => add_named_symbol(ext, node, source, SymbolKind::Type), "interface_declaration" => add_named_symbol(ext, node, source, SymbolKind::Interface),
    "enum_declaration" => add_named_symbol(ext, node, source, SymbolKind::Enum), "invocation_expression" => { if let Some(function) = field_child(node, "function") { ext.add_call(node, source, &function); } } "using_directive" => { if let Some(path) = csharp_using_path(node, source) { ext.add_import(node, source, &path); } } _ => {}
}); fn inside_type_declaration(node: &tree_sitter::Node) -> bool {
    let mut current = node.parent(); while let Some(n) = current {
        if matches!( n.kind(), "class_declaration" | "struct_declaration" | "interface_declaration" | "record_declaration"
        ) { return true; } current = n.parent();
    } false
} fn csharp_using_path(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk(); for child in node.children(&mut cursor) {
        if matches!(child.kind(), "qualified_name" | "identifier") { if let Some(text) = node_text(&child, source) { if text != "using" && text != "static" && text != "global" { return Some(text.to_string()); } } }
    } None
}
parser!(RubyParser, Ruby, tree_sitter_ruby::LANGUAGE, |ext, node, source| match node.kind() {
    "method" => add_named_symbol( ext, node, source, if is_inside_kind(node, "class") { SymbolKind::Method } else { SymbolKind::Function },
    ), "class" => add_named_symbol(ext, node, source, SymbolKind::Class), "module" => add_named_symbol(ext, node, source, SymbolKind::Type), "call" => {
        if let Some(method) = field_child(node, "method") {
            if let Some(method_name) = node_text(&method, source) {
                match method_name { "require" | "require_relative" | "load" => { if let Some(path) = ruby_string_argument(node, source) { ext.add_import(node, source, &path); } } _ => ext.add_call(node, source, &method), }
            }
        }
    } _ => {}
}); fn ruby_string_argument(call_node: &tree_sitter::Node, source: &str) -> Option<String> { extract_ruby_string(&field_child(call_node, "arguments")?, source) } fn extract_ruby_string(node: &tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "string" | "string_content" | "interpreted_string_literal" | "bare_string_literal" => { node_text(node, source).map(|raw| { raw.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').to_string() }) } _ => {
            let mut cursor = node.walk(); let mut found = None; for c in node.children(&mut cursor) { if let Some(s) = extract_ruby_string(&c, source) { found = Some(s); break; } } found
        }
    }
} pub struct TypeScriptParser; pub struct JavaScriptParser; impl LanguageParser for TypeScriptParser {
    fn language(&self) -> Language { Language::TypeScript } fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        crate::extract::parse_ts_language_for( Some(Language::TypeScript), tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), source, handle_ts_js_node, )
    }
} impl LanguageParser for JavaScriptParser {
    fn language(&self) -> Language { Language::JavaScript } fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
        crate::extract::parse_ts_language_for( Some(Language::JavaScript), tree_sitter_javascript::LANGUAGE.into(), source, handle_ts_js_node, )
    }
} fn handle_ts_js_node(ext: &mut crate::extract::Extractor, node: &tree_sitter::Node, source: &str) {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => { add_named_symbol(ext, node, source, SymbolKind::Function) } "method_definition" => add_named_symbol(ext, node, source, SymbolKind::Method), "class_declaration" | "abstract_class_declaration" => { add_named_symbol(ext, node, source, SymbolKind::Class) } "interface_declaration" => add_named_symbol(ext, node, source, SymbolKind::Interface),
        "type_alias_declaration" => add_named_symbol(ext, node, source, SymbolKind::Type), "enum_declaration" => add_named_symbol(ext, node, source, SymbolKind::Enum), "function_expression" | "arrow_function" => { if node.parent().is_some_and(|p| p.kind() == "variable_declarator") { if let Some(parent) = node.parent() { add_named_symbol(ext, &parent, source, SymbolKind::Function); } } } "call_expression" => { if let Some(func) = field_child(node, "function") { ext.add_call(node, source, &func); } } "import_statement" => {
            if let Some(source_node) = field_child(node, "source") { if let Some(path) = node_text(&source_node, source) { ext.add_import(node, source, path.trim_matches(|c| c == '"' || c == '\'' || c == '`')); } }
        } _ => {}
    }
}
