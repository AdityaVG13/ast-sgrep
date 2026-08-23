use super::*;

#[test]
fn classifies_common_metavariable_shapes() {
    assert!(classify_native("fn $NAME($$$)").is_some());
    assert!(classify_native("def $NAME").is_some());
    assert!(classify_native("$OBJ.$METHOD($$$)").is_some());
    assert!(classify_native("foo($$$)").is_some());
    assert!(classify_native("process_request($$$)").is_some());
}

#[test]
fn classifies_nested_statement_templates() {
    // If templates: paren, brace, and colon forms normalize to the same kind.
    assert_eq!(
        classify_native("if ($COND) { $BODY }"),
        Some(NativeKind::If {
            body: Some(BodyTemplate::Exactly(1)),
        })
    );
    assert_eq!(
        classify_native("if $COND { $BODY }"),
        Some(NativeKind::If {
            body: Some(BodyTemplate::Exactly(1)),
        })
    );
    assert_eq!(
        classify_native("if $COND: $BODY"),
        Some(NativeKind::If {
            body: Some(BodyTemplate::Exactly(1)),
        })
    );
    assert_eq!(
        classify_native("if ($COND) { $$$ }"),
        Some(NativeKind::If {
            body: Some(BodyTemplate::Any),
        })
    );
    assert_eq!(
        classify_native("if ($COND)"),
        Some(NativeKind::If { body: None })
    );
    // Function body templates.
    assert_eq!(
        classify_native("fn $N($$$) { $STMT }"),
        Some(NativeKind::Function {
            name: None,
            body: Some(BodyTemplate::Exactly(1)),
        })
    );
    assert_eq!(
        classify_native("fn process($$$) {}"),
        Some(NativeKind::Function {
            name: Some("process".to_string()),
            body: Some(BodyTemplate::Exactly(0)),
        })
    );
    assert_eq!(
        classify_native("fn $N($$$) { $$$BODY }"),
        Some(NativeKind::Function {
            name: None,
            body: Some(BodyTemplate::Any),
        })
    );
}

#[test]
fn unsupported_nested_shapes_stay_out_of_subset() {
    // Concrete conditions are out (fail-closed, never a call to `if`).
    assert!(classify_native("if (x > 0) { $BODY }").is_none());
    // Multi-statement bodies are out.
    assert!(classify_native("if ($COND) { $A; $B }").is_none());
    assert!(classify_native("fn $N($$$) { $A; $B }").is_none());
    // Statement-count templates on type bodies are out.
    assert!(classify_native("struct $N { $FIELD }").is_none());
    // `iffy(...)` is a call, not an if template.
    assert!(matches!(
        classify_native("iffy($$$)"),
        Some(NativeKind::Call { .. })
    ));
}

#[test]
fn function_declaration_tails_fail_closed() {
    for malformed in [
        "fn $NAME($$$",
        "fn $NAME($$$) trailing",
        "def $NAME nonsense",
        "fn $NAME(concrete)",
        "fn $NAME($ARG) garbage",
    ] {
        assert!(
            classify_native(malformed).is_none(),
            "accepted {malformed:?}"
        );
    }
    assert!(classify_native("def $NAME").is_some());
    assert!(classify_native("fn $NAME($$$)").is_some());
    assert!(classify_native("fn $NAME($$$) { $STMT }").is_some());
    assert!(classify_native("def $NAME($ARG): $BODY").is_some());
}

#[test]
fn native_fn_meta_matches_rust() {
    let src = "fn process_request(x: i32) {}\nfn other() {}\n";
    let hits = match_pattern(Language::Rust, src, "fn $NAME($$$)").unwrap();
    assert!(hits.len() >= 2, "hits={hits:?}");
}

#[test]
fn declaration_modifiers_match_in_process() {
    let rust = r#"
struct Hidden {
    value: i32,
}
pub struct Visible {
    value: i32,
}
fn hidden() {}
pub(crate) fn scoped() {}
"#;

    let public_struct =
        match_pattern(Language::Rust, rust, "pub struct $NAME { $$$BODY }").unwrap();
    assert_eq!(public_struct.len(), 1, "hits={public_struct:?}");
    assert_eq!(public_struct[0].captures["NAME"], "Visible");
    assert!(!needs_ast_grep_fallback("pub struct $NAME { $$$BODY }"));

    let scoped_fn =
        match_pattern(Language::Rust, rust, "pub(crate) fn $NAME($$$) { $$$BODY }").unwrap();
    assert_eq!(scoped_fn.len(), 1, "hits={scoped_fn:?}");
    assert_eq!(scoped_fn[0].captures["NAME"], "scoped");
}

#[test]
fn kernel_pattern_matrix_covers_rust_typescript_and_python() {
    let cases = [
        (
            Language::Rust,
            "struct Service { value: i32 }\nfn dispatch() { service_call(); }\n",
            "struct $NAME { $$$BODY }",
            "fn $NAME($$$) { $$$BODY }",
            "service_call($$$)",
        ),
        (
            Language::TypeScript,
            "class Service { run() { serviceCall(); } }\nfunction dispatch() { serviceCall(); }\n",
            "class $NAME { $$$BODY }",
            "function $NAME($$$) { $$$BODY }",
            "serviceCall($$$)",
        ),
        (
            Language::Python,
            "class Service:\n    def run(self):\n        service_call()\n\ndef dispatch():\n    service_call()\n",
            "class $NAME: $$$BODY",
            "def $NAME($$$): $$$BODY",
            "service_call($$$)",
        ),
    ];

    for (language, source, declaration, function, call) in cases {
        assert!(
            classify_native(declaration).is_some(),
            "unclassified declaration pattern {declaration:?}"
        );
        for pattern in [declaration, function, call] {
            let hits = match_pattern(language, source, pattern).unwrap();
            assert!(
                !hits.is_empty(),
                "language={language:?} pattern={pattern:?}"
            );
            assert!(
                !needs_ast_grep_fallback(pattern),
                "language={language:?} pattern={pattern:?}"
            );
        }
    }
}

#[test]
fn native_call_matches_exact_callee() {
    let src = "fn main() { process_request(1); other(2); }\n";
    let hits = match_pattern(Language::Rust, src, "process_request($$$)").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].excerpt.contains("process_request"));
}

#[test]
fn argument_templates_constrain_and_capture_calls() {
    let src = "fn main() { legacy(); legacy(alpha); legacy(alpha, beta); }\n";
    let empty = match_pattern(Language::Rust, src, "legacy()").unwrap();
    assert!(
        empty.is_empty(),
        "patterns without metavariables are literal"
    );

    let one = match_pattern(Language::Rust, src, "legacy($ARG)").unwrap();
    assert_eq!(one.len(), 1, "one={one:?}");
    assert_eq!(one[0].captures["ARG"], "alpha");

    let two = match_pattern(Language::Rust, src, "legacy($LEFT, $RIGHT)").unwrap();
    assert_eq!(two.len(), 1, "two={two:?}");
    assert_eq!(two[0].captures["LEFT"], "alpha");
    assert_eq!(two[0].captures["RIGHT"], "beta");

    let any = match_pattern(Language::Rust, src, "legacy($$$ARGS)").unwrap();
    assert_eq!(any.len(), 3, "any={any:?}");
    assert_eq!(any[0].captures["ARGS"], "");
    assert_eq!(any[2].captures["ARGS"], "alpha, beta");
}

/// `self.helper()` / `this.render()` are two-segment method calls: keyword
/// receivers must satisfy `$OBJ` exactly like identifier receivers (ast-grep
/// agrees on this match set).
#[test]
fn wildcard_method_call_matches_keyword_receivers() {
    let rust = "impl App {\n    fn tick(&self) {\n        self.helper();\n    }\n}\nfn f(app: App) {\n    app.tick();\n}\n";
    let hits = match_pattern(Language::Rust, rust, "$OBJ.$METHOD($$$)").unwrap();
    let lines: Vec<u32> = hits.iter().map(|h| h.line_start).collect();
    assert_eq!(lines, [3, 7], "hits={hits:?}");
    assert!(hits[0].excerpt.contains("self.helper"), "hits={hits:?}");

    let ts = "class W {\n    render() {\n        this.draw();\n    }\n}\n";
    let ts_hits = match_pattern(Language::TypeScript, ts, "$OBJ.$METHOD($$$)").unwrap();
    assert!(
        ts_hits.iter().any(|h| h.excerpt.contains("this.draw")),
        "ts hits={ts_hits:?}"
    );
}

#[test]
fn fn_body_template_counts_statements_rust() {
    let src = "fn one() { tick(); }\nfn two() { tick(); tock(); }\nfn empty() {}\n";
    let one = match_pattern(Language::Rust, src, "fn $N($$$) { $STMT }").unwrap();
    assert_eq!(one.len(), 1, "one={one:?}");
    assert!(one[0].excerpt.contains("fn one"));
    let empty = match_pattern(Language::Rust, src, "fn $N($$$) {}").unwrap();
    assert_eq!(empty.len(), 1, "empty={empty:?}");
    assert!(empty[0].excerpt.contains("fn empty"));
    let any = match_pattern(Language::Rust, src, "fn $N($$$) { $$$ }").unwrap();
    assert_eq!(any.len(), 3, "any={any:?}");
}

#[test]
fn if_template_matches_across_languages() {
    let rust = "fn f(x: i32) {\n    if x > 0 { tick(); }\n    if x < 0 { tick(); tock(); }\n}\n";
    let single = match_pattern(Language::Rust, rust, "if $COND { $BODY }").unwrap();
    assert_eq!(single.len(), 1, "single={single:?}");
    assert_eq!(single[0].line_start, 2);
    // Paren form normalizes to the same template.
    let paren = match_pattern(Language::Rust, rust, "if ($COND) { $BODY }").unwrap();
    assert_eq!(paren, single);
    let any = match_pattern(Language::Rust, rust, "if ($COND) { $$$ }").unwrap();
    assert_eq!(any.len(), 2, "any={any:?}");

    let ts =
        "function f(x: number) {\n  if (x > 0) { tick(); }\n  if (x < 0) { tick(); tock(); }\n}\n";
    let ts_hits = match_pattern(Language::TypeScript, ts, "if ($COND) { $BODY }").unwrap();
    assert_eq!(ts_hits.len(), 1, "ts_hits={ts_hits:?}");
    assert_eq!(ts_hits[0].line_start, 2);

    let py =
        "def f(x):\n    if x > 0:\n        tick()\n    if x < 0:\n        tick()\n        tock()\n";
    let py_hits = match_pattern(Language::Python, py, "if $COND: $BODY").unwrap();
    assert_eq!(py_hits.len(), 1, "py_hits={py_hits:?}");
    assert_eq!(py_hits[0].line_start, 2);
    // Brace form matches Python too (template semantics, not token syntax).
    let py_brace = match_pattern(Language::Python, py, "if ($COND) { $BODY }").unwrap();
    assert_eq!(py_brace, py_hits);
}

#[test]
fn if_template_skips_strings_and_counts_comments_as_trivia() {
    let src = "fn f(x: i32) {\n    let _ = \"if x { y() }\";\n    if x > 0 {\n        // explains\n        tick();\n    }\n}\n";
    let hits = match_pattern(Language::Rust, src, "if $COND { $BODY }").unwrap();
    assert_eq!(hits.len(), 1, "hits={hits:?}");
    assert_eq!(hits[0].line_start, 3);
}
