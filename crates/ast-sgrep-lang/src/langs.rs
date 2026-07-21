//! Per-language extraction tables.
//!
//! Each language is a `KindRule` map plus the shared walker in `extract`.
//! Special cases (Go type kinds, Java fields, Ruby require, TS arrows) are
//! expressed as table rules rather than copy-pasted match arms.

use crate::extract::{apply_kind_table, KindRule};
use crate::{ExtractionResult, Language, LanguageParser, SymbolKind};
use KindRule::*;
use SymbolKind::*;

macro_rules! parser {
    ($name:ident, $lang:ident, $ts:expr, $table:expr) => {
        pub struct $name;
        impl LanguageParser for $name {
            fn language(&self) -> Language {
                Language::$lang
            }
            fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult> {
                crate::extract::parse_ts_language_for(
                    Some(Language::$lang),
                    $ts.into(),
                    source,
                    |ext, node, src| {
                        let _ = apply_kind_table(ext, node, src, $table);
                    },
                )
            }
        }
    };
}

// Parent sets for Method vs Function classification.
const IMPL: &[&str] = &["impl_item"];
const PY_CLASS: &[&str] = &["class_definition"];
const JAVA_CLASS: &[&str] = &["class_declaration"];
const CS_TYPES: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "interface_declaration",
    "record_declaration",
];
const RB_CLASS: &[&str] = &["class"];
const GO_TYPE_CASES: &[(&str, SymbolKind)] = &[("interface_type", Interface)];
const RUBY_REQUIRE: &[&str] = &["require", "require_relative", "load"];

// ─── Kind maps ──────────────────────────────────────────────────────────────

#[rustfmt::skip]
const RUST: &[(&str, KindRule)] = &[
    ("function_item",            MethodIn(IMPL)),
    ("struct_item",              Sym(Type)),
    ("union_item",               Sym(Type)),
    ("type_item",                Sym(Type)),
    ("enum_item",                Sym(Enum)),
    ("trait_item",               Sym(Interface)),
    ("call_expression",          Call("function")),
    ("use_declaration",          ImportJoin("::")),
    ("extern_crate_declaration", ImportJoin("::")),
];

#[rustfmt::skip]
const PYTHON: &[(&str, KindRule)] = &[
    ("function_definition",   MethodIn(PY_CLASS)),
    ("class_definition",      Sym(Class)),
    ("call",                  Call("function")),
    ("import_statement",      ImportJoin(".")),
    ("import_from_statement", ImportJoin(".")),
];

#[rustfmt::skip]
const GO: &[(&str, KindRule)] = &[
    ("function_declaration", Sym(Function)),
    ("method_declaration",   Sym(Method)),
    ("type_spec",            SymByField("type", GO_TYPE_CASES, Type)),
    ("call_expression",      Call("function")),
    ("import_spec",          ImportQuotedOrChild("path", "interpreted_string_literal")),
];

#[rustfmt::skip]
const JAVA: &[(&str, KindRule)] = &[
    ("method_declaration",      MethodIn(JAVA_CLASS)),
    ("constructor_declaration", MethodIn(JAVA_CLASS)),
    ("class_declaration",       Sym(Class)),
    ("record_declaration",      Sym(Class)),
    ("interface_declaration",   Sym(Interface)),
    ("enum_declaration",        Sym(Enum)),
    ("field_declaration",       SymChildren("variable_declarator", Method)),
    ("method_invocation",       Call("name")),
    (
        "import_declaration",
        ImportPath(
            &["scoped_identifier", "identifier"],
            &["import", "static"],
            true,
            Some("."),
        ),
    ),
];

#[rustfmt::skip]
const CSHARP: &[(&str, KindRule)] = &[
    ("method_declaration",       MethodIn(CS_TYPES)),
    ("constructor_declaration",  MethodIn(CS_TYPES)),
    ("local_function_statement", Sym(Function)),
    ("class_declaration",        Sym(Class)),
    ("record_declaration",       Sym(Class)),
    ("struct_declaration",       Sym(Type)),
    ("interface_declaration",    Sym(Interface)),
    ("enum_declaration",         Sym(Enum)),
    ("invocation_expression",    Call("function")),
    (
        "using_directive",
        ImportPath(
            &["qualified_name", "identifier"],
            &["using", "static", "global"],
            false,
            None,
        ),
    ),
];

#[rustfmt::skip]
const RUBY: &[(&str, KindRule)] = &[
    ("method", MethodIn(RB_CLASS)),
    ("class",  Sym(Class)),
    ("module", Sym(Type)),
    ("call",   CallOrImport("method", RUBY_REQUIRE, "arguments")),
];

/// Shared TypeScript + JavaScript rules (same grammar shape for decls/calls/imports).
#[rustfmt::skip]
const TS_JS: &[(&str, KindRule)] = &[
    ("function_declaration",           Sym(Function)),
    ("generator_function_declaration", Sym(Function)),
    ("method_definition",              Sym(Method)),
    ("class_declaration",              Sym(Class)),
    ("abstract_class_declaration",     Sym(Class)),
    ("interface_declaration",          Sym(Interface)),
    ("type_alias_declaration",         Sym(Type)),
    ("enum_declaration",               Sym(Enum)),
    ("function_expression",            SymParent("variable_declarator", Function)),
    ("arrow_function",                 SymParent("variable_declarator", Function)),
    ("call_expression",                Call("function")),
    ("import_statement",               ImportQuoted("source")),
];

// ─── Parsers (all 8 languages) ──────────────────────────────────────────────

parser!(RustParser, Rust, tree_sitter_rust::LANGUAGE, RUST);
parser!(PythonParser, Python, tree_sitter_python::LANGUAGE, PYTHON);
parser!(GoParser, Go, tree_sitter_go::LANGUAGE, GO);
parser!(JavaParser, Java, tree_sitter_java::LANGUAGE, JAVA);
parser!(CSharpParser, CSharp, tree_sitter_c_sharp::LANGUAGE, CSHARP);
parser!(RubyParser, Ruby, tree_sitter_ruby::LANGUAGE, RUBY);
parser!(
    TypeScriptParser,
    TypeScript,
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
    TS_JS
);
parser!(
    JavaScriptParser,
    JavaScript,
    tree_sitter_javascript::LANGUAGE,
    TS_JS
);
