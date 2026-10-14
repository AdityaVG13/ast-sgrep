//! Regression for partial unversioned-semantic layout migration.
//!
//! A store advertising embed_backend="semantic" must not flip to the
//! active backend after a single-file update under Auto — that opened the
//! search gate while sibling chunks stayed on the old layout. Full index_all may promote.
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;

fn write_py(root: &std::path::Path, name: &str, body: &str) {
    fs::write(root.join(name), body).unwrap();
}

#[test]
fn single_file_update_does_not_promote_unversioned_semantic_meta() {
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
        Some("static")
    );

    // Simulate a store that still advertises the unversioned backend.
    indexer
        .store()
        .set_meta("embed_backend", "semantic")
        .unwrap();
    assert!(indexer.store().needs_legacy_semantic_rewrite().unwrap());

    // Content change on only one file (watch / update_paths path). The
    // active backend (static via Auto) differs from the store's vectors,
    // so the targeted update must refuse rather than mix vector spaces —
    // and the unversioned meta must remain untouched.
    write_py(
        corpus.path(),
        "a.py",
        "def alpha():\n    return 'credential legacy updated'\n",
    );
    let err = indexer
        .index_file(&corpus.path().join("a.py"), "a.py")
        .expect_err("targeted update must refuse to mix embedding identities");
    assert!(
        err.to_string()
            .contains("does not match existing repository vectors"),
        "unexpected error: {err}"
    );

    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("semantic"),
        "partial update must not advertise semantic-v2 while siblings may still be unversioned"
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
        .expect_err("search must refuse unversioned semantic meta");
    let msg = err.to_string();
    assert!(
        msg.contains("unversioned semantic backend") || msg.contains("reindex"),
        "unexpected error: {msg}"
    );
}

#[test]
fn index_all_promotes_unversioned_semantic_after_full_rewrite() {
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
        "legacy rewrite must re-embed reachable files, got {:?}",
        stats
    );
    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("static"),
        "full index_all must promote after rewriting all reachable files"
    );
    assert!(!indexer.store().needs_legacy_semantic_rewrite().unwrap());
}

#[test]
fn partial_full_index_does_not_promote_unversioned_semantic_meta() {
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
        Some("static")
    );
    assert_eq!(
        indexer.store().get_meta("embed_model").unwrap().as_deref(),
        Some("static:potion-code-16M-v2:256")
    );
}

/// INTENT (br-6oh Phase 2B): a store built under an explicit legacy backend
/// (hashed semantic-v2) migrates on the next default (Auto) index_all —
/// identity rewritten to the active static model, dead-model embed_cache
/// rows purged. Before migration, default search must REFUSE the stale
/// store with an actionable error, never silently serve vectors embedded
/// by a different model. KILLS: mixed-backend stores served silently.
#[test]
fn auto_index_migrates_legacy_semantic_store_to_static() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let index_path = index_dir.path().join("index.db");
    write_py(
        corpus.path(),
        "a.py",
        "def refresh_credentials():\n    return 'rotate the access token'\n",
    );
    write_py(
        corpus.path(),
        "b.py",
        "def compute_invoice():\n    return 'sum the line items'\n",
    );

    // Build a legacy hashed store (explicit Semantic preference).
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: true,
        embed_backend: EmbedBackend::Semantic,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    // Stamp as a pre-2A default (Auto) build did: hashed vectors with
    // embed_backend_pref="auto". This is the common real-world store the
    // migration must catch — the backend-part identity check passes for
    // it, so only the active-model comparison can trigger the rewrite.
    indexer
        .store()
        .set_meta("embed_backend_pref", "auto")
        .unwrap();
    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("semantic-v2")
    );
    drop(indexer);

    // Default (Auto) search must refuse the stale-identity store.
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        use_embed: true,
        ..SearchOptions::default()
    })
    .unwrap();
    let err = searcher
        .search("rotate credentials")
        .expect_err("search must refuse a store embedded by a retired model");
    let msg = err.to_string();
    assert!(
        msg.contains("asgrep index"),
        "refusal must name the migration command: {msg}"
    );

    // Default index_all (no force flag) performs the migration.
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    assert_eq!(
        indexer
            .store()
            .get_meta("embed_backend")
            .unwrap()
            .as_deref(),
        Some("static")
    );
    assert_eq!(
        indexer.store().get_meta("embed_model").unwrap().as_deref(),
        Some("static:potion-code-16M-v2:256")
    );
    drop(indexer);

    // Dead-model cache rows are gone.
    let conn = rusqlite::Connection::open(&index_path).unwrap();
    let dead: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embed_cache WHERE model_id NOT LIKE 'static:%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dead, 0, "embed_cache must not retain retired-model rows");
    drop(conn);

    // Search works over the migrated store.
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: true,
        ..SearchOptions::default()
    })
    .unwrap();
    let resp = searcher.search("rotate credentials").unwrap();
    assert!(!resp.hits.is_empty(), "migrated store must serve hits");
}
