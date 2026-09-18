//! R4 end-to-end crash drills for ast-sgrep-core durable state (pass 4).
//!
//! R1 pins passive recovery contracts, R2 injects single active faults, R3
//! asserts relations over recovery. R4 runs FULL crash→recover→serve drills
//! through public APIs: each drill builds and populates a real store,
//! captures a pre-crash serving baseline from that same store, crashes it,
//! runs the documented recovery path (reopen / rebuild / repair), then
//! SERVES real queries and proves the post-recovery hits are identical to
//! the pre-crash baseline. Covered: SIGKILLed child writer, page
//! corruption, truncation, deleted db, deleted lexical sidecar, zeroed db,
//! and a chained double-crash finale.
//!
//! All failure assertions match `StoreError` discriminants, row counts, byte
//! equality, or sorted hit keys — never message text.
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::tantivy_index::sidecar_path;
use ast_sgrep_core::{
    IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError, INDEX_SCHEMA_VERSION,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const CORPUS: &[(&str, &str)] = &[
    ("src/a.py", "def alpha_needle():\n    return 1\n"),
    ("src/b.py", "def beta_needle():\n    return 2\n"),
    ("src/c.py", "import os\n\ndef gamma_caller():\n    return os.getcwd()\n"),
];

const QUERIES: &[&str] = &["alpha_needle", "beta_needle", "gamma_caller"];

/// `IndexStore`/`Searcher` do not implement `Debug`, so `unwrap_err` cannot
/// be used; this is the equivalent fail-loud extractor.
fn err_of<T>(result: Result<T, StoreError>) -> StoreError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(err) => err,
    }
}

fn write_corpus(dir: &Path) -> PathBuf {
    let root = dir.join("corpus");
    for (rel, body) in CORPUS {
        let abs = root.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
    }
    root
}

fn index_options(root: &Path, db: &Path, force: bool, use_tantivy: bool) -> IndexOptions {
    IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        embed_semantic: false,
        force_reindex: force,
        use_tantivy,
        ..IndexOptions::default()
    }
}

/// Build (or force-rebuild) the corpus index, checkpoint WAL content into the
/// main db, and drop all handles so raw bytes are stable for comparison.
fn build_and_quiet(root: &Path, db: &Path, force: bool, use_tantivy: bool) {
    let mut indexer = Indexer::new(index_options(root, db, force, use_tantivy)).unwrap();
    if force {
        indexer.reindex_all().unwrap();
    } else {
        indexer.index_all().unwrap();
    }
    indexer.store().checkpoint_wal().unwrap();
    drop(indexer);
    remove_wal_sidecars(db);
}

fn remove_wal_sidecars(db: &Path) {
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
}

fn db_bytes(db: &Path) -> Vec<u8> {
    remove_wal_sidecars(db);
    std::fs::read(db).unwrap()
}

/// Deterministic logical snapshot: status counts, per-file content hashes,
/// and every indexed line row.
fn snapshot(root: &Path, db: &Path) -> String {
    let store = IndexStore::open_readonly(root, Some(db)).unwrap();
    let status = store.status().unwrap();
    let mut parts = vec![format!(
        "files={} lines={} symbols={} callers={} imports={}",
        status.file_count,
        status.line_count,
        status.symbol_count,
        status.caller_count,
        status.import_count,
    )];
    for path in store.all_file_paths().unwrap() {
        parts.push(format!(
            "file:{path}={:?}",
            store.file_hash(&path).unwrap()
        ));
    }
    for row in store.all_indexed_lines().unwrap() {
        parts.push(format!("line:{}:{}:{}", row.0, row.1, row.2));
    }
    parts.join("\n")
}

fn integrity(root: &Path, db: &Path) -> String {
    let store = IndexStore::open_readonly(root, Some(db)).unwrap();
    store
        .connection()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap()
}

/// Sorted, float-free hit keys: file, span, symbol, excerpt. Scores are f64
/// and excluded; ranking breadth is pinned by key-vector equality.
fn hits_key(root: &Path, db: &Path, query: &str, use_tantivy: bool) -> Vec<String> {
    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        use_embed: false,
        use_tantivy,
        ..SearchOptions::default()
    })
    .unwrap();
    let mut keys: Vec<String> = searcher
        .search(query)
        .unwrap()
        .hits
        .iter()
        .map(|hit| {
            format!(
                "{}:{}-{}:{:?}:{}",
                hit.file, hit.line_start, hit.line_end, hit.symbol, hit.excerpt
            )
        })
        .collect();
    keys.sort();
    keys
}

fn parity_keys(root: &Path, db: &Path, use_tantivy: bool) -> Vec<(String, Vec<String>)> {
    QUERIES
        .iter()
        .map(|q| ((*q).to_string(), hits_key(root, db, q, use_tantivy)))
        .collect()
}

fn assert_real_hits(keys: &[(String, Vec<String>)]) {
    assert!(
        keys.iter().any(|(_, hits)| !hits.is_empty()),
        "parity must be over real hits, not mutual emptiness: {keys:?}"
    );
}

/// Fixture-validity gate: torn bytes must actually be torn under a raw SQLite
/// open, otherwise the "fault" is accidentally coherent and vacates the test.
fn assert_torn(db: &Path) {
    match rusqlite::Connection::open(db) {
        Err(_) => {}
        Ok(conn) => match conn.query_row("PRAGMA integrity_check", [], |r| {
            r.get::<_, String>(0)
        }) {
            Err(_) => {}
            Ok(detail) => assert_ne!(
                detail, "ok",
                "fault fixture is not torn: {}",
                db.display()
            ),
        },
    }
}

fn flip_mid(bytes: &mut [u8], len: usize) {
    let mid = bytes.len() / 2;
    for i in 0..len {
        bytes[mid + i] ^= 0xFF;
    }
}

fn quarantine_path(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.file_name().unwrap().to_os_string();
    name.push(suffix);
    db.with_file_name(name)
}

fn home_names(db: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(db.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn upsert_file(store: &IndexStore, rel: &str, body: String, hash: &str) {
    let lines = [(1u32, body)];
    store
        .upsert_file(UpsertFileInput {
            rel_path: rel,
            language: Some("python"),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: hash,
            lines: &lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            depth_truncated: false,
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
        })
        .unwrap();
}

/// DRILL kill-crash: populate a real store and capture its serving baseline,
/// then SIGKILL a child writer holding an uncommitted bulk transaction. The
/// documented recovery is a plain reopen (crash rollback, no quarantine),
/// and the reopened store must serve the pre-crash baseline identically.
#[test]
#[cfg(unix)]
fn drill_sigkill_child_writer_reopen_serves_baseline() {
    use std::os::unix::process::ExitStatusExt;

    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false, false);
    let snap_before = snapshot(&root, &db);
    let hits_before = parity_keys(&root, &db, false);
    assert_real_hits(&hits_before);
    let files_before = IndexStore::open_readonly(&root, Some(&db))
        .unwrap()
        .status()
        .unwrap()
        .file_count;
    assert_eq!(files_before, CORPUS.len());

    // CRASH: a real child process is SIGKILLed inside an uncommitted bulk tx.
    let ready = temp.path().join("child-ready");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "r4_child_writer_entry", "--nocapture"])
        .env("ASGREP_R4_ROOT", &root)
        .env("ASGREP_R4_DB", &db)
        .env("ASGREP_R4_READY", &ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "child writer never signalled readiness"
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before the kill window"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let pid = child.id();
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
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
    assert!(
        home_names(&db).iter().all(|n| !n.contains(".corrupt")),
        "crash recovery must not quarantine: {:?}",
        home_names(&db)
    );
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    assert_eq!(
        store.status().unwrap().file_count,
        files_before,
        "uncommitted victim rows must be gone, committed baseline intact"
    );
    drop(store);

    // SERVE: the reopened store answers exactly the pre-crash baseline.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(snapshot(&root, &db), snap_before);
    assert_eq!(parity_keys(&root, &db, false), hits_before);
}

/// Child entry point for the SIGKILL drill: without the spec env it is a
/// no-op so normal suite runs are unaffected. With it, the child opens the
/// store, holds an uncommitted bulk transaction open, and writes until killed.
#[test]
fn r4_child_writer_entry() {
    let (Ok(root), Ok(db), Ok(ready)) = (
        std::env::var("ASGREP_R4_ROOT"),
        std::env::var("ASGREP_R4_DB"),
        std::env::var("ASGREP_R4_READY"),
    ) else {
        return;
    };
    let store = IndexStore::open(Path::new(&root), Some(Path::new(&db))).unwrap();
    store.begin_bulk_tx().unwrap();
    for i in 0..600 {
        upsert_file(
            &store,
            &format!("victim/{i}.py"),
            "x = 1\n".to_string(),
            &format!("r4-victim-{i}"),
        );
        if i == 0 {
            std::fs::write(&ready, b"ready").unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    store.commit_bulk_tx().unwrap();
}

/// DRILL page-corrupt: baseline a serving store, flip a mid-db page, then run
/// the documented repair path — forced reindex, which quarantines the torn
/// image — and prove the repaired store serves the pre-crash baseline.
#[test]
fn drill_page_corrupt_force_rebuild_serves_baseline() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false, false);
    let snap_before = snapshot(&root, &db);
    let hits_before = parity_keys(&root, &db, false);
    assert_real_hits(&hits_before);

    // CRASH: a mid-db page flip.
    let mut torn = db_bytes(&db);
    flip_mid(&mut torn, 64);
    std::fs::write(&db, &torn).unwrap();
    assert_torn(&db);

    // RECOVER: forced rebuild quarantines the torn image and rebuilds.
    build_and_quiet(&root, &db, true, false);
    assert_eq!(
        std::fs::read(quarantine_path(&db, ".corrupt")).unwrap(),
        torn,
        "quarantine must preserve the torn image exactly"
    );

    // SERVE: verification clean, baseline fully restored.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(snapshot(&root, &db), snap_before);
    assert_eq!(parity_keys(&root, &db, false), hits_before);
}

/// DRILL truncate: baseline a serving store, tear it to half its bytes, then
/// run the documented repair path (forced reindex + quarantine) and prove
/// the repaired store serves the pre-crash baseline.
#[test]
fn drill_truncate_force_rebuild_serves_baseline() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false, false);
    let snap_before = snapshot(&root, &db);
    let hits_before = parity_keys(&root, &db, false);
    assert_real_hits(&hits_before);

    // CRASH: half-truncation.
    let full = db_bytes(&db);
    let torn = full[..full.len() / 2].to_vec();
    std::fs::write(&db, &torn).unwrap();
    assert_torn(&db);

    // RECOVER: forced rebuild quarantines the torn image and rebuilds.
    build_and_quiet(&root, &db, true, false);
    assert_eq!(
        std::fs::read(quarantine_path(&db, ".corrupt")).unwrap(),
        torn,
        "quarantine must preserve the torn image exactly"
    );

    // SERVE: verification clean, baseline fully restored.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(snapshot(&root, &db), snap_before);
    assert_eq!(parity_keys(&root, &db, false), hits_before);
}

/// DRILL delete-db: baseline a serving store, delete the db file and its WAL
/// sidecars outright, then run the documented recovery — a plain rebuild over
/// the missing path — and prove the rebuilt store serves the baseline.
#[test]
fn drill_deleted_db_rebuild_serves_baseline() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false, false);
    let snap_before = snapshot(&root, &db);
    let hits_before = parity_keys(&root, &db, false);
    assert_real_hits(&hits_before);

    // CRASH: the whole database file vanishes.
    std::fs::remove_file(&db).unwrap();
    remove_wal_sidecars(&db);
    assert!(!db.exists());
    let err = err_of(IndexStore::open_readonly(&root, Some(&db)));
    assert!(
        matches!(err, StoreError::Other(_)),
        "missing db must refuse read-only as Other, got {err:?}"
    );

    // RECOVER: plain rebuild recreates the store from the corpus.
    build_and_quiet(&root, &db, false, false);
    assert!(db.is_file());

    // SERVE: verification clean, baseline fully restored, no quarantine.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(snapshot(&root, &db), snap_before);
    assert_eq!(parity_keys(&root, &db, false), hits_before);
    assert!(
        home_names(&db).iter().all(|n| !n.contains(".corrupt")),
        "rebuild over a missing db must not quarantine"
    );
}

/// DRILL delete-sidecar: baseline a serving store with the lexical sidecar
/// forced on, delete `lexical.db`, then run the documented recovery — an
/// incremental build, which must notice the missing sidecar and rebuild it —
/// and prove the store serves the pre-crash baseline.
#[test]
fn drill_deleted_lexical_sidecar_rebuild_serves_baseline() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false, true);
    let sidecar = sidecar_path(&root, Some(&db));
    assert!(sidecar.is_file(), "fixture must build a lexical sidecar");
    let hits_before = parity_keys(&root, &db, true);
    assert_real_hits(&hits_before);

    // CRASH: the lexical sidecar vanishes; the main db is untouched.
    std::fs::remove_file(&sidecar).unwrap();
    assert!(!sidecar.exists());
    assert_eq!(integrity(&root, &db), "ok");

    // RECOVER: an incremental build must rebuild the missing sidecar.
    let mut indexer = Indexer::new(index_options(&root, &db, false, true)).unwrap();
    indexer.index_all().unwrap();
    indexer.store().checkpoint_wal().unwrap();
    drop(indexer);
    remove_wal_sidecars(&db);
    assert!(sidecar.is_file(), "rebuild must restore the sidecar");

    // SERVE: sidecar-backed search answers exactly the pre-crash baseline.
    assert_eq!(parity_keys(&root, &db, true), hits_before);
}

/// DRILL zeroed-db: baseline a serving store, zero the db file, then run the
/// documented recovery — a writable reopen initializes the empty image in
/// place, and an incremental build repopulates it — and prove the store
/// serves the pre-crash baseline.
#[test]
fn drill_zeroed_db_reopen_rebuild_serves_baseline() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false, false);
    let snap_before = snapshot(&root, &db);
    let hits_before = parity_keys(&root, &db, false);
    assert_real_hits(&hits_before);

    // CRASH: the db is zeroed.
    std::fs::write(&db, b"").unwrap();
    assert_eq!(std::fs::metadata(&db).unwrap().len(), 0);

    // RECOVER: writable reopen initializes in place, then a build fills it.
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    assert_eq!(
        store.status().unwrap().file_count,
        0,
        "zeroed db reopens as an empty current-schema store"
    );
    drop(store);
    build_and_quiet(&root, &db, false, false);

    // SERVE: verification clean, baseline fully restored.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(snapshot(&root, &db), snap_before);
    assert_eq!(parity_keys(&root, &db, false), hits_before);
}

/// DRILL chained double-crash (finale): baseline a serving store, crash it by
/// truncation and recover to a SERVING store mid-chain, then crash the
/// recovered store by page corruption and recover again. Both recoveries
/// quarantine distinctly, and the twice-recovered store serves the original
/// pre-crash baseline.
#[test]
fn drill_chained_truncate_then_corrupt_serves_baseline() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false, false);
    let snap_before = snapshot(&root, &db);
    let hits_before = parity_keys(&root, &db, false);
    assert_real_hits(&hits_before);

    // CRASH 1: half-truncation. RECOVER 1: forced rebuild + quarantine.
    let full = db_bytes(&db);
    let torn_first = full[..full.len() / 2].to_vec();
    std::fs::write(&db, &torn_first).unwrap();
    assert_torn(&db);
    build_and_quiet(&root, &db, true, false);
    assert_eq!(
        std::fs::read(quarantine_path(&db, ".corrupt")).unwrap(),
        torn_first
    );

    // SERVE mid-chain: the once-recovered store answers the baseline.
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(snapshot(&root, &db), snap_before);
    assert_eq!(parity_keys(&root, &db, false), hits_before);

    // CRASH 2: page corruption of the recovered store. RECOVER 2: rebuild.
    let mut torn_second = db_bytes(&db);
    flip_mid(&mut torn_second, 64);
    std::fs::write(&db, &torn_second).unwrap();
    assert_torn(&db);
    build_and_quiet(&root, &db, true, false);

    // Both quarantines preserved distinctly; first never overwritten.
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
    assert_eq!(snapshot(&root, &db), snap_before);
    assert_eq!(parity_keys(&root, &db, false), hits_before);
}
