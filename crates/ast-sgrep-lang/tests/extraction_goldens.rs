use ast_sgrep_lang::{ExtractionResult, ImportSite, Language, SymbolKind};
use ast_sgrep_testkit::parse;

#[derive(Clone, Copy)]
struct ExpectedSymbol {
    name: &'static str,
    kind: SymbolKind,
}

#[derive(Clone, Copy)]
struct ExpectedCall {
    caller: &'static str,
    callee: &'static str,
}

struct GoldenCase {
    language: Language,
    source: &'static str,
    expected_symbols: &'static [ExpectedSymbol],
    expected_imports: &'static [&'static str],
    expected_calls: &'static [ExpectedCall],
    forbidden_comment_terms: &'static [&'static str],
}

const RUST_SOURCE: &str = include_str!("fixtures/extract/rust.rs");
const TYPESCRIPT_SOURCE: &str = include_str!("fixtures/extract/typescript.ts");
const JAVASCRIPT_SOURCE: &str = include_str!("fixtures/extract/javascript.js");
const PYTHON_SOURCE: &str = include_str!("fixtures/extract/python.py");
const GO_SOURCE: &str = include_str!("fixtures/extract/go.go");
const JAVA_SOURCE: &str = include_str!("fixtures/extract/java.java");
const CSHARP_SOURCE: &str = include_str!("fixtures/extract/csharp.cs");
const RUBY_SOURCE: &str = include_str!("fixtures/extract/ruby.rb");

#[test]
fn extractors_emit_language_goldens_with_sane_spans_and_relationships() {
    for case in golden_cases() {
        let result = parse(case.language, case.source);

        assert_expected_symbols(&case, &result);
        assert_expected_imports(&case, &result);
        assert_expected_calls(&case, &result);
        assert_spans_are_sane(&case, &result);
        assert_doc_comments_are_not_indexed_as_code(&case, &result);
    }
}

fn golden_cases() -> Vec<GoldenCase> {
    vec![
        GoldenCase {
            language: Language::Rust,
            source: RUST_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "top_level_helper",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "new",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "process",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Type,
                },
                ExpectedSymbol {
                    name: "GoldenState",
                    kind: SymbolKind::Enum,
                },
                ExpectedSymbol {
                    name: "GoldenRender",
                    kind: SymbolKind::Interface,
                },
            ],
            expected_imports: &["std::collections::HashMap"],
            expected_calls: &[ExpectedCall {
                caller: "process",
                callee: "top_level_helper",
            }],
            forbidden_comment_terms: &["doc_only_rust"],
        },
        GoldenCase {
            language: Language::TypeScript,
            source: TYPESCRIPT_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "makeWidget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "render",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "formatWidget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Class,
                },
                ExpectedSymbol {
                    name: "WidgetName",
                    kind: SymbolKind::Type,
                },
                ExpectedSymbol {
                    name: "WidgetSourceLike",
                    kind: SymbolKind::Interface,
                },
                ExpectedSymbol {
                    name: "WidgetState",
                    kind: SymbolKind::Enum,
                },
            ],
            expected_imports: &["lib/widgets"],
            expected_calls: &[
                ExpectedCall {
                    caller: "render",
                    callee: "formatWidget",
                },
                ExpectedCall {
                    caller: "formatWidget",
                    callee: "trim",
                },
            ],
            forbidden_comment_terms: &["docOnlyTypeScript"],
        },
        GoldenCase {
            language: Language::JavaScript,
            source: JAVASCRIPT_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "makeWidget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "render",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "formatWidget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Class,
                },
            ],
            expected_imports: &["./widgets.js"],
            expected_calls: &[
                ExpectedCall {
                    caller: "render",
                    callee: "formatWidget",
                },
                ExpectedCall {
                    caller: "formatWidget",
                    callee: "trim",
                },
            ],
            forbidden_comment_terms: &["docOnlyJavaScript"],
        },
        GoldenCase {
            language: Language::Python,
            source: PYTHON_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "make_widget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "render",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "format_widget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Class,
                },
            ],
            expected_imports: &["pathlib.Path"],
            expected_calls: &[ExpectedCall {
                caller: "render",
                callee: "format_widget",
            }],
            forbidden_comment_terms: &["doc_only_python"],
        },
        GoldenCase {
            language: Language::Go,
            source: GO_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "MakeWidget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "Render",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "formatWidget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Type,
                },
            ],
            expected_imports: &["fmt"],
            expected_calls: &[ExpectedCall {
                caller: "Render",
                callee: "formatWidget",
            }],
            forbidden_comment_terms: &["docOnlyGo"],
        },
        GoldenCase {
            language: Language::Java,
            source: JAVA_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "render",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "formatWidget",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Class,
                },
            ],
            expected_imports: &["java.util.List"],
            expected_calls: &[
                ExpectedCall {
                    caller: "render",
                    callee: "formatWidget",
                },
                ExpectedCall {
                    caller: "formatWidget",
                    callee: "trim",
                },
            ],
            forbidden_comment_terms: &["docOnlyJava"],
        },
        GoldenCase {
            language: Language::CSharp,
            source: CSHARP_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "Render",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "Helper",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Class,
                },
            ],
            expected_imports: &["System.Text"],
            expected_calls: &[
                ExpectedCall {
                    caller: "Render",
                    callee: "Helper",
                },
                ExpectedCall {
                    caller: "Helper",
                    callee: "Trim",
                },
            ],
            forbidden_comment_terms: &["DocOnlyCSharp"],
        },
        GoldenCase {
            language: Language::Ruby,
            source: RUBY_SOURCE,
            expected_symbols: &[
                ExpectedSymbol {
                    name: "make_widget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "render",
                    kind: SymbolKind::Method,
                },
                ExpectedSymbol {
                    name: "format_widget",
                    kind: SymbolKind::Function,
                },
                ExpectedSymbol {
                    name: "GoldenWidget",
                    kind: SymbolKind::Class,
                },
            ],
            expected_imports: &["json"],
            expected_calls: &[ExpectedCall {
                caller: "render",
                callee: "format_widget",
            }],
            forbidden_comment_terms: &["doc_only_ruby"],
        },
    ]
}

fn assert_expected_symbols(case: &GoldenCase, result: &ExtractionResult) {
    for expected in case.expected_symbols {
        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == expected.name && symbol.kind == expected.kind),
            "{} extractor must emit {:?} symbol {}; got {:?}",
            case.language,
            expected.kind,
            expected.name,
            result.symbols
        );
    }
}

fn assert_expected_imports(case: &GoldenCase, result: &ExtractionResult) {
    for expected in case.expected_imports {
        assert!(
            result
                .imports
                .iter()
                .any(|import| import.module_path == *expected),
            "{} extractor must emit import {}; got {:?}",
            case.language,
            expected,
            import_modules(&result.imports)
        );
    }
}

fn assert_expected_calls(case: &GoldenCase, result: &ExtractionResult) {
    for expected in case.expected_calls {
        assert!(
            result
                .calls
                .iter()
                .any(|call| call.caller == expected.caller && call.callee == expected.callee),
            "{} extractor must preserve caller {} -> callee {} relationship; got {:?}",
            case.language,
            expected.caller,
            expected.callee,
            result.calls
        );
    }
}

fn assert_spans_are_sane(case: &GoldenCase, result: &ExtractionResult) {
    let line_count = case.source.lines().count() as u32;
    let byte_len = case.source.len();

    for symbol in &result.symbols {
        assert!(
            symbol.line_start >= 1
                && symbol.line_start <= symbol.line_end
                && symbol.line_end <= line_count,
            "{} symbol {} must have a valid line span within the fixture; got {}..{} over {} lines",
            case.language,
            symbol.name,
            symbol.line_start,
            symbol.line_end,
            line_count
        );
        assert!(
            symbol.byte_start < symbol.byte_end && symbol.byte_end <= byte_len,
            "{} symbol {} must have a valid byte span; got {}..{} over {} bytes",
            case.language,
            symbol.name,
            symbol.byte_start,
            symbol.byte_end,
            byte_len
        );
        let excerpt = &case.source[symbol.byte_start..symbol.byte_end];
        assert!(
            excerpt.contains(&symbol.name),
            "{} symbol {} span must cover its declared name; excerpt was {:?}",
            case.language,
            symbol.name,
            excerpt
        );
    }

    for call in &result.calls {
        assert!(
            call.line >= 1 && call.line <= line_count,
            "{} call {} -> {} must have a line inside the fixture; got {} over {} lines",
            case.language,
            call.caller,
            call.callee,
            call.line,
            line_count
        );
        assert!(
            call.byte_start < call.byte_end && call.byte_end <= byte_len,
            "{} call {} -> {} must have a valid byte span; got {}..{} over {} bytes",
            case.language,
            call.caller,
            call.callee,
            call.byte_start,
            call.byte_end,
            byte_len
        );
    }
}

fn assert_doc_comments_are_not_indexed_as_code(case: &GoldenCase, result: &ExtractionResult) {
    for term in case.forbidden_comment_terms {
        assert!(
            !result.symbols.iter().any(|symbol| symbol.name == *term),
            "{} doc/comment-only name {} must not be emitted as a symbol",
            case.language,
            term
        );
        assert!(
            !result.calls.iter().any(|call| call.callee == *term),
            "{} doc/comment-only name {} must not be emitted as a call",
            case.language,
            term
        );
        assert!(
            !result
                .imports
                .iter()
                .any(|import| import.module_path.contains(term)),
            "{} doc/comment-only name {} must not be emitted as an import",
            case.language,
            term
        );
    }
}

fn import_modules(imports: &[ImportSite]) -> Vec<&str> {
    imports
        .iter()
        .map(|import| import.module_path.as_str())
        .collect()
}
