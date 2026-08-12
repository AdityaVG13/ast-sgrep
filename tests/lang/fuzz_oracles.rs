//! Durable checks for native parse / classify APIs used by cargo-fuzz targets.

use ast_sgrep_lang::{classify_native, needs_ast_grep_fallback, Language, ParserRegistry};
use std::sync::OnceLock;

fn registry() -> &'static ParserRegistry {
    static REG: OnceLock<ParserRegistry> = OnceLock::new();
    REG.get_or_init(ParserRegistry::new)
}

#[test]
fn lang_parse_polyglot_snippets_do_not_panic() {
    let samples = [
        (Language::Rust, "fn main() { let x = 1; }"),
        (Language::Python, "def foo(x):\n    return x\n"),
        (Language::JavaScript, "function bar(a) { return a; }"),
        (Language::Go, "package main\nfunc Hello() {}\n"),
        (Language::Java, "class Foo { void bar() {} }\n"),
    ];
    for (lang, src) in samples {
        let _ = registry().parse(lang, src);
    }
}

#[test]
fn classify_native_consistency_with_fallback() {
    for p in [
        "fn $NAME() {}",
        "class Foo",
        "def $F",
        "foo.bar($X)",
        "no dollars",
    ] {
        let kind = classify_native(p);
        let needs = needs_ast_grep_fallback(p);
        if kind.is_some() {
            assert!(!needs, "native Some must not need fallback for {p:?}");
        }
        if !p.contains('$') {
            assert!(!needs);
        }
    }
}
