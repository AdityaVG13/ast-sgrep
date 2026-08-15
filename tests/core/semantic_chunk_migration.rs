use ast_sgrep_core::semantic_ivf::semantic_ivf_path;
use ast_sgrep_core::{EmbedBackend, IndexOptions, IndexStore, Indexer};
use rusqlite::params;
use std::path::PathBuf;
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
    let sidecar = semantic_ivf_path(store.db_path());
    std::fs::write(&sidecar, b"legacy semantic sidecar").unwrap();
    drop(store);

    let migrated = IndexStore::open(temp.path(), None).unwrap();
    assert!(!sidecar.exists());
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
        Some("semantic-layout-v3:original-hash")
    );
    let version: i64 = migrated
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 11, "migration must land on the current schema");
}

#[test]
fn schema_6_main_indexes_still_get_semantic_wipe_at_7() {
    // Main independently used SCHEMA_VERSION=6 for symbols_name_lower. A store
    // already at 6 must still run the semantic-layout wipe introduced in 7,
    // even though later migrations advance it to the current schema.
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
        .execute_batch("PRAGMA user_version = 6")
        .unwrap();
    drop(store);

    let migrated = IndexStore::open(temp.path(), None).unwrap();
    let count: i64 = migrated
        .connection()
        .query_row("SELECT COUNT(*) FROM semantic_chunks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "schema-6 stores must still wipe semantic layout at 7"
    );
    let version: i64 = migrated
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 11);
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

fn migration_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/migration")
        .join(name)
}

/// ghiw.4: checked-in user_version=5 DB migrates to current schema (11).
#[test]
fn committed_v5_sqlite_migrates_to_current_schema() {
    let temp = TempDir::new().unwrap();
    let dest = temp.path().join("index.db");
    std::fs::copy(migration_fixture("v5_empty.sqlite"), &dest).expect("copy v5 fixture");
    let store =
        IndexStore::open(temp.path(), Some(&dest)).expect("v5 fixture must open and migrate");
    let version: i64 = store
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 11, "migration must land on SCHEMA_VERSION=11");
}

/// ghiw.4: newer-than-supported user_version fails closed (no panic).
#[test]
fn committed_v99_sqlite_is_rejected_without_panic() {
    let temp = TempDir::new().unwrap();
    let dest = temp.path().join("index.db");
    std::fs::copy(migration_fixture("v99_unsupported.sqlite"), &dest).expect("copy v99 fixture");
    match IndexStore::open(temp.path(), Some(&dest)) {
        Ok(_) => panic!("newer schema must fail closed"),
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("newer than supported"),
                "unexpected error: {message}"
            );
        }
    }
}

#[test]
fn schema_9_invalidates_legacy_semantic_state() {
    let temp = TempDir::new().unwrap();
    let dest = temp.path().join("index.db");
    let conn = rusqlite::Connection::open(&dest).unwrap();
    conn.execute_batch(
        "CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, language TEXT,
           mtime_secs INTEGER NOT NULL, mtime_nanos INTEGER NOT NULL, content_hash TEXT NOT NULL);
         CREATE TABLE semantic_chunks (id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, symbol_id INTEGER,
           chunk_kind TEXT NOT NULL, line_start INTEGER NOT NULL, line_end INTEGER NOT NULL, symbol_name TEXT,
           text TEXT NOT NULL, vector BLOB NOT NULL,
           FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);
         INSERT INTO files(path, language, mtime_secs, mtime_nanos, content_hash)
           VALUES('legacy.rs', 'rust', 1, 0, 'keep-me');
         INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector)
           VALUES(1, NULL, 'symbol', 1, 3, 'legacy', 'whole parent', x'00000000');
         PRAGMA user_version = 9;",
    )
    .unwrap();
    drop(conn);

    let migrated = IndexStore::open(temp.path(), Some(&dest)).unwrap();
    let version: i64 = migrated
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 11);
    let count: i64 = migrated
        .connection()
        .query_row("SELECT COUNT(*) FROM semantic_chunks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "v9 chunks use the obsolete rendering and vectors");
    let content_hash: String = migrated
        .connection()
        .query_row("SELECT content_hash FROM files", [], |row| row.get(0))
        .unwrap();
    assert_eq!(content_hash, "semantic-layout-v3:keep-me");
    let cols: Vec<String> = {
        let mut stmt = migrated
            .connection()
            .prepare("PRAGMA table_info(semantic_chunks)")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    for col in ["vector_name", "vector_docs", "vector_body", "vector_graph"] {
        assert!(cols.iter().any(|c| c == col), "missing {col} in {cols:?}");
    }
}

#[test]
fn persist_per_field_vectors_on_index() {
    let temp = TempDir::new().unwrap();
    let content =
        "/// renews billing\nfn renew_account() { charge(); }\nfn main() { renew_account(); }\n";
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: true,
        embed_backend: EmbedBackend::Semantic,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_content("account.rs", content).unwrap();
    let store = indexer.store();
    assert_eq!(store.schema_version(), 11);
    let fields = store.semantic_chunk_field_vectors().unwrap();
    assert!(
        !fields.is_empty(),
        "indexing with embed must persist semantic chunks"
    );
    let with_body = fields
        .iter()
        .filter(|(_, v)| v.body.is_some() && v.name.is_some())
        .count();
    assert!(
        with_body > 0,
        "at least one chunk must store name and body field vectors"
    );
    let docs = fields.iter().filter(|(_, v)| v.docs.is_some()).count();
    assert!(docs > 0, "doc comment must produce a docs field vector");
    for (_, v) in &fields {
        if let (Some(name), Some(body)) = (&v.name, &v.body) {
            assert_ne!(
                name, body,
                "name and body field vectors must not be identical"
            );
        }
    }
}
