use super::*;

/// ghiw.2 QG-001…026 — see `docs/QUERY_GRAMMAR.md`.
#[test]
fn qg_must_matrix() {
    struct Row {
        id: &'static str,
        input: &'static str,
        mode: QueryMode,
        raw: &'static str,
        target: Option<&'static str>,
    }
    let rows = [
        Row {
            id: "QG-001",
            input: "process_request",
            mode: QueryMode::Hybrid,
            raw: "process_request",
            target: None,
        },
        Row {
            id: "QG-002",
            input: "callers:RefreshToken",
            mode: QueryMode::Callers,
            raw: "callers:RefreshToken",
            target: Some("RefreshToken"),
        },
        Row {
            id: "QG-003",
            input: "defs:auth_refresh",
            mode: QueryMode::Defs,
            raw: "defs:auth_refresh",
            target: Some("auth_refresh"),
        },
        Row {
            id: "QG-004",
            input: "imports:./Utils",
            mode: QueryMode::Imports,
            raw: "imports:./Utils",
            target: Some("./Utils"),
        },
        Row {
            id: "QG-005",
            input: "pattern:function $NAME($$$)",
            mode: QueryMode::Pattern,
            raw: "pattern:function $NAME($$$)",
            target: Some("function $NAME($$$)"),
        },
        Row {
            id: "QG-006",
            input: "literal:FooBar",
            mode: QueryMode::Literal,
            raw: "literal:FooBar",
            target: Some("FooBar"),
        },
        Row {
            id: "QG-007",
            input: "regex:Foo.*Bar",
            mode: QueryMode::Regex,
            raw: "regex:Foo.*Bar",
            target: Some("Foo.*Bar"),
        },
        Row {
            id: "QG-008",
            input: "word:Token",
            mode: QueryMode::Word,
            raw: "word:Token",
            target: Some("Token"),
        },
        Row {
            id: "QG-011",
            input: "callers:",
            mode: QueryMode::Callers,
            raw: "callers:",
            target: Some(""),
        },
        Row {
            id: "QG-011b",
            input: "pattern:",
            mode: QueryMode::Pattern,
            raw: "pattern:",
            target: Some(""),
        },
        Row {
            id: "QG-012",
            input: "defs:  auth",
            mode: QueryMode::Defs,
            raw: "defs:  auth",
            target: Some("auth"),
        },
        Row {
            id: "QG-020",
            input: "sem:foo",
            mode: QueryMode::Hybrid,
            raw: "sem:foo",
            target: None,
        },
        Row {
            id: "QG-021",
            input: "path:src/",
            mode: QueryMode::Hybrid,
            raw: "path:src/",
            target: None,
        },
        Row {
            id: "QG-022",
            input: "lang:rust foo",
            mode: QueryMode::Hybrid,
            raw: "lang:rust foo",
            target: None,
        },
        Row {
            id: "QG-023",
            input: "callers:Foo defs:Bar",
            mode: QueryMode::Callers,
            raw: "callers:Foo defs:Bar",
            target: Some("Foo defs:Bar"),
        },
        Row {
            id: "QG-024",
            input: "(defs:Foo AND callers:Bar)",
            mode: QueryMode::Hybrid,
            raw: "(defs:Foo AND callers:Bar)",
            target: None,
        },
        Row {
            id: "QG-025",
            input: "Callers:Foo",
            mode: QueryMode::Hybrid,
            raw: "Callers:Foo",
            target: None,
        },
        Row {
            id: "QG-026",
            input: "xyzzy:Foo",
            mode: QueryMode::Hybrid,
            raw: "xyzzy:Foo",
            target: None,
        },
    ];
    for row in rows {
        let p = ParsedQuery::parse(row.input);
        assert_eq!(p.mode, row.mode, "{} mode for {:?}", row.id, row.input);
        assert_eq!(p.raw, row.raw, "{} raw for {:?}", row.id, row.input);
        assert_eq!(
            p.target.as_deref(),
            row.target,
            "{} target for {:?}",
            row.id,
            row.input
        );
        if row.mode == QueryMode::Literal {
            assert_eq!(p.terms, vec!["FooBar".to_string()], "{}", row.id);
        }
        if row.mode == QueryMode::Regex {
            assert_eq!(p.terms, vec!["Foo.*Bar".to_string()], "{}", row.id);
        }
        if row.mode == QueryMode::Word {
            assert_eq!(p.terms, vec!["token".to_string()], "{}", row.id);
        }
        if row.mode == QueryMode::Pattern {
            assert_eq!(
                p.terms,
                vec![row.target.unwrap_or_default().to_string()],
                "{}",
                row.id
            );
        }
    }
}

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
