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
fn rust_no_call_in_string_or_comment() {
    let r = ParserRegistry::new();
    assert_no_callee(
        &r,
        Language::Rust,
        r#"
// fake_call(ghost)
fn main() {
    let s = "not_real(ghost)";
    /* another_fake(ghost) */
    real_call();
}
fn real_call() {}
"#,
        "ghost",
    );
    assert_has_callee(&r, Language::Rust, "fn main() { real_call(); }", "real_call");
}

#[test]
fn python_no_call_in_string_or_comment() {
    let r = ParserRegistry::new();
    assert_no_callee(
        &r,
        Language::Python,
        r#"
# fake_call(ghost)
def main():
    s = "not_real(ghost)"
  #  fake_call(ghost)
    real_call()
def real_call(): pass
"#,
        "ghost",
    );
    assert_has_callee(
        &r,
        Language::Python,
        "def main():\n    real_call()",
        "real_call",
    );
}

#[test]
fn typescript_no_call_in_string_or_comment() {
    let r = ParserRegistry::new();
    assert_no_callee(
        &r,
        Language::TypeScript,
        r#"
// fakeCall(ghost)
function main() {
  const s = "notReal(ghost)";
  realCall();
}
function realCall() {}
"#,
        "ghost",
    );
    assert_has_callee(
        &r,
        Language::TypeScript,
        "function main() { realCall(); }",
        "realCall",
    );
}

#[test]
fn javascript_no_call_in_string_or_comment() {
    let r = ParserRegistry::new();
    assert_no_callee(
        &r,
        Language::JavaScript,
        r#"
// fakeCall(ghost)
function main() {
  const s = 'notReal(ghost)';
  realCall();
}
function realCall() {}
"#,
        "ghost",
    );
}

#[test]
fn go_no_call_in_string_or_comment() {
    let r = ParserRegistry::new();
    assert_no_callee(
        &r,
        Language::Go,
        r#"
package main
// fakeCall(ghost)
func main() {
    s := "notReal(ghost)"
    realCall()
}
func realCall() {}
"#,
        "ghost",
    );
    assert_has_callee(
        &r,
        Language::Go,
        "package main\nfunc main() { realCall() }",
        "realCall",
    );
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
    assert!(result.imports.iter().any(|i| i.module_path.contains("std")));
}

#[test]
fn python_extracts_imports() {
    let r = ParserRegistry::new();
    let result = r
        .parse(Language::Python, "import os\ndef main(): pass")
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
    assert!(result.imports.iter().any(|i| i.module_path.contains("bar")));
}

#[test]
fn go_extracts_imports() {
    let r = ParserRegistry::new();
    let result = r
        .parse(Language::Go, "package main\nimport \"fmt\"\nfunc main() {}")
        .unwrap();
    assert!(result.imports.iter().any(|i| i.module_path.contains("fmt")));
}
