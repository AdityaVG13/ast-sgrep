//! Tree-sitter query tables for native declaration matching.
//!
//! Kept separate from walk/match control flow so language→query maps stay data-only.

use crate::Language;

pub(crate) fn queries_for(
    table: &'static [(&'static [Language], &'static [&'static str])],
    lang: Language,
) -> &'static [&'static str] {
    for &(langs, queries) in table {
        if langs.contains(&lang) {
            return queries;
        }
    }
    &[]
}

pub(crate) const FUNCTION_QUERY_TABLE: &[(&[Language], &[&str])] = &[
    (
        &[Language::Rust],
        &[
            "(function_item name: (identifier) @name) @match",
            "(impl_item body: (declaration_list (function_item name: (identifier) @name) @match))",
        ],
    ),
    (
        &[Language::Python],
        &["(function_definition name: (identifier) @name) @match"],
    ),
    (
        &[Language::Go],
        &["(function_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Java, Language::CSharp],
        &[
            "(method_declaration name: (identifier) @name) @match",
            "(constructor_declaration name: (identifier) @name) @match",
        ],
    ),
    (
        &[Language::JavaScript, Language::TypeScript],
        &[
            "(function_declaration name: (identifier) @name) @match",
            "(method_definition name: (property_identifier) @name) @match",
            "(lexical_declaration (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @match)",
        ],
    ),
    (
        &[Language::Ruby],
        &[
            "(method name: (identifier) @name) @match",
            "(singleton_method name: (identifier) @name) @match",
        ],
    ),
];

pub(crate) const CLASS_QUERY_TABLE: &[(&[Language], &[&str])] = &[
    (
        &[Language::Rust],
        &[
            "(struct_item name: (type_identifier) @name) @match",
            "(enum_item name: (type_identifier) @name) @match",
            "(trait_item name: (type_identifier) @name) @match",
        ],
    ),
    (
        &[Language::Python],
        &["(class_definition name: (identifier) @name) @match"],
    ),
    (
        &[Language::Go],
        &["(type_declaration (type_spec name: (type_identifier) @name) @match)"],
    ),
    (
        &[Language::Java, Language::CSharp],
        &[
            "(class_declaration name: (identifier) @name) @match",
            "(interface_declaration name: (identifier) @name) @match",
        ],
    ),
    (
        &[Language::JavaScript, Language::TypeScript],
        &["(class_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Ruby],
        &["(class name: (constant) @name) @match"],
    ),
];
