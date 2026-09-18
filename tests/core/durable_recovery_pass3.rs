//! R3 metamorphic-recovery relations for ast-sgrep-core durable state (pass 3).
//!
//! R1 pins passive recovery contracts; R2 injects single active faults. R3
//! asserts RELATIONS over recovery: two or more histories must converge.
//! Covered: rebuild idempotence (incremental byte-identical, forced logical,
//! IVF sidecar byte-identical), repair-then-verify roundtrips (page flip,
//! truncation), recovery determinism, partial-progress monotonicity (cancel,
//! abandoned bulk tx), recovered-index search parity, fault-order
//! commutativity, and quarantine-sequence monotonicity.
//!
//! All failure assertions match `StoreError` discriminants, row counts, byte
//! equality, or sorted hit keys — never message text.
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{compute_ann_fingerprint, load_semantic_ivf, save_semantic_ivf};
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{
    IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError,
};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CORPUS: &[(&str, &str)] = &[
    ("src/a.py", "def alpha_needle():\n    return 1\n"),
    ("src/b.py", "def beta_needle():\n    return 2\n"),
    ("src/c.py", "import os\n\ndef gamma_caller():\n    return os.getcwd()\n"),
];

const QUERIES: &[&str] = &["alpha_needle", "beta_needle", "gamma_caller"];
const UNKNOWN_QUERY: &str = "needle_no_such_token_xyz";

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

fn index_options(root: &Path, db: &Path, force: bool) -> IndexOptions {
    IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        embed_semantic: false,
        force_reindex: force,
        ..IndexOptions::default()
    }
}

/// Build (or force-rebuild) the corpus index, checkpoint WAL content into the
/// main db, and drop all handles so raw bytes are stable for comparison.
fn build_and_quiet(root: &Path, db: &Path, force: bool) {
    let mut indexer = Indexer::new(index_options(root, db, force)).unwrap();
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
/// and every indexed line row. Volatile meta (writer generation,
/// data-version seeds) is excluded by construction.
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

fn quick_check(root: &Path, db: &Path) -> String {
    let store = IndexStore::open_readonly(root, Some(db)).unwrap();
    store
        .connection()
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .unwrap()
}

/// Sorted, float-free hit keys: file, span, symbol, excerpt. Scores are f64
/// and excluded; ranking breadth is pinned by key-vector equality.
fn hits_key(root: &Path, db: &Path, query: &str) -> Vec<String> {
    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        use_embed: false,
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

fn parity_keys(root: &Path, db: &Path) -> Vec<(String, Vec<String>)> {
    QUERIES
        .iter()
        .map(|q| ((*q).to_string(), hits_key(root, db, q)))
        .collect()
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

fn flip_range(bytes: &mut [u8], offset: usize, len: usize) {
    for i in 0..len {
        bytes[offset + i] ^= 0xFF;
    }
}

fn flip_mid(bytes: &mut Vec<u8>, len: usize) {
    let mid = bytes.len() / 2;
    flip_range(bytes, mid, len);
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

/// RELATION rebuild-idempotence: a second incremental `index_all` over an
/// unchanged corpus is a byte-identical no-op — same db bytes, same logical
/// snapshot, same search keys, zero files re-indexed, no quarantine.
#[test]
fn rebuild_incremental_second_pass_is_byte_identical_noop() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false);
    let bytes_once = db_bytes(&db);
    let snap_once = snapshot(&root, &db);
    let hits_once = parity_keys(&root, &db);

    let mut indexer = Indexer::new(index_options(&root, &db, false)).unwrap();
    let stats = indexer.index_all().unwrap();
    assert_eq!(stats.files_indexed, 0, "unchanged corpus must index nothing");
    assert_eq!(stats.files_failed, 0);
    indexer.store().checkpoint_wal().unwrap();
    drop(indexer);

    assert_eq!(
        db_bytes(&db),
        bytes_once,
        "second incremental pass must be byte-identical"
    );
    assert_eq!(snapshot(&root, &db), snap_once);
    assert_eq!(parity_keys(&root, &db), hits_once);
    assert_eq!(integrity(&root, &db), "ok");
    assert!(
        home_names(&db).iter().all(|n| !n.contains(".corrupt")),
        "healthy rebuild must not quarantine"
    );
}

/// RELATION rebuild-idempotence (logical): two forced rebuilds converge to
/// the same logical snapshot and search keys. Raw db bytes are NOT compared:
/// page layout varies run to run, so the forced-rebuild relation is logical.
#[test]
fn rebuild_force_twice_is_logically_identical() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, true);
    let snap_once = snapshot(&root, &db);
    let hits_once = parity_keys(&root, &db);
    assert_eq!(integrity(&root, &db), "ok");

    build_and_quiet(&root, &db, true);
    assert_eq!(
        snapshot(&root, &db),
        snap_once,
        "forced rebuild must be logically idempotent"
    );
    assert_eq!(parity_keys(&root, &db), hits_once);
    assert_eq!(integrity(&root, &db), "ok");
}

/// RELATION rebuild-idempotence (sidecar): saving the same IVF payload twice —
/// to sibling paths or over the same path — is byte-identical, and every copy
/// loads back the same vectors.
#[test]
fn rebuild_ivf_save_twice_is_byte_identical() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let dim = 4usize;
    let vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 1);

    let first = dir.join("first.ivf");
    let second = dir.join("second.ivf");
    save_semantic_ivf(&first, fp, dim, &vectors, &index).unwrap();
    save_semantic_ivf(&second, fp, dim, &vectors, &index).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
        "same IVF payload must serialize byte-identically"
    );

    save_semantic_ivf(&first, fp, dim, &vectors, &index).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
        "IVF overwrite must be byte-stable"
    );

    for path in [&first, &second] {
        assert_eq!(
            load_semantic_ivf(path, fp).unwrap().unwrap().vectors,
            vectors
        );
    }
}

/// RELATION repair-then-verify: a mid-db page flip is torn (fixture gate),
/// then a forced rebuild repairs to a clean bill — `integrity_check` and
/// `quick_check` both "ok" and stable across repeated verification — with the
/// pre-fault logical snapshot and search keys fully restored.
#[test]
fn repair_page_flip_then_verify_clean_and_restored() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false);
    let snap_healthy = snapshot(&root, &db);
    let hits_healthy = parity_keys(&root, &db);

    let full = db_bytes(&db);
    let mut torn = full.clone();
    flip_mid(&mut torn, 64);
    std::fs::write(&db, &torn).unwrap();
    assert_torn(&db);

    build_and_quiet(&root, &db, true);

    assert!(
        quarantine_path(&db, ".corrupt").is_file(),
        "recovery must preserve the torn inode"
    );
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(quick_check(&root, &db), "ok");
    assert_eq!(integrity(&root, &db), "ok", "verification must be stable");
    assert_eq!(
        snapshot(&root, &db),
        snap_healthy,
        "repair must restore the pre-fault snapshot"
    );
    assert_eq!(
        parity_keys(&root, &db),
        hits_healthy,
        "repair must restore search keys"
    );
}

/// RELATION repair-then-verify (truncation): a half-truncated store repairs
/// through the same forced rebuild to the identical restored state — clean
/// verification, pre-fault snapshot, pre-fault search keys.
#[test]
fn repair_truncation_then_verify_clean_and_restored() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");

    build_and_quiet(&root, &db, false);
    let snap_healthy = snapshot(&root, &db);
    let hits_healthy = parity_keys(&root, &db);

    let full = db_bytes(&db);
    std::fs::write(&db, &full[..full.len() / 2]).unwrap();
    assert_torn(&db);

    build_and_quiet(&root, &db, true);

    assert!(
        quarantine_path(&db, ".corrupt").is_file(),
        "recovery must preserve the torn inode"
    );
    assert_eq!(integrity(&root, &db), "ok");
    assert_eq!(quick_check(&root, &db), "ok");
    assert_eq!(
        snapshot(&root, &db),
        snap_healthy,
        "repair must restore the pre-fault snapshot"
    );
    assert_eq!(
        parity_keys(&root, &db),
        hits_healthy,
        "repair must restore search keys"
    );
}

/// RELATION recovery-determinism: the same fault applied to two copies of one
/// base image tears identically, quarantines byte-identical images, and
/// recovers to identical snapshots and search keys.
#[test]
fn recovery_same_fault_twice_is_deterministic() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let base_db = temp.path().join("base").join("index.db");
    build_and_quiet(&root, &base_db, false);
    let base_bytes = db_bytes(&base_db);

    let mut torn_images = Vec::new();
    let mut quarantines = Vec::new();
    let mut snaps = Vec::new();
    let mut hits = Vec::new();
    for tag in ["copy-a", "copy-b"] {
        let db = temp.path().join(tag).join("index.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let mut torn = base_bytes.clone();
        flip_mid(&mut torn, 64);
        std::fs::write(&db, &torn).unwrap();
        assert_torn(&db);
        torn_images.push(torn);

        build_and_quiet(&root, &db, true);
        quarantines.push(std::fs::read(quarantine_path(&db, ".corrupt")).unwrap());
        assert_eq!(integrity(&root, &db), "ok");
        snaps.push(snapshot(&root, &db));
        hits.push(parity_keys(&root, &db));
    }
    assert_eq!(
        torn_images[0], torn_images[1],
        "same fault on same base must tear identically"
    );
    assert_eq!(
        quarantines[0], quarantines[1],
        "quarantines of identical faults must be byte-identical"
    );
    assert_eq!(
        quarantines[0], torn_images[0],
        "quarantine must preserve the torn image exactly"
    );
    assert_eq!(
        snaps[0], snaps[1],
        "post-recovery snapshots must be identical"
    );
    assert_eq!(hits[0], hits[1], "post-recovery search keys must be identical");
}

/// RELATION partial-progress monotonicity: an `index_all` cancelled before it
/// can commit (discriminant `Other`, zero rows) leaves no residue — resuming
/// over the same db converges to the identical snapshot and keys as a fresh
/// single-pass build.
#[test]
fn recovery_cancelled_build_resumed_equals_fresh() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let resumed_db = temp.path().join("resumed").join("index.db");
    let fresh_db = temp.path().join("fresh").join("index.db");

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let indexer = Indexer::new(index_options(&root, &resumed_db, false)).unwrap();
    let mut indexer = indexer;
    indexer.set_cancel(cancel.clone());
    let err = err_of(indexer.index_all());
    assert!(
        matches!(err, StoreError::Other(_)),
        "cancelled build must fail as Other, got {err:?}"
    );
    assert_eq!(
        indexer.store().status().unwrap().file_count,
        0,
        "cancelled build must commit nothing"
    );
    drop(indexer);

    cancel.store(false, std::sync::atomic::Ordering::SeqCst);
    build_and_quiet(&root, &resumed_db, false);
    build_and_quiet(&root, &fresh_db, false);

    assert_eq!(
        snapshot(&root, &resumed_db),
        snapshot(&root, &fresh_db),
        "resumed build must equal a fresh build"
    );
    assert_eq!(
        parity_keys(&root, &resumed_db),
        parity_keys(&root, &fresh_db)
    );
    assert_eq!(integrity(&root, &resumed_db), "ok");
}

/// RELATION partial-progress monotonicity (uncommitted bulk write): a bulk
/// transaction abandoned without commit rolls back — the partial row is gone —
/// and a subsequent full build converges to the fresh-build state.
#[test]
fn recovery_abandoned_bulk_tx_resumed_equals_fresh() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let resumed_db = temp.path().join("resumed").join("index.db");
    let fresh_db = temp.path().join("fresh").join("index.db");

    {
        let store = IndexStore::open(&root, Some(&resumed_db)).unwrap();
        store.begin_bulk_tx().unwrap();
        let lines = [(1u32, "partial = 1\n".to_string())];
        store
            .upsert_file(UpsertFileInput {
                rel_path: "partial.py",
                language: Some("python"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: "r3-partial",
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
        // Abandoned: dropped without commit, must roll back.
        drop(store);
    }

    build_and_quiet(&root, &resumed_db, false);
    build_and_quiet(&root, &fresh_db, false);

    let store = IndexStore::open_readonly(&root, Some(&resumed_db)).unwrap();
    assert!(
        store.file_hash("partial.py").unwrap().is_none(),
        "abandoned row must not survive"
    );
    drop(store);
    assert_eq!(
        snapshot(&root, &resumed_db),
        snapshot(&root, &fresh_db),
        "resumed build must equal a fresh build"
    );
    assert_eq!(
        parity_keys(&root, &resumed_db),
        parity_keys(&root, &fresh_db)
    );
}

/// RELATION recovered-index search parity: a corrupt-then-recovered index
/// serves exactly the same hits as a never-crashed index over the same corpus —
/// per-query keys identical, including whatever the engine answers for an
/// unknown token. The parity is over real hits, pinned non-empty on the
/// healthy arm.
#[test]
fn recovered_index_search_parity_with_never_crashed() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let healthy_db = temp.path().join("healthy").join("index.db");
    let recovered_db = temp.path().join("recovered").join("index.db");

    build_and_quiet(&root, &healthy_db, false);
    build_and_quiet(&root, &recovered_db, false);

    let full = db_bytes(&recovered_db);
    let mut torn = full.clone();
    flip_mid(&mut torn, 64);
    std::fs::write(&recovered_db, &torn).unwrap();
    assert_torn(&recovered_db);
    build_and_quiet(&root, &recovered_db, true);

    assert_eq!(
        snapshot(&root, &recovered_db),
        snapshot(&root, &healthy_db)
    );
    assert_eq!(
        parity_keys(&root, &recovered_db),
        parity_keys(&root, &healthy_db)
    );
    assert_eq!(
        hits_key(&root, &recovered_db, UNKNOWN_QUERY),
        hits_key(&root, &healthy_db, UNKNOWN_QUERY),
        "unknown-token answers must agree across arms"
    );
    assert!(
        !hits_key(&root, &healthy_db, QUERIES[0]).is_empty(),
        "parity must be over real hits, not mutual emptiness"
    );
}

/// RELATION fault-order independence: two disjoint page flips commute — F1;F2
/// and F2;F1 tear one base image to identical bytes — and both recover to
/// identical quarantine bytes, snapshots, and search keys.
#[test]
fn recovery_disjoint_fault_order_commutes() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let base_db = temp.path().join("base").join("index.db");
    build_and_quiet(&root, &base_db, false);
    let base_bytes = db_bytes(&base_db);
    let offsets = [base_bytes.len() / 3, 2 * base_bytes.len() / 3];

    let mut torn_images = Vec::new();
    let mut quarantines = Vec::new();
    let mut snaps = Vec::new();
    let mut hits = Vec::new();
    for (tag, order) in [("order-12", [0usize, 1usize]), ("order-21", [1usize, 0usize])] {
        let db = temp.path().join(tag).join("index.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let mut torn = base_bytes.clone();
        for k in order {
            flip_range(&mut torn, offsets[k], 32);
        }
        std::fs::write(&db, &torn).unwrap();
        assert_torn(&db);
        torn_images.push(torn);

        build_and_quiet(&root, &db, true);
        quarantines.push(std::fs::read(quarantine_path(&db, ".corrupt")).unwrap());
        assert_eq!(integrity(&root, &db), "ok");
        snaps.push(snapshot(&root, &db));
        hits.push(parity_keys(&root, &db));
    }
    assert_eq!(
        torn_images[0], torn_images[1],
        "disjoint flips must commute at the byte layer"
    );
    assert_eq!(
        quarantines[0], quarantines[1],
        "commuted faults must quarantine identically"
    );
    assert_eq!(
        snaps[0], snaps[1],
        "recovery must converge regardless of fault order"
    );
    assert_eq!(hits[0], hits[1]);
}

/// RELATION recovery-sequence monotonicity: a second fault after a completed
/// recovery allocates a fresh quarantine (never overwrites the first, whose
/// bytes stay intact) and reconverges to the same snapshot and keys.
#[test]
fn recovery_second_fault_preserves_first_quarantine_and_reconverges() {
    let temp = TempDir::new().unwrap();
    let root = write_corpus(temp.path());
    let db = temp.path().join("home").join("index.db");
    build_and_quiet(&root, &db, false);

    let full = db_bytes(&db);
    let mut torn = full.clone();
    flip_mid(&mut torn, 64);
    std::fs::write(&db, &torn).unwrap();
    assert_torn(&db);
    build_and_quiet(&root, &db, true);
    let first_quarantine = std::fs::read(quarantine_path(&db, ".corrupt")).unwrap();
    let snap_once = snapshot(&root, &db);
    let hits_once = parity_keys(&root, &db);

    let full = db_bytes(&db);
    std::fs::write(&db, &full[..full.len() / 2]).unwrap();
    assert_torn(&db);
    build_and_quiet(&root, &db, true);

    assert_eq!(
        std::fs::read(quarantine_path(&db, ".corrupt")).unwrap(),
        first_quarantine,
        "second recovery must not overwrite the first quarantine"
    );
    assert!(
        quarantine_path(&db, ".corrupt.1").is_file(),
        "second recovery must allocate a fresh quarantine"
    );
    assert_eq!(
        snapshot(&root, &db),
        snap_once,
        "second recovery must reconverge"
    );
    assert_eq!(parity_keys(&root, &db), hits_once);
    assert_eq!(integrity(&root, &db), "ok");
}
