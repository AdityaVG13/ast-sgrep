use ast_sgrep_lang::Language;
use ast_sgrep_testkit::{assert_has_callee, assert_has_symbol, parse, sample_file};

#[test]
fn extraction_smoke() {
    let cases = [
        (Language::Rust, "src/main.rs", "process_request"),
        (Language::Java, "src/Main.java", "processRequest"),
    ];
    for (lang, path, symbol) in cases {
        let result = parse(lang, &sample_file(path));
        assert_has_symbol(&result, symbol);
        assert_has_callee(&result, symbol);
    }
}
