use super::body_structure_hash;
use ast_sgrep_lang::Language;

#[test]
fn trailing_comment_preserves_body_hash_for_its_language() {
    let a = "export function x() {\n  return 1;\n}\n";
    let js_comment = format!("{a}\n// sub1ms-bench-marker\n");
    assert_eq!(
        body_structure_hash(a, Some(Language::JavaScript)),
        body_structure_hash(&js_comment, Some(Language::JavaScript))
    );
    let hash_line = format!("{a}\n# not-a-javascript-comment\n");
    assert_ne!(
        body_structure_hash(a, Some(Language::JavaScript)),
        body_structure_hash(&hash_line, Some(Language::JavaScript))
    );
    assert_eq!(
        body_structure_hash(a, Some(Language::Python)),
        body_structure_hash(&hash_line, Some(Language::Python))
    );
}
