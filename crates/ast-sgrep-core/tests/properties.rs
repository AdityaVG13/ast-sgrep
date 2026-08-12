//! Restored proptest property suite (ok49).
use ast_sgrep_core::search::{HitKind, SearchHit, SearchOptions, Searcher, SpanHitInput};
use ast_sgrep_core::{clamp_output_limit, IndexOptions, Indexer, ParsedQuery, MAX_OUTPUT_RESULTS};
use proptest::prelude::*;
use std::fs;
use tempfile::TempDir;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn parse_never_panics(s in ".*") {
        let _ = ParsedQuery::parse(&s);
    }

    #[test]
    fn clamp_limit_never_zero(n in 0usize..10_000) {
        let clamped = clamp_output_limit(Some(n), 16);
        assert!(clamped >= 1);
        assert!(clamped <= MAX_OUTPUT_RESULTS);
    }
}

#[test]
fn store_upsert_delete_roundtrip() {
    let corpus = TempDir::new().unwrap();
    fs::write(
        corpus.path().join("lib.rs"),
        "fn alpha() {}\nfn beta() {}\n",
    )
    .unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer");
    let stats = indexer.index_all().expect("index");
    assert!(stats.files_indexed >= 1);
    let store = indexer.store();
    assert!(store.status().expect("status").file_count >= 1);
    store.remove_file("lib.rs").expect("delete");
    assert_eq!(store.file_hash("lib.rs").expect("hash"), None);
}

#[test]
fn rank_scores_are_finite() {
    let corpus = TempDir::new().unwrap();
    fs::write(
        corpus.path().join("a.rs"),
        "fn process_request() { let x = 1; }\n",
    )
    .unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 16,
        ..SearchOptions::default()
    })
    .expect("searcher");
    let response = searcher.search("process_request").expect("search");
    for hit in &response.hits {
        assert!(hit.score.is_finite(), "non-finite score {}", hit.score);
        assert!(hit.score >= 0.0);
    }
}

#[test]
fn cache_identity_changes_with_options() {
    let a = SearchOptions {
        limit: 8,
        use_embed: false,
        ..SearchOptions::default()
    };
    let b = SearchOptions {
        limit: 16,
        use_embed: false,
        ..SearchOptions::default()
    };
    assert_ne!(a.cache_identity(), b.cache_identity());
}

#[test]
fn single_char_route_hits_not_zeroed() {
    let parsed = ParsedQuery::parse("x");
    let mut hits = vec![SearchHit::span(SpanHitInput {
        kind: HitKind::Asgrep,
        file: "a.rs".into(),
        line_start: 1,
        line_end: 1,
        score: 1.0,
        excerpt: "x = 1".into(),
        symbol: None,
        language: None,
    })];
    ast_sgrep_core::intent::route_hits(&parsed, &mut hits);
    assert!(
        hits[0].score > 0.0,
        "single-char query must not zero text channels"
    );
}

#[test]
fn response_cache_isolates_option_identity() {
    let corpus = TempDir::new().unwrap();
    fs::write(corpus.path().join("a.rs"), "fn needle_alpha() {}\n").unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");
    let root = corpus.path().to_path_buf();
    let s8 = Searcher::new(SearchOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        use_embed: false,
        limit: 8,
        ..SearchOptions::default()
    })
    .expect("s8");
    let s1 = Searcher::new(SearchOptions {
        root,
        index_path: Some(index_path),
        use_embed: false,
        limit: 1,
        ..SearchOptions::default()
    })
    .expect("s1");
    let r8 = s8.search("needle_alpha").expect("r8");
    let r1 = s1.search("needle_alpha").expect("r1");
    assert_eq!(r8.limit, 8);
    assert_eq!(r1.limit, 1);
}
