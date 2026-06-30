use ast_sgrep_lang::{detect_language, ParserRegistry, Language};
use std::path::Path;

#[test]
fn java_extracts_methods_and_calls() {
    let src = include_str!("../../../tests/fixtures/sample/src/Main.java");
    let reg = ParserRegistry::new();
    let result = reg.parse(Language::Java, src).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "processRequest"));
    assert!(result.calls.iter().any(|c| c.callee == "processRequest"));
}

#[test]
fn csharp_extracts_methods() {
    let src = include_str!("../../../tests/fixtures/sample/src/Program.cs");
    let reg = ParserRegistry::new();
    let result = reg.parse(Language::CSharp, src).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "ProcessRequest"));
}

#[test]
fn ruby_extracts_defs_and_calls() {
    let src = include_str!("../../../tests/fixtures/sample/src/app.rb");
    let reg = ParserRegistry::new();
    let result = reg.parse(Language::Ruby, src).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "process_request"));
    assert!(result.calls.iter().any(|c| c.callee == "process_request"));
}

#[test]
fn detects_new_language_extensions() {
    assert_eq!(detect_language(Path::new("x.java"), None), Some(Language::Java));
    assert_eq!(detect_language(Path::new("x.cs"), None), Some(Language::CSharp));
    assert_eq!(detect_language(Path::new("x.rb"), None), Some(Language::Ruby));
}
