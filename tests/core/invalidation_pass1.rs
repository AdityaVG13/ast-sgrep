//! I1 invalidation-contract oracles: STALENESS-DETECTION discriminants.
//!
//! Scope: writer-generation bump/read roundtrip + uniqueness; `index_data_version`
//! exact-+1 monotonicity; mtime (secs,nanos) identity trust + the
//! `mtime_identity_root` gate (within-root vs cross-root); schema-version
//! staleness refusal (newer-than-binary on both open modes; stale lifecycle via
//! peek/refuse/migrate); cache-home isolation + routing; index-status fresh /
//! stale / empty / missing discriminants.
//!
//! Non-duplication vs prior art: `freshness_identity.rs` pins the sidecar-skip
//! lever, byte-identical noop refresh, lexical fallback on generation change,
//! git-head freshness, and the cross-connection `index_data_version` memo;
//! `index_schema_rekey_freshness.rs` pins the stamp-14 re-key refusal/migration
//! and codemod-plan refusal, all with message-text assertions. These oracles
//! assert DISCRIMINANTS ONLY (is_ok/is_err, counts, u64/i64 equality and
//! inequality, path equality, Option shape) and never inspect message text.
//!
//! Source sites pinned (enumerated from `crates/ast-sgrep-core/src`):
//! - `store/writer_generation.rs`: `read_writer_generation` (absent/corrupt →
//!   0 fail-open), `bump_writer_generation` (unique epoch, not read+1),
//!   `writer_generation_home` (root vs explicit-index-path isolation).
//! - `store/sqlite/mod.rs`: `index_data_version` (+1 per mutation via the
//!   private `bump_index_data_version`), `index_generation` alias,
//!   `peek_schema_version` (side-effect-free stamp read), `open_readonly`
//!   refusal gates (`init_schema_readonly`, newer-than-binary).
//! - `store/sqlite/queries.rs`: `status` (stored-row counts + live
//!   writer-generation stamp).
//! - `store/mod.rs`: `cache_index_path` (root-hashed, XDG/HOME base,
//!   fail-closed), `try_index_db_path` (explicit > local > cache routing).
//! - `index.rs` + `index_prepare.rs`: mtime fast path (`stored == walk` ⇒
//!   `Unchanged` without hashing) gated by `mtime_identity_root`, hash fast
//!   path, `files_indexed/files_skipped/files_removed` discriminants.
//! - `lib.rs`: `StoreError::parse_schema_mismatch` (machine-readable
//!   (on_disk, supported) pair).
//!
//! Hermeticity: every store/indexer open uses an explicit `index_path`, so no
//! test depends on ambient `ASGREP_*` routing. The two cache tests that mutate
//! process env serialize on a static mutex and restore via an RAII guard.
//! No wall-clock sleeps: mtimes are set to explicit whole-second stamps with
//! read-back preconditions.

use ast_sgrep_core::store::{
    cache_index_path, try_index_db_path, UpsertFileInput,
};
use ast_sgrep_core::{
    bump_writer_generation, read_writer_generation, writer_generation_path, IndexOptions, IndexStore,
    Indexer, StoreError, INDEX_SCHEMA_VERSION,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn indexer_at(root: &Path, db: &Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        use_tantivy: false,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap()
}

fn plain_input<'a>(
    path: &'a str,
    hash: &'a str,
    lines: &'a [(u32, String)],
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("rust"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        depth_truncated: false,
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
    }
}

/// Fixed whole-second stamp (nanos = 0 survives every filesystem timestamp
/// granularity). All mtime-forging tests use whole seconds + read-back asserts.
const WHOLE_SECOND_T0: u64 = 1_700_000_000;

fn whole_second(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::new(secs, 0)
}

/// Set an exact mtime and assert the filesystem stored it back bit-identically
/// (precondition: proves the forge below is real, not truncated).
fn set_mtime_checked(path: &Path, time: SystemTime) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
    assert_eq!(
        path.metadata().unwrap().modified().unwrap(),
        time,
        "filesystem must store the forged mtime exactly"
    );
}

fn stored_mtime(store: &IndexStore, rel: &str) -> (i64, u32) {
    store
        .connection()
        .query_row(
            "SELECT mtime_secs, mtime_nanos FROM files WHERE path = ?1",
            [rel],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

struct EnvRestore {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvRestore {
    fn capture() -> Self {
        let saved = ["ASGREP_INDEX_PATH", "ASGREP_USE_CACHE", "XDG_CACHE_HOME", "HOME", "USERPROFILE"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        Self { saved }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

// ---- writer generation: roundtrip, uniqueness, isolation, fail-open ----

#[test]
fn writer_generation_absent_reads_zero_and_bump_roundtrips() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    assert_eq!(read_writer_generation(root, None), 0);

    let stamped = bump_writer_generation(root, None).unwrap();
    assert_eq!(read_writer_generation(root, None), stamped);

    // On-disk format discriminant: decimal text that parses to the same epoch.
    let body = std::fs::read_to_string(writer_generation_path(root, None)).unwrap();
    assert_eq!(body.trim().parse::<u64>().unwrap(), stamped);
}

#[test]
fn writer_generation_bumps_are_unique_not_sequential() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // The stamp is a unique epoch (time + pid + sequence), explicitly NOT a
    // read+1 counter: two writers that both read N must not both publish N+1.
    let first = bump_writer_generation(root, None).unwrap();
    assert_eq!(read_writer_generation(root, None), first);
    let second = bump_writer_generation(root, None).unwrap();
    assert_eq!(read_writer_generation(root, None), second);
    assert_ne!(first, second, "successive bumps must publish distinct epochs");
}

#[test]
fn writer_generation_isolated_per_home_and_fail_open_on_corrupt_stamp() {
    let temp_a = tempfile::tempdir().unwrap();
    let temp_b = tempfile::tempdir().unwrap();
    let (root_a, root_b) = (temp_a.path(), temp_b.path());

    let epoch_a = bump_writer_generation(root_a, None).unwrap();
    assert_eq!(
        read_writer_generation(root_b, None),
        0,
        "bumping root A must not move root B's epoch"
    );
    let epoch_b = bump_writer_generation(root_b, None).unwrap();
    assert_eq!(read_writer_generation(root_a, None), epoch_a);
    assert_eq!(read_writer_generation(root_b, None), epoch_b);

    // Explicit index paths isolate by DB parent directory, not by root.
    let db1 = root_a.join("idx1").join("index.db");
    let db2 = root_a.join("idx2").join("index.db");
    let pinned = bump_writer_generation(root_a, Some(&db1)).unwrap();
    assert_eq!(read_writer_generation(root_a, Some(&db1)), pinned);
    assert_eq!(
        read_writer_generation(root_a, Some(&db2)),
        0,
        "a pinned index_path must own an independent stamp"
    );

    // Corrupt stamp content fails open to the cold-start epoch, never an error.
    std::fs::write(writer_generation_path(root_a, None), "not-a-number\n").unwrap();
    assert_eq!(read_writer_generation(root_a, None), 0);
    std::fs::write(writer_generation_path(root_a, None), "").unwrap();
    assert_eq!(read_writer_generation(root_a, None), 0);
    // Root B is unaffected by A's corruption.
    assert_eq!(read_writer_generation(root_b, None), epoch_b);
}

// ---- index_data_version: exact-+1 monotonicity + alias ----

#[test]
fn index_data_version_bumps_exactly_one_per_upsert() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("v").join("index.db");
    let store = IndexStore::open(temp.path(), Some(&db)).unwrap();
    let v0 = store.index_data_version().unwrap();
    assert_eq!(store.index_generation().unwrap(), v0);

    let first = [(1, "one".to_string())];
    store.upsert_file(plain_input("src/a.rs", "h1", &first)).unwrap();
    assert_eq!(store.index_data_version().unwrap(), v0 + 1);

    let other = [(1, "other".to_string())];
    store.upsert_file(plain_input("src/b.rs", "h2", &other)).unwrap();
    assert_eq!(store.index_data_version().unwrap(), v0 + 2);

    // Same-structure re-upsert routes to refresh_lines_only; still exactly +1.
    let changed = [(1, "two".to_string())];
    store
        .upsert_file(plain_input("src/a.rs", "h3", &changed))
        .unwrap();
    let v3 = store.index_data_version().unwrap();
    assert_eq!(v3, v0 + 3);
    assert_eq!(store.index_generation().unwrap(), v3);
}

// ---- mtime identity: trust, gate defeat, nanos, cross-root ----

#[test]
fn mtime_match_short_circuits_hash_check_within_certified_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let db = root.join(".asgrep").join("index.db");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn alpha_one() {}\n").unwrap();

    let stats = indexer_at(root, &db).index_all().unwrap();
    assert_eq!(stats.files_indexed, 1);

    // Tamper the stored hash WITHOUT touching the file: mtime still matches,
    // so the mtime fast path must skip without ever consulting the hash.
    let generation_before;
    {
        let store = IndexStore::open(root, Some(&db)).unwrap();
        store
            .connection()
            .execute_batch("UPDATE files SET content_hash = 'tampered' WHERE path = 'src/a.rs'")
            .unwrap();
        assert_eq!(store.file_hash("src/a.rs").unwrap().as_deref(), Some("tampered"));
        assert!(store.get_meta("mtime_identity_root").unwrap().is_some());
        generation_before = store.index_data_version().unwrap();
    }
    let stats = indexer_at(root, &db).index_all().unwrap();
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_skipped, 1);

    // A pure mtime skip performs no mutation: generation unmoved, tamper intact.
    let store = IndexStore::open(root, Some(&db)).unwrap();
    assert_eq!(store.index_data_version().unwrap(), generation_before);
    assert_eq!(store.file_hash("src/a.rs").unwrap().as_deref(), Some("tampered"));
}

#[test]
fn mtime_gate_deletion_and_nanos_mismatch_force_hash_recheck() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let db = root.join(".asgrep").join("index.db");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn alpha_one() {}\n").unwrap();
    assert_eq!(indexer_at(root, &db).index_all().unwrap().files_indexed, 1);

    // Phase A: same tamper as the trust test, but with the gate deleted the
    // mtime fast path is defeated and the hash mismatch forces re-extraction.
    {
        let store = IndexStore::open(root, Some(&db)).unwrap();
        store
            .connection()
            .execute_batch("UPDATE files SET content_hash = 'tampered' WHERE path = 'src/a.rs'")
            .unwrap();
        store.delete_meta("mtime_identity_root").unwrap();
        assert_eq!(store.get_meta("mtime_identity_root").unwrap(), None);
    }
    let stats = indexer_at(root, &db).index_all().unwrap();
    assert_eq!(stats.files_indexed, 1);
    {
        let store = IndexStore::open(root, Some(&db)).unwrap();
        assert_ne!(store.file_hash("src/a.rs").unwrap().as_deref(), Some("tampered"));
        assert!(store.get_meta("mtime_identity_root").unwrap().is_some());
    }

    // Phase B: the identity is the (secs, nanos) PAIR — perturbing only the
    // stored nanos (DB-side, so no filesystem-granularity risk) with the gate
    // intact must still force the hash consult and catch the re-tampered hash.
    {
        let store = IndexStore::open(root, Some(&db)).unwrap();
        store
            .connection()
            .execute_batch(
                "UPDATE files SET content_hash = 'tampered-again' WHERE path = 'src/a.rs';
                 UPDATE files SET mtime_nanos = CASE WHEN mtime_nanos = 0 THEN 1 ELSE mtime_nanos - 1 END
                 WHERE path = 'src/a.rs'",
            )
            .unwrap();
        assert!(store.get_meta("mtime_identity_root").unwrap().is_some());
    }
    let stats = indexer_at(root, &db).index_all().unwrap();
    assert_eq!(
        stats.files_indexed, 1,
        "nanos-only stored/fresh mismatch must defeat the mtime skip"
    );
}

#[test]
fn cross_root_db_reuse_disables_mtime_trust() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let dir_db = tempfile::tempdir().unwrap();
    let (root_a, root_b) = (dir_a.path(), dir_b.path());
    let db = dir_db.path().join("shared").join("index.db");
    let t0 = whole_second(WHOLE_SECOND_T0);

    std::fs::create_dir_all(root_a.join("src")).unwrap();
    std::fs::write(root_a.join("src/a.rs"), "fn content_ax() {}\n").unwrap();
    set_mtime_checked(&root_a.join("src/a.rs"), t0);
    assert_eq!(indexer_at(root_a, &db).index_all().unwrap().files_indexed, 1);
    let hash_a = IndexStore::open(root_a, Some(&db))
        .unwrap()
        .file_hash("src/a.rs")
        .unwrap()
        .unwrap();

    // Root B: DIFFERENT bytes but the forged SAME mtime. Within one root this
    // would skip (see the control below); across roots the gate must force a
    // content-hash decision, which detects the change.
    std::fs::create_dir_all(root_b.join("src")).unwrap();
    std::fs::write(root_b.join("src/a.rs"), "fn content_by() {}\n").unwrap();
    set_mtime_checked(&root_b.join("src/a.rs"), t0);
    let stats = indexer_at(root_b, &db).index_all().unwrap();
    assert_eq!(
        stats.files_indexed, 1,
        "cross-root mtime agreement must not skip changed content"
    );
    let hash_b = IndexStore::open(root_b, Some(&db))
        .unwrap()
        .file_hash("src/a.rs")
        .unwrap()
        .unwrap();
    assert_ne!(hash_a, hash_b);

    // Control: within root B (gate now certifies B), the same forge DOES skip.
    let (secs, nanos) = stored_mtime(&IndexStore::open(root_b, Some(&db)).unwrap(), "src/a.rs");
    std::fs::write(root_b.join("src/a.rs"), "fn content_cz() {}\n").unwrap();
    set_mtime_checked(
        &root_b.join("src/a.rs"),
        UNIX_EPOCH + Duration::new(secs as u64, nanos),
    );
    let stats = indexer_at(root_b, &db).index_all().unwrap();
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_skipped, 1);
}

// ---- schema version: newer refusal, stale lifecycle ----

#[test]
fn newer_than_binary_schema_refused_on_both_open_modes() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("n").join("index.db");
    let future = INDEX_SCHEMA_VERSION + 1;
    {
        let store = IndexStore::open(temp.path(), Some(&db)).unwrap();
        store
            .connection()
            .execute_batch(&format!("PRAGMA user_version = {future}"))
            .unwrap();
    }
    assert!(IndexStore::open(temp.path(), Some(&db)).is_err());
    assert!(IndexStore::open_readonly(temp.path(), Some(&db)).is_err());
    // Machine-readable discriminant carried by the refusal (ints, not text).
    let error = IndexStore::open(temp.path(), Some(&db)).err().unwrap();
    assert_eq!(
        StoreError::parse_schema_mismatch(&error.to_string()),
        Some((future, INDEX_SCHEMA_VERSION))
    );
    // The side-effect-free peek still reports the on-disk stamp.
    assert_eq!(IndexStore::peek_schema_version(temp.path(), Some(&db)).unwrap(), future);
}

#[test]
fn stale_schema_lifecycle_peek_refuse_migrate_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("s").join("index.db");
    // Stamp 15: one migration behind (the additive depth-truncation flag), so
    // this lifecycle is disjoint from the stamp-14 re-key oracles.
    let stale: i64 = INDEX_SCHEMA_VERSION - 1;
    {
        let store = IndexStore::open(temp.path(), Some(&db)).unwrap();
        store
            .connection()
            .execute_batch(&format!("PRAGMA user_version = {stale}"))
            .unwrap();
    }
    // Peek reports the stale stamp without migrating.
    assert_eq!(IndexStore::peek_schema_version(temp.path(), Some(&db)).unwrap(), stale);
    // Read-only open refuses stale rows (discriminant only).
    assert!(IndexStore::open_readonly(temp.path(), Some(&db)).is_err());
    // The refusal is side-effect-free: the stamp is untouched.
    assert_eq!(IndexStore::peek_schema_version(temp.path(), Some(&db)).unwrap(), stale);
    // Writable open migrates in place: on-disk stamp advances to the binary's.
    {
        let store = IndexStore::open(temp.path(), Some(&db)).unwrap();
        assert!(!store.is_read_only());
        assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        assert_eq!(store.schema_version(), INDEX_SCHEMA_VERSION);
    }
    // Idempotent: a second writable open is a no-op, and read-only now opens.
    {
        let store = IndexStore::open(temp.path(), Some(&db)).unwrap();
        assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
    }
    let store = IndexStore::open_readonly(temp.path(), Some(&db)).unwrap();
    assert!(store.is_read_only());
    assert_eq!(store.on_disk_schema_version().unwrap(), INDEX_SCHEMA_VERSION);
}

// ---- cache home: isolation, routing, fail-closed ----

#[test]
fn cache_index_path_deterministic_and_root_isolated() {
    let _lock = env_lock().lock().unwrap();
    let _restore = EnvRestore::capture();
    let xdg = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CACHE_HOME", xdg.path());

    let root_a = PathBuf::from("/repo/alpha");
    let root_b = PathBuf::from("/repo/beta");
    assert_eq!(cache_index_path(&root_a).unwrap(), cache_index_path(&root_a).unwrap());
    assert_ne!(
        cache_index_path(&root_a).unwrap(),
        cache_index_path(&root_b).unwrap(),
        "distinct roots must hash to distinct cache homes"
    );
    for root in [&root_a, &root_b] {
        let path = cache_index_path(root).unwrap();
        assert_eq!(path.file_name().unwrap(), "index.db");
        assert!(path.starts_with(xdg.path().join("asgrep")));
    }
}

#[test]
fn cache_routing_local_wins_and_env_selects_base_fail_closed() {
    let _lock = env_lock().lock().unwrap();
    let _restore = EnvRestore::capture();
    std::env::remove_var("ASGREP_INDEX_PATH");
    let xdg = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CACHE_HOME", xdg.path());
    std::env::set_var("ASGREP_USE_CACHE", "1");

    // No local db + USE_CACHE: routing resolves to the cache home.
    let corpus = tempfile::tempdir().unwrap();
    let root = corpus.path();
    assert_eq!(
        try_index_db_path(root, None).unwrap(),
        cache_index_path(root).unwrap()
    );
    assert!(try_index_db_path(root, None).unwrap().starts_with(xdg.path().join("asgrep")));

    // A present local db wins over the cache, even with USE_CACHE set.
    let local_db = root.join(".asgrep").join("index.db");
    std::fs::create_dir_all(local_db.parent().unwrap()).unwrap();
    std::fs::write(&local_db, []).unwrap();
    assert_eq!(try_index_db_path(root, None).unwrap(), local_db);

    // HOME fallback when XDG_CACHE_HOME is unset.
    std::env::remove_var("XDG_CACHE_HOME");
    std::env::set_var("HOME", home.path());
    std::env::remove_var("USERPROFILE");
    assert!(cache_index_path(root)
        .unwrap()
        .starts_with(home.path().join(".cache").join("asgrep")));

    // Fail closed: no HOME/XDG/USERPROFILE ⇒ cache routing is an error.
    std::env::remove_var("HOME");
    std::env::remove_var("XDG_CACHE_HOME");
    std::env::remove_var("USERPROFILE");
    assert!(cache_index_path(root).is_err());
    let bare = tempfile::tempdir().unwrap();
    assert!(try_index_db_path(bare.path(), None).is_err());
}

// ---- index status: fresh vs stale vs empty vs missing ----

#[test]
fn status_reports_stored_counts_and_live_writer_epoch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let db = root.join(".asgrep").join("index.db");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn alpha_one() {}\n").unwrap();
    std::fs::write(root.join("src/b.rs"), "fn beta_two() {}\n").unwrap();
    assert_eq!(indexer_at(root, &db).index_all().unwrap().files_indexed, 2);

    let store = IndexStore::open(root, Some(&db)).unwrap();
    let status = store.status().unwrap();
    assert_eq!(status.file_count, 2);
    // `split_content_lines` stores the post-`\n` tail: one trailing-newline
    // file yields 2 line rows, so two such files report 4.
    assert_eq!(status.line_count, 4);
    assert!(status.symbol_count >= 2);
    assert_eq!(status.index_path, db.display().to_string());
    assert_eq!(status.root, root.display().to_string());
    assert_eq!(status.writer_generation, read_writer_generation(root, Some(&db)));

    // Status reads the stamp live: a fresh bump is visible on the next status.
    let bumped = bump_writer_generation(root, Some(&db)).unwrap();
    assert_eq!(store.status().unwrap().writer_generation, bumped);
}

#[test]
fn status_distinguishes_empty_stored_stale_and_missing() {
    // Missing: no database file at all.
    let missing_dir = tempfile::tempdir().unwrap();
    let missing_db = missing_dir.path().join("m").join("index.db");
    assert!(!missing_db.is_file());
    assert_eq!(
        try_index_db_path(missing_dir.path(), Some(&missing_db)).unwrap(),
        missing_db
    );
    assert!(IndexStore::open_readonly(missing_dir.path(), Some(&missing_db)).is_err());
    assert!(IndexStore::peek_schema_version(missing_dir.path(), Some(&missing_db)).is_err());

    // Empty-but-stored: a fresh database opens and reports zero counts.
    let empty_dir = tempfile::tempdir().unwrap();
    let empty_db = empty_dir.path().join("e").join("index.db");
    let store = IndexStore::open(empty_dir.path(), Some(&empty_db)).unwrap();
    let status = store.status().unwrap();
    assert_eq!((status.file_count, status.line_count), (0, 0));

    // Stale: status reflects STORED rows, not the live tree. Deleting the file
    // leaves the stored count until a reindex prunes it.
    let live_dir = tempfile::tempdir().unwrap();
    let live_root = live_dir.path();
    let live_db = live_root.join(".asgrep").join("index.db");
    std::fs::create_dir_all(live_root.join("src")).unwrap();
    std::fs::write(live_root.join("src/a.rs"), "fn alpha_one() {}\n").unwrap();
    assert_eq!(indexer_at(live_root, &live_db).index_all().unwrap().files_indexed, 1);
    std::fs::remove_file(live_root.join("src/a.rs")).unwrap();
    let store = IndexStore::open(live_root, Some(&live_db)).unwrap();
    assert_eq!(
        store.status().unwrap().file_count,
        1,
        "status must report stored rows even after the file is deleted"
    );
    drop(store);
    let stats = indexer_at(live_root, &live_db).index_all().unwrap();
    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_indexed, 0);
    let store = IndexStore::open(live_root, Some(&live_db)).unwrap();
    assert_eq!(store.status().unwrap().file_count, 0);
}
