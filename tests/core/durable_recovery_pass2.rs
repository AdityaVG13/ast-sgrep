//! R2 fault-injection discriminating tests for ast-sgrep-core durable state.
//!
//! R1 (`durable_recovery_pass1.rs`) pins passive recovery contracts: truncated,
//! empty, newer-schema, missing, garbage-sidecar, and read-only-dir states.
//! R2 injects ACTIVE faults — a SIGKILLed writer, torn splices, multi-offset
//! truncation, stale lock files, live lock contention, read-only stores,
//! concurrent openers, interrupted renames, orphan temporaries, uncommitted
//! reads, and path-type confusion — and asserts the store fails closed or
//! recovers. Every test name carries its fault class (`fault_<class>_...`).
//!
//! All failure assertions match `StoreError` discriminants, row counts, byte
//! equality, or file existence — never message text. SQLite is a lazy engine:
//! page faults surface when touched, so torn fixtures "refuse loudly" either
//! at open or at `integrity_check`, but never verify as silently healthy.
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{compute_ann_fingerprint, load_semantic_ivf, save_semantic_ivf};
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{
    IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError, INDEX_SCHEMA_VERSION,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// `IndexStore` does not implement `Debug`, so `unwrap_err` cannot be used on
/// its constructors; this is the equivalent fail-loud extractor.
fn err_of<T>(result: Result<T, StoreError>) -> StoreError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(err) => err,
    }
}

fn discriminant_name(err: &StoreError) -> &'static str {
    match err {
        StoreError::Database(_) => "Database",
        StoreError::Io(_) => "Io",
        StoreError::Other(_) => "Other",
    }
}

fn upsert_file(store: &IndexStore, rel: &str, body: String, hash: &str) -> i64 {
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
        .unwrap()
}

/// Build a healthy store with `files`, checkpoint away WAL sidecars so the
/// fault under test is isolated to the main db bytes, and return the db path.
fn fixture_store(dir: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = dir.join(format!("{name}-root"));
    let db = dir.join(name).join("index.db");
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    for (i, (rel, body)) in files.iter().enumerate() {
        upsert_file(&store, rel, (*body).to_string(), &format!("h{i}-{name}"));
    }
    store.checkpoint_wal().unwrap();
    drop(store);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    assert_eq!(
        integrity_of(&root, &db),
        "ok",
        "fixture {name} must start healthy"
    );
    db
}

fn integrity_of(root: &Path, db: &Path) -> String {
    let store = IndexStore::open_readonly(root, Some(db)).unwrap();
    store
        .connection()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap()
}

/// Fixture-validity gate: the torn bytes must actually be torn. A raw SQLite
/// open (no store DDL rebuild) must fail or report corruption; otherwise the
/// fixture is accidentally coherent and would vacate the test.
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

fn dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Outcome of opening a torn fixture: loud refusal, an open handle whose
/// full verification reports corruption, or a normalized-healthy open (the
/// writable-open DDL rebuild path). Callers pin which arms are legal per
/// fault; a normalized open must serve zero torn rows (file_count 0).
#[derive(Debug, PartialEq, Eq)]
enum Probe {
    Refused(&'static str),
    Dirty(String),
    Healthy { file_count: usize },
}

fn probe_open(root: &Path, db: &Path, readonly: bool) -> Probe {
    let result = if readonly {
        IndexStore::open_readonly(root, Some(db))
    } else {
        IndexStore::open(root, Some(db))
    };
    match result {
        Err(err) => Probe::Refused(discriminant_name(&err)),
        Ok(store) => {
            let detail: String = store
                .connection()
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap();
            if detail == "ok" {
                Probe::Healthy {
                    file_count: store.status().unwrap().file_count,
                }
            } else {
                Probe::Dirty(detail)
            }
        }
    }
}

/// FAULT kill-crash: a real child process is SIGKILLed mid-index-build (inside
/// an uncommitted bulk transaction). The parent asserts the kill landed, the
/// next ordinary open recovers with pre-crash committed state, no quarantine
/// is fabricated, and an explicit force-reindex rebuilds to a serving index.
#[test]
#[cfg(unix)]
fn fault_crash_sigkill_child_writer_mid_build_reopen_recovers() {
    use std::os::unix::process::ExitStatusExt;

    let temp = TempDir::new().unwrap();
    let corpus = temp.path().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    std::fs::write(
        corpus.join("k.py"),
        "def kill_crash_needle():\n    return 1\n",
    )
    .unwrap();
    let root = temp.path().join("root");
    let db = temp.path().join("home").join("index.db");
    drop(IndexStore::open(&root, Some(&db)).unwrap());
    let ready = temp.path().join("child-ready");

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "r2_child_writer_entry", "--nocapture"])
        .env("ASGREP_R2_ROOT", &root)
        .env("ASGREP_R2_DB", &db)
        .env("ASGREP_R2_READY", &ready)
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
    // Child is inside its bulk transaction: SIGKILL, no cleanup handlers.
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

    // Ordinary recovery: no quarantine from the crash, uncommitted rows gone.
    let home = db.parent().unwrap();
    assert!(
        dir_names(home).iter().all(|n| !n.contains(".corrupt")),
        "crash recovery must not quarantine: {:?}",
        dir_names(home)
    );
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    assert_eq!(store.status().unwrap().file_count, 0);
    drop(store);

    // Explicit rebuild over the recovered store serves, still unquarantined.
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.clone(),
        index_path: Some(db.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.reindex_all().unwrap();
    assert_eq!(indexer.store().status().unwrap().file_count, 1);
    drop(indexer);
    assert!(
        dir_names(home).iter().all(|n| !n.contains(".corrupt")),
        "healthy rebuild must not quarantine"
    );
    let searcher = Searcher::new(SearchOptions {
        root: corpus.clone(),
        index_path: Some(db.clone()),
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    assert!(
        searcher
            .search("kill_crash_needle")
            .unwrap()
            .hits
            .iter()
            .any(|hit| hit.file == "k.py"),
        "rebuilt index must serve the needle"
    );
}

/// Child entry point for the SIGKILL test: without the spec env it is a no-op
/// so normal suite runs are unaffected. With it, the child opens the store,
/// holds an uncommitted bulk transaction open, and writes until killed.
#[test]
fn r2_child_writer_entry() {
    let (Ok(root), Ok(db), Ok(ready)) = (
        std::env::var("ASGREP_R2_ROOT"),
        std::env::var("ASGREP_R2_DB"),
        std::env::var("ASGREP_R2_READY"),
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
            &format!("victim-{i}"),
        );
        if i == 0 {
            std::fs::write(&ready, b"ready").unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    store.commit_bulk_tx().unwrap();
}

/// FAULT torn-write: prefix of store A spliced to suffix of store B (a torn
/// page-1). The splice must fail closed — loud refusal at open, or an open
/// handle whose verification reports corruption — deterministically, with no
/// quarantine or fabricated sidecars from the failed opens.
#[test]
fn fault_torn_splice_prefix_suffix_never_silently_healthy() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    // Structurally divergent stores: near-identical stores splice into an
    // accidentally coherent db (same page count, aligned pointers). Different
    // row counts and sizes tear by construction (db-size header vs file size).
    let big_body = format!("def big():\n{}\n    return 1\n", "# pad\n".repeat(5000));
    let a_files: Vec<(&str, &str)> = vec![
        ("a0.py", "def a0():\n    return 0\n"),
        ("a1.py", "def a1():\n    return 1\n"),
        ("a2.py", "def a2():\n    return 2\n"),
        ("a3.py", "def a3():\n    return 3\n"),
        ("big.py", &big_body),
    ];
    let a = fixture_store(dir, "splice-a", &a_files);
    let b = fixture_store(dir, "splice-b", &[("b.py", "x = 1\n")]);
    let a_bytes = std::fs::read(&a).unwrap();
    let b_bytes = std::fs::read(&b).unwrap();
    assert!(
        a_bytes.len() > b_bytes.len() + 4096,
        "splice sources must diverge in size"
    );
    let min = b_bytes.len();
    assert!(min > 4096, "fixtures must span several pages");

    // Splice mid-page-1 so the schema page itself is torn.
    let mut splice = a_bytes[..2000].to_vec();
    splice.extend_from_slice(&b_bytes[2000..min]);

    // Fresh copy per probe: a writable open may normalize in place, so each
    // outcome is measured against pristine torn bytes.
    let probe_fresh = |tag: &str, readonly: bool| -> (Probe, PathBuf) {
        let root = dir.join(format!("splice-{tag}-root"));
        let db = dir.join(format!("splice-{tag}")).join("index.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        std::fs::write(&db, &splice).unwrap();
        assert_torn(&db);
        (probe_open(&root, &db, readonly), db)
    };

    // Read-only opens never normalize: refuse or detect, deterministically.
    let (first, _) = probe_fresh("ro-a", true);
    let (second, db_ro) = probe_fresh("ro-b", true);
    assert_eq!(first, second, "read-only splice outcome must be deterministic");
    assert!(
        matches!(first, Probe::Refused(_) | Probe::Dirty(_)),
        "read-only splice must refuse or detect corruption, got {first:?}"
    );
    if let Probe::Refused(name) = &first {
        assert_eq!(*name, "Database", "read-only splice refusal must be Database");
    }
    assert_eq!(std::fs::read(&db_ro).unwrap(), splice);

    // Writable opens refuse, detect, or normalize to an empty healthy store —
    // never serve torn rows as authoritative.
    let (first, _) = probe_fresh("rw-a", false);
    let (second, db_rw) = probe_fresh("rw-b", false);
    assert_eq!(first, second, "writable splice outcome must be deterministic");
    match &first {
        Probe::Refused(name) => assert_eq!(*name, "Database"),
        Probe::Dirty(_) => {}
        Probe::Healthy { file_count } => assert_eq!(
            *file_count, 0,
            "normalized splice must serve zero torn rows, got {first:?}"
        ),
    }

    for db in [&db_ro, &db_rw] {
        let after = dir_names(db.parent().unwrap());
        assert!(
            after.iter().all(|n| !n.contains(".corrupt")),
            "failed splice opens must not quarantine: {after:?}"
        );
        assert!(
            after
                .iter()
                .all(|n| n != "lexical.db" && n != "semantic.ivf"),
            "failed splice opens must not fabricate sidecars: {after:?}"
        );
    }
}

/// FAULT truncation sweep: 0/1/header/mid/near-end tears of a real store all
/// refuse loudly. Exact discriminants where SQLite is strict (0→Other,
/// torn-header→Database); mid/near-end tears refuse at open or fail full
/// verification, never silently healthy, never with side effects.
#[test]
fn fault_truncation_offsets_refuse_or_detect_loudly() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let src = fixture_store(
        dir,
        "trunc-src",
        &[
            ("one.py", "def one():\n    return 1\n"),
            ("two.py", "def two():\n    return 2\n"),
            ("three.py", "def three():\n    return 3\n"),
            ("four.py", "def four():\n    return 4\n"),
        ],
    );
    let full = std::fs::read(&src).unwrap();
    // Near-end tears a whole live page: a 1-byte-short tail zero-fills and
    // verifies clean (no page checksums), so that offset is not a fault.
    // With an empty freelist every page is live and the last page tears.
    let conn = rusqlite::Connection::open(&src).unwrap();
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let freelist: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert!(page_count >= 5, "fixture must span several pages");
    assert_eq!(freelist, 0, "fixture must have no dead pages");
    let page_size = full.len() / page_count as usize;
    let mid = full.len() / 2;
    let near_end = full.len() - page_size;

    // Per-offset arms. The zero-byte writable arm is R1-owned (empty store
    // initializes in place) and is not re-pinned; a one-byte file normalizes
    // the same way (SQLite reads it as no-index-yet), pinned here as
    // read-only-refuses / writable-normalizes-empty. Mid/near-end tears
    // refuse, detect, or normalize empty — never serve torn rows.
    for (label, len) in [
        ("zero", 0usize),
        ("one-byte", 1usize),
        ("header", 64usize),
        ("mid", mid),
        ("near-end", near_end),
    ] {
        let root = dir.join(format!("trunc-{label}-root"));
        let db = dir.join(format!("trunc-{label}")).join("index.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        std::fs::write(&db, &full[..len]).unwrap();
        if len > 100 {
            assert_torn(&db);
        }

        match (label, probe_open(&root, &db, true)) {
            ("zero" | "one-byte", probe) => assert_eq!(
                probe,
                Probe::Refused("Other"),
                "{label} read-only must refuse as Other"
            ),
            ("header", probe) => assert_eq!(
                probe,
                Probe::Refused("Database"),
                "{label} read-only must refuse as Database"
            ),
            (_, Probe::Refused(name)) => assert_eq!(name, "Database"),
            (_, Probe::Dirty(_)) => {}
            (_, probe) => panic!("{label} read-only must never verify healthy: {probe:?}"),
        }
        assert_eq!(std::fs::read(&db).unwrap(), full[..len]);

        // Fresh copy: the read-only probe is side-effect free, but the
        // writable probe may normalize, so re-lay pristine torn bytes.
        std::fs::write(&db, &full[..len]).unwrap();
        match (label, probe_open(&root, &db, false)) {
            ("zero", _) => {}
            ("one-byte", probe) => assert_eq!(
                probe,
                Probe::Healthy { file_count: 0 },
                "one-byte writable must normalize to an empty store"
            ),
            ("header", probe) => {
                assert_eq!(
                    probe,
                    Probe::Refused("Database"),
                    "header writable must refuse as Database"
                );
                assert_eq!(std::fs::read(&db).unwrap(), full[..len]);
            }
            (_, Probe::Refused(name)) => assert_eq!(name, "Database"),
            (_, Probe::Dirty(_)) => {}
            (_, Probe::Healthy { file_count }) => assert_eq!(
                file_count, 0,
                "{label} writable must serve zero torn rows"
            ),
        }
        let after = dir_names(db.parent().unwrap());
        assert!(
            after.iter().all(|n| !n.contains(".corrupt")),
            "{label}: failed opens must not quarantine: {after:?}"
        );
    }
}

/// FAULT locks: stray `*.lock` files are not part of the contract — SQLite
/// owns concurrency — so planting them must neither block nor be consumed.
/// A live SQLite write lock, in contrast, blocks a second writer, which then
/// proceeds once the holder rolls back (never fails fast, never wedges).
#[test]
fn fault_stale_lockfile_ignored_live_lock_blocks_then_proceeds() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let root = dir.join("lock-root");
    let db = dir.join("locks").join("index.db");
    drop(IndexStore::open(&root, Some(&db)).unwrap());

    // Stale lock files: garbage pid, never honored, never reaped.
    let lock_a = db.with_extension("db.lock");
    let lock_b = db.parent().unwrap().join(".asgrep-index.lock");
    std::fs::write(&lock_a, b"pid 99999\n").unwrap();
    std::fs::write(&lock_b, b"stale\n").unwrap();
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    assert_eq!(store.status().unwrap().file_count, 0);
    upsert_file(&store, "l.py", "x = 1\n".to_string(), "lock-h");
    assert_eq!(std::fs::read(&lock_a).unwrap(), b"pid 99999\n");
    assert_eq!(std::fs::read(&lock_b).unwrap(), b"stale\n");
    drop(store);

    // Live lock: a raw connection holds BEGIN IMMEDIATE while a second
    // opener writes; the writer must block, then succeed after rollback.
    let holder = rusqlite::Connection::open(&db).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            let second = IndexStore::open(&root, Some(&db)).unwrap();
            upsert_file(&second, "w.py", "y = 2\n".to_string(), "live-w");
        });
        std::thread::sleep(Duration::from_millis(300));
        holder.execute_batch("ROLLBACK").unwrap();
        writer.join().unwrap();
    });
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "second writer must have blocked on the live lock"
    );
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    assert!(store.file_hash("w.py").unwrap().is_some());
}

/// FAULT read-only store: with the db file and its directory chmodded
/// read-only, every access path fails closed as `Database` — the read-only
/// open because WAL mode needs a writable directory for `-shm`, the writable
/// path at open or at first write. Never a panic, never partial bytes. (R1
/// owns the missing-db-under-readonly-dir open; this pins an existing store
/// frozen read-only.)
#[test]
#[cfg(unix)]
fn fault_readonly_store_first_write_fails_database() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let root = dir.join("ro-root");
    let home = dir.join("ro-home");
    let db = home.join("index.db");
    {
        let store = IndexStore::open(&root, Some(&db)).unwrap();
        upsert_file(&store, "r.py", "z = 3\n".to_string(), "ro-h");
        store.checkpoint_wal().unwrap();
    }
    // Control arm: the fixture is a healthy serving store before the chmod.
    assert_eq!(integrity_of(&root, &db), "ok");
    // Remove sidecars AFTER the control: even a read-only open can materialize
    // `-shm` while the directory is writable, which would change post-chmod
    // behavior (existing shm = servable). The fault is a lone frozen db.
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    let before = std::fs::read(&db).unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o555)).unwrap();

    // Privileged runners (root) bypass permission bits; detect and skip.
    let privileged = std::fs::OpenOptions::new().write(true).open(&db).is_ok();
    if privileged {
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
        eprintln!("SKIP: privileged uid bypasses read-only bits");
        return;
    }

    let ro_err = err_of(IndexStore::open_readonly(&root, Some(&db)));
    let ro_name = discriminant_name(&ro_err).to_string();
    let rw_open_failed = match IndexStore::open(&root, Some(&db)) {
        Err(err) => Some(discriminant_name(&err).to_string()),
        Ok(store) => {
            // SQLite opens lazily; the first write must fail closed.
            let err = err_of(store.set_meta("r2_probe", "1"));
            assert_eq!(
                discriminant_name(&err),
                "Database",
                "first write must fail as Database"
            );
            None
        }
    };
    let after = std::fs::read(&db).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(ro_name, "Database", "read-only open must fail as Database");
    if let Some(name) = rw_open_failed {
        assert_eq!(name, "Database", "writable open must fail as Database");
    }
    assert_eq!(after, before, "failed access must not alter db bytes");
}

/// FAULT concurrent openers: two writable handles on one db both succeed and
/// observe each other's committed rows in both directions — the second opener
/// is a peer, never wedged, never forked.
#[test]
fn fault_concurrent_second_opener_bidirectional_visibility() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let root = dir.join("conc-root");
    let db = dir.join("conc").join("index.db");
    drop(IndexStore::open(&root, Some(&db)).unwrap());

    let first = IndexStore::open(&root, Some(&db)).unwrap();
    let second = IndexStore::open(&root, Some(&db)).unwrap();
    upsert_file(&first, "from-first.py", "a = 1\n".to_string(), "c1");
    assert_eq!(
        second.file_hash("from-first.py").unwrap().as_deref(),
        Some("c1"),
        "second opener must see the first's commit"
    );
    upsert_file(&second, "from-second.py", "b = 2\n".to_string(), "c2");
    assert_eq!(
        first.file_hash("from-second.py").unwrap().as_deref(),
        Some("c2"),
        "first opener must see the second's commit"
    );
    assert_eq!(first.status().unwrap().file_count, 2);
    assert_eq!(second.status().unwrap().file_count, 2);
}

/// FAULT interrupted rename: the IVF sidecar publishes via tmp+fsync+rename.
/// A crash between tmp-write and rename leaves the old file intact (old
/// served, new invisible); completing the rename publishes the new payload.
/// Either side of the crash the loader sees exactly old-or-new, never mixed.
#[test]
fn fault_interrupted_rename_old_or_new_never_mixed() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let dim = 4usize;
    let old_vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let new_vectors: Vec<f32> = (0..16).map(|i| 100.0 + i as f32).collect();
    assert_ne!(old_vectors, new_vectors);
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
    assert_eq!(
        load_semantic_ivf(&final_path, old_fp).unwrap().unwrap().vectors,
        old_vectors
    );

    // Crash point: new payload fsynced to tmp, rename never ran.
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
    assert!(
        load_semantic_ivf(&final_path, new_fp).unwrap().is_none(),
        "pre-rename crash must not expose the new payload"
    );

    // The rename completes: exactly the new payload, never a mixture.
    std::fs::rename(&orphan_tmp, &final_path).unwrap();
    assert!(
        load_semantic_ivf(&final_path, old_fp).unwrap().is_none(),
        "post-rename load must not serve the old payload"
    );
    assert_eq!(
        load_semantic_ivf(&final_path, new_fp).unwrap().unwrap().vectors,
        new_vectors,
        "post-rename load must serve exactly the new payload"
    );
}

/// FAULT orphan tmp: a stale `.semantic.ivf.*.tmp` left by a crashed save must
/// not poison the next save — the next publish succeeds (unique tmp names)
/// and the final sidecar loads exactly the fresh payload.
#[test]
fn fault_orphan_ivf_tmp_next_save_unaffected() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    std::fs::write(dir.join(".semantic.ivf.99999.0.tmp"), vec![0xABu8; 512]).unwrap();

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

/// FAULT torn read: a read-only open during another handle's uncommitted bulk
/// write must serve the last committed snapshot (never partial rows); after
/// commit a fresh read observes the new rows.
#[test]
fn fault_read_during_uncommitted_write_serves_last_committed() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let root = dir.join("snap-root");
    let db = dir.join("snap").join("index.db");
    let writer = IndexStore::open(&root, Some(&db)).unwrap();
    upsert_file(&writer, "committed.py", "c = 1\n".to_string(), "s1");

    writer.begin_bulk_tx().unwrap();
    upsert_file(&writer, "uncommitted.py", "u = 2\n".to_string(), "s2");

    let reader = IndexStore::open_readonly(&root, Some(&db)).unwrap();
    assert_eq!(
        reader.status().unwrap().file_count,
        1,
        "reader must see last committed, not the open tx"
    );
    assert!(reader.file_hash("uncommitted.py").unwrap().is_none());
    drop(reader);

    writer.commit_bulk_tx().unwrap();
    let reader = IndexStore::open_readonly(&root, Some(&db)).unwrap();
    assert_eq!(reader.status().unwrap().file_count, 2);
    assert!(reader.file_hash("uncommitted.py").unwrap().is_some());
}

/// FAULT path-type confusion: the index path exists but is a directory, not a
/// file. Both opens refuse loudly — writable as `Database` (SQLite cannot
/// open a directory), read-only as `Other` (not-a-file guard) — and the
/// directory is left untouched.
#[test]
fn fault_index_path_is_directory_fails_closed() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let root = dir.join("dir-root");
    std::fs::create_dir_all(&root).unwrap();
    let db = dir.join("as-dir").join("index.db");
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
    assert!(
        dir_names(&db).is_empty(),
        "failed opens must not write into the directory"
    );
}
