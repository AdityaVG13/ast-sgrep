use ast_sgrep_lang::{ExtractionResult, Language, ParserRegistry};

pub fn parse(lang: Language, source: &str) -> ExtractionResult {
    ParserRegistry::new()
        .parse(lang, source)
        .expect("parse")
}

pub fn assert_has_symbol(result: &ExtractionResult, name: &str) {
    assert!(
        result.symbols.iter().any(|s| s.name == name),
        "missing symbol {name}: {:?}",
        result.symbols
    );
}

pub fn assert_has_callee(result: &ExtractionResult, callee: &str) {
    assert!(
        result.calls.iter().any(|c| c.callee == callee),
        "missing callee {callee}: {:?}",
        result.calls
    );
}

pub fn assert_no_callee(result: &ExtractionResult, callee: &str) {
    assert!(
        !result.calls.iter().any(|c| c.callee == callee),
        "false positive callee {callee}: {:?}",
        result.calls
    );
}

pub fn assert_has_import(result: &ExtractionResult, module: &str) {
    assert!(
        result.imports.iter().any(|i| i.module_path.contains(module)),
        "missing import {module}: {:?}",
        result.imports
    );
}

/// Assert a ghost callee inside strings/comments is ignored and a real call is kept.
pub fn run_false_positive_case(lang: Language, source: &str, ghost: &str, real_callee: &str) {
    let result = parse(lang, source);
    assert_no_callee(&result, ghost);
    assert_has_callee(&result, real_callee);
}
