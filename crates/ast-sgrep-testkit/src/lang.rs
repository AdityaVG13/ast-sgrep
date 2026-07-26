use ast_sgrep_lang::{
    match_pattern, parse_is_error_free, ExtractionResult, Language, ParserRegistry, SymbolKind,
};

pub type ExpectedSymbol = (&'static str, SymbolKind);
pub type ExpectedCall = (&'static str, &'static str);
pub type ExpectedPattern = (&'static str, &'static str);

/// Shared conformance contract for every supported language.
pub struct LanguageConformanceCase {
    pub language: Language,
    pub source: &'static str,
    pub symbols: &'static [ExpectedSymbol],
    pub imports: &'static [&'static str],
    pub calls: &'static [ExpectedCall],
    pub patterns: &'static [ExpectedPattern],
    pub forbid: &'static [&'static str],
}

pub fn parse(lang: Language, source: &str) -> ExtractionResult {
    ParserRegistry::new().parse(lang, source).expect("parse")
}

pub fn assert_language_conformance(case: &LanguageConformanceCase) -> ExtractionResult {
    assert!(
        parse_is_error_free(case.language, case.source).expect("parse fidelity"),
        "{} fixture must parse without ERROR nodes",
        case.language
    );
    let result = parse(case.language, case.source);
    for &(name, kind) in case.symbols {
        assert!(
            result
                .symbols
                .iter()
                .any(|symbol| symbol.name == name && symbol.kind == kind),
            "{} must emit {kind:?} {name}; got {:?}",
            case.language,
            result.symbols
        );
    }
    for &module in case.imports {
        assert!(
            result
                .imports
                .iter()
                .any(|import| import.module_path == module),
            "{} must emit import {module}; got {:?}",
            case.language,
            result.imports
        );
    }
    for &(caller, callee) in case.calls {
        assert!(
            result
                .calls
                .iter()
                .any(|call| call.caller == caller && call.callee == callee),
            "{} must preserve {caller} -> {callee}; got {:?}",
            case.language,
            result.calls
        );
    }
    for &(pattern, expected) in case.patterns {
        let hits = match_pattern(case.language, case.source, pattern).expect("pattern match");
        assert!(
            hits.iter().any(|hit| hit.excerpt.contains(expected)),
            "{} pattern {pattern:?} must match {expected}; got {hits:?}",
            case.language
        );
    }
    assert_spans(case, &result);
    for term in case.forbid {
        assert!(
            !result.symbols.iter().any(|symbol| symbol.name == *term),
            "{} must not emit symbol {term}",
            case.language
        );
        assert!(
            !result.calls.iter().any(|call| call.callee == *term),
            "{} must not emit call {term}",
            case.language
        );
        assert!(
            !result
                .imports
                .iter()
                .any(|import| import.module_path.contains(term)),
            "{} must not emit import {term}",
            case.language
        );
    }
    result
}

pub fn assert_has_symbol(result: &ExtractionResult, name: &str) {
    assert!(
        result.symbols.iter().any(|s| s.name == name),
        "missing symbol {name}"
    );
}

pub fn assert_has_callee(result: &ExtractionResult, callee: &str) {
    assert!(
        result.calls.iter().any(|c| c.callee == callee),
        "missing callee {callee}"
    );
}

fn assert_spans(case: &LanguageConformanceCase, result: &ExtractionResult) {
    let lines = case.source.lines().count() as u32;
    let bytes = case.source.len();
    for symbol in &result.symbols {
        assert!(
            symbol.line_start >= 1
                && symbol.line_start <= symbol.line_end
                && symbol.line_end <= lines,
            "{} {} bad line span {}..{} / {lines}",
            case.language,
            symbol.name,
            symbol.line_start,
            symbol.line_end
        );
        assert!(
            symbol.byte_start < symbol.byte_end && symbol.byte_end <= bytes,
            "{} {} bad byte span {}..{} / {bytes}",
            case.language,
            symbol.name,
            symbol.byte_start,
            symbol.byte_end
        );
        assert!(
            case.source[symbol.byte_start..symbol.byte_end].contains(&symbol.name),
            "{} {} span must cover name",
            case.language,
            symbol.name
        );
    }
    for call in &result.calls {
        assert!(
            call.line >= 1 && call.line <= lines,
            "{} call line {}",
            case.language,
            call.line
        );
        assert!(
            call.byte_start < call.byte_end && call.byte_end <= bytes,
            "{} call byte span",
            case.language
        );
    }
}
