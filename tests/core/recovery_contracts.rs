//! Recovery contracts: fail-closed opens and cold-start defaults (core).
//!
//! Consolidates `durable_recovery_pass1` (12 tests) plus the open/path and
//! sidecar-publish contracts from pass 2 (path confusion, atomic publish,
//! orphan tmp, frozen store) into 3 intent-grouped tests. Each test pins ONE
//! intent across multiple fault-shape facets. Every failure assertion matches
//! the documented `StoreError` discriminant, never message text.
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{
    compute_ann_fingerprint, load_semantic_ivf, peek_semantic_ivf_fingerprint, save_semantic_ivf,
};
use ast_sgrep_core::store::{read_writer_generation, writer_generation_path};
use ast_sgrep_core::tantivy_index::{sidecar_path, TantivySidecar};
use ast_sgrep_core::{
    IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError, INDEX_SCHEMA_VERSION,
};
use ast_sgrep_testkit::{
    err_of, home_names, isolated_index_session, truncate_file, write_garbage, IsolatedIndexSession,
};
use std::path::PathBuf;

/// First N bytes of a real store: a torn write that kept a valid prefix.
const TRUNCATED_LEN: u64 = 37;

/// Sibling db path under the session temp root, sharing one corpus root.
/// File-local: a one-line path join, not worth a shared helper.
fn sibling_db(session: &IsolatedIndexSession, name: &str) -> PathBuf {
    session.index_path.parent().unwrap().join(name)
}

/// INTENT: a root nested inside an indexed checkout says where the index it is
/// not finding lives, instead of reporting its own emptiness as the whole story.
/// FACETS: nested default-layout root names the enclosing index; a root with no
/// indexed ancestor keeps the plain fail-closed message.
/// KILLS: opaque-missing-index, hint-without-enclosing-index, wrong-ancestor-hint.
#[test]
fn missing_index_names_the_enclosing_checkout() {
    let session = isolated_index_session();
    let checkout = session.corpus_root.join("checkout");
    let default_db = checkout.join(".asgrep").join("index.db");
    drop(IndexStore::open(&checkout, Some(&default_db)).unwrap());
    let nested = checkout.join("crates").join("kernel").join("src");
    std::fs::create_dir_all(&nested).unwrap();

    let err = err_of(IndexStore::open_readonly(
        &nested,
        Some(&nested.join(".asgrep").join("index.db")),
    ));
    let message = err.to_string();
    assert!(message.contains("index is empty"), "{message}");
    assert!(
        message.contains(&default_db.display().to_string()),
        "the enclosing index must be named: {message}"
    );

    let orphan = session.index_path.parent().unwrap().join("orphan-root");
    std::fs::create_dir_all(&orphan).unwrap();
    let orphan_message = err_of(IndexStore::open_readonly(&orphan, None)).to_string();
    assert!(
        !orphan_message.contains("enclosing index"),
        "no ancestor index, no hint: {orphan_message}"
    );
}

/// INTENT: every store open/peek path fails with its documented discriminant
/// and never creates, migrates, or repairs as a side effect.
/// FACETS: torn→Database×2, zero-byte→Other/peek-0/writable-init,
/// missing→Other with no creation, future-schema→Other×2 + structured pair,
/// dir-as-db→Database/Other, missing-root search→Other, deep parents created,
/// readonly-dir→error (unix), frozen store→Database (unix).
/// KILLS: silent-repair, auto-create, migrate-down, serve-empty, panic /
/// partial-write, write-into-dir, serve-frozen mutants.
#[test]
fn store_open_fail_closed_matrix() {
    let session = isolated_index_session();
    let root = session.corpus_root.clone();
    let temp = session.index_path.parent().unwrap().to_path_buf();

    // Facet: torn store fails as Database on both opens; an ordinary
    // (non-reindex) open neither quarantines nor fabricates sidecars.
    {
        let db = sibling_db(&session, "torn.db");
        {
            let store = IndexStore::open(&root, Some(&db)).unwrap();
            assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        }
        let full = std::fs::read(&db).unwrap();
        assert!(full.len() as u64 > TRUNCATED_LEN, "fixture must be truncatable");
        let torn = full[..TRUNCATED_LEN as usize].to_vec();
        std::fs::write(&db, &torn).unwrap();
        for readonly in [false, true] {
            let err = err_of(if readonly {
                IndexStore::open_readonly(&root, Some(&db))
            } else {
                IndexStore::open(&root, Some(&db))
            });
            assert!(
                matches!(err, StoreError::Database(_)),
                "torn store must fail as Database (readonly={readonly}), got {err:?}"
            );
        }
        assert_eq!(std::fs::read(&db).unwrap(), torn);
        let after = home_names(&temp);
        assert!(
            after.iter().all(|n| !n.contains(".corrupt")),
            "ordinary open must not quarantine: {after:?}"
        );
        assert!(
            after.iter().all(|n| n != "lexical.db" && n != "semantic.ivf"),
            "failed open must not fabricate derived sidecars: {after:?}"
        );
    }

    // Facet: zero-byte is "no index yet" — read-only refuses (Other), peek
    // reports 0 without migrating, writable initializes in place.
    {
        let db = sibling_db(&session, "empty.db");
        std::fs::write(&db, b"").unwrap();
        let err = err_of(IndexStore::open_readonly(&root, Some(&db)));
        assert!(
            matches!(err, StoreError::Other(_)),
            "empty store must refuse read-only as Other, got {err:?}"
        );
        assert_eq!(IndexStore::peek_schema_version(&root, Some(&db)).unwrap(), 0);
        assert_eq!(std::fs::metadata(&db).unwrap().len(), 0);
        let store = IndexStore::open(&root, Some(&db)).unwrap();
        assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        assert_eq!(store.status().unwrap().file_count, 0);
    }

    // Facet: missing index errors (Other) and creates nothing.
    {
        let db = temp.join("ghost").join("nested").join("index.db");
        let err = err_of(IndexStore::peek_schema_version(&root, Some(&db)));
        assert!(matches!(err, StoreError::Other(_)), "missing peek must error as Other, got {err:?}");
        let err = err_of(IndexStore::open_readonly(&root, Some(&db)));
        assert!(
            matches!(err, StoreError::Other(_)),
            "missing read-only open must error as Other, got {err:?}"
        );
        assert!(!temp.join("ghost").exists(), "failed opens must not create parent dirs");
        let fresh = temp.join("untouched-root");
        std::fs::create_dir_all(&fresh).unwrap();
        let err = err_of(IndexStore::open_readonly(&fresh, None));
        assert!(matches!(err, StoreError::Other(_)));
        assert!(!fresh.join(".asgrep").exists());
    }

    // Facet: newer-than-binary schema refuses loudly on both opens with the
    // hand-computed (on-disk, supported) pair; the stamp is left untouched.
    {
        let db = sibling_db(&session, "future.db");
        drop(IndexStore::open(&root, Some(&db)).unwrap());
        let newer = INDEX_SCHEMA_VERSION + 1;
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(&format!("PRAGMA user_version = {newer}")).unwrap();
        }
        for refused in [
            err_of(IndexStore::open(&root, Some(&db))),
            err_of(IndexStore::open_readonly(&root, Some(&db))),
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
        let conn = rusqlite::Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let stamp: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(stamp, newer);
    }

    // Facet: directory-as-db refuses (writable Database, read-only Other) and
    // the directory is left untouched.
    {
        let db = temp.join("as-dir").join("index.db");
        std::fs::create_dir_all(&db).unwrap();
        let err = err_of(IndexStore::open(&root, Some(&db)));
        assert!(
            matches!(err, StoreError::Database(_)),
            "directory-as-db must fail writable open as Database, got {err:?}"
        );
        let err = err_of(IndexStore::open_readonly(&root, Some(&db)));
        assert!(
            matches!(err, StoreError::Other(_)),
            "directory-as-db must fail read-only open as Other, got {err:?}"
        );
        assert!(home_names(&db).is_empty(), "failed opens must not write into the directory");
    }

    // Facet: searching a missing root fails closed (Other), never creates it.
    {
        let missing = temp.join("no-such-root");
        let err = err_of(Searcher::new(SearchOptions {
            root: missing.clone(),
            use_embed: false,
            ..SearchOptions::default()
        }));
        assert!(matches!(err, StoreError::Other(_)), "missing root must fail as Other, got {err:?}");
        assert!(!missing.exists());
    }

    // Facet: a writable open creates arbitrarily deep missing parents itself
    // and lands a current-schema store.
    {
        let db = temp.join("deep").join("nested").join("index.db");
        assert!(!temp.join("deep").exists());
        let store = IndexStore::open(&root, Some(&db)).unwrap();
        assert_eq!(store.db_path(), db);
        assert!(db.is_file());
        assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        assert_eq!(store.status().unwrap().file_count, 0);
    }

    // Facet (unix): a writable open under an unwritable directory errors
    // (never panics) before any partial database file appears.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let locked = temp.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        let db = locked.join("inner").join("index.db");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Privileged runners (root) bypass permission bits; detect and skip.
        let privileged = std::fs::create_dir(locked.join("probe")).is_ok();
        let _ = std::fs::remove_dir(locked.join("probe"));
        if privileged {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("SKIP: privileged uid bypasses read-only bits (locked-dir facet)");
        } else {
            let result = IndexStore::open(&locked, Some(&db));
            // Restore before asserting so session cleanup cannot fail.
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            let err = err_of(result);
            assert!(
                matches!(err, StoreError::Other(_) | StoreError::Database(_)),
                "unwritable dir must error with a documented discriminant, got {err:?}"
            );
            assert!(!db.exists(), "failed open must not leave a partial db");
        }
    }

    // Facet (unix): an existing store frozen read-only fails every access
    // path as Database (WAL mode needs a writable directory for -shm), never
    // panics, never alters bytes. Complements the missing-db facet above.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let home = temp.join("ro-home");
        let db = home.join("index.db");
        drop(IndexStore::open(&root, Some(&db)).unwrap());
        // Control arm: the fixture is a healthy serving store before chmod.
        {
            let store = IndexStore::open_readonly(&root, Some(&db)).unwrap();
            let detail: String = store
                .connection()
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap();
            assert_eq!(detail, "ok");
        }
        // Remove sidecars AFTER the control: even a read-only open can
        // materialize -shm while the directory is writable, which would
        // change post-chmod behavior. The fault is a lone frozen db.
        ast_sgrep_testkit::remove_sqlite_sidecars(&db);
        let before = std::fs::read(&db).unwrap();
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o555)).unwrap();

        let privileged = std::fs::OpenOptions::new().write(true).open(&db).is_ok();
        if privileged {
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
            eprintln!("SKIP: privileged uid bypasses read-only bits (frozen-store facet)");
        } else {
            let ro_err = err_of(IndexStore::open_readonly(&root, Some(&db)));
            assert!(
                matches!(ro_err, StoreError::Database(_)),
                "frozen read-only open must fail as Database, got {ro_err:?}"
            );
            // SQLite opens lazily; the writable path fails at open or at
            // first write, both as Database.
            if let Ok(store) = IndexStore::open(&root, Some(&db)) {
                let err = err_of(store.set_meta("fault_probe", "1"));
                assert!(
                    matches!(err, StoreError::Database(_)),
                    "first write must fail as Database, got {err:?}"
                );
            }
            let after = std::fs::read(&db).unwrap();
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(after, before, "failed access must not alter db bytes");
        }
    }
}

/// INTENT: derived sidecars are never served torn and never published torn:
/// garbage is refused, unservable reads None, misaligned saves are rejected
/// pre-write, and tmp+rename publishes atomically (old-or-new, orphan-safe).
/// FACETS: lexical garbage+truncated→Database, IVF 5-shape→None,
/// misaligned-save 3-shape→Other, interrupted-rename old/new, orphan-tmp.
/// KILLS: serve-garbage-sidecar, forged-fingerprint, partial-write,
/// torn-publish, tmp-poison mutants.
#[test]
fn sidecar_refusal_and_publish_matrix() {
    let session = isolated_index_session();
    let root = session.corpus_root.clone();
    let temp = session.index_path.parent().unwrap().to_path_buf();

    // Facet: nonzero-length garbage or truncated lexical.db is never served.
    {
        let db = sibling_db(&session, "side.db");
        let lexical = sidecar_path(&root, Some(&db));
        std::fs::create_dir_all(lexical.parent().unwrap()).unwrap();
        write_garbage(&lexical);
        let err = err_of(TantivySidecar::open_existing_for_search(&root, Some(&db)));
        assert!(
            matches!(err, StoreError::Database(_)),
            "garbage lexical.db must fail as Database, got {err:?}"
        );

        std::fs::remove_file(&lexical).unwrap();
        drop(TantivySidecar::open_for_index(&root, Some(&db)).unwrap());
        let full = std::fs::read(&lexical).unwrap();
        assert!(full.len() as u64 > TRUNCATED_LEN);
        truncate_file(&lexical, TRUNCATED_LEN);
        let err = err_of(TantivySidecar::open_existing_for_search(&root, Some(&db)));
        assert!(
            matches!(err, StoreError::Database(_)),
            "truncated lexical.db must fail as Database, got {err:?}"
        );
    }

    // Facet: the IVF loader returns Ok(None) — never Err, never panic — for
    // missing, empty, garbage, truncated, and fingerprint-mismatched sidecars.
    {
        let dir = temp.join("ivf");
        std::fs::create_dir_all(&dir).unwrap();
        let dim = 4usize;
        let vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
        let fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 1);
        let valid = dir.join("semantic.ivf");
        save_semantic_ivf(&valid, fp, dim, &vectors, &index).unwrap();
        let valid_bytes = std::fs::read(&valid).unwrap();
        assert!(load_semantic_ivf(&valid, fp).unwrap().is_some());

        assert!(load_semantic_ivf(&dir.join("missing.ivf"), fp).unwrap().is_none());
        let empty = dir.join("empty.ivf");
        std::fs::write(&empty, b"").unwrap();
        assert!(load_semantic_ivf(&empty, fp).unwrap().is_none());
        let garbage = dir.join("garbage.ivf");
        write_garbage(&garbage);
        assert!(load_semantic_ivf(&garbage, fp).unwrap().is_none());
        assert_eq!(peek_semantic_ivf_fingerprint(&garbage), None);
        let torn = dir.join("torn.ivf");
        std::fs::write(&torn, &valid_bytes[..valid_bytes.len() / 2]).unwrap();
        assert!(load_semantic_ivf(&torn, fp).unwrap().is_none());
        let other_fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 2);
        assert_ne!(fp, other_fp);
        assert!(load_semantic_ivf(&valid, other_fp).unwrap().is_none());
        assert_eq!(peek_semantic_ivf_fingerprint(&valid), Some(fp));
    }

    // Facet: dimension-misaligned IVF saves are rejected as Other before
    // touching the filesystem: no partial file, no temp left behind.
    {
        let dir = temp.join("ivf-reject");
        std::fs::create_dir_all(&dir).unwrap();
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
            assert!(matches!(err, StoreError::Other(_)), "{name} must reject as Other, got {err:?}");
            assert!(!path.exists(), "rejected save must not create {name}");
        }
        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert!(entries.is_empty(), "rejected saves must leave no files");
    }

    // Facet: a crash between tmp-write and rename serves exactly old (new
    // invisible); completing the rename serves exactly new. Never mixed.
    {
        let dir = temp.join("ivf-publish");
        std::fs::create_dir_all(&dir).unwrap();
        let dim = 4usize;
        let old_vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let new_vectors: Vec<f32> = (0..16).map(|i| 100.0 + i as f32).collect();
        let old_fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 1);
        let new_fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 2);
        assert_ne!(old_fp, new_fp);

        let final_path = dir.join("semantic.ivf");
        save_semantic_ivf(
            &final_path,
            old_fp,
            dim,
            &old_vectors,
            &SemanticAnnIndex::build_from_flat(&old_vectors, dim),
        )
        .unwrap();

        let scratch = dir.join("scratch.ivf");
        save_semantic_ivf(
            &scratch,
            new_fp,
            dim,
            &new_vectors,
            &SemanticAnnIndex::build_from_flat(&new_vectors, dim),
        )
        .unwrap();
        let new_bytes = std::fs::read(&scratch).unwrap();
        std::fs::remove_file(&scratch).unwrap();
        let orphan_tmp = dir.join(format!(".semantic.ivf.{}.4242.tmp", std::process::id()));
        std::fs::write(&orphan_tmp, &new_bytes).unwrap();

        assert_eq!(
            load_semantic_ivf(&final_path, old_fp).unwrap().unwrap().vectors,
            old_vectors,
            "pre-rename crash must still serve exactly the old payload"
        );
        assert!(load_semantic_ivf(&final_path, new_fp).unwrap().is_none());

        std::fs::rename(&orphan_tmp, &final_path).unwrap();
        assert!(load_semantic_ivf(&final_path, old_fp).unwrap().is_none());
        assert_eq!(
            load_semantic_ivf(&final_path, new_fp).unwrap().unwrap().vectors,
            new_vectors,
            "post-rename load must serve exactly the new payload"
        );
    }

    // Facet: a stale tmp left by a crashed save never poisons the next save.
    {
        let dir = temp.join("ivf-orphan");
        std::fs::create_dir_all(&dir).unwrap();
        write_garbage(&dir.join(".semantic.ivf.99999.0.tmp"));
        let dim = 4usize;
        let vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 1);
        let final_path = dir.join("semantic.ivf");
        save_semantic_ivf(
            &final_path,
            fp,
            dim,
            &vectors,
            &SemanticAnnIndex::build_from_flat(&vectors, dim),
        )
        .unwrap();
        assert_eq!(
            load_semantic_ivf(&final_path, fp).unwrap().unwrap().vectors,
            vectors,
            "save beside an orphan tmp must publish exactly"
        );
    }
}

/// INTENT: cold-start state serves as zero, never errors — an empty corpus
/// serves deterministic zero hits, and a missing/empty/corrupt
/// writer-generation stamp reads as epoch 0 (the cold-start protocol).
/// KILLS: phantom-hit, stamp-gating mutants.
#[test]
fn cold_start_state_serves_as_zero() {
    let session = isolated_index_session();

    // Facet: an empty corpus serves 0 hits twice with hand-computed 0/0.
    {
        let mut indexer = Indexer::new(IndexOptions {
            root: session.corpus_root.clone(),
            index_path: Some(session.index_path.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap();
        indexer.index_all().unwrap();
        assert_eq!(indexer.store().status().unwrap().file_count, 0);
        assert_eq!(indexer.store().status().unwrap().line_count, 0);
        drop(indexer);

        let searcher = Searcher::new(SearchOptions {
            root: session.corpus_root.clone(),
            index_path: Some(session.index_path.clone()),
            use_embed: false,
            ..SearchOptions::default()
        })
        .unwrap();
        for _ in 0..2 {
            let response = searcher.search("needle_no_such_token_xyz").unwrap();
            assert_eq!(response.hits.len(), 0, "empty index must serve zero hits");
        }
    }

    // Facet: missing, corrupt, and empty stamps all read as epoch 0.
    // Fresh session: the stamp path is shared per home dir, so the indexing
    // facet above would otherwise leak its stamp into the "missing" arm.
    {
        let stamp_session = isolated_index_session();
        let db = sibling_db(&stamp_session, "home.db");
        assert_eq!(read_writer_generation(&stamp_session.corpus_root, Some(&db)), 0);
        let stamp = writer_generation_path(&stamp_session.corpus_root, Some(&db));
        std::fs::create_dir_all(stamp.parent().unwrap()).unwrap();
        std::fs::write(&stamp, b"not-a-number\n").unwrap();
        assert_eq!(read_writer_generation(&stamp_session.corpus_root, Some(&db)), 0);
        std::fs::write(&stamp, b"").unwrap();
        assert_eq!(read_writer_generation(&stamp_session.corpus_root, Some(&db)), 0);
    }
}

/// INTENT: one `.asgrep` per checkout. A writable open inside an indexed tree
/// refuses to create a second index and names the one that already exists.
/// FACETS: nested root refuses with the enclosing path; explicit index path
/// keeps its own location.
/// KILLS: silent-second-index, wrong-enclosing-pointer, guard-on-explicit-path.
#[test]
fn writable_open_refuses_a_second_index_inside_an_indexed_checkout() {
    let session = isolated_index_session();
    let checkout = session.corpus_root.join("checkout");
    let default_db = checkout.join(".asgrep").join("index.db");
    drop(IndexStore::open(&checkout, Some(&default_db)).unwrap());
    let nested = checkout.join("crates").join("kernel");
    std::fs::create_dir_all(&nested).unwrap();

    let message = err_of(IndexStore::open(&nested, None)).to_string();
    assert!(
        message.contains("refusing to create a second index"),
        "nested writable open must refuse: {message}"
    );
    assert!(
        message.contains(&default_db.display().to_string()),
        "the enclosing index must be named: {message}"
    );
    assert!(
        !nested.join(".asgrep").exists(),
        "the refusal must not leave a partial index directory"
    );

    // An explicit index path is a deliberate separate index, never nested magic.
    let explicit = nested.join("own.db");
    drop(IndexStore::open(&nested, Some(&explicit)).unwrap());
    assert!(explicit.is_file(), "explicit index paths stay allowed");
}


/// INTENT: the one-index rule is scoped to the checkout. An index that merely
/// lives above the git work tree root (a home dir, a shared scratch tree) is not
/// this project's and must not block a new project from indexing itself.
/// FACETS: same-work-tree ancestor refuses; outside-the-work-tree index ignored.
/// KILLS: global-parent-index-capture, work-tree-blind-walk.
#[test]
fn only_the_checkout_own_index_blocks_a_second_one() {
    let session = isolated_index_session();
    let outer = session.corpus_root.join("outer");
    // A parent that happens to hold an index, with no checkout of its own.
    drop(IndexStore::open(&outer, None).unwrap());

    let checkout = outer.join("fresh-project");
    std::fs::create_dir_all(checkout.join(".git")).unwrap();
    let nested = checkout.join("src");
    std::fs::create_dir_all(&nested).unwrap();

    // The outer index is outside this work tree: the new project indexes itself.
    drop(IndexStore::open(&checkout, None).unwrap());
    assert!(
        checkout.join(".asgrep").join("index.db").is_file(),
        "a fresh project under an indexed parent must still be able to index"
    );

    // Inside the work tree the checkout's own index now blocks a nested one.
    let message = err_of(IndexStore::open(&nested, None)).to_string();
    assert!(
        message.contains("refusing to create a second index"),
        "same-checkout nesting must still refuse: {message}"
    );
    assert!(!nested.join(".asgrep").exists());
}

