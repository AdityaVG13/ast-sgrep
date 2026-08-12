use super::*;
use tempfile::TempDir;

fn empty_upsert<'a>(
    path: &'a str,
    lines: &'a [(u32, String)],
    hash: &'a str,
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("python"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
    }
}

/// pass3: semantic_chunks_by_ids must fail closed like all_semantic_chunks.
#[test]
fn semantic_chunks_by_ids_fails_closed_on_corrupt_blob() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "emb".into())];
    let file_id = store
        .upsert_file(empty_upsert("c.py", &lines, "h"))
        .unwrap();
    store
            .connection()
            .execute(
                "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector) \
                 VALUES(?1, NULL, 'file', 1, 1, '', 't', ?2)",
                rusqlite::params![file_id, vec![1u8, 2, 3]],
            )
            .unwrap();
    let id: i64 = store
        .connection()
        .query_row("SELECT id FROM semantic_chunks LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let err = store
        .semantic_chunks_by_ids(&[id])
        .expect_err("corrupt vector must not become an empty embedding");
    let msg = err.to_string();
    assert!(
        msg.contains("embedding")
            || msg.contains("multiple of 4")
            || msg.contains("database")
            || msg.contains("InvalidData"),
        "corrupt blob must error, got: {msg}"
    );
}

#[test]
fn symbols_in_file_rejects_negative_byte_offsets() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "fn corrupt() {}".into())];
    let file_id = store
        .upsert_file(empty_upsert("corrupt.py", &lines, "h"))
        .unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO symbols(file_id, name, kind, line_start, line_end, byte_start, byte_end) \
                 VALUES(?1, 'corrupt', 'function', 1, 1, -1, 4)",
            [file_id],
        )
        .unwrap();
    let error = store
        .symbols_in_file("corrupt.py")
        .expect_err("negative byte offsets must not wrap to usize::MAX");
    assert!(matches!(
        error,
        crate::StoreError::Database(rusqlite::Error::IntegralValueOutOfRange(4, -1))
    ));
}

/// pass3: with_file_tx must not Ok after nested poison+rollback.
#[test]
fn with_file_tx_poisoned_ok_closure_returns_err() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "keep".into())];
    store
        .upsert_file(empty_upsert("keep.py", &lines, "h0"))
        .unwrap();

    let result = store.with_file_tx(|| {
            // Nested begin + rollback poisons the outer write set.
            store.begin_file_tx()?;
            store
                .connection()
                .execute(
                    "INSERT INTO meta(key, value) VALUES('poison_probe', '1')                      ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    [],
                )
                .map_err(crate::StoreError::from)?;
            store.rollback_file_tx()?;
            // Closure still returns Ok — with_file_tx must refuse success.
            Ok(42i64)
        });
    assert!(
        result.is_err(),
        "poisoned with_file_tx must not return Ok after rollback"
    );
    assert!(
        store.get_meta("poison_probe").unwrap().is_none(),
        "poisoned writes must not be visible"
    );
    assert!(store.connection().is_autocommit());
}
