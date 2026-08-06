use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{IndexStore, SearchOptions, Searcher};
use tempfile::TempDir;

fn upsert(store: &IndexStore, content: &str, hash: &str) {
    let lines = [(1, content.to_string())];
    store
        .upsert_file(UpsertFileInput {
            rel_path: "same.rs",
            language: Some("rust"),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: hash,
            lines: &lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
        })
        .unwrap();
}

#[test]
fn same_connection_write_invalidates_cached_response() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        use_embed: false,
        ..SearchOptions::default()
    };
    let searcher = Searcher::with_store(store, options);

    upsert(searcher.store(), "alpha sentinel", "alpha-hash");
    assert!(!searcher.search("alpha").unwrap().hits.is_empty());

    upsert(searcher.store(), "beta sentinel", "beta-hash");
    assert!(
        searcher.search("alpha").unwrap().hits.is_empty(),
        "same-connection update must invalidate the cached alpha response"
    );
    assert!(!searcher.search("beta").unwrap().hits.is_empty());
}

#[test]
fn external_connection_write_invalidates_cached_response() {
    let temp = TempDir::new().unwrap();
    let reader = IndexStore::open(temp.path(), None).unwrap();
    let db = reader.db_path().to_path_buf();
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db.clone()),
        use_embed: false,
        ..SearchOptions::default()
    };
    let searcher = Searcher::with_store(reader, options);
    let writer = IndexStore::open(temp.path(), Some(&db)).unwrap();

    upsert(&writer, "alpha sentinel", "alpha-hash");
    assert!(!searcher.search("alpha").unwrap().hits.is_empty());

    upsert(&writer, "beta sentinel", "beta-hash");
    assert!(
        searcher.search("alpha").unwrap().hits.is_empty(),
        "external update must invalidate the cached alpha response"
    );
}
