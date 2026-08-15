//! Evidence for the 7d5x.4 concat A/B arm: `use_field_rescoring = false`
//! ranks embed hits by the concatenated chunk vector alone, so no hit may
//! carry per-field embed scores, while the default arm attaches them.
use ast_sgrep_core::search::{SearchOptions, Searcher};
use ast_sgrep_core::{IndexOptions, Indexer};
use std::fs;
use tempfile::TempDir;

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn embedded_root() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_src(
        root,
        "src/auth.rs",
        "/// Refresh the session credential before it expires.\n\
         fn refresh_auth_token() {\n    renew_credentials();\n}\n\
         fn renew_credentials() {}\n",
    );
    write_src(
        root,
        "src/style.rs",
        "/// Repaint the widget after a theme change.\n\
         fn refresh_widget() {}\n",
    );
    write_src(
        root,
        "tests/session_test.rs",
        "fn renews_expired_session() { refresh_auth_token(); }\n",
    );
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(root.join("index.db")),
        force_reindex: true,
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    temp
}

fn searcher(root: &std::path::Path, use_field_rescoring: bool) -> Searcher {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(root.join("index.db")),
        use_embed: true,
        use_field_rescoring,
        ..SearchOptions::default()
    })
    .unwrap()
}

const QUERY: &str = "renew the session credential";

#[test]
fn default_arm_attaches_per_field_embed_scores() {
    let temp = embedded_root();
    let response = searcher(temp.path(), true).search_semantic(QUERY).unwrap();
    assert!(!response.hits.is_empty(), "semantic search must hit");
    assert!(
        response.hits.iter().any(|hit| hit.embed_fields.is_some()),
        "multi-field arm must expose per-field embed scores on some hit"
    );
}

#[test]
fn test_hit_reports_test_example_similarity() {
    let temp = embedded_root();
    let response = searcher(temp.path(), true).search_semantic(QUERY).unwrap();
    let test_hit = response
        .hits
        .iter()
        .find(|hit| hit.file == "tests/session_test.rs")
        .expect("test fixture must be returned");
    assert!(
        test_hit
            .embed_fields
            .as_ref()
            .and_then(|scores| scores.tests_examples)
            .is_some(),
        "test hit must report its tests/examples similarity: {test_hit:#?}"
    );
}

#[test]
fn concat_arm_never_attaches_per_field_embed_scores() {
    let temp = embedded_root();
    let response = searcher(temp.path(), false).search_semantic(QUERY).unwrap();
    assert!(!response.hits.is_empty(), "semantic search must hit");
    assert!(
        response.hits.iter().all(|hit| hit.embed_fields.is_none()),
        "concat arm must rank by the concatenated vector only"
    );
}

#[test]
fn both_arms_return_the_same_files_on_this_corpus() {
    // Two files, distinct topics: arm choice may reorder scores but must not
    // invent or lose files here. This is a sanity floor, not a quality claim.
    let temp = embedded_root();
    let mut with_fields: Vec<String> = searcher(temp.path(), true)
        .search_semantic(QUERY)
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.file)
        .collect();
    let mut concat: Vec<String> = searcher(temp.path(), false)
        .search_semantic(QUERY)
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.file)
        .collect();
    with_fields.sort();
    with_fields.dedup();
    concat.sort();
    concat.dedup();
    assert_eq!(with_fields, concat);
}
