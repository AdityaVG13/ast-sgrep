use ast_sgrep_core::{EmbedBackend, IndexOptions, IndexStore, Indexer};
use rusqlite::params;
use tempfile::TempDir;

#[test]
fn schema_upgrade_invalidates_legacy_semantic_layouts() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO files(path, language, mtime_secs, mtime_nanos, content_hash) VALUES(?1, ?2, 1, 0, ?3)",
            params!["legacy.rs", "rust", "original-hash"],
        )
        .unwrap();
    let file_id = store.connection().last_insert_rowid();
    store
        .connection()
        .execute(
            "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector) VALUES(?1, NULL, 'symbol', 1, 3, 'legacy', 'whole parent', ?2)",
            params![file_id, vec![0_u8; 4]],
        )
        .unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO embeddings(file_id, line_no, vector) VALUES(?1, 1, ?2)",
            params![file_id, vec![0_u8; 4]],
        )
        .unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO embed_cache(chunk_hash, model_id, backend, dim, vector, accessed_at) VALUES('old', 'old', 'semantic', 1, ?1, 1)",
            params![vec![0_u8; 4]],
        )
        .unwrap();
    store
        .connection()
        .execute_batch(
            "INSERT INTO meta(key, value) VALUES('body:legacy.rs', 'old-body');
             INSERT INTO meta(key, value) VALUES('embed_backend', 'cloud');
             INSERT INTO meta(key, value) VALUES('embed_model', 'cloud:old-model');
             INSERT INTO meta(key, value) VALUES('embed_dim', '1');",
        )
        .unwrap();
    store
        .connection()
        .execute_batch("PRAGMA user_version = 5")
        .unwrap();
    drop(store);

    let migrated = IndexStore::open(temp.path(), None).unwrap();
    for table in ["semantic_chunks", "embeddings", "embed_cache"] {
        let count: i64 = migrated
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} retained a legacy layout");
    }
    assert_eq!(migrated.get_meta("body:legacy.rs").unwrap(), None);
    assert_eq!(migrated.get_meta("embed_backend").unwrap(), None);
    assert_eq!(migrated.get_meta("embed_model").unwrap(), None);
    assert_eq!(migrated.get_meta("embed_dim").unwrap(), None);
    assert_eq!(
        migrated.file_hash("legacy.rs").unwrap().as_deref(),
        Some("semantic-layout-v2:original-hash")
    );
}

#[test]
fn enabling_embeddings_rebuilds_an_unchanged_file() {
    let temp = TempDir::new().unwrap();
    let content = "fn renew_account() { charge_subscription(); }";
    let mut disabled = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    disabled.index_content("account.rs", content).unwrap();
    assert_eq!(
        disabled.store().semantic_chunk_stats(None).unwrap().count,
        0
    );
    drop(disabled);

    let mut enabled = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: true,
        embed_backend: EmbedBackend::Semantic,
        ..IndexOptions::default()
    })
    .unwrap();
    let stats = enabled.index_content("account.rs", content).unwrap();
    assert!(!stats.skipped);
    assert!(enabled.store().semantic_chunk_stats(None).unwrap().count > 0);
}
