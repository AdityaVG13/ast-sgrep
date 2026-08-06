//! Regression for e2hc.13 partial semantic-v1 → v2 migration.
//!
//! A store advertising embed_backend="semantic" (unversioned v1) must not flip
//! to "semantic-v2" after a single-file update under Auto — that opened the
//! search gate while sibling chunks remained v1. Full index_all may promote.
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;

fn write_py(root: &std::path::Path, name: &str, body: &str) {
    fs::write(root.join(name), body).unwrap();
}

#[test]
fn single_file_update_does_not_promote_semantic_v1_meta() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let index_path = index_dir.path().join("index.db");
    write_py(corpus.path(), "a.py", "def alpha():\n    return 'credential legacy'\n");
    write_py(corpus.path(), "b.py", "def beta():\n    return 'payment renewal'\n");

    let opts = IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: true,
        ..IndexOptions::default()
    };
    let mut indexer = Indexer::new(opts.clone()).unwrap();
    indexer.index_all().unwrap();
    assert_eq!(
        indexer.store().get_meta("embed_backend").unwrap().as_deref(),
        Some("semantic-v2")
    );

    // Simulate a pre-e2hc.13 store that still advertises unversioned v1.
    indexer.store().set_meta("embed_backend", "semantic").unwrap();
    assert!(indexer.store().needs_semantic_v1_rewrite().unwrap());

    // Content change on only one file (watch / update_paths path).
    write_py(
        corpus.path(),
        "a.py",
        "def alpha():\n    return 'credential legacy updated'\n",
    );
    indexer
        .index_file(&corpus.path().join("a.py"), "a.py")
        .unwrap();

    assert_eq!(
        indexer.store().get_meta("embed_backend").unwrap().as_deref(),
        Some("semantic"),
        "partial update must not advertise semantic-v2 while siblings may still be v1"
    );

    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: true,
        use_semantic_only: true,
        ..SearchOptions::default()
    })
    .unwrap();
    let err = searcher
        .search("credential legacy")
        .expect_err("search must refuse semantic-v1 meta");
    let msg = err.to_string();
    assert!(
        msg.contains("semantic backend is v1") || msg.contains("reindex"),
        "unexpected error: {msg}"
    );
}

#[test]
fn index_all_promotes_semantic_v1_after_full_rewrite() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let index_path = index_dir.path().join("index.db");
    write_py(corpus.path(), "a.py", "def alpha():\n    return 1\n");
    write_py(corpus.path(), "b.py", "def beta():\n    return 2\n");

    let opts = IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        embed_semantic: true,
        force_reindex: false,
        ..IndexOptions::default()
    };
    let mut indexer = Indexer::new(opts).unwrap();
    indexer.index_all().unwrap();
    indexer.store().set_meta("embed_backend", "semantic").unwrap();

    // Unchanged files must still be rewritten/promoted via index_all.
    let stats = indexer.index_all().unwrap();
    assert!(
        stats.files_indexed >= 2,
        "v1 rewrite must re-embed reachable files, got {:?}",
        stats
    );
    assert_eq!(
        indexer.store().get_meta("embed_backend").unwrap().as_deref(),
        Some("semantic-v2"),
        "full index_all must promote after rewriting all reachable files"
    );
    assert!(!indexer.store().needs_semantic_v1_rewrite().unwrap());
}
