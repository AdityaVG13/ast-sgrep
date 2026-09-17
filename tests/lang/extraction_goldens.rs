use ast_sgrep_lang::{Language, SymbolKind};
use ast_sgrep_testkit::{
    assert_golden_json_at, assert_language_conformance, canonicalize_extraction,
    LanguageConformanceCase,
};
use std::path::Path;

const RUST: &str = include_str!("fixtures/extract/rust.rs");
const TS: &str = include_str!("fixtures/extract/typescript.ts");
const JS: &str = include_str!("fixtures/extract/javascript.js");
const PY: &str = include_str!("fixtures/extract/python.py");
const GO: &str = include_str!("fixtures/extract/go.go");
const JAVA: &str = include_str!("fixtures/extract/java.java");
const CS: &str = include_str!("fixtures/extract/csharp.cs");
const RB: &str = include_str!("fixtures/extract/ruby.rb");
const SWIFT: &str = include_str!("fixtures/extract/swift.swift");
const C: &str = include_str!("fixtures/extract/c.c");
const CPP: &str = include_str!("fixtures/extract/cpp.cpp");
const KT: &str = include_str!("fixtures/extract/kotlin.kt");
const PHP: &str = include_str!("fixtures/extract/php.php");
const DART: &str = include_str!("fixtures/extract/dart.dart");
const MOONBIT: &str = include_str!("fixtures/extract/moonbit.mbt");

use SymbolKind::*;

#[test]
fn all_languages_satisfy_shared_parse_extract_and_pattern_contract() {
    for case in CASES {
        let dump = canonicalize_extraction(assert_language_conformance(case));
        // Full dump is the extra-symbol / kind-name-drift gate; tuples stay presence/forbid.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/lang/fixtures/extract_dumps")
            .join(format!("{}.json", case.language.as_str()));
        assert_golden_json_at(
            &path,
            &serde_json::to_value(&dump).expect("extraction dump serializes"),
        );
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
        patterns: &[("top_level_helper($$$A)", "top_level_helper")],
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
        patterns: &[("makeWidget($$$A)", "makeWidget")],
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
        patterns: &[("makeWidget($$$A)", "makeWidget")],
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
        patterns: &[("make_widget($$$A)", "make_widget")],
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
        patterns: &[("formatWidget($$$A)", "formatWidget")],
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
        patterns: &[("formatWidget($$$A)", "formatWidget")],
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
        patterns: &[("Helper($$$A)", "Helper"), ("Local($$$)", "Local")],
        forbid: &["DocOnlyCSharp"],
    },
    LanguageConformanceCase {
        language: Language::Ruby,
        source: RB,
        symbols: &[
            ("create", Method),
            ("make_widget", Function),
            ("render", Method),
            ("format_widget", Function),
            ("GoldenWidget", Class),
        ],
        imports: &["json"],
        calls: &[
            ("create", "format_widget"),
            ("render", "format_widget"),
            ("render", "make_widget"),
        ],
        patterns: &[
            ("make_widget($$$A)", "make_widget"),
            ("format_widget($$$A)", "format_widget"),
        ],
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
            ("GoldenWidget($$$)", "GoldenWidget"),
            ("formatWidget($$$)", "formatWidget"),
        ],
        forbid: &[
            "docOnlySwift",
            "stringOnlySwift",
            "multilineOnlySwift",
            "blockOnlySwift",
        ],
    },
    LanguageConformanceCase {
        language: Language::C,
        source: C,
        symbols: &[
            ("render", Function),
            ("format_widget", Function),
            ("GoldenWidget", Type),
            ("GoldenState", Enum),
            ("GoldenAlias", Type),
        ],
        imports: &["<stdio.h>", "local.h"],
        calls: &[("render", "helper"), ("format_widget", "render")],
        patterns: &[("render($$$A)", "render")],
        forbid: &["doc_only_c"],
    },
    LanguageConformanceCase {
        language: Language::Cpp,
        source: CPP,
        symbols: &[
            ("render", Method),
            ("move", Method),
            ("make_widget", Function),
            ("GoldenWidget", Class),
            ("GoldenPoint", Type),
            ("GoldenState", Enum),
        ],
        imports: &["<string>", "local.hpp"],
        calls: &[
            ("render", "helper"),
            ("move", "touch"),
            ("make_widget", "render"),
        ],
        patterns: &[("helper($$$A)", "helper")],
        forbid: &["doc_only_cpp"],
    },
    LanguageConformanceCase {
        language: Language::Kotlin,
        source: KT,
        symbols: &[
            ("GoldenRenderable", Interface),
            ("GoldenWidget", Class),
            ("GoldenState", Enum),
            ("render", Method),
            ("makeWidget", Function),
            ("formatWidget", Function),
        ],
        imports: &["kotlin.text.trim"],
        calls: &[
            ("render", "formatWidget"),
            ("formatWidget", "trim"),
            ("makeWidget", "GoldenWidget"),
        ],
        patterns: &[
            ("GoldenWidget($$$)", "GoldenWidget"),
            ("formatWidget($$$)", "formatWidget"),
        ],
        forbid: &["doc_only_kotlin"],
    },
    LanguageConformanceCase {
        language: Language::Php,
        source: PHP,
        symbols: &[
            ("GoldenRenderable", Interface),
            ("GoldenWidget", Class),
            ("GoldenState", Enum),
            ("render", Method),
            ("make_widget", Function),
            ("format_widget", Function),
        ],
        imports: &["App\\Support\\Helper"],
        calls: &[("render", "format_widget"), ("format_widget", "trim")],
        patterns: &[
            ("format_widget($$$A)", "format_widget"),
            ("format_widget($$$)", "format_widget"),
        ],
        forbid: &["doc_only_php"],
    },
    LanguageConformanceCase {
        language: Language::Dart,
        source: DART,
        symbols: &[
            ("GoldenRenderable", Class),
            ("render", Method),
            ("GoldenWidget", Class),
            ("GoldenWidget", Method),
            ("label", Method),
            ("GoldenState", Enum),
            ("makeWidget", Function),
            ("formatWidget", Function),
        ],
        imports: &["dart:async"],
        calls: &[
            ("render", "formatWidget"),
            ("formatWidget", "trim"),
            ("makeWidget", "GoldenWidget"),
        ],
        patterns: &[
            ("function $NAME($$$)", "makeWidget"),
            ("function makeWidget($$$)", "makeWidget"),
            ("function label($$$)", "label"),
            ("formatWidget($$$)", "formatWidget"),
        ],
        forbid: &["doc_only_dart"],
    },
    LanguageConformanceCase {
        language: Language::MoonBit,
        source: MOONBIT,
        symbols: &[
            ("GoldenRenderable", Interface),
            ("GoldenWidget", Type),
            ("GoldenAlias", Type),
            ("GoldenState", Enum),
            ("render", Function),
            ("make_widget", Function),
            ("format_widget", Function),
            ("decorate", Function),
        ],
        imports: &["moonbitlang/core/string"],
        calls: &[
            ("render", "format_widget"),
            ("format_widget", "trim"),
            ("format_widget", "decorate"),
            ("make_widget", "format_widget"),
            ("make_widget", "HttpRequest"),
        ],
        patterns: &[
            ("fn $NAME($$$)", "make_widget"),
            ("fn decorate($$$)", "decorate"),
            ("fn render($$$)", "render"),
            ("interface $NAME", "GoldenRenderable"),
            ("struct $NAME", "GoldenWidget"),
            ("type $NAME", "GoldenAlias"),
            ("format_widget($$$)", "format_widget"),
        ],
        forbid: &["doc_only_moonbit"],
    },
];
