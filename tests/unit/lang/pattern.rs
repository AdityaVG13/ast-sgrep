use super::*;

#[test]
fn classifies_common_metavariable_shapes() {
    assert!(classify_native("fn $NAME($$$)").is_some());
    assert!(classify_native("def $NAME").is_some());
    assert!(classify_native("$OBJ.$METHOD($$$)").is_some());
    assert!(classify_native("foo($$$)").is_some());
    assert!(classify_native("process_request($$$)").is_some());
    // Nested / exotic → external
    assert!(classify_native("if ($COND) { $BODY }").is_none());
}

#[test]
fn native_fn_meta_matches_rust() {
    let src = "fn process_request(x: i32) {}\nfn other() {}\n";
    let hits = match_pattern(Language::Rust, src, "fn $NAME($$$)").unwrap();
    assert!(hits.len() >= 2, "hits={hits:?}");
}

#[test]
fn native_call_matches_exact_callee() {
    let src = "fn main() { process_request(1); other(2); }\n";
    let hits = match_pattern(Language::Rust, src, "process_request($$$)").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].excerpt.contains("process_request"));
}
