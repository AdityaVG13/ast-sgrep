use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::semantic_chunk::SemanticChunkInput;
use ast_sgrep_core::semantic_ivf::compute_ann_fingerprint;
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{IndexStore, SearchOptions, Searcher};
use tempfile::TempDir;

// Regression for bead ast-sgrep-44a4 (F-02): SemanticCache + ANN fingerprint
// collided after delete+re-add when max_id was reused. Cache hit used
// max_id+lang_filter+embed_backend only; fingerprint used chunks.len()+max_id+
// dim+backend. A file deleted then re-added could yield an identical key with
// stale chunks/vectors. Fix: a monotonic semantic_data_version meta bumped on
// every semantic_chunks mutation (insert/remove/clear), included in both the
// SemanticCache identity check and the IVF fingerprint hash.
fn base<'a>(
    path: &'a str,
    lines: &'a [(u32, String)],
    hash: &'a str,
    chunks: &'a [SemanticChunkInput],
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
        semantic_chunks: chunks,
        embed_semantic: true,
        embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
    }
}

fn chunk(name: &str, excerpt: &str) -> SemanticChunkInput {
    SemanticChunkInput {
        symbol_name: name.into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        excerpt: excerpt.into(),
        callers: vec![],
        callees: vec![],
        doc: String::new(),
        scope: String::new(),
    }
}

#[test]
fn semantic_data_version_bumps_on_insert_remove_and_readd() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();

    let v0 = store.semantic_data_version().unwrap();
    assert_eq!(v0, 0, "fresh store starts at version 0");

    // Index file A with one semantic chunk -> version bumps to 1.
    let lines_a = [(1u32, "def foo(): pass".into())];
    let chunks_a = [chunk("foo", "def foo(): pass")];
    store
        .upsert_file(base("a.py", &lines_a, "h1", &chunks_a))
        .unwrap();
    let v1 = store.semantic_data_version().unwrap();
    assert_eq!(v1, 1, "insert must bump data_version");

    let max_id_after_add = store.semantic_chunk_max_id().unwrap().unwrap_or(0);
    let backend = store
        .get_meta("embed_backend")
        .unwrap()
        .unwrap_or_else(|| "semantic".into());
    let dim = store
        .get_meta("embed_dim")
        .unwrap()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let fp_after_add = compute_ann_fingerprint(1, max_id_after_add, dim, Some(&backend), v1);

    // Remove file A -> version bumps to 2.
    store.remove_file("a.py").unwrap();
    let v2 = store.semantic_data_version().unwrap();
    assert_eq!(v2, 2, "remove must bump data_version");
    assert_eq!(
        store.semantic_chunk_max_id().unwrap(),
        None,
        "no chunks remain after remove"
    );

    // Re-add file A with the SAME content. Even if SQLite reuses the rowid
    // (max_id collides with the pre-delete value), the data_version must differ
    // so the SemanticCache misses and the IVF fingerprint changes.
    store
        .upsert_file(base("a.py", &lines_a, "h1", &chunks_a))
        .unwrap();
    let v3 = store.semantic_data_version().unwrap();
    assert_eq!(v3, 3, "re-add must bump data_version");

    let max_id_after_readd = store.semantic_chunk_max_id().unwrap().unwrap_or(0);
    let fp_after_readd = compute_ann_fingerprint(1, max_id_after_readd, dim, Some(&backend), v3);

    // The fingerprint must differ across the delete boundary even if max_id
    // happens to be reused, because data_version is hashed in.
    let fp_readd_with_old_version =
        compute_ann_fingerprint(1, max_id_after_readd, dim, Some(&backend), v1);
    assert_ne!(
        fp_after_readd, fp_readd_with_old_version,
        "fingerprint must be sensitive to data_version even when max_id collides"
    );

    // Sanity: if SQLite did not reuse the rowid, the fingerprints differ anyway;
    // if it did, the data_version still saves us. Either way the post-readd
    // fingerprint must not equal the pre-delete one.
    let _ = fp_after_add; // computed for documentation; the v1 vs v3 gap is the real gate.
    assert_ne!(
        compute_ann_fingerprint(1, max_id_after_readd, dim, Some(&backend), v3),
        compute_ann_fingerprint(1, max_id_after_add, dim, Some(&backend), v1),
        "pre-delete and post-readd fingerprints must differ"
    );
}

#[test]
fn delete_readd_with_changed_content_serves_fresh_semantic_vectors() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(store.db_path().to_path_buf()),
        use_embed: true,
        use_semantic_only: true,
        ann_threshold: Some(usize::MAX),
        ..SearchOptions::default()
    };
    let searcher = Searcher::with_store(store, options);

    let old_lines = [(1u32, "def legacy_handler(): return 'obsolete'".into())];
    let old_chunks = [chunk(
        "legacy_handler",
        "credential legacy obsolete handler",
    )];
    searcher
        .store()
        .upsert_file(base("a.py", &old_lines, "old-hash", &old_chunks))
        .unwrap();
    let old = searcher.search("credential legacy obsolete").unwrap();
    assert!(old.hits.iter().any(|hit| {
        (hit.kind == HitKind::Embed
            || hit.contributors.iter().any(|k| *k == HitKind::Embed))
            && hit.symbol.as_deref() == Some("legacy_handler")
    }));

    searcher.store().remove_file("a.py").unwrap();
    let fresh_lines = [(1u32, "def fresh_handler(): return 'renewed'".into())];
    let fresh_chunks = [chunk("fresh_handler", "payment renewal fresh handler")];
    searcher
        .store()
        .upsert_file(base("a.py", &fresh_lines, "fresh-hash", &fresh_chunks))
        .unwrap();

    let fresh = searcher.search("payment renewal fresh").unwrap();
    assert!(fresh.hits.iter().any(|hit| {
        (hit.kind == HitKind::Embed
            || hit.contributors.iter().any(|k| *k == HitKind::Embed))
            && hit.symbol.as_deref() == Some("fresh_handler")
    }));
    assert!(!fresh
        .hits
        .iter()
        .any(|hit| hit.symbol.as_deref() == Some("legacy_handler")));
}

#[test]
fn clear_all_data_bumps_semantic_data_version() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1u32, "def bar(): pass".into())];
    let chunks = [chunk("bar", "def bar(): pass")];
    store
        .upsert_file(base("b.py", &lines, "h1", &chunks))
        .unwrap();
    let v_before = store.semantic_data_version().unwrap();
    assert_eq!(v_before, 1);

    store.clear_all_data().unwrap();
    let v_after = store.semantic_data_version().unwrap();
    assert_eq!(v_after, 2, "clear_all_data must bump semantic_data_version");
}

// Regression for the emb-empty re-upsert path: a re-upsert of an existing file
// with embed_semantic=false (or empty chunks) reaches insert_semantic_chunks
// AFTER upsert_file_row's delete_file_children already removed the file's old
// semantic_chunks. The emb-empty early return must still bump
// semantic_data_version so SemanticCache + IVF fingerprint detect the deletion
// (bead ast-sgrep-44a4). Without this bump, a stale cache hit returns deleted
// chunks as phantom hits.
#[test]
fn reupsert_with_empty_chunks_bumps_data_version_after_deleting_old() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();

    // File A with chunks -> version 1.
    let lines_a = [(1u32, "def foo(): return 1".into())];
    let chunks_a = [chunk("foo", "def foo(): return 1")];
    store
        .upsert_file(base("a.py", &lines_a, "h1", &chunks_a))
        .unwrap();
    assert_eq!(store.semantic_data_version().unwrap(), 1);

    // File B with chunks -> version 2; max_id advances past A's chunks.
    let lines_b = [(1u32, "def bar(): return 2".into())];
    let chunks_b = [chunk("bar", "def bar(): return 2")];
    store
        .upsert_file(base("b.py", &lines_b, "h2", &chunks_b))
        .unwrap();
    let v_after_b = store.semantic_data_version().unwrap();
    assert_eq!(v_after_b, 2);
    assert_eq!(
        store.semantic_chunk_stats(None).unwrap().count,
        2,
        "two chunks indexed"
    );

    // Re-upsert file A with NO chunks (empty slice). The structure fingerprint
    // differs (chunks went [foo] -> []), so upsert_file_inner runs:
    // upsert_file_row deletes A's old chunks via delete_file_children, then
    // insert_semantic_chunks hits the emb-empty early return. The bump on that
    // path is what this test guards. embed_semantic stays true but emb is empty
    // because the chunks slice is empty.
    store
        .upsert_file(base("a.py", &lines_a, "h3", &[]))
        .unwrap();
    let v_after_reupsert = store.semantic_data_version().unwrap();
    assert_eq!(
        v_after_reupsert, 3,
        "emb-empty re-upsert that deleted old chunks must bump semantic_data_version"
    );
    assert_eq!(
        store.semantic_chunk_stats(None).unwrap().count,
        1,
        "only file B's chunk remains after A's chunks were deleted"
    );
}
