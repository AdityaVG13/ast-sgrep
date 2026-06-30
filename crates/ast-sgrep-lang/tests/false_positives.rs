//! False-positive regression tests: calls in strings/comments must NOT be graph edges.

use ast_sgrep_lang::{Language, ParserRegistry};

fn assert_no_callee(registry: &ParserRegistry, lang: Language, source: &str, false_callee: &str) {
    let result = registry.parse(lang, source).unwrap();
    assert!(
        !result.calls.iter().any(|c| c.callee == false_callee),
        "false positive: {false_callee} in {:?}",
        result.calls
    );
}

fn assert_has_callee(registry: &ParserRegistry, lang: Language, source: &str, callee: &str) {
    let result = registry.parse(lang, source).unwrap();
    assert!(
        result.calls.iter().any(|c| c.callee == callee),
        "missing callee {callee} in {:?}",
        result.calls
    );
}

#[test]
fn no_call_in_string_or_comment_across_languages() {
    let registry = ParserRegistry::new();
    let cases = [
        (
            Language::Rust,
            r#"
// fake_call(ghost)
fn main() {
    let s = "not_real(ghost)";
    real_call();
}
fn real_call() {}
"#,
            "real_call",
        ),
        (
            Language::Python,
            r#"
def main():
    s = "not_real(ghost)"
    real_call()
def real_call(): pass
"#,
            "real_call",
        ),
        (
            Language::TypeScript,
            r#"
function main() {
  const s = "notReal(ghost)";
  realCall();
}
function realCall() {}
"#,
            "realCall",
        ),
        (
            Language::JavaScript,
            r#"
function main() {
  const s = "notReal(ghost)";
  realCall();
}
function realCall() {}
"#,
            "realCall",
        ),
        (
            Language::Go,
            r#"
func main() {
    s := "notReal(ghost)"
    realCall()
}
func realCall() {}
"#,
            "realCall",
        ),
    ];

    for (lang, source, callee) in cases {
        assert_no_callee(&registry, lang, source, "ghost");
        assert_has_callee(&registry, lang, source, callee);
    }
}

#[test]
fn rust_extracts_use_imports() {
    let r = ParserRegistry::new();
    let result = r
        .parse(
            Language::Rust,
            "use std::collections::HashMap;\nfn main() {}",
        )
        .unwrap();
    assert!(result.imports.iter().any(|i| i.module_path.contains("HashMap")));
}

#[test]
fn python_extracts_imports() {
    let r = ParserRegistry::new();
    let result = r
        .parse(Language::Python, "import os\nfrom pathlib import Path")
        .unwrap();
    assert!(result.imports.iter().any(|i| i.module_path.contains("os")));
}

#[test]
fn typescript_extracts_imports() {
    let r = ParserRegistry::new();
    let result = r
        .parse(
            Language::TypeScript,
            "import { foo } from './bar';\nexport function main() {}",
        )
        .unwrap();
    assert!(!result.imports.is_empty());
}

#[test]
fn go_extracts_imports() {
    let r = ParserRegistry::new();
    let result = r
        .parse(Language::Go, "package main\nimport \"fmt\"\nfunc main() {}")
        .unwrap();
    assert!(result.imports.iter().any(|i| i.module_path == "fmt"));
}
