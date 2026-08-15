use super::{embed_pass_for_files, embed_pass_with_context, embed_similarity_hits};
use crate::query::ParsedQuery;
use crate::search::SearchOptions;
use crate::semantic_chunk::SemanticChunkInput;
use crate::store::{IndexStore, UpsertFileInput};
use std::collections::HashSet;
use tempfile::TempDir;

#[test]
fn child_scores_use_parent_max_and_return_one_parent_hit() {
    let chunks = vec![
        (
            "parent.rs".into(),
            10,
            20,
            "parent".into(),
            "weaker child".into(),
            vec![0.0],
        ),
        (
            "parent.rs".into(),
            10,
            20,
            "parent".into(),
            "best child".into(),
            vec![0.0],
        ),
        (
            "other.rs".into(),
            1,
            3,
            "other".into(),
            "other child".into(),
            vec![0.0],
        ),
    ];
    let hits = embed_similarity_hits(&chunks, vec![(0, 0.2), (2, 0.8), (1, 0.9)], &[]);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].file, "parent.rs");
    assert_eq!((hits[0].line_start, hits[0].line_end), (10, 20));
    assert_eq!(hits[0].score, super::SCORE_EMBED * f64::from(0.9_f32));
    assert_eq!(hits[0].excerpt, "best child\n...\nweaker child");
}

#[test]
fn language_filtered_semantic_search_does_not_publish_global_sidecar() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "fn filtered_handler() {}".to_string())];
    let chunks = [SemanticChunkInput {
        symbol_name: "filtered_handler".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        excerpt: "filtered semantic handler".into(),
        callers: Vec::new(),
        callees: Vec::new(),
        doc: String::new(),
        scope: String::new(),
    }];
    store
        .upsert_file(UpsertFileInput {
            rel_path: "filtered.rs",
            language: Some("rust"),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: "filtered",
            lines: &lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            semantic_chunks: &chunks,
            embed_semantic: true,
            embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
        })
        .unwrap();
    let hits = embed_pass_with_context(
        &store,
        &SearchOptions {
            root: temp.path().to_path_buf(),
            use_embed: true,
            lang_filter: Some("rust".into()),
            ann_threshold: Some(1),
            ..SearchOptions::default()
        },
        &ParsedQuery::parse("filtered semantic"),
        None,
    )
    .unwrap();
    assert!(!hits.is_empty());
    assert!(!crate::semantic_ivf::semantic_ivf_path(store.db_path()).exists());
}

#[test]
fn cascade_ranks_modern_and_legacy_vectors_in_allowed_files() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "fn renewal_handler() {}".to_string())];
    store
        .upsert_file(UpsertFileInput {
            rel_path: "allowed.rs",
            language: Some("rust"),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: "legacy",
            lines: &lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
        })
        .unwrap();
    let file_id = store.file_id("allowed.rs").unwrap().unwrap();
    let vector = ast_sgrep_embed::embed_query(
        "renewal handler",
        None,
        0,
        ast_sgrep_embed::EmbedPreference::Semantic,
    )
    .unwrap()
    .vector;
    store
        .connection()
        .execute(
            "INSERT INTO embeddings(file_id, line_no, vector) VALUES(?1, ?2, ?3)",
            rusqlite::params![file_id, 1, ast_sgrep_embed::embed_to_bytes(&vector)],
        )
        .unwrap();

    let modern_lines = [(1, "fn payment_renewal() {}".to_string())];
    let modern_chunks = [SemanticChunkInput {
        symbol_name: "payment_renewal".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        excerpt: "payment renewal modern handler".into(),
        callers: Vec::new(),
        callees: Vec::new(),
        doc: String::new(),
        scope: String::new(),
    }];
    store
        .upsert_file(UpsertFileInput {
            rel_path: "modern.rs",
            language: Some("rust"),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: "modern",
            lines: &modern_lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            semantic_chunks: &modern_chunks,
            embed_semantic: true,
            embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
        })
        .unwrap();

    let allowed = HashSet::from(["allowed.rs".to_string(), "modern.rs".to_string()]);
    let stored = store.semantic_chunks_for_files(&allowed, None).unwrap();
    assert!(stored
        .iter()
        .any(|chunk| { chunk.0 == "modern.rs" && chunk.4 == "payment renewal modern handler" }));
    assert!(stored.iter().all(|chunk| !chunk.4.starts_with("symbol:")));
    let hits = embed_pass_for_files(
        &store,
        &SearchOptions {
            root: temp.path().to_path_buf(),
            use_embed: true,
            ..SearchOptions::default()
        },
        &ParsedQuery::parse("renewal handler"),
        &allowed,
    )
    .unwrap();
    let hit_files = hits
        .iter()
        .map(|hit| hit.file.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(hit_files, HashSet::from(["allowed.rs", "modern.rs"]));

    store.set_meta("embed_model", "stale-model").unwrap();
    let error = embed_pass_for_files(
        &store,
        &SearchOptions {
            root: temp.path().to_path_buf(),
            use_embed: true,
            ..SearchOptions::default()
        },
        &ParsedQuery::parse("renewal handler"),
        &allowed,
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not match active model"));
}
