use super::{escape_glob_literal, escape_like_term};

#[test]
fn glob_escapes_metachars() {
    assert_eq!(escape_glob_literal("arr[0]"), "arr[[]0[]]");
    assert_eq!(escape_glob_literal("a*b?c"), "a[*]b[?]c");
}

#[test]
fn like_escapes_metachars() {
    assert_eq!(escape_like_term("a%b_c\\d"), "a\\%b\\_c\\\\d");
}
