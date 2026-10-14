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

#[test]
fn ascii_trigram_unicode_preimages_match_the_rust_unicode_table() {
    // The cold FTS superset enumerates these preimages. A Unicode-table change
    // must fail this proof, not silently drop matches behind the shortcut.
    let actual: Vec<_> = (128..=char::MAX as u32)
        .filter_map(char::from_u32)
        .filter(|c| c.to_lowercase().any(|lower| lower.is_ascii()))
        .collect();
    assert_eq!(actual, vec!['İ', 'K']);
}
