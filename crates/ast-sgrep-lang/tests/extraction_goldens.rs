use ast_sgrep_lang::{ExtractionResult, ImportSite, Language, SymbolKind}; use ast_sgrep_testkit::parse;
type Sym = (&'static str, SymbolKind); type Call = (&'static str, &'static str); struct Case {
    language: Language, source: &'static str, symbols: &'static [Sym], imports: &'static [&'static str], calls: &'static [Call], forbid: &'static [&'static str],
}
const RUST: &str = include_str!("fixtures/extract/rust.rs"); const TS: &str = include_str!("fixtures/extract/typescript.ts");
const JS: &str = include_str!("fixtures/extract/javascript.js"); const PY: &str = include_str!("fixtures/extract/python.py");
const GO: &str = include_str!("fixtures/extract/go.go"); const JAVA: &str = include_str!("fixtures/extract/java.java"); const CS: &str = include_str!("fixtures/extract/csharp.cs"); const RB: &str = include_str!("fixtures/extract/ruby.rb");
use SymbolKind::*;
#[test] fn extractors_emit_language_goldens_with_sane_spans_and_relationships() {
    for c in CASES {
        let r = parse(c.language, c.source); for &(name, kind) in c.symbols {
            assert!(r.symbols.iter().any(|s| s.name == name && s.kind == kind),
                "{} must emit {:?} {}; got {:?}", c.language, kind, name, r.symbols);
        } for &imp in c.imports {
            assert!(r.imports.iter().any(|i| i.module_path == imp),
                "{} must emit import {}; got {:?}", c.language, imp, mods(&r.imports));
        } for &(caller, callee) in c.calls {
            assert!(r.calls.iter().any(|x| x.caller == caller && x.callee == callee),
                "{} must preserve {} -> {}; got {:?}", c.language, caller, callee, r.calls);
        } assert_spans(c, &r); for term in c.forbid {
            assert!(!r.symbols.iter().any(|s| s.name == *term), "{} must not emit symbol {term}", c.language); assert!(!r.calls.iter().any(|x| x.callee == *term), "{} must not emit call {term}", c.language);
            assert!(!r.imports.iter().any(|i| i.module_path.contains(term)), "{} must not emit import {term}", c.language);
        }
    }
}
const CASES: &[Case] = &[
    Case {
        language: Language::Rust, source: RUST, symbols: &[("top_level_helper", Function), ("new", Method), ("process", Method),
            ("GoldenWidget", Type), ("GoldenState", Enum), ("GoldenRender", Interface)],
        imports: &["std::collections::HashMap"], calls: &[("process", "top_level_helper")], forbid: &["doc_only_rust"],
    }, Case {
        language: Language::TypeScript, source: TS, symbols: &[("makeWidget", Function), ("render", Method), ("formatWidget", Function),
            ("GoldenWidget", Class), ("WidgetName", Type), ("WidgetSourceLike", Interface), ("WidgetState", Enum)],
        imports: &["lib/widgets"], calls: &[("render", "formatWidget"), ("formatWidget", "trim")], forbid: &["docOnlyTypeScript"],
    }, Case {
        language: Language::JavaScript, source: JS, symbols: &[("makeWidget", Function), ("render", Method), ("formatWidget", Function), ("GoldenWidget", Class)],
        imports: &["./widgets.js"], calls: &[("render", "formatWidget"), ("formatWidget", "trim")], forbid: &["docOnlyJavaScript"],
    }, Case {
        language: Language::Python, source: PY, symbols: &[("make_widget", Function), ("render", Method), ("format_widget", Function), ("GoldenWidget", Class)],
        imports: &["pathlib.Path"], calls: &[("render", "format_widget")], forbid: &["doc_only_python"],
    }, Case {
        language: Language::Go, source: GO, symbols: &[("MakeWidget", Function), ("Render", Method), ("formatWidget", Function), ("GoldenWidget", Type)], imports: &["fmt"], calls: &[("Render", "formatWidget")], forbid: &["docOnlyGo"],
    }, Case {
        language: Language::Java, source: JAVA, symbols: &[("GoldenWidget", Method), ("render", Method), ("formatWidget", Method), ("GoldenWidget", Class)],
        imports: &["java.util.List"], calls: &[("render", "formatWidget"), ("formatWidget", "trim")], forbid: &["docOnlyJava"],
    }, Case {
        language: Language::CSharp, source: CS, symbols: &[("Render", Method), ("Helper", Method), ("GoldenWidget", Class)], imports: &["System.Text"], calls: &[("Render", "Helper"), ("Helper", "Trim")], forbid: &["DocOnlyCSharp"],
    }, Case {
        language: Language::Ruby, source: RB, symbols: &[("make_widget", Function), ("render", Method), ("format_widget", Function), ("GoldenWidget", Class)],
        imports: &["json"], calls: &[("render", "format_widget")], forbid: &["doc_only_ruby"],
    },
];
fn assert_spans(c: &Case, r: &ExtractionResult) {
    let lines = c.source.lines().count() as u32; let bytes = c.source.len(); for s in &r.symbols {
        assert!(s.line_start >= 1 && s.line_start <= s.line_end && s.line_end <= lines,
            "{} {} bad line span {}..{} / {lines}", c.language, s.name, s.line_start, s.line_end);
        assert!(s.byte_start < s.byte_end && s.byte_end <= bytes,
            "{} {} bad byte span {}..{} / {bytes}", c.language, s.name, s.byte_start, s.byte_end);
        assert!(c.source[s.byte_start..s.byte_end].contains(&s.name),
            "{} {} span must cover name", c.language, s.name);
    } for call in &r.calls {
        assert!(call.line >= 1 && call.line <= lines, "{} call line {}", c.language, call.line); assert!(call.byte_start < call.byte_end && call.byte_end <= bytes, "{} call byte span", c.language);
    }
} fn mods(imports: &[ImportSite]) -> Vec<&str> { imports.iter().map(|i| i.module_path.as_str()).collect() }
