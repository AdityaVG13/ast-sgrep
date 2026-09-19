//! Recovery crash drills: full crash→recover→serve through public APIs (core).
//!
//! Consolidates `durable_recovery_pass4` (7 tests) plus the pass-2 SIGKILL
//! fault into 5 intent-grouped drills. Each drill builds a real store,
//! captures a pre-crash serving baseline, crashes it, runs the documented
//! recovery path, then SERVES real queries and proves the post-recovery hits
//! are identical to the baseline. The chained drill also pins the
//! quarantine-monotonicity relation (fresh slot, first intact, reconvergence
//! across fault shapes). All assertions match discriminants, row counts, byte
//! equality, or sorted hit keys — never message text.
use ast_sgrep_core::tantivy_index::sidecar_path;
use ast_sgrep_core::{
    IndexOptions, IndexStore, Indexer, StoreError, INDEX_SCHEMA_VERSION,
};
use ast_sgrep_testkit::{
    assert_torn, build_and_quiet, corpus_session, err_of, flip_bytes, home_names,
    quarantine_path, quiesced_db_bytes, remove_sqlite_sidecars, search_parity_keys,
    store_snapshot, truncate_file, upsert_test_file, RECOVERY_CORPUS,
};
use std::path::Path;
use std::time::{Duration, Instant};

const QUERIES: &[&str] = &["alpha_needle", "beta_needle", "gamma_caller"];

fn integrity(root: &Path, db: &Path) -> String {
    let store = IndexStore::open_readonly(root, Some(db)).unwrap();
    store.connection().query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap()
}

fn assert_real_hits(keys: &[(String, Vec<String>)]) {
    assert!(keys.iter().any(|(_, hits)| !hits.is_empty()), "parity must be over real hits, not mutual emptiness: {keys:?}");
}

/// INTENT: SIGKILL inside an uncommitted bulk tx loses only uncommitted rows —
/// a plain reopen rolls back with no quarantine and serves the pre-crash
/// baseline, and an explicit force-reindex converges and still serves it.
/// Merges the pass-2 kill fault (empty-store reopen + rebuild) with the
/// pass-4 kill drill (populated baseline serve); the populated baseline is
/// strictly richer (committed rows preserved, not just emptiness).
/// KILLS: phantom-row, quarantine-on-crash, kill-residue mutants.
#[test]
#[cfg(unix)]
fn sigkill_mid_bulk_tx_loses_only_uncommitted() {
    use std::os::unix::process::ExitStatusExt;

    let session = corpus_session();
    let root = session.corpus_root.clone();
    let db = session.index_path.clone();
    let temp = db.parent().unwrap().to_path_buf();

    build_and_quiet(&root, &db, false, false);
    let snap_before = store_snapshot(&root, &db);
    let hits_before = search_parity_keys(&root, &db, QUERIES, false);
    assert_real_hits(&hits_before);
    assert_eq!(
        IndexStore::open_readonly(&root, Some(&db)).unwrap().status().unwrap().file_count,
        RECOVERY_CORPUS.len()
    );

    // CRASH: a real child process is SIGKILLed inside an uncommitted bulk tx.
    let ready = temp.join("child-ready");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_writer_entry", "--nocapture"])
        .env("ASGREP_DRILL_ROOT", &root)
        .env("ASGREP_DRILL_DB", &db)
        .env("ASGREP_DRILL_READY", &ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "child writer never signalled readiness");
        assert!(child.try_wait().unwrap().is_none(), "child exited before the kill window");
        std::thread::sleep(Duration::from_millis(50));
    }
    let pid = child.id();
    let killed = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status().unwrap();
    assert!(killed.success(), "kill -9 must dispatch");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "killed child never reaped");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.signal(), Some(9), "child must die by SIGKILL");

    // RECOVER: plain reopen rolls back the uncommitted tx; no quarantine.
    assert!(home_names(db.parent().unwrap()).iter().all(|n| !n.contains(".corrupt")));
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    assert_eq!(
        store.status().unwrap().file_count,
        RECOVERY_CORPUS.len(),
        "uncommitted victim rows must be gone, committed baseline intact"
    );
    drop(store);

    // SERVE: the reopened store answers exactly the pre-crash baseline.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(store_snapshot(&root, &db), snap_before);
    assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_before);

    // REBUILD: an explicit force-reindex over the recovered store converges
    // and still serves the baseline, still unquarantined.
    build_and_quiet(&root, &db, true, false);
    assert!(home_names(db.parent().unwrap()).iter().all(|n| !n.contains(".corrupt")));
    assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_before);
}

/// Child entry point for the SIGKILL drill: without the spec env it is a
/// no-op so normal suite runs are unaffected. With it, the child opens the
/// store, holds an uncommitted bulk transaction open, and writes until killed.
/// Harness, not a test: excluded from the ≤15 test count per the catalog.
#[test]
fn child_writer_entry() {
    let (Ok(root), Ok(db), Ok(ready)) = (
        std::env::var("ASGREP_DRILL_ROOT"),
        std::env::var("ASGREP_DRILL_DB"),
        std::env::var("ASGREP_DRILL_READY"),
    ) else {
        return;
    };
    let store = IndexStore::open(Path::new(&root), Some(Path::new(&db))).unwrap();
    store.begin_bulk_tx().unwrap();
    for i in 0..600 {
        upsert_test_file(&store, &format!("victim/{i}.py"), "x = 1\n".to_string(), &format!("drill-victim-{i}"));
        if i == 0 {
            std::fs::write(&ready, b"ready").unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    store.commit_bulk_tx().unwrap();
}

/// INTENT: a torn store repairs through forced reindex — quarantining the
/// torn image exactly — and serves the pre-crash baseline, identically for
/// bit-flip and truncation crash shapes.
/// FACETS: mid-db page flip, half-truncation (same crash→recover→serve body).
/// KILLS: lossy-repair mutants.
#[test]
fn corrupt_store_repairs_to_baseline() {
    for shape in ["page-flip", "truncate"] {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let db = session.index_path.clone();
        build_and_quiet(&root, &db, false, false);
        let snap_before = store_snapshot(&root, &db);
        let hits_before = search_parity_keys(&root, &db, QUERIES, false);
        assert_real_hits(&hits_before);

        // CRASH.
        let torn = match shape {
            "page-flip" => {
                let mut torn = quiesced_db_bytes(&db);
                let mid = torn.len() / 2;
                flip_bytes(&mut torn, mid, 64);
                std::fs::write(&db, &torn).unwrap();
                torn
            }
            _ => {
                let full = quiesced_db_bytes(&db);
                let torn = full[..full.len() / 2].to_vec();
                truncate_file(&db, torn.len() as u64);
                torn
            }
        };
        assert_torn(&db);

        // RECOVER: forced rebuild quarantines the torn image and rebuilds.
        build_and_quiet(&root, &db, true, false);
        assert_eq!(
            std::fs::read(quarantine_path(&db, ".corrupt")).unwrap(),
            torn,
            "{shape}: quarantine must preserve the torn image exactly"
        );

        // SERVE: verification clean, baseline fully restored.
        assert_eq!(integrity(&root, &db), "ok");
        assert_eq!(store_snapshot(&root, &db), snap_before);
        assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_before);
    }
}

/// INTENT: a missing or zeroed store cold-starts through the documented
/// recovery — no quarantine, just rebuild — and serves the pre-crash
/// baseline.
/// FACETS: deleted db+sidecars → plain rebuild; zeroed db → writable reopen
/// initializes, then incremental build repopulates.
/// KILLS: cold-start mutants.
#[test]
fn missing_or_zeroed_store_cold_starts_to_baseline() {
    // Facet: the whole database file vanishes; a plain rebuild recreates it.
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let db = session.index_path.clone();
        build_and_quiet(&root, &db, false, false);
        let snap_before = store_snapshot(&root, &db);
        let hits_before = search_parity_keys(&root, &db, QUERIES, false);
        assert_real_hits(&hits_before);

        std::fs::remove_file(&db).unwrap();
        remove_sqlite_sidecars(&db);
        assert!(!db.exists());
        let err = err_of(IndexStore::open_readonly(&root, Some(&db)));
        assert!(matches!(err, StoreError::Other(_)), "missing db must refuse read-only as Other, got {err:?}");

        build_and_quiet(&root, &db, false, false);
        assert!(db.is_file());
        assert_eq!(integrity(&root, &db), "ok");
        assert_eq!(store_snapshot(&root, &db), snap_before);
        assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_before);
        assert!(home_names(db.parent().unwrap()).iter().all(|n| !n.contains(".corrupt")));
    }

    // Facet: the db is zeroed; a writable reopen initializes in place, then a
    // build fills it.
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let db = session.index_path.clone();
        build_and_quiet(&root, &db, false, false);
        let snap_before = store_snapshot(&root, &db);
        let hits_before = search_parity_keys(&root, &db, QUERIES, false);
        assert_real_hits(&hits_before);

        std::fs::write(&db, b"").unwrap();
        assert_eq!(std::fs::metadata(&db).unwrap().len(), 0);

        let store = IndexStore::open(&root, Some(&db)).unwrap();
        assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        assert_eq!(store.status().unwrap().file_count, 0);
        drop(store);
        build_and_quiet(&root, &db, false, false);

        assert_eq!(integrity(&root, &db), "ok");
        assert_eq!(store_snapshot(&root, &db), snap_before);
        assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_before);
    }
}

/// INTENT: a deleted lexical sidecar is noticed and rebuilt by an incremental
/// build, and sidecar-backed search serves the pre-crash baseline.
/// KILLS: missing-sidecar-blind mutants.
#[test]
fn deleted_lexical_sidecar_rebuilds_to_baseline() {
    let session = corpus_session();
    let root = session.corpus_root.clone();
    let db = session.index_path.clone();

    build_and_quiet(&root, &db, false, true);
    let sidecar = sidecar_path(&root, Some(&db));
    assert!(sidecar.is_file(), "fixture must build a lexical sidecar");
    let hits_before = search_parity_keys(&root, &db, QUERIES, true);
    assert_real_hits(&hits_before);

    // CRASH: the lexical sidecar vanishes; the main db is untouched.
    std::fs::remove_file(&sidecar).unwrap();
    assert!(!sidecar.exists());
    assert_eq!(integrity(&root, &db), "ok");

    // RECOVER: an incremental build must rebuild the missing sidecar.
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(db.clone()),
        embed_semantic: false,
        use_tantivy: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    indexer.store().checkpoint_wal().unwrap();
    drop(indexer);
    remove_sqlite_sidecars(&db);
    assert!(sidecar.is_file(), "rebuild must restore the sidecar");

    // SERVE: sidecar-backed search answers exactly the pre-crash baseline.
    assert_eq!(search_parity_keys(&root, &db, QUERIES, true), hits_before);
}

/// INTENT: two sequential crashes with an attested serving state between them
/// both quarantine distinctly and the twice-recovered store serves the
/// ORIGINAL pre-crash baseline. This also pins the quarantine-monotonicity
/// relation: the second recovery allocates a fresh slot, never overwrites
/// the first, and reconverges across fault shapes.
/// KILLS: chain-residue, overwrite-evidence mutants.
#[test]
fn chained_double_crash_serves_original_baseline() {
    let session = corpus_session();
    let root = session.corpus_root.clone();
    let db = session.index_path.clone();

    build_and_quiet(&root, &db, false, false);
    let snap_before = store_snapshot(&root, &db);
    let hits_before = search_parity_keys(&root, &db, QUERIES, false);
    assert_real_hits(&hits_before);

    // CRASH 1: half-truncation. RECOVER 1: forced rebuild + quarantine.
    let full = quiesced_db_bytes(&db);
    let torn_first = full[..full.len() / 2].to_vec();
    truncate_file(&db, torn_first.len() as u64);
    assert_torn(&db);
    build_and_quiet(&root, &db, true, false);
    assert_eq!(std::fs::read(quarantine_path(&db, ".corrupt")).unwrap(), torn_first);

    // SERVE mid-chain: the once-recovered store answers the baseline.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(store_snapshot(&root, &db), snap_before);
    assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_before);

    // CRASH 2: page corruption of the recovered store. RECOVER 2: rebuild.
    let mut torn_second = quiesced_db_bytes(&db);
    let mid = torn_second.len() / 2;
    flip_bytes(&mut torn_second, mid, 64);
    std::fs::write(&db, &torn_second).unwrap();
    assert_torn(&db);
    build_and_quiet(&root, &db, true, false);

    assert_eq!(
        std::fs::read(quarantine_path(&db, ".corrupt")).unwrap(),
        torn_first,
        "second recovery must not overwrite the first quarantine"
    );
    assert_eq!(
        std::fs::read(quarantine_path(&db, ".corrupt.1")).unwrap(),
        torn_second,
        "second recovery must quarantine the second torn image"
    );

    // SERVE final: the twice-recovered store answers the original baseline.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(store_snapshot(&root, &db), snap_before);
    assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_before);
}
