use super::*;
#[test]
fn short_cased_identifier_is_the_primary_symbol() {
    assert_eq!(ParsedQuery::parse("Map").primary_symbol(), Some("map"));
}
#[test]
fn camel_split_does_not_emit_underscore_ghost_terms() {
    let p = ParsedQuery::parse("User_Id");
    assert!(!p.terms.iter().any(|t| t.ends_with('_')));
    assert!(p.terms.iter().any(|t| t == "user"));
    assert!(p.terms.iter().any(|t| t == "id"));
}

/// 54if: every prefixed mode keeps the prefix in `raw`.
#[test]
fn raw_keeps_mode_prefix_across_all_modes() {
    for (q, mode) in [
        ("callers:Foo", QueryMode::Callers),
        ("defs:Foo", QueryMode::Defs),
        ("imports:foo", QueryMode::Imports),
        ("pattern:fn $X() {}", QueryMode::Pattern),
        ("literal:FooBar", QueryMode::Literal),
        ("regex:Foo.*Bar", QueryMode::Regex),
        ("word:Foo", QueryMode::Word),
    ] {
        let p = ParsedQuery::parse(q);
        assert_eq!(p.mode, mode, "mode for {q}");
        assert_eq!(p.raw, q, "raw must keep full query for {q}");
    }
    let hybrid = ParsedQuery::parse("process_request");
    assert_eq!(hybrid.mode, QueryMode::Hybrid);
    assert_eq!(hybrid.raw, "process_request");
}

/// eh5a: mode_query / parse must not lowercase literal or regex terms.
#[test]
fn literal_and_regex_terms_preserve_case() {
    let lit = ParsedQuery::literal("FooBar");
    assert_eq!(lit.terms, vec!["FooBar".to_string()]);
    let re = ParsedQuery::regex("Foo.*Bar");
    assert_eq!(re.terms, vec!["Foo.*Bar".to_string()]);
    let word = ParsedQuery::word("FooBar");
    assert_eq!(word.terms, vec!["foobar".to_string()]);

    let lit_p = ParsedQuery::parse("literal:FooBar");
    assert_eq!(lit_p.terms, vec!["FooBar".to_string()]);
    let re_p = ParsedQuery::parse("regex:Foo.*Bar");
    assert_eq!(re_p.terms, vec!["Foo.*Bar".to_string()]);
}
