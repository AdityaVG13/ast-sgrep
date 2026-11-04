use ast_sgrep_core::store::trigram_df::*;
use ast_sgrep_core::store::IndexStore;

#[test]
fn ascii_trigram_extraction_dedups_and_bounds() {
    // Byte-window extraction only supports ASCII. Public helper callers must
    // receive the same conservative fallback as scan_shortcut, not a UTF-8 panic.
    for needle in ["éab", "aébc", "abcé", "😀ab", "界ab", "界"] {
        assert!(
            distinct_trigrams(needle).is_none(),
            "non-ASCII needle {needle:?} must decline byte-window extraction"
        );
    }
    let tris = distinct_trigrams("process_request").unwrap();
    // 15 chars -> 13 sliding windows; none repeat.
    assert_eq!(tris.len(), 13);
    assert_eq!(tris.first(), Some(&"pro"));
    assert_eq!(tris.last(), Some(&"est"));
    assert!(distinct_trigrams("ab").is_none());
    assert!(distinct_trigrams("").is_none());
    let long = "x".repeat(40);
    assert!(distinct_trigrams(&long).is_none(), "over lookup budget");
}

#[test]
fn pick_shortcut_ands_two_rarest_when_selective() {
    let ranked = [(12_i64, "ial"), (80_i64, "cre"), (4000_i64, "den")];
    match pick_shortcut(&ranked) {
        TrigramShortcut::Match(terms) => {
            assert_eq!(terms, vec!["ial".to_string(), "cre".to_string()])
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

#[test]
fn pick_shortcut_falls_back_when_all_trigrams_are_common() {
    let ranked = [(3000_i64, "the"), (5000_i64, "and")];
    assert_eq!(pick_shortcut(&ranked), TrigramShortcut::Full);
}

#[test]
fn sql_control_arm_without_bundle_is_full_phrase() {
    // P3 deleted the static prior. In-memory stores never publish a read bundle,
    // so the SQL control arm must not invent a shortcut.
    let store = IndexStore::open_in_memory(std::path::Path::new("mem")).expect("in-memory store");
    assert!(!df_ready(&store));
    assert_eq!(scan_shortcut(&store, "zzquux"), TrigramShortcut::Full);
    assert_eq!(min_df(&store, "zzquux"), None);
}
