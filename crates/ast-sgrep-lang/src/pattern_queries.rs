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

/// Function-only queries for ts/php BODY templates carrying a spelled
/// return section (br-hs1): the reference answers free functions only —
/// class methods never (probed on class-method fixtures, 2026-10-27) —
/// so the method rows of the legacy table are excluded on this lane.
pub(crate) fn return_function_queries_for(lang: Language) -> &'static [&'static str] {
    const TABLE: &[(&[Language], &[&str])] = &[
        (
            &[Language::TypeScript],
            &["(function_declaration name: (identifier) @name) @match"],
        ),
        (
            &[Language::Php],
            &["(function_definition name: (name) @name) @match"],
        ),
    ];
    queries_for(TABLE, lang)
}

/// Candidate queries for BODY-LESS fn patterns — argument list present, no
/// `{...}` section. Reference semantics are grammar-specific because the
/// reference's pattern parse recovery is: ast-grep 0.45.2 + 0.51.0 probes
/// (2026-10-27, 3 stable rounds, br-f4w; spelled-return receipt br-hs1).
///
/// - Rust: a body-less `fn ...(...)` pattern answers `function_signature_item`
///   nodes ONLY (trait method declarations; a pattern-absent return type is a
///   wildcard). Body-having `function_item`s never answer it. A SPELLED
///   return keeps the same query — the section filters per candidate.
/// - Go: body-less `func ...(...)` is natively valid syntax; it answers
///   `function_declaration` nodes with body AND return type wildcarded.
/// - TypeScript: the reference's recovery yields nothing matchable — empty
///   lane, with or without a spelled return.
/// - Php: no-return body-less stays empty (br-f4w), but a SPELLED return
///   answers `function_definition` nodes (probed: `function $A($$$): int`
///   matched the return-typed free function; methods never answer).
///
/// Languages absent from this table keep the legacy FUNCTION_QUERY_TABLE
/// path for body-less patterns (their reference behavior is unprobed; js/py
/// are probed parity on that path).
pub(crate) fn bodyless_function_queries_for(
    lang: Language,
    spelled_return: bool,
) -> Option<&'static [&'static str]> {
    if lang == Language::Php && spelled_return {
        return Some(&["(function_definition name: (name) @name) @match"]);
    }
    const TABLE: &[(&[Language], &[&str])] = &[
        (
            &[Language::Rust],
            &["(function_signature_item name: (identifier) @name) @match"],
        ),
        (
            &[Language::Go],
            &["(function_declaration name: (identifier) @name) @match"],
        ),
        // ts/php: the reference answers nothing; empty = silent parity.
        (&[Language::TypeScript, Language::Php], &[]),
    ];
    for &(langs, queries) in TABLE {
        if langs.contains(&lang) {
            return Some(queries);
        }
    }
    None
}

/// br-osk: the no-parens PURE-NAME lane (`fn $A` — argument list absent,
/// no sections). The reference (0.45.2, probed 2026-10-27, 4 stable
/// rounds/cell) answers every fn-shaped node of the grammar: rust =
/// function items (impl-nested included, generics wildcarded) ∪ trait
/// signature items. Other grammars keep the legacy FUNCTION_QUERY_TABLE
/// rows — the probed fixtures' fn-shaped kinds are exactly those.
pub(crate) fn name_only_function_queries_for(lang: Language) -> &'static [&'static str] {
    if lang == Language::Rust {
        return &[
            "(function_item name: (identifier) @name) @match",
            "(impl_item body: (declaration_list (function_item name: (identifier) @name) @match))",
            "(function_signature_item name: (identifier) @name) @match",
        ];
    }
    // ts/php: the no-parens face answers FREE FUNCTIONS only — class
    // methods and arrows never answer the `function` keyword face
    // (probed on method/arrow fixtures 2026-10-27; the legacy table's
    // method rows serve other lanes, not this one).
    if matches!(lang, Language::TypeScript | Language::Php) {
        return return_function_queries_for(lang);
    }
    queries_for(FUNCTION_QUERY_TABLE, lang)
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
