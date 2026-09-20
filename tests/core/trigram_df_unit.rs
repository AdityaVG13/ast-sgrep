use ast_sgrep_core::store::trigram_df::*;
use ast_sgrep_core::store::IndexStore;

#[test]
fn ascii_trigram_extraction_dedups_and_bounds() {
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
fn static_bake_ranks_common_above_rare() {
    // Triple-space tops every code corpus; 'ion' is common English;
    // 'zzq' is absent-or-trace in a 10GB bake. Absent reads as 0.
    let spaces = bake_count("   ");
    let ion = bake_count("ion");
    let zzq = bake_count("zzq");
    assert!(spaces > ion, "triple-space must top the bake");
    assert!(ion > zzq, "common English must beat rare 'zzq'");
}

#[test]
fn unarmed_scan_uses_static_prior() {
    let store = IndexStore::open_in_memory(std::path::Path::new("mem")).expect("in-memory store");
    let cache = TrigramDfCache::new();
    assert!(!cache.is_armed(), "fresh cache starts unarmed");
    match cache.scan_shortcut(&store, "zzquux") {
        TrigramShortcut::Match(terms) => {
            assert_eq!(terms[0], "zzq", "rarest static trigram leads");
        }
        TrigramShortcut::Full => {
            panic!("cold one-shot must use the static prior, not Full")
        }
    }
}
