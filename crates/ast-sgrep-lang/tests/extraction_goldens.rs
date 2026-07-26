use ast_sgrep_lang::{Language, SymbolKind};
use ast_sgrep_testkit::{assert_language_conformance, LanguageConformanceCase};

const RUST: &str = include_str!("fixtures/extract/rust.rs");
const TS: &str = include_str!("fixtures/extract/typescript.ts");
const JS: &str = include_str!("fixtures/extract/javascript.js");
const PY: &str = include_str!("fixtures/extract/python.py");
const GO: &str = include_str!("fixtures/extract/go.go");
const JAVA: &str = include_str!("fixtures/extract/java.java");
const CS: &str = include_str!("fixtures/extract/csharp.cs");
const RB: &str = include_str!("fixtures/extract/ruby.rb");
const SWIFT: &str = include_str!("fixtures/extract/swift.swift");

use SymbolKind::*;

#[test]
fn all_languages_satisfy_shared_parse_extract_and_pattern_contract() {
    for case in CASES {
        assert_language_conformance(case);
    }
}

const CASES: &[LanguageConformanceCase] = &[
    LanguageConformanceCase {
        language: Language::Rust,
        source: RUST,
        symbols: &[
            ("top_level_helper", Function),
            ("new", Method),
            ("process", Method),
            ("GoldenWidget", Type),
            ("GoldenState", Enum),
            ("GoldenRender", Interface),
        ],
        imports: &["std::collections::HashMap"],
        calls: &[("process", "top_level_helper")],
        patterns: &[("function $NAME($$$)", "top_level_helper")],
        forbid: &["doc_only_rust"],
    },
    LanguageConformanceCase {
        language: Language::TypeScript,
        source: TS,
        symbols: &[
            ("makeWidget", Function),
            ("render", Method),
            ("formatWidget", Function),
            ("GoldenWidget", Class),
            ("WidgetName", Type),
            ("WidgetSourceLike", Interface),
            ("WidgetState", Enum),
        ],
        imports: &["lib/widgets"],
        calls: &[("render", "formatWidget"), ("formatWidget", "trim")],
        patterns: &[("function $NAME($$$)", "makeWidget")],
        forbid: &["docOnlyTypeScript"],
    },
    LanguageConformanceCase {
        language: Language::JavaScript,
        source: JS,
        symbols: &[
            ("makeWidget", Function),
            ("render", Method),
            ("formatWidget", Function),
            ("GoldenWidget", Class),
        ],
        imports: &["./widgets.js"],
        calls: &[("render", "formatWidget"), ("formatWidget", "trim")],
        patterns: &[("function $NAME($$$)", "makeWidget")],
        forbid: &["docOnlyJavaScript"],
    },
    LanguageConformanceCase {
        language: Language::Python,
        source: PY,
        symbols: &[
            ("make_widget", Function),
            ("render", Method),
            ("format_widget", Function),
            ("GoldenWidget", Class),
        ],
        imports: &["pathlib.Path"],
        calls: &[("render", "format_widget")],
        patterns: &[("function $NAME($$$)", "make_widget")],
        forbid: &["doc_only_python"],
    },
    LanguageConformanceCase {
        language: Language::Go,
        source: GO,
        symbols: &[
            ("MakeWidget", Function),
            ("Render", Method),
            ("formatWidget", Function),
            ("GoldenWidget", Type),
        ],
        imports: &["fmt"],
        calls: &[("Render", "formatWidget")],
        patterns: &[("function $NAME($$$)", "MakeWidget")],
        forbid: &["docOnlyGo"],
    },
    LanguageConformanceCase {
        language: Language::Java,
        source: JAVA,
        symbols: &[
            ("GoldenWidget", Method),
            ("render", Method),
            ("formatWidget", Method),
            ("GoldenWidget", Class),
        ],
        imports: &["java.util.List"],
        calls: &[("render", "formatWidget"), ("formatWidget", "trim")],
        patterns: &[("function $NAME($$$)", "render")],
        forbid: &["docOnlyJava"],
    },
    LanguageConformanceCase {
        language: Language::CSharp,
        source: CS,
        symbols: &[
            ("Render", Method),
            ("Echo", Method),
            ("Helper", Method),
            ("Move", Method),
            ("Local", Function),
            ("Touch", Method),
            ("GoldenWidget", Class),
            ("GoldenPoint", Type),
            ("GoldenRecord", Class),
            ("GoldenState", Enum),
        ],
        imports: &["System.Text"],
        calls: &[
            ("GoldenWidget", "Helper"),
            ("Render", "Helper"),
            ("Helper", "Trim"),
            ("Move", "Local"),
            ("Local", "Touch"),
        ],
        patterns: &[("function $NAME($$$)", "Render"), ("Local($$$)", "Local")],
        forbid: &["DocOnlyCSharp"],
    },
    LanguageConformanceCase {
        language: Language::Ruby,
        source: RB,
        symbols: &[
            ("make_widget", Function),
            ("render", Method),
            ("format_widget", Function),
            ("GoldenWidget", Class),
        ],
        imports: &["json"],
        calls: &[("render", "format_widget")],
        patterns: &[("function $NAME($$$)", "make_widget")],
        forbid: &["doc_only_ruby"],
    },
    LanguageConformanceCase {
        language: Language::Swift,
        source: SWIFT,
        symbols: &[
            ("GoldenRenderable", Interface),
            ("GoldenWidget", Type),
            ("GoldenWorker", Type),
            ("GoldenState", Enum),
            ("render", Method),
            ("makeWidget", Function),
            ("formatWidget", Function),
        ],
        imports: &["Foundation"],
        calls: &[("render", "formatWidget"), ("makeWidget", "GoldenWidget")],
        patterns: &[
            ("function $NAME($$$)", "makeWidget"),
            ("formatWidget($$$)", "formatWidget"),
        ],
        forbid: &[
            "docOnlySwift",
            "stringOnlySwift",
            "multilineOnlySwift",
            "blockOnlySwift",
        ],
    },
];
