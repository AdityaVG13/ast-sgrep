use ast_sgrep_lang::{ExtractionResult, Language, ParserRegistry};

pub fn parse(lang: Language, source: &str) -> ExtractionResult {
    ParserRegistry::new()
        .parse(lang, source)
        .expect("parse")
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
