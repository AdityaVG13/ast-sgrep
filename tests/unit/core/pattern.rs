use ast_sgrep_lang::cached_pattern_signatures;

#[test]
fn fixed_bakeoff_suite_is_index_or_native_resolvable() {
    const PATTERNS: &[&str] = &[
        "fn gitignore_matched",
        "fn parse_low",
        "struct WalkBuilder",
        "fn search_slice",
        "struct RegexMatcherBuilder",
        "struct StandardBuilder",
        "struct JSONBuilder",
        "struct GlobBuilder",
        "DecompressionMatcherBuilder",
        "struct TypesBuilder",
        "fn run",
        "struct OverrideBuilder",
        "fn open_mmap",
        "fn multi_line_with_matcher",
        "def full_dispatch_request",
        "class Blueprint",
        "class SecureCookieSessionInterface",
        "class DispatchingJinjaLoader",
        "class FlaskGroup",
        "def from_pyfile",
        "class AppContext",
        "class DefaultJSONProvider",
        "request_started",
        "class MethodView",
        "def get_flashed_messages",
        "class Request",
        "class App",
        "def setupmethod",
        "class TaggedJSONSerializer",
    ];
    assert_eq!(PATTERNS.len(), 29);
    for pattern in PATTERNS {
        assert!(
            cached_pattern_signatures(pattern).is_some(),
            "no indexed signature for {pattern}"
        );
        assert!(
            !ast_sgrep_lang::needs_ast_grep_fallback(pattern),
            "fixed suite unexpectedly requires a subprocess: {pattern}"
        );
    }
}

#[test]
fn cached_metavariables_cover_kind_predicates() {
    assert!(cached_pattern_signatures("function $NAME($$$)")
        .unwrap()
        .contains(&"kind:method_declaration".to_string()));
    assert_eq!(
        cached_pattern_signatures("kind:function_item").unwrap(),
        vec!["kind:function_item"]
    );
}

#[test]
fn external_ast_grep_is_disabled_without_explicit_allow() {
    // Even if PATH has ast-grep, production/bench helpers stay inert.
    std::env::remove_var("ASGREP_ALLOW_AST_GREP");
    std::env::remove_var("ASGREP_AST_GREP");
    assert!(super::find_ast_grep_binary().is_none());
    assert!(super::bench_ast_grep("fn foo", std::path::Path::new("."), 1).is_none());
}
