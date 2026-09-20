use ast_sgrep_codemode::io::dispatch_find_query;

#[test]
fn blast_symbol_becomes_callers() {
    assert_eq!(
        dispatch_find_query("blast:process_request"),
        "callers:process_request"
    );
}

#[test]
fn blast_path_becomes_imports() {
    assert_eq!(
        dispatch_find_query("blast:src/auth.ts"),
        "imports:src/auth.ts"
    );
}

#[test]
fn unprefixed_is_word() {
    assert_eq!(dispatch_find_query("hello"), "word:hello");
}
