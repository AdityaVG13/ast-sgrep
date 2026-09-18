//! R1 recovery-contract oracles for ast-sgrep-core durable state (pass 1).
//!
//! Prior art already covers (do NOT re-pin here): IVF atomic tmp+rename save,
//! schema-only/zero-byte lexical sidecar refused as not-ready, `clear_all_data`
//! completeness, `remove_file` meta cleanup, `--lang` no-wipe, sync-restore
//! after tx commit/rollback, nested-tx poisoning, corrupt embedding blobs,
//! SQL identifier allowlists, IVF fingerprint binding, cosine-threshold unity,
//! corrupt-DB quarantine on explicit reindex (+ sidecar removal + `.corrupt.N`
//! allocation), busy-timeout/NORMAL-sync on open, hybrid cache invalidation on
//! generation bump, body-hash meta persistence, graph-row cleanup, the
//! stamp-14 re-key migration + read-only/codemod refusal, WAL/pragma profiles,
//! snapshot-generation fencing, and the HOME-unset cache refusal.
//!
//! This file pins the UNCOVERED recovery surface: truncated stores, empty
//! stores, newer-than-binary schemas, missing-index peeks, garbage lexical
//! sidecars, unservable IVF sidecars, misaligned IVF saves, missing roots,
//! missing parents, read-only dirs, empty-index serving, and the
//! writer-generation fail-open default.
//!
//! Every failure assertion matches the documented `StoreError` discriminant
//! (`Database` / `Io` / `Other`), never message text. The one structured
//! exception is `StoreError::parse_schema_mismatch`, the documented parser
//! for the newer-schema refusal, asserted against a hand-computed pair.
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{
    compute_ann_fingerprint, load_semantic_ivf, peek_semantic_ivf_fingerprint, save_semantic_ivf,
};
use ast_sgrep_core::store::{read_writer_generation, writer_generation_path};
use ast_sgrep_core::tantivy_index::TantivySidecar;
use ast_sgrep_core::{
    IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError, INDEX_SCHEMA_VERSION,
};
use tempfile::TempDir;

/// First N bytes of a real store: a torn write that kept a valid prefix.
const TRUNCATED_LEN: usize = 37;

/// `IndexStore`/`Searcher` do not implement `Debug`, so `unwrap_err` cannot
/// be used on their constructors; this is the equivalent fail-loud extractor.
fn err_of<T>(result: Result<T, StoreError>) -> StoreError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(err) => err,
    }
}

/// A truncated store must fail closed with the `Database` discriminant on
/// both writable and read-only opens, and an ordinary (non-reindex) open must
/// not quarantine, repair, or otherwise mutate the torn file.
#[test]
fn truncated_index_db_fails_closed_as_database_without_side_effects() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("torn").join("index.db");
    {
        let store = IndexStore::open(root, Some(&db)).unwrap();
        assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    }
    let full = std::fs::read(&db).unwrap();
    assert!(full.len() > TRUNCATED_LEN, "fixture must be truncatable");
    let torn = full[..TRUNCATED_LEN].to_vec();
    std::fs::write(&db, &torn).unwrap();
    let dir_names = || {
        let mut names: Vec<String> = std::fs::read_dir(db.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    let err = err_of(IndexStore::open(root, Some(&db)));
    assert!(
        matches!(err, StoreError::Database(_)),
        "torn store must fail as Database, got {err:?}"
    );
    let err = err_of(IndexStore::open_readonly(root, Some(&db)));
    assert!(
        matches!(err, StoreError::Database(_)),
        "torn store must fail as Database read-only, got {err:?}"
    );
    // No quarantine, no repair from an ordinary open: the torn bytes are
    // intact and no `.corrupt*` quarantine appears. (SQLite itself may create
    // empty -wal/-shm on the failed open attempt; that is engine noise, not
    // recovery action, so only quarantine/derived sidecars are asserted.)
    assert_eq!(std::fs::read(&db).unwrap(), torn);
    let after = dir_names();
    assert!(
        after.iter().all(|n| !n.contains(".corrupt")),
        "ordinary open must not quarantine: {after:?}"
    );
    assert!(
        after
            .iter()
            .all(|n| n != "lexical.db" && n != "semantic.ivf"),
        "failed open must not fabricate derived sidecars: {after:?}"
    );
}

/// A zero-byte store is unambiguously "no index yet": read-only opens refuse
/// loudly (`Other`), a peek reports schema 0 without migrating, and a
/// writable open initializes it in place to the current schema with zero rows.
#[test]
fn empty_store_readonly_refuses_writable_initializes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("empty").join("index.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    std::fs::write(&db, b"").unwrap();

    let err = err_of(IndexStore::open_readonly(root, Some(&db)));
    assert!(
        matches!(err, StoreError::Other(_)),
        "empty store must refuse read-only as Other, got {err:?}"
    );
    assert_eq!(
        IndexStore::peek_schema_version(root, Some(&db)).unwrap(),
        0,
        "peek of a zero-byte store reports version 0"
    );
    // Peek must not migrate: still zero bytes on disk.
    assert_eq!(std::fs::metadata(&db).unwrap().len(), 0);

    let store = IndexStore::open(root, Some(&db)).unwrap();
    assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    assert_eq!(store.status().unwrap().file_count, 0);
}

/// A store stamped newer than this binary must refuse LOUDLY on both writable
/// and read-only opens (never serve, never migrate down), and the refusal
/// must carry the hand-computed (on-disk, supported) pair through the
/// documented `parse_schema_mismatch` parser. The stamp is left untouched.
#[test]
fn schema_newer_than_binary_refuses_both_opens_with_structured_pair() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("future").join("index.db");
    {
        IndexStore::open(root, Some(&db)).unwrap();
    }
    let newer = INDEX_SCHEMA_VERSION + 1;
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {newer}"))
            .unwrap();
    }
    for refused in [
        err_of(IndexStore::open(root, Some(&db))),
        err_of(IndexStore::open_readonly(root, Some(&db))),
    ] {
        assert!(
            matches!(refused, StoreError::Other(_)),
            "newer schema must refuse as Other, got {refused:?}"
        );
        assert_eq!(
            StoreError::parse_schema_mismatch(&refused.to_string()),
            Some((newer, INDEX_SCHEMA_VERSION)),
            "refusal must carry the structured version pair"
        );
    }
    // Refusal is read-only: the future stamp survives both attempts.
    let conn = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let stamp: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamp, newer);
}

/// Peeking or read-only-opening a missing index errors (`Other`) and creates
/// nothing: no parent dirs, no `.asgrep` dir, no zero-byte file.
#[test]
fn missing_index_peek_and_readonly_open_error_without_creating() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("ghost").join("nested").join("index.db");

    let err = err_of(IndexStore::peek_schema_version(root, Some(&db)));
    assert!(
        matches!(err, StoreError::Other(_)),
        "missing peek must error as Other, got {err:?}"
    );
    let err = err_of(IndexStore::open_readonly(root, Some(&db)));
    assert!(
        matches!(err, StoreError::Other(_)),
        "missing read-only open must error as Other, got {err:?}"
    );
    assert!(
        !root.join("ghost").exists(),
        "failed opens must not create parent dirs"
    );

    // Default layout: no `.asgrep` dir may appear as a side effect either.
    let fresh = root.join("untouched-root");
    std::fs::create_dir_all(&fresh).unwrap();
    let err = err_of(IndexStore::open_readonly(&fresh, None));
    assert!(matches!(err, StoreError::Other(_)));
    assert!(!fresh.join(".asgrep").exists());
}

/// A nonzero-length garbage or truncated lexical sidecar must never be
/// served: the search open fails closed with the `Database` discriminant
/// (header read fails during connection setup), never `Ok(Some(_))`.
#[test]
fn garbage_lexical_sidecar_fails_closed_as_database() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("side").join("index.db");
    let lexical = root.join("side").join("lexical.db");
    std::fs::create_dir_all(lexical.parent().unwrap()).unwrap();

    std::fs::write(&lexical, vec![0xABu8; 200]).unwrap();
    let err = err_of(TantivySidecar::open_existing_for_search(root, Some(&db)));
    assert!(
        matches!(err, StoreError::Database(_)),
        "garbage lexical.db must fail as Database, got {err:?}"
    );

    // Truncated variant: a real sidecar torn to a header fragment.
    std::fs::remove_file(&lexical).unwrap();
    drop(TantivySidecar::open_for_index(root, Some(&db)).unwrap());
    let full = std::fs::read(&lexical).unwrap();
    assert!(full.len() > TRUNCATED_LEN);
    std::fs::write(&lexical, &full[..TRUNCATED_LEN]).unwrap();
    let err = err_of(TantivySidecar::open_existing_for_search(root, Some(&db)));
    assert!(
        matches!(err, StoreError::Database(_)),
        "truncated lexical.db must fail as Database, got {err:?}"
    );
}

/// The IVF sidecar loader refuses to serve anything it cannot fully validate,
/// returning `Ok(None)` (never `Err`, never panic): missing file, empty file,
/// garbage bytes, a truncated valid sidecar, and a fingerprint mismatch all
/// read as "no sidecar". The fingerprint peek likewise returns `None` on
/// garbage instead of a forged value.
#[test]
fn unservable_ivf_sidecars_read_as_none_without_error() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let dim = 4usize;
    let vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 1);
    let valid = dir.join("semantic.ivf");
    save_semantic_ivf(&valid, fp, dim, &vectors, &index).unwrap();
    let valid_bytes = std::fs::read(&valid).unwrap();

    // Sanity: the fixture itself round-trips before we start breaking copies.
    assert!(load_semantic_ivf(&valid, fp).unwrap().is_some());

    assert!(
        load_semantic_ivf(&dir.join("missing.ivf"), fp).unwrap().is_none(),
        "missing sidecar reads as None"
    );
    let empty = dir.join("empty.ivf");
    std::fs::write(&empty, b"").unwrap();
    assert!(load_semantic_ivf(&empty, fp).unwrap().is_none());
    let garbage = dir.join("garbage.ivf");
    std::fs::write(&garbage, vec![0xABu8; 200]).unwrap();
    assert!(load_semantic_ivf(&garbage, fp).unwrap().is_none());
    assert_eq!(
        peek_semantic_ivf_fingerprint(&garbage),
        None,
        "peek on garbage must not forge a fingerprint"
    );
    let torn = dir.join("torn.ivf");
    std::fs::write(&torn, &valid_bytes[..valid_bytes.len() / 2]).unwrap();
    assert!(load_semantic_ivf(&torn, fp).unwrap().is_none());
    let other_fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 2);
    assert_ne!(fp, other_fp);
    assert!(
        load_semantic_ivf(&valid, other_fp).unwrap().is_none(),
        "fingerprint mismatch reads as None"
    );
    assert_eq!(peek_semantic_ivf_fingerprint(&valid), Some(fp));
}

/// Dimension-misaligned IVF saves are rejected with the `Other` discriminant
/// before touching the filesystem: no partial file, no temp left behind.
#[test]
fn save_semantic_ivf_rejects_misaligned_vectors_as_other() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let index = SemanticAnnIndex::build_from_flat(&vectors, 4);

    for (name, dim, flat) in [
        ("zero-dim.ivf", 0usize, vectors.as_slice()),
        ("empty.ivf", 4usize, &[][..]),
        ("ragged.ivf", 2usize, &[1.0f32, 2.0, 3.0, 4.0, 5.0][..]),
    ] {
        let path = dir.join(name);
        let fp = compute_ann_fingerprint(1, 1, 4, Some("test"), 1);
        let err = save_semantic_ivf(&path, fp, dim, flat, &index).unwrap_err();
        assert!(
            matches!(err, StoreError::Other(_)),
            "{name} must reject as Other, got {err:?}"
        );
        assert!(!path.exists(), "rejected save must not create {name}");
    }
    // No temp droppings beside the rejections.
    let entries: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
    assert!(entries.is_empty(), "rejected saves must leave no files");
}

/// Searching a project root that does not exist fails closed (`Other`), never
/// panics, never creates the root as a side effect.
#[test]
fn searcher_new_on_missing_root_fails_closed_as_other() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("no-such-root");
    let err = err_of(Searcher::new(SearchOptions {
        root: missing.clone(),
        use_embed: false,
        ..SearchOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Other(_)),
        "missing root must fail as Other, got {err:?}"
    );
    assert!(!missing.exists());
}

/// A writable open creates arbitrarily deep missing parents itself and lands
/// a current-schema store: the caller never pre-creates index directories.
#[test]
fn writable_open_creates_missing_parents_deterministically() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("deep").join("nested").join("index.db");
    assert!(!root.join("deep").exists());

    let store = IndexStore::open(root, Some(&db)).unwrap();
    assert_eq!(store.db_path(), db);
    assert!(db.is_file());
    assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    assert_eq!(store.status().unwrap().file_count, 0);
}

/// A writable open under an unwritable directory errors (never panics); the
/// failure surfaces before any partial database file appears.
#[test]
#[cfg(unix)]
fn read_only_dir_writable_open_errors_without_panic() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let locked = temp.path().join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    let db = locked.join("inner").join("index.db");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = IndexStore::open(&locked, Some(&db));
    // Restore before asserting so TempDir cleanup cannot fail.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    let err = err_of(result);
    assert!(
        matches!(err, StoreError::Other(_) | StoreError::Database(_)),
        "unwritable dir must error with a documented discriminant, got {err:?}"
    );
    assert!(!db.exists(), "failed open must not leave a partial db");
}

/// An index over an empty corpus serves zero hits deterministically: two
/// identical searches both answer `Ok` with exactly 0 hits, and the store
/// reports hand-computed 0 files / 0 lines.
#[test]
fn empty_index_serves_zero_hits_deterministically() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("corpus");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let db = temp.path().join("home").join("index.db");

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(db.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    assert_eq!(indexer.store().status().unwrap().file_count, 0);
    assert_eq!(indexer.store().status().unwrap().line_count, 0);
    drop(indexer);

    let searcher = Searcher::new(SearchOptions {
        root: root.clone(),
        index_path: Some(db.clone()),
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    for _ in 0..2 {
        let response = searcher.search("needle_no_such_token_xyz").unwrap();
        assert_eq!(response.hits.len(), 0, "empty index must serve zero hits");
    }
}

/// The writer-generation stamp is DOCUMENTED fail-open: a missing stamp, an
/// empty stamp, or a corrupt (non-numeric) stamp all read as epoch 0 — the
/// cold-start protocol — rather than erroring.
#[test]
fn writer_generation_absent_or_corrupt_reads_zero_fail_open() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("home").join("index.db");
    assert_eq!(
        read_writer_generation(root, Some(&db)),
        0,
        "missing stamp reads as epoch 0"
    );

    let stamp = writer_generation_path(root, Some(&db));
    std::fs::create_dir_all(stamp.parent().unwrap()).unwrap();
    std::fs::write(&stamp, b"not-a-number\n").unwrap();
    assert_eq!(
        read_writer_generation(root, Some(&db)),
        0,
        "corrupt stamp reads as epoch 0"
    );
    std::fs::write(&stamp, b"").unwrap();
    assert_eq!(
        read_writer_generation(root, Some(&db)),
        0,
        "empty stamp reads as epoch 0"
    );
}
