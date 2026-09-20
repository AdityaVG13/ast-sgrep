use ast_sgrep_lang::signature::{cached_pattern_signatures, index_can_serve_pattern};

#[test]
fn ident_and_decl_are_index_complete_kind_is_not() {
    let ident = cached_pattern_signatures("SearchHit").unwrap();
    assert!(index_can_serve_pattern("SearchHit", &ident));
    let decl = cached_pattern_signatures("fn greet_user").unwrap();
    assert!(index_can_serve_pattern("fn greet_user", &decl));
    let kind = cached_pattern_signatures("fn $NAME").unwrap();
    assert!(!index_can_serve_pattern("fn $NAME", &kind));
    assert!(!index_can_serve_pattern("fn $NAME() { $$$BODY }", &[]));
}
