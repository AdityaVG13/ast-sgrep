use ast_sgrep_lang::{ParserRegistry, Language};

#[test]
fn rust_extracts_functions_and_calls() {
    let source = r#"
fn main() {
    process_request("x");
}

fn process_request(s: &str) {
    validate(s);
}

fn validate(s: &str) {}
"#;
    let registry = ParserRegistry::new();
    let result = registry.parse(Language::Rust, source).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "process_request"));
    assert!(result.symbols.iter().any(|s| s.name == "main"));
    assert!(result.calls.iter().any(|c| c.callee == "process_request" && c.caller == "main"));
}

#[test]
fn ignores_calls_in_strings() {
    let source = r#"
fn main() {
    let s = "not_a_call(process_request)";
    real_call();
}

fn real_call() {}
"#;
    let registry = ParserRegistry::new();
    let result = registry.parse(Language::Rust, source).unwrap();
    assert!(!result.calls.iter().any(|c| c.callee == "process_request"));
    assert!(result.calls.iter().any(|c| c.callee == "real_call"));
}

#[test]
fn python_extracts_defs_and_calls() {
    let source = r#"
def main():
    auth_refresh()

def auth_refresh():
    fetch_token()
"#;
    let registry = ParserRegistry::new();
    let result = registry.parse(Language::Python, source).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "auth_refresh"));
    assert!(result.calls.iter().any(|c| c.callee == "auth_refresh"));
}

#[test]
fn go_extracts_methods() {
    let source = r#"
package main

func main() {
    svc.Serve()
}

func (s *Handler) Serve() {
    processRequest()
}

func processRequest() {}
"#;
    let registry = ParserRegistry::new();
    let result = registry.parse(Language::Go, source).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "Serve"));
    assert!(result.calls.iter().any(|c| c.callee == "Serve"));
}
