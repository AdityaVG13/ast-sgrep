use ast_sgrep_lang::signature::{cached_pattern_signatures, index_can_serve_pattern};
use ast_sgrep_lang::{match_pattern, required_pattern_literal, Language, ParserRegistry};

#[test]
fn ident_and_decl_are_index_complete_kind_is_not() {
    let ident = cached_pattern_signatures("SearchHit").unwrap();
    assert!(index_can_serve_pattern("SearchHit", &ident));
    let decl = cached_pattern_signatures("fn greet_user").unwrap();
    assert!(index_can_serve_pattern("fn greet_user", &decl));
    let kind = cached_pattern_signatures("fn $NAME").unwrap();
    assert!(!index_can_serve_pattern("fn $NAME", &kind));
    assert!(!index_can_serve_pattern("fn $NAME() { $$$BODY }", &[]));
}

/// INTENT: a byte prefilter must retain a file containing the exact matching
/// AST spelling, even when a block comment separates two identifier tokens.
/// KILLS: deleting comment bytes and inventing `newWidget` / `returnvalue`.
#[test]
fn block_comment_prefilter_preserves_token_boundaries() {
    for (source, pattern) in [
        ("new/*c*/Widget();", "new/*c*/Widget()"),
        ("function f() { return/*c*/value; }", "return/*c*/value;"),
    ] {
        let hits = match_pattern(Language::JavaScript, source, pattern).unwrap();
        assert!(!hits.is_empty(), "fixture must match: {pattern:?}");
        if let Some(literal) = required_pattern_literal(pattern) {
            assert!(
                source.contains(&literal),
                "prefilter {literal:?} would discard a matching source for {pattern:?}"
            );
        }
    }
}

/// INTENT: a name-only declaration signature cannot authoritatively answer a
/// parameter-constrained template. The native matcher must still decide arity.
/// KILLS: returning both zero- and one-parameter declarations from decl:def:foo.
#[test]
fn parameter_constrained_declaration_is_not_index_complete() {
    let source = "def foo():\n    pass\ndef foo(x):\n    pass\n";
    let pattern = "def foo($X)";
    let hits = match_pattern(Language::Python, source, pattern).unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.line_start).collect::<Vec<_>>(),
        vec![3]
    );
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("x"));

    let extraction = ParserRegistry::new()
        .parse(Language::Python, source)
        .unwrap();
    let indexed_lines: Vec<_> = extraction
        .pattern_nodes
        .iter()
        .filter(|node| node.signature == "decl:def:foo")
        .map(|node| node.line_start)
        .collect();
    assert_eq!(indexed_lines, vec![1, 3]);
    assert!(
        cached_pattern_signatures(pattern).is_none(),
        "depth-truncated search trusts decl rows without consulting index_can_serve_pattern"
    );
    for constrained in ["def foo()", "def foo():", "fn foo($$$)", "function foo($X)"] {
        assert!(
            cached_pattern_signatures(constrained).is_none(),
            "only bare declaration names are index-complete: {constrained:?}"
        );
    }
    let bare = cached_pattern_signatures("def foo").unwrap();
    assert!(index_can_serve_pattern("def foo", &bare));
    let candidates = ast_sgrep_lang::candidate_kind_signatures(pattern).unwrap();
    assert!(candidates.contains(&"kind:function_definition".to_string()));
}

/// INTENT: a bare declaration shorthand must address the rows actually stored
/// for its language; syntax keywords and shared grammar kind names can differ.
/// KILLS: `func`/`def`/`function` signatures that cannot reach their declaration.
#[test]
fn bare_declaration_signatures_reach_ruby_go_and_php() {
    for (language, source, pattern) in [
        (Language::Ruby, "def greet\nend\n", "def greet"),
        (Language::Go, "package p\nfunc greet() {}\n", "func greet"),
        (
            Language::Php,
            "<?php function greet() {}\n",
            "function greet",
        ),
    ] {
        let extraction = ParserRegistry::new().parse(language, source).unwrap();
        assert!(extraction
            .symbols
            .iter()
            .any(|symbol| symbol.name == "greet"));
        let signatures = cached_pattern_signatures(pattern).unwrap();
        assert!(
            extraction
                .pattern_nodes
                .iter()
                .any(|node| signatures.contains(&node.signature)),
            "{language}: {pattern:?} addresses {signatures:?}, not its stored declaration"
        );
    }
}

/// INTENT: header declarations (prototypes without bodies) must extract as
/// symbols so their Doxygen docs become semantic chunks. The c/cpp census
/// (competitor campaign, 2026-10-26) measured 11% docs coverage with the
/// docs mass living above header declarations: oatpp carries 2185 doxygen
/// blocks and 0 header symbols — extraction tables only match
/// `function_definition`, so documented header APIs never enter the index.
/// KILLS: tables that skip `declaration`/`field_declaration`, and rules
/// that promote plain variable declarations to function symbols.
#[test]
fn cpp_header_declarations_extract_as_symbols() {
    let header = r#"/**
 * Parses a widget from raw bytes.
 */
Widget parse_widget(const uint8_t *data, size_t len);

extern int cache_size;

class Reader {
public:
    /** Reads the next record. */
    Record next();
};
"#;
    let extraction = ParserRegistry::new().parse(Language::Cpp, header).unwrap();
    let names: Vec<(String, String)> = extraction
        .symbols
        .iter()
        .map(|s| (s.name.clone(), format!("{:?}", s.kind).to_lowercase()))
        .collect();
    assert!(
        names.iter().any(|(n, k)| n == "parse_widget" && k == "function"),
        "free-function prototype extracted: {names:?}"
    );
    assert!(
        names.iter().any(|(n, k)| n == "next" && k == "method"),
        "class-method declaration extracted: {names:?}"
    );
    assert!(
        !names.iter().any(|(n, k)| n == "cache_size" && (k == "function" || k == "method")),
        "plain variable declaration must not become a function/method symbol: {names:?}"
    );

    let c_src = "/* Adds two ints. */\nint add(int a, int b);\n";
    let c_extraction = ParserRegistry::new().parse(Language::C, c_src).unwrap();
    assert!(
        c_extraction.symbols.iter().any(|s| s.name == "add"),
        "C prototype extracted: {:?}",
        c_extraction.symbols
    );
}
