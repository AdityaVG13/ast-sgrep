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
    write_py(
        corpus.path(),
        "a.py",
        "def alpha():\n    return 'credential legacy'\n",
    );
    write_py(
        corpus.path(),
        "b.py",
        "def beta():\n    return 'payment renewal'\n",
    );

    let opts = IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: true,
        ..IndexOptions::default()
    };
    let mut indexer = Indexer::new(opts.clone()).unwrap();
    indexer.index_all().unwrap();
    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("semantic-v2")
    );

    // Simulate a pre-e2hc.13 store that still advertises unversioned v1.
    indexer
        .store()
        .set_meta("embed_backend", "semantic")
        .unwrap();
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
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
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
    indexer
        .store()
        .set_meta("embed_backend", "semantic")
        .unwrap();

    // Unchanged files must still be rewritten/promoted via index_all.
    let stats = indexer.index_all().unwrap();
    assert!(
        stats.files_indexed >= 2,
        "v1 rewrite must re-embed reachable files, got {:?}",
        stats
    );
    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("semantic-v2"),
        "full index_all must promote after rewriting all reachable files"
    );
    assert!(!indexer.store().needs_semantic_v1_rewrite().unwrap());
}

#[test]
fn partial_full_index_does_not_promote_semantic_v1_meta() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let index_path = index_dir.path().join("index.db");
    write_py(corpus.path(), "a.py", "def alpha():\n    return 1\n");
    write_py(corpus.path(), "b.py", "def beta():\n    return 2\n");

    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    indexer
        .store()
        .set_meta("embed_backend", "semantic")
        .unwrap();

    fs::write(corpus.path().join("b.py"), [0xff]).unwrap();
    let stats = indexer.index_all().unwrap();
    assert_eq!(stats.files_failed, 1);
    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("semantic"),
        "a retained failed sibling prevents layout promotion"
    );

    fs::write(corpus.path().join("b.py"), "def beta():\n    return 2\n").unwrap();
    let mut filtered = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        embed_semantic: true,
        lang_filter: Some("python".into()),
        ..IndexOptions::default()
    })
    .unwrap();
    filtered.index_all().unwrap();
    assert_eq!(
        filtered
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("semantic"),
        "a language-filtered rewrite cannot prove every stored row was rewritten"
    );
}

#[test]
fn targeted_update_refuses_to_mix_resolved_embedding_identities() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let index_path = index_dir.path().join("index.db");
    write_py(corpus.path(), "a.py", "def alpha():\n    return 1\n");
    write_py(corpus.path(), "b.py", "def beta():\n    return 2\n");

    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let original_hash = indexer.store().file_hash("a.py").unwrap().unwrap();

    // Simulate a repository whose untouched siblings were produced by another
    // provider. A targeted local update must roll back rather than creating a
    // mixed vector space under one global metadata identity.
    indexer.store().set_meta("embed_backend", "cloud").unwrap();
    indexer
        .store()
        .set_meta("embed_model", "cloud:test-model")
        .unwrap();
    write_py(corpus.path(), "a.py", "def alpha():\n    return 3\n");
    let error = indexer
        .index_file(&corpus.path().join("a.py"), "a.py")
        .expect_err("mixed identity must be rejected");
    assert!(error.to_string().contains("does not match"), "{error}");
    assert_eq!(
        indexer.store().file_hash("a.py").unwrap().as_deref(),
        Some(original_hash.as_str()),
        "failed identity migration must preserve the prior file row"
    );

    // A complete walk can safely clear old vectors transactionally and let the
    // first resolved batch establish the new identity for every file.
    indexer.index_all().unwrap();
    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("semantic-v2")
    );
    assert_eq!(
        indexer.store().get_meta("embed_model").unwrap().as_deref(),
        Some("semantic:hashed-v2:256")
    );
}
