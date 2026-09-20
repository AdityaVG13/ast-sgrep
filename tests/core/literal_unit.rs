use ast_sgrep_core::search::passes::literal::{content_matches_literal, has_literal_match};

fn agree(content: &str, needle: &str, word: bool) {
    let lower = needle.to_lowercase();
    let ascii = content_matches_literal(content, needle, Some(&lower), word);
    let unicode = has_literal_match(&content.to_lowercase(), &lower, word);
    assert_eq!(
        ascii, unicode,
        "content={content:?} needle={needle:?} word={word}"
    );
}

#[test]
fn ascii_ci_matches_unicode_lowercase_on_ascii_inputs() {
    for content in [
        "Encode payload",
        "encode payload",
        "ENCODE",
        "x_encode_y",
        "en",
    ] {
        for needle in ["encode", "Encode", "payload"] {
            agree(content, needle, false);
            agree(content, needle, true);
        }
    }
}
