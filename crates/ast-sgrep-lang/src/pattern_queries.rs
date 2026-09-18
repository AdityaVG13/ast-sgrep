//! Tree-sitter query tables for native declaration matching.
//!
//! Kept separate from walk/match control flow so language→query maps stay data-only.
//! Class queries are keyword-scoped (`class` / `struct` / `interface` / `type`) so
//! C#/Swift/C++/Kotlin/PHP singleton kind filters stay exact.

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

/// Look up class/type queries for `(lang, keyword)`.
pub(crate) fn class_queries_for(lang: Language, keyword: &str) -> &'static [&'static str] {
    for &(langs, kw, queries) in CLASS_QUERY_TABLE {
        if kw == keyword && langs.contains(&lang) {
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
        &[Language::Java],
        &[
            "(method_declaration name: (identifier) @name) @match",
            "(constructor_declaration name: (identifier) @name) @match",
        ],
    ),
    // e2hc/difu.5: C# has local_function_statement in addition to methods.
    (
        &[Language::CSharp],
        &[
            "(method_declaration name: (identifier) @name) @match",
            "(constructor_declaration name: (identifier) @name) @match",
            "(local_function_statement name: (identifier) @name) @match",
        ],
    ),
    // Dart nests `name` under signature wrappers; queries must follow the
    // method_declaration → method_signature → *signature → name chain.
    (
        &[Language::Dart],
        &[
            "(function_declaration (function_signature name: (identifier) @name)) @match",
            "(external_function_declaration (function_signature name: (identifier) @name)) @match",
            "(method_declaration (method_signature (function_signature name: (identifier) @name)) @match)",
            "(method_declaration (method_signature (constructor_signature name: (identifier) @name)) @match)",
            "(method_declaration (method_signature (getter_signature name: (identifier) @name)) @match)",
            "(method_declaration (method_signature (setter_signature name: (identifier) @name)) @match)",
            "(local_function_declaration (function_signature name: (identifier) @name)) @match",
            "(getter_declaration (getter_signature name: (identifier) @name)) @match",
            "(setter_declaration (setter_signature name: (identifier) @name)) @match",
            "(declaration (constructor_signature name: (identifier) @name)) @match",
        ],
    ),
    // MoonBit names are positional. Function/method names are
    // `lowercase_identifier`; capturing uppercase would bind `fn Type::method`
    // to the type name and make `fn Type` a false hit.
    (
        &[Language::MoonBit],
        &[
            "(function_definition (function_identifier (lowercase_identifier) @name)) @match",
            "(impl_definition (function_identifier (lowercase_identifier) @name)) @match",
            "(named_lambda_expression (lowercase_identifier) @name) @match",
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
    (
        &[Language::Swift],
        &[
            "(function_declaration name: (simple_identifier) @name) @match",
            "(protocol_function_declaration name: (simple_identifier) @name) @match",
        ],
    ),
    (
        &[Language::C, Language::Cpp],
        &[
            "(function_definition declarator: (function_declarator declarator: (identifier) @name)) @match",
            "(function_definition declarator: (function_declarator declarator: (field_identifier) @name)) @match",
            "(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @match",
        ],
    ),
    (
        &[Language::Kotlin],
        &["(function_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Php],
        &[
            "(function_definition name: (name) @name) @match",
            "(method_declaration name: (name) @name) @match",
        ],
    ),
];

/// `(languages, keyword, queries)` — keyword is the pattern prefix without trailing space.
pub(crate) const CLASS_QUERY_TABLE: &[(&[Language], &str, &[&str])] = &[
    // Rust
    (
        &[Language::Rust],
        "struct",
        &["(struct_item name: (type_identifier) @name) @match"],
    ),
    (
        &[Language::Rust],
        "interface",
        &["(trait_item name: (type_identifier) @name) @match"],
    ),
    (
        &[Language::Rust],
        "type",
        &[
            "(struct_item name: (type_identifier) @name) @match",
            "(enum_item name: (type_identifier) @name) @match",
            "(trait_item name: (type_identifier) @name) @match",
        ],
    ),
    // Python
    (
        &[Language::Python],
        "class",
        &["(class_definition name: (identifier) @name) @match"],
    ),
    (
        &[Language::Python],
        "type",
        &["(class_definition name: (identifier) @name) @match"],
    ),
    // Go — all type-ish keywords share type_declaration
    (
        &[Language::Go],
        "type",
        &["(type_declaration (type_spec name: (type_identifier) @name) @match)"],
    ),
    (
        &[Language::Go],
        "struct",
        &["(type_declaration (type_spec name: (type_identifier) @name) @match)"],
    ),
    (
        &[Language::Go],
        "interface",
        &["(type_declaration (type_spec name: (type_identifier) @name) @match)"],
    ),
    (
        &[Language::Go],
        "class",
        &["(type_declaration (type_spec name: (type_identifier) @name) @match)"],
    ),
    // Java
    (
        &[Language::Java],
        "class",
        &["(class_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Java],
        "interface",
        &["(interface_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Java],
        "type",
        &[
            "(class_declaration name: (identifier) @name) @match",
            "(interface_declaration name: (identifier) @name) @match",
        ],
    ),
    // C#
    (
        &[Language::CSharp],
        "class",
        &["(class_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::CSharp],
        "interface",
        &["(interface_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::CSharp],
        "struct",
        &["(struct_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::CSharp],
        "type",
        &[
            "(class_declaration name: (identifier) @name) @match",
            "(interface_declaration name: (identifier) @name) @match",
            "(struct_declaration name: (identifier) @name) @match",
            "(record_declaration name: (identifier) @name) @match",
            "(enum_declaration name: (identifier) @name) @match",
        ],
    ),
    // JS / TS
    (
        &[Language::JavaScript, Language::TypeScript],
        "class",
        &[
            "(class_declaration name: (identifier) @name) @match",
            "(class_declaration name: (type_identifier) @name) @match",
        ],
    ),
    (
        &[Language::JavaScript, Language::TypeScript],
        "type",
        &[
            "(class_declaration name: (identifier) @name) @match",
            "(class_declaration name: (type_identifier) @name) @match",
        ],
    ),
    // The ts interface row — the Class lane's member-count template
    // (`interface $N { $B }`) needs the kind query to
    // engage at all. The ts `interface_declaration` name node is a
    // `type_identifier` (the js grammar has no interfaces; ts-only row).
    (
        &[Language::TypeScript],
        "interface",
        &["(interface_declaration name: (type_identifier) @name) @match"],
    ),
    // The ts alias-object shape (`type $N = { $B }`) — keyword `type-alias`
    // (classifier strips the leading `=` off a plain
    // `type` tail) never collides with the plain `type` class rows. The ts
    // `type_alias_declaration` name node is a `type_identifier` (node-types
    // receipt, tree-sitter-typescript 0.23.2); brace-less alias tails stay
    // unclassified and keep the general lane.
    (
        &[Language::TypeScript],
        "type-alias",
        &["(type_alias_declaration name: (type_identifier) @name) @match"],
    ),
    // Ruby
    (
        &[Language::Ruby],
        "class",
        &["(class name: (constant) @name) @match"],
    ),
    (
        &[Language::Ruby],
        "type",
        &["(class name: (constant) @name) @match"],
    ),
    // Swift — class_declaration forms selected by declaration_kind in the query
    (
        &[Language::Swift],
        "class",
        &[r#"(class_declaration declaration_kind: "class" name: (type_identifier) @name) @match"#],
    ),
    (
        &[Language::Swift],
        "struct",
        &[r#"(class_declaration declaration_kind: "struct" name: (type_identifier) @name) @match"#],
    ),
    (
        &[Language::Swift],
        "interface",
        &["(protocol_declaration name: (type_identifier) @name) @match"],
    ),
    (
        &[Language::Swift],
        "type",
        &[
            "(class_declaration name: (type_identifier) @name) @match",
            "(protocol_declaration name: (type_identifier) @name) @match",
        ],
    ),
    // C
    (
        &[Language::C],
        "struct",
        &["(struct_specifier name: (type_identifier) @name) @match"],
    ),
    (
        &[Language::C],
        "type",
        &[
            "(struct_specifier name: (type_identifier) @name) @match",
            "(enum_specifier name: (type_identifier) @name) @match",
        ],
    ),
    // C++
    (
        &[Language::Cpp],
        "class",
        &["(class_specifier name: (type_identifier) @name) @match"],
    ),
    (
        &[Language::Cpp],
        "struct",
        &["(struct_specifier name: (type_identifier) @name) @match"],
    ),
    (
        &[Language::Cpp],
        "type",
        &[
            "(class_specifier name: (type_identifier) @name) @match",
            "(struct_specifier name: (type_identifier) @name) @match",
            "(enum_specifier name: (type_identifier) @name) @match",
        ],
    ),
    // Kotlin — one class_declaration node; filter in run_queries via class_keyword_matches
    (
        &[Language::Kotlin],
        "class",
        &["(class_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Kotlin],
        "interface",
        &["(class_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Kotlin],
        "type",
        &["(class_declaration name: (identifier) @name) @match"],
    ),
    // PHP
    (
        &[Language::Php],
        "class",
        &["(class_declaration name: (name) @name) @match"],
    ),
    (
        &[Language::Php],
        "interface",
        &["(interface_declaration name: (name) @name) @match"],
    ),
    (
        &[Language::Php],
        "type",
        &[
            "(class_declaration name: (name) @name) @match",
            "(interface_declaration name: (name) @name) @match",
            "(enum_declaration name: (name) @name) @match",
        ],
    ),
    // Dart
    (
        &[Language::Dart],
        "class",
        &["(class_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Dart],
        "interface",
        &["(mixin_declaration name: (identifier) @name) @match"],
    ),
    (
        &[Language::Dart],
        "type",
        &[
            "(class_declaration name: (identifier) @name) @match",
            "(mixin_declaration name: (identifier) @name) @match",
            "(extension_declaration name: (identifier) @name) @match",
            "(extension_type_declaration name: (identifier) @name) @match",
            "(enum_declaration name: (identifier) @name) @match",
        ],
    ),
    // MoonBit — positional names sit directly under the definition node.
    (
        &[Language::MoonBit],
        "type",
        &["(type_definition (identifier) @name) @match"],
    ),
    (
        &[Language::MoonBit],
        "struct",
        &[
            "(struct_definition (identifier) @name) @match",
            "(tuple_struct_definition (identifier) @name) @match",
        ],
    ),
    (
        &[Language::MoonBit],
        "interface",
        &["(trait_definition (identifier) @name) @match"],
    ),
];
