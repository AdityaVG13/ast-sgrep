//! Hard-evidence tests for store/IVF/SQLite durability epics (y1oy, jiyy, j97d, ht1h, esyi).
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{
    compute_ann_fingerprint, compute_ann_fingerprint_with_content, load_semantic_ivf,
    save_semantic_ivf, vectors_content_digest,
};
use ast_sgrep_core::store::{
    assert_sql_ident, CallerRow, ImportRow, SymbolRow, UpsertFileInput, CALLER_COLUMN_ALLOWLIST,
    COUNT_TABLE_ALLOWLIST,
};
use ast_sgrep_core::tantivy_index::{TantivySidecar, LEXICAL_DB};
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;

fn base<'a>(path: &'a str, lines: &'a [(u32, String)], hash: &'a str) -> UpsertFileInput<'a> {
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

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// y1oy.3 — semantic.ivf is published via tmp + fsync + rename (no torn final file).
#[test]
fn semantic_ivf_save_is_atomic_tmp_rename() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("semantic.ivf");
    let dim = 4usize;
    let vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 1);
    save_semantic_ivf(&path, fp, dim, &vectors, &index).unwrap();
    assert!(path.exists());
    assert!(
        !path.with_extension("ivf.tmp").exists(),
        "temp file must be renamed away"
    );
    let loaded = load_semantic_ivf(&path, fp).unwrap().expect("roundtrip");
    assert_eq!(loaded.vectors, vectors);
}

/// y1oy.4 — empty / unpopulated lexical.db is never a ready search target.
#[test]
fn empty_lexical_db_is_not_search_ready() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // Creating via open_for_index yields schema-only DB with no lines meta.
    let sidecar = TantivySidecar::open_for_index(root, None).unwrap();
    assert!(sidecar.exists());
    assert!(
        !sidecar.is_search_ready().unwrap(),
        "schema-only lexical.db must not be search-ready"
    );
    assert!(
        TantivySidecar::open_existing_for_search(root, None)
            .unwrap()
            .is_none(),
        "search open must refuse empty lexical sidecar"
    );
    // Zero-byte file must also be refused.
    let zero = root.join(".asgrep").join(LEXICAL_DB);
    std::fs::write(&zero, b"").unwrap();
    assert!(TantivySidecar::open_existing_for_search(root, None)
        .unwrap()
        .is_none());
}

/// y1oy.5 — clear_all_data wipes content, per-file meta, and bumps generations.
#[test]
fn clear_all_data_is_transactional_and_complete() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "hello clear".into())];
    let symbols = [SymbolRow {
        name: "hello".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 5,
    }];
    let mut input = base("a.py", &lines, "h1");
    input.symbols = &symbols;
    store.upsert_file(input).unwrap();
    store.set_meta("struct:a.py", "fp").unwrap();
    store.set_meta("body:a.py", "bh").unwrap();
    store.set_meta("embed_backend", "semantic-v2").unwrap();
    store.set_meta("embed_cache_hits", "1").unwrap();
    let v_before = store.semantic_data_version().unwrap();
    let i_before = store.index_data_version().unwrap();
    store.clear_all_data().unwrap();
    assert_eq!(
        store
            .connection()
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(store.get_meta("struct:a.py").unwrap().is_none());
    assert!(store.get_meta("body:a.py").unwrap().is_none());
    assert!(store.get_meta("eol:a.py").unwrap().is_none());
    assert!(
        store.get_meta("embed_backend").unwrap().is_none(),
        "28vo: embed_* fingerprints must be wiped"
    );
    assert!(store.get_meta("embed_cache_hits").unwrap().is_none());
    assert!(store.semantic_data_version().unwrap() > v_before);
    assert!(store.index_data_version().unwrap() > i_before);
}

/// y1oy.6 — remove_file deletes struct/body/eol meta and marks IVF stale safely.
#[test]
fn remove_file_deletes_struct_body_meta_and_ivf() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let path = "gone.py";
    let lines = [(1, "x = 1".into())];
    store.upsert_file(base(path, &lines, "h")).unwrap();
    store.set_meta(&format!("struct:{path}"), "s").unwrap();
    store.set_meta(&format!("body:{path}"), "b").unwrap();
    let ivf = ast_sgrep_core::semantic_ivf::semantic_ivf_path(store.db_path());
    std::fs::write(&ivf, b"stale").unwrap();
    store.remove_file(path).unwrap();
    assert!(store.get_meta(&format!("struct:{path}")).unwrap().is_none());
    assert!(store.get_meta(&format!("body:{path}")).unwrap().is_none());
    assert!(store.get_meta(&format!("eol:{path}")).unwrap().is_none());
    assert!(!ivf.exists(), "IVF sidecar must be removed on remove_file");
    assert_eq!(
        store.get_meta("semantic_ivf_stale").unwrap().as_deref(),
        Some("1")
    );
}

/// y1oy.8 — indexing with --lang must not wipe other languages.
#[test]
fn lang_filter_index_does_not_wipe_other_languages() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_src(root, "a.py", "def py_only():\n    return 1\n");
    write_src(root, "b.rs", "fn rs_only() -> i32 { 2 }\n");
    let mut all = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    all.index_all().unwrap();
    assert!(all.store().file_hash("a.py").unwrap().is_some());
    assert!(all.store().file_hash("b.rs").unwrap().is_some());
    drop(all);

    let mut py_only = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        lang_filter: Some("python".into()),
        embed_semantic: false,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    py_only.index_all().unwrap();
    assert!(
        py_only.store().file_hash("b.rs").unwrap().is_some(),
        "rust file must survive python --lang reindex"
    );
    assert!(py_only.store().file_hash("a.py").unwrap().is_some());
}

/// j97d.5kj8 — PRAGMA synchronous restored after file_tx and bulk rollback.
#[test]
fn synchronous_restored_after_file_tx_and_bulk_rollback() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let sync = |s: &IndexStore| -> i64 {
        s.connection()
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(sync(&store), 1, "NORMAL at open");
    store.begin_file_tx().unwrap();
    store.rollback_file_tx().unwrap();
    assert_eq!(sync(&store), 1, "NORMAL after file_tx rollback");
    store.begin_file_tx().unwrap();
    store.commit_file_tx().unwrap();
    assert_eq!(sync(&store), 1, "NORMAL after file_tx commit");
    store.begin_bulk_tx().unwrap();
    store.rollback_bulk_tx().unwrap();
    assert_eq!(sync(&store), 1, "NORMAL after bulk rollback");
}

/// j97d.37er — nested with_file_tx must not commit outer on inner error.
#[test]
fn nested_file_tx_inner_error_rolls_back_outer() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "nested".into())];
    store.upsert_file(base("keep.py", &lines, "h0")).unwrap();

    store.begin_file_tx().unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO meta(key, value) VALUES('outer_probe', '1') \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [],
        )
        .unwrap();
    // Simulate nested begin + inner rollback (poison), then outer commit attempt.
    store.begin_file_tx().unwrap();
    store.rollback_file_tx().unwrap();
    let commit = store.commit_file_tx();
    assert!(
        commit.is_err(),
        "outer commit must fail after nested rollback"
    );
    assert!(
        store.get_meta("outer_probe").unwrap().is_none(),
        "outer writes must not be visible after nested failure"
    );
    assert!(store.connection().is_autocommit());
}

/// j97d.5qpa — corrupt embedding blobs fail closed (no zero-vector default).
#[test]
fn corrupt_embedding_blob_fails_closed() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "emb".into())];
    let file_id = store.upsert_file(base("c.py", &lines, "h")).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector) \
             VALUES(?1, NULL, 'file', 1, 1, '', 't', ?2)",
            rusqlite::params![file_id, vec![1u8, 2, 3]], // not multiple of 4
        )
        .unwrap();
    let err = store.all_semantic_chunks(None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("embedding") || msg.contains("multiple of 4") || msg.contains("database"),
        "corrupt blob must error, got: {msg}"
    );
}

/// j97d.045r — dynamic SQL identifiers are allowlisted.
#[test]
fn sql_identifier_allowlist_rejects_unknown() {
    assert!(assert_sql_ident("caller", CALLER_COLUMN_ALLOWLIST).is_ok());
    assert!(assert_sql_ident("DROP TABLE", CALLER_COLUMN_ALLOWLIST).is_err());
    assert!(assert_sql_ident("files", COUNT_TABLE_ALLOWLIST).is_ok());
    assert!(assert_sql_ident("files; DROP", COUNT_TABLE_ALLOWLIST).is_err());
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    assert!(store.incoming_calls("x").is_ok());
}

/// jiyy.2 / ht1h.2 / ht1h.4 — fingerprint binds generation + content digest.
#[test]
fn ivf_fingerprint_binds_generation_and_content() {
    let dim = 4usize;
    let a = compute_ann_fingerprint(2, 9, dim, Some("semantic-v2"), 1);
    let b = compute_ann_fingerprint(2, 9, dim, Some("semantic-v2"), 2);
    assert_ne!(a, b, "generation counter must change fingerprint");
    let v1 = vec![1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let v2 = vec![0.0f32, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let d1 = vectors_content_digest(&v1);
    let d2 = vectors_content_digest(&v2);
    assert_ne!(d1, d2);
    let f1 = compute_ann_fingerprint_with_content(2, 9, dim, Some("semantic-v2"), 1, &d1);
    let f2 = compute_ann_fingerprint_with_content(2, 9, dim, Some("semantic-v2"), 1, &d2);
    assert_ne!(
        f1, f2,
        "content digest must bind fingerprint to vector identity"
    );
}

/// jiyy.5 — unified ULP threshold path rejects exact-min boundary.
#[test]
fn cosine_threshold_paths_are_unified() {
    use ast_sgrep_embed::{top_by_similarity, top_k_similarity};
    let min = 0.5_f32;
    let one = f32::from_bits(min.to_bits() + 1);
    let two = f32::from_bits(min.to_bits() + 2);
    assert!(top_k_similarity([(0, one)], 1, Some(min)).is_empty());
    assert!(top_by_similarity(vec![(0, one)], 1, Some(min)).is_empty());
    assert_eq!(top_k_similarity([(0, two)], 1, Some(min)), vec![(0, two)]);
    assert_eq!(
        top_by_similarity(vec![(0, two)], 1, Some(min)),
        vec![(0, two)]
    );
}

/// Ordinary opens fail closed; explicit reindex quarantines corruption first.
#[test]
fn explicit_reindex_quarantines_corrupt_db() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_src(root, "a.py", "def recovered_needle():\n    return 1\n");
    {
        let store = IndexStore::open(root, None).unwrap();
        let lines = [(1, "ok".into())];
        store.upsert_file(base("a.py", &lines, "h")).unwrap();
    }
    let db = root.join(".asgrep").join("index.db");
    let old_quarantine = root.join(".asgrep/index.db.corrupt");
    std::fs::write(&old_quarantine, b"older recovery copy").unwrap();
    let lexical = root.join(".asgrep/lexical.db");
    let semantic = root.join(".asgrep/semantic.ivf");
    std::fs::write(&lexical, b"stale lexical sidecar").unwrap();
    std::fs::write(&semantic, b"stale semantic sidecar").unwrap();
    // Truncate into an obviously corrupt SQLite header.
    let corrupt_bytes = b"NOT A SQLITE DATABASE............";
    std::fs::write(&db, corrupt_bytes).unwrap();
    let err = match IndexStore::open(root, None) {
        Ok(_) => panic!("corrupt DB must not open successfully"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("integrity")
            || msg.contains("quarantined")
            || msg.contains("reindex")
            || msg.contains("not a database")
            || msg.contains("database"),
        "corrupt open must fail closed, got: {msg}"
    );
    assert_eq!(std::fs::read(&db).unwrap(), corrupt_bytes);
    assert_eq!(
        std::fs::read(&old_quarantine).unwrap(),
        b"older recovery copy"
    );
    assert!(!root.join(".asgrep/index.db.corrupt.1").exists());

    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("explicit reindex should quarantine the corrupt DB");
    indexer.reindex_all().expect("replacement should index");
    assert_eq!(
        std::fs::read(root.join(".asgrep/index.db.corrupt.1")).unwrap(),
        corrupt_bytes
    );
    assert!(!lexical.exists(), "stale lexical sidecar must be removed");
    assert!(!semantic.exists(), "stale semantic sidecar must be removed");
    assert_eq!(indexer.store().status().unwrap().file_count, 1);
    assert!(indexer.store().index_data_version().unwrap() > 1_000_000);
    drop(indexer);

    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    assert!(searcher
        .search("recovered_needle")
        .unwrap()
        .hits
        .iter()
        .any(|hit| hit.file == "a.py"));
}

/// esyi.4 — busy_timeout + NORMAL sync configured on open (documented concurrent writers).
#[test]
fn open_sets_busy_timeout_and_normal_sync() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let busy: i64 = store
        .connection()
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert!(busy >= 5000, "busy_timeout must be >= 5s, got {busy}");
    let sync: i64 = store
        .connection()
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sync, 1, "NORMAL synchronous");
}

/// ht1h.3 — hybrid ResponseCache key includes local index generation.
#[test]
fn hybrid_response_cache_invalidates_on_index_generation() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        use_embed: false,
        ..SearchOptions::default()
    };
    let searcher = Searcher::with_store(store, options);
    let lines_a = [(1, "alpha sentinel unique".into())];
    searcher
        .store()
        .upsert_file(base("h.py", &lines_a, "ha"))
        .unwrap();
    let v1 = searcher.store().index_data_version().unwrap();
    assert!(!searcher.search("alpha").unwrap().hits.is_empty());
    let lines_b = [(1, "beta sentinel unique".into())];
    searcher
        .store()
        .upsert_file(base("h.py", &lines_b, "hb"))
        .unwrap();
    let v2 = searcher.store().index_data_version().unwrap();
    assert!(
        v2 > v1,
        "upsert must bump index_data_version ({v1} -> {v2})"
    );
    assert!(
        searcher.search("alpha").unwrap().hits.is_empty(),
        "generation bump must invalidate hybrid/response cache; hits={:?}",
        searcher
            .search("alpha")
            .unwrap()
            .hits
            .iter()
            .map(|h| h.excerpt.clone())
            .collect::<Vec<_>>()
    );
    assert!(!searcher.search("beta").unwrap().hits.is_empty());
}

/// j97d.3ddd — body-hash set_meta is required after upsert (smoke via meta presence).
#[test]
fn body_hash_meta_persisted_after_index() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_src(root, "m.py", "def meta_probe():\n    return 1\n");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    assert!(
        indexer.store().get_meta("body:m.py").unwrap().is_some(),
        "body hash meta must be persisted (3ddd)"
    );
}

/// Smoke: remove_file + callers/imports cleanup still works after transactional remove.
#[test]
fn remove_file_clears_graph_rows() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "import os".into())];
    let imports = [ImportRow {
        module_path: "os".into(),
        line_no: 1,
    }];
    let callers = [CallerRow {
        caller: "a".into(),
        callee: "b".into(),
        line_no: 1,
        byte_start: 0,
        byte_end: 1,
    }];
    let mut input = base("g.py", &lines, "h");
    input.imports = &imports;
    input.callers = &callers;
    store.upsert_file(input).unwrap();
    store.remove_file("g.py").unwrap();
    assert_eq!(
        store
            .connection()
            .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

/// ubs-body-hash-set-meta-1vrm: structure-skip path must only fire when body meta
/// matches; a deliberate mismatch forces a full re-upsert (not refresh_lines_only).
#[test]
fn body_hash_mismatch_prevents_structure_skip() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_src(root, "skip.py", "def original():\n    return 1\n");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let body = indexer
        .store()
        .get_meta("body:skip.py")
        .unwrap()
        .expect("body meta after first index");
    // Corrupt body fingerprint so the next index cannot structure-skip.
    indexer
        .store()
        .set_meta("body:skip.py", "stale-body-fp")
        .unwrap();
    // Trailing trivia only -- real body hash is unchanged.
    write_src(
        root,
        "skip.py",
        "def original():\n    return 1\n# trailing\n",
    );
    indexer.index_all().unwrap();
    let after = indexer
        .store()
        .get_meta("body:skip.py")
        .unwrap()
        .expect("body meta after reindex");
    assert_ne!(
        after.as_str(),
        "stale-body-fp",
        "reindex must rewrite body meta when prior value was wrong"
    );
    assert_eq!(
        after, body,
        "trailing trivia must restore the original body fingerprint"
    );
}

/// ubs-semantic-ivf-stale-swallow-skif: mark_semantic_ivf_stale must set the gate
/// bit and remove an on-disk sidecar (Result, not fire-and-forget).
#[test]
fn mark_semantic_ivf_stale_sets_flag_and_invalidates_sidecar() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let sidecar = ast_sgrep_core::semantic_ivf::semantic_ivf_path(store.db_path());
    std::fs::write(&sidecar, b"stale-ivf-bytes").unwrap();
    assert!(sidecar.is_file());
    ast_sgrep_core::semantic_ann::mark_semantic_ivf_stale(&store).unwrap();
    assert_eq!(
        store.get_meta("semantic_ivf_stale").unwrap().as_deref(),
        Some("1"),
        "stale flag must be durable so rebuild gate cannot miss it"
    );
    assert!(
        !sidecar.exists(),
        "IVF sidecar must be invalidated when mark succeeds"
    );
    // Idempotent second mark still Ok and keeps the flag.
    ast_sgrep_core::semantic_ann::mark_semantic_ivf_stale(&store).unwrap();
    assert_eq!(
        store.get_meta("semantic_ivf_stale").unwrap().as_deref(),
        Some("1")
    );
}
