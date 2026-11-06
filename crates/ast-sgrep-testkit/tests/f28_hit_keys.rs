//! F28 regression: one `HitKey` (`u64 line_start`) plus one scored variant,
//! with file-only / name-only views as methods, backing the whole hit-key
//! projection family (`response_hit_keys*`, `json_hit_keys`,
//! `core_search_hit_keys`, `lsp_search_hit_keys`, the six `HitKey` views
//! used at the `codemode_invalidation` call sites), and a widened
//! `indexed_tree` that takes owned pairs so `multi_hit_tree` needs no
//! `&str`-refs workaround.
//!
//! Fails before the fix (`index::HitKey` is a separate `u32` struct,
//! `ScoredHitKey` / `HitKey::file_keys` / generic `indexed_tree` missing);
//! passes after.

use ast_sgrep_testkit::{
    HitKey, ScoredHitKey, json_hit_keys, mk_hit, response_hit_keys,
};
use ast_sgrep_core::HitKind;
use serde_json::json;

#[test]
fn index_and_hit_keys_are_one_type_with_u64_lines() {
    let value = json!({"hits": [{"file": "a.rs", "line_start": 7, "kind": "def", "symbol": "s"}]});
    // `json_hit_keys` is `hit_keys(...).expect(...)`, so comparing the two
    // proves nothing; pin the projection against a hand-built literal.
    assert_eq!(
        json_hit_keys(&value),
        vec![HitKey {
            file: "a.rs".to_string(),
            line_start: 7u64,
            kind: "def".to_string(),
            symbol: Some("s".to_string()),
            callee: None,
            caller: None,
        }]
    );
}

#[test]
fn from_search_hit_carries_identity_fields() {
    let hit = mk_hit(HitKind::Def, "a.rs", 7, 1.0);
    let key = HitKey::from_search_hit(&hit);
    assert_eq!(key.file, "a.rs");
    assert_eq!(key.line_start, 7u64);
    assert_eq!(key.kind, "def");
    assert_eq!(key.name_key(), "a.rs");
}

#[test]
fn scored_tuple_is_a_pinned_shape() {
    // One independently-constructed expected tuple: `mk_hit` spans are
    // single-line (`line_end == line`), `Def` wires as `"def"`, and the
    // score is the exact `f64` bits. Both sides of the old equality
    // (`response_hit_keys_with_scores` vs `as_tuple`) were library-built,
    // so the comparison proved nothing; this pins the shape instead.
    let hit = mk_hit(HitKind::Def, "a.rs", 7, 1.0);
    assert_eq!(
        ScoredHitKey::from_search_hit(&hit).as_tuple(),
        (
            "a.rs".to_string(),
            7u64,
            7u32,
            "def".to_string(),
            None,
            1.0f64.to_bits()
        )
    );
    let session = ast_sgrep_testkit::IsolatedIndexSession::new();
    session.write("src/a.rs", "fn scored_probe_token() {}\n");
    session.index_all(ast_sgrep_testkit::hermetic_index_options(
        &session.corpus_root,
        &session.index_path,
    ));
    let searcher = session.searcher(ast_sgrep_testkit::hermetic_search_options(
        &session.corpus_root,
        &session.index_path,
        8,
    ));
    let response = searcher.search("scored_probe_token").expect("search");
    assert!(
        !response.hits.is_empty(),
        "fixture must be non-vacuous or the pin proves nothing"
    );
    // Expected values come from the source `SearchHit` fields, not from
    // another library projection.
    let keys = response_hit_keys(&response);
    assert_eq!(keys.len(), response.hits.len());
    assert_eq!(keys[0].file, response.hits[0].file);
    assert_eq!(keys[0].line_start, u64::from(response.hits[0].line_start));
}

#[test]
fn file_only_views_match_lenient_json() {
    let value = json!({"hits": [
        {"file": "src/a.rs"},
        {"path": "src/b.rs"},
        {"no-file": true},
    ]});
    assert_eq!(
        HitKey::file_keys(&value),
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
    );
    assert_eq!(HitKey::hit_count(&value), 3);
    assert!(HitKey::has_file(&value, "src/a.rs"));
    assert!(!HitKey::has_file(&value, "src/zzz.rs"));
    assert_eq!(HitKey::name_key_of("src/a.rs"), "a.rs");
}

#[test]
fn answer_keys_keep_recovery_tolerant_defaults() {
    let value = json!({"hits": [
        {"file": "a.rs", "line_start": 3, "symbol": "s"},
        {},
    ]});
    assert_eq!(
        HitKey::answer_keys(&value),
        vec![
            (String::new(), 0u64, String::new()),
            ("a.rs".to_string(), 3u64, "s".to_string()),
        ]
    );
}

#[test]
fn indexed_tree_accepts_owned_pairs_and_matches_multi_hit_tree() {
    let owned: Vec<(String, String)> = (0..2)
        .map(|i| {
            (
                format!("src/m{i}.rs"),
                format!("fn owned_token_{i}() {{}}\n"),
            )
        })
        .collect();
    let temp = ast_sgrep_testkit::indexed_tree(&owned);
    for (rel, _) in &owned {
        assert!(
            temp.path().join(rel).is_file(),
            "owned pair {rel} must be planted"
        );
    }
    let multi = ast_sgrep_testkit::multi_hit_tree();
    for name in ["src/m1.rs", "src/m6.rs", "src/lib.rs"] {
        assert!(
            multi.path().join(name).is_file(),
            "multi_hit_tree keeps planting {name}"
        );
    }
}
