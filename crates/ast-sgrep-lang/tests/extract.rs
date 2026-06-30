//! Consolidated language extraction tests.

use std::path::Path;

use ast_sgrep_lang::{detect_language, Language};
use ast_sgrep_testkit::{
    assert_has_callee, assert_has_import, assert_has_symbol, parse, run_false_positive_case,
    sample_file,
};

#[test]
fn fixture_files_extract_expected_symbols() {
    let cases: &[(&str, Language, &str, Option<&str>, Option<&str>)] = &[
        ("src/Main.java", Language::Java, "processRequest", Some("processRequest"), None),
        ("src/Program.cs", Language::CSharp, "ProcessRequest", None, None),
        ("src/app.rb", Language::Ruby, "process_request", Some("process_request"), Some("json")),
    ];

    for (path, lang, symbol, callee, import) in cases {
        let result = parse(*lang, &sample_file(path));
        assert_has_symbol(&result, symbol);
        if let Some(c) = callee {
            assert_has_callee(&result, c);
        }
        if let Some(m) = import {
            assert_has_import(&result, m);
        }
    }
}

#[test]
fn inline_sources_extract_calls() {
    let cases: &[(&str, Language, &str)] = &[
        (
            r#"
fn main() { process_request("x"); }
fn process_request(s: &str) { validate(s); }
fn validate(s: &str) {}
"#,
            Language::Rust,
            "process_request",
        ),
        (
            r#"
def main():
    auth_refresh()
def auth_refresh():
    fetch_token()
"#,
            Language::Python,
            "auth_refresh",
        ),
        (
            r#"
package main
func main() { svc.Serve() }
func (s *Handler) Serve() { processRequest() }
func processRequest() {}
"#,
            Language::Go,
            "Serve",
        ),
    ];

    for (source, lang, symbol) in cases {
        let result = parse(*lang, source);
        assert_has_symbol(&result, symbol);
        assert_has_callee(&result, symbol);
    }
}

#[test]
fn calls_in_strings_and_comments_are_ignored() {
    let cases: &[(&str, Language, &str, &str)] = &[
        (
            r#"
fn main() {
    let s = "not_real(ghost)";
    real_call();
}
fn real_call() {}
"#,
            Language::Rust,
            "ghost",
            "real_call",
        ),
        (
            r#"
def main():
    s = "not_real(ghost)"
    real_call()
def real_call(): pass
"#,
            Language::Python,
            "ghost",
            "real_call",
        ),
        (
            r#"
function main() {
  const s = "notReal(ghost)";
  realCall();
}
function realCall() {}
"#,
            Language::TypeScript,
            "ghost",
            "realCall",
        ),
        (
            r#"
function main() {
  const s = "notReal(ghost)";
  realCall();
}
function realCall() {}
"#,
            Language::JavaScript,
            "ghost",
            "realCall",
        ),
        (
            r#"
func main() {
    s := "notReal(ghost)"
    realCall()
}
func realCall() {}
"#,
            Language::Go,
            "ghost",
            "realCall",
        ),
    ];

    for (source, lang, ghost, real) in cases {
        run_false_positive_case(*lang, source, ghost, real);
    }
}

#[test]
fn imports_are_extracted() {
    let cases: &[(&str, Language, &str)] = &[
        ("use std::collections::HashMap;\nfn main() {}", Language::Rust, "HashMap"),
        ("import os\nfrom pathlib import Path", Language::Python, "os"),
        (
            "import { foo } from './bar';\nexport function main() {}",
            Language::TypeScript,
            "bar",
        ),
        ("package main\nimport \"fmt\"\nfunc main() {}", Language::Go, "fmt"),
    ];

    for (source, lang, module) in cases {
        assert_has_import(&parse(*lang, source), module);
    }
}

#[test]
fn extension_detection() {
    let cases: &[(&str, Language)] = &[
        ("x.java", Language::Java),
        ("x.cs", Language::CSharp),
        ("x.rb", Language::Ruby),
    ];
    for (path, lang) in cases {
        assert_eq!(detect_language(Path::new(path), None), Some(*lang));
    }
}
