//! Close-path WAL fold must not block on a concurrent reader snapshot.
//!
//! KILLS: serve-ambient-refresh-stall. `IndexStore::drop` runs
//! `PRAGMA wal_checkpoint(TRUNCATE)`; a serve session holding a long-lived
//! read transaction pins the WAL, so the close fold sleeps out the writer's
//! 5s busy_timeout inside Drop — measured ~8s per serve call on a stale
//! root (sampled: 8503/9484 main-thread samples inside
//! sqliteDefaultBusyCallback). The close fold must instead bound its wait
//! to the reader budget and treat BUSY as a benign skip: the WAL is the
//! normal on-disk state mid-session, and `wal_autocheckpoint` +
//! `journal_size_limit` bound growth until an uncontended close folds it.

use std::fs;
use std::time::{Duration, Instant};

use ast_sgrep_core::{IndexOptions, Indexer};
use rusqlite::{Connection, OpenFlags};

fn options(root: &std::path::Path, db: &std::path::Path) -> IndexOptions {
    IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        embed_semantic: false,
        use_tantivy: true,
        ..IndexOptions::default()
    }
}

fn write_corpus(root: &std::path::Path, generation: usize) {
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("app.rs"),
        format!("pub fn alpha_needle_{generation}() -> u32 {{ {generation} }}\n"),
    )
    .unwrap();
    fs::write(
        src.join("util.py"),
        format!("def beta_needle_{generation}():\n    return {generation}\n"),
    )
    .unwrap();
}

#[test]
fn writer_close_does_not_block_on_pinned_reader_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("corpus");
    let index_dir = root.join(".asgrep");
    fs::create_dir_all(&index_dir).unwrap();
    let db = index_dir.join("index.db");

    // Clean initial build, then an uncontended close.
    write_corpus(&root, 0);
    {
        let mut indexer = Indexer::new(options(&root, &db)).unwrap();
        indexer.index_all().unwrap();
    }
    assert!(db.exists(), "index.db missing after writer close");

    // Pin the WAL with a long-lived read snapshot, the way a serve
    // session's searchers do.
    let pin = Connection::open_with_flags(
        &db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    pin.execute_batch("BEGIN;").unwrap();
    let _: i64 = pin
        .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
        .expect("snapshot query opens a read transaction");

    // A second writer commits a delta and drops while the pin holds.
    let start = Instant::now();
    write_corpus(&root, 1);
    {
        let mut indexer = Indexer::new(options(&root, &db)).unwrap();
        indexer.index_all().unwrap();
    }
    let pinned_close = start.elapsed();
    assert!(
        pinned_close < Duration::from_secs(2),
        "writer close blocked {pinned_close:?} against a pinned reader snapshot; \
         the close fold must bound its busy wait (serve calls stall ~8s/call on \
         stale roots)"
    );

    drop(pin);

    // Sanity: an uncontended writer close still folds the WAL.
    let start = Instant::now();
    {
        let mut indexer = Indexer::new(options(&root, &db)).unwrap();
        indexer.index_all().unwrap();
    }
    let uncontended_close = start.elapsed();
    assert!(
        uncontended_close < Duration::from_secs(2),
        "uncontended close regressed: {uncontended_close:?}"
    );
    let wal = index_dir.join("index.db-wal");
    let wal_size = wal
        .exists()
        .then(|| fs::metadata(&wal).unwrap().len())
        .unwrap_or(0);
    assert!(
        wal_size < 1 << 20,
        "uncontended close left a {wal_size}-byte WAL; clean close should fold it"
    );
}
