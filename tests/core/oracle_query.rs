//! Core query-parse oracles (consolidated).
//!
//! Consolidates the CAT=query-parse facets of `oracle_foundry_pass{1,2}`
//! (FTS escaping, prefix→mode tables, `in:` scopes) into 2 intent-grouped
//! tests. Every expectation is hand-computed from the documented contract.

use ast_sgrep_core::{fts, path_scope_glob, ParsedQuery, QueryMode};

/// INTENT: FTS terms quote with doubled-quote escaping (incl empty/bare-quote)
/// and queries OR-join the escaped terms (empty→"").
/// KILLS: escape-drop, join-separator, and empty-default mutants.
/// ABSORBS: fts_term_escaping_matches_hand_table (+MERGE fts_query_joins_terms_with_or).
#[test]
fn fts_escaping_and_query_join() {
    // Facet 1: term quoting with doubled-quote escape.
    assert_eq!(fts::escape_fts_term("abc"), "\"abc\"");
    assert_eq!(fts::escape_fts_term("a\"b"), "\"a\"\"b\"");
    assert_eq!(fts::escape_fts_term("\""), "\"\"\"\"");
    assert_eq!(fts::escape_fts_term(""), "\"\"");
    // Facet 2 (MERGE): the query joins escaped terms with OR; empty→"".
    let terms = vec!["foo".to_string(), "a\"b".to_string()];
    assert_eq!(fts::escape_fts_query(&terms), "\"foo\" OR \"a\"\"b\"");
    let empty: Vec<String> = vec![];
    assert_eq!(fts::escape_fts_query(&empty), "");
    let single = vec!["x".to_string()];
    assert_eq!(fts::escape_fts_query(&single), "\"x\"");
}

/// INTENT: prefix→mode/target/terms routing (incl Word-vs-Literal case rules
/// and raw echo) plus `in:` path-scope split with loud refusal.
/// KILLS: prefix-misroute, case-swap, trim, silent-drop fail-open,
/// escape-accept, and glob-passthrough mutants.
/// ABSORBS: parsed_query_mode_table_kills_prefix_mutants,
/// path_scope_splits_and_refuses_loudly.
#[test]
fn parsed_query_modes_scopes_and_targets() {
    // Facet 1: prefix→mode/target/terms table.
    let defs = ParsedQuery::parse("defs:foo");
    assert_eq!(defs.mode, QueryMode::Defs);
    assert_eq!(defs.target.as_deref(), Some("foo"));
    let callers = ParsedQuery::parse("callers: Bar");
    assert_eq!(callers.mode, QueryMode::Callers);
    assert_eq!(callers.target.as_deref(), Some("Bar"));
    assert_eq!(callers.terms, vec!["bar".to_string()]);
    assert_eq!(ParsedQuery::parse("imports:os").mode, QueryMode::Imports);
    let pattern = ParsedQuery::parse("pattern:$A");
    assert_eq!(pattern.mode, QueryMode::Pattern);
    assert_eq!(pattern.terms, vec!["$A".to_string()]);
    let literal = ParsedQuery::parse("literal:Foo");
    assert_eq!(literal.mode, QueryMode::Literal);
    assert_eq!(literal.terms, vec!["Foo".to_string()]);
    assert_eq!(literal.raw, "literal:Foo");
    let regex = ParsedQuery::parse("regex:A+");
    assert_eq!(regex.mode, QueryMode::Regex);
    assert_eq!(regex.terms, vec!["A+".to_string()]);
    let word = ParsedQuery::parse("word:Foo");
    assert_eq!(word.mode, QueryMode::Word);
    assert_eq!(word.terms, vec!["foo".to_string()]);
    let hybrid = ParsedQuery::parse("hello world");
    assert_eq!(hybrid.mode, QueryMode::Hybrid);
    assert_eq!(hybrid.target, None);
    assert_eq!(hybrid.terms, vec!["hello".to_string(), "world".to_string()]);
    let empty = ParsedQuery::parse("");
    assert_eq!(empty.mode, QueryMode::Hybrid);
    assert_eq!(empty.target, None);
    assert!(empty.terms.is_empty());
    // Facet 2: `in:` scope split with loud refusal of bad scopes.
    let scoped = ParsedQuery::parse("foo in:src");
    assert_eq!(scoped.path_scope.as_deref(), Some("src"));
    assert_eq!(scoped.path_scope_error, None);
    assert_eq!(scoped.terms, vec!["foo".to_string()]);
    assert_eq!(ParsedQuery::parse("foo in:").path_scope_error.is_some(), true);
    assert_eq!(ParsedQuery::parse("foo in:").path_scope, None);
    assert!(ParsedQuery::parse("a in:x in:y").path_scope_error.is_some());
    assert!(ParsedQuery::parse("a in:../up").path_scope_error.is_some());
    let abs = tempfile::tempdir().expect("tempdir");
    let abs_query = format!("a in:{}", abs.path().display());
    assert!(ParsedQuery::parse(&abs_query).path_scope_error.is_some());
    let quoted = ParsedQuery::parse("\"a in:src\" b");
    assert_eq!(quoted.path_scope, None);
    assert_eq!(quoted.path_scope_error, None);
    assert_eq!(path_scope_glob("src"), "src/**");
    assert_eq!(path_scope_glob("src/"), "src/**");
    assert_eq!(path_scope_glob("a*b"), "a*b");
    assert_eq!(path_scope_glob("a?"), "a?");
}
