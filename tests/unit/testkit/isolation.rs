use super::*;
use std::sync::{Mutex, OnceLock};

/// Serialize env mutation: these tests touch process-global env.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[test]
fn sessions_get_distinct_on_disk_paths() {
    let a = isolated_index_session();
    let b = isolated_index_session();
    assert_ne!(a.corpus_root, b.corpus_root);
    assert_ne!(a.index_path, b.index_path);
    assert!(a.index_path.ends_with("index.db"));
    assert!(b.index_path.ends_with("index.db"));
    // Paths live under distinct temp roots (parent of corpus).
    assert_ne!(
        a.corpus_root.parent().unwrap(),
        b.corpus_root.parent().unwrap()
    );
}

#[test]
fn open_store_creates_real_sqlite_file_not_memory() {
    with_temp_index(|session| {
        let store = session.open_store();
        assert_eq!(store.db_path(), session.index_path);
        assert!(
            session.index_path.is_file(),
            "expected real on-disk db at {}",
            session.index_path.display()
        );
        // SQLite file signature "SQLite format 3\0"
        let header = fs::read(&session.index_path).expect("read db");
        assert!(
            header.starts_with(b"SQLite format 3"),
            "not a real SQLite file"
        );
        let journal: String = store
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(journal.to_ascii_lowercase(), "wal");
    });
}

#[test]
fn explicit_index_path_ignores_asgrep_index_path_env() {
    let _guard = env_lock();
    let poison = TempDir::new().expect("poison temp");
    let poison_db = poison.path().join("shared_poison.db");
    // Create a decoy that must not be used.
    let _ = IndexStore::open(poison.path(), Some(&poison_db)).expect("poison store");
    let prev = std::env::var_os("ASGREP_INDEX_PATH");
    std::env::set_var("ASGREP_INDEX_PATH", &poison_db);
    let result = std::panic::catch_unwind(|| {
        let session = isolated_index_session();
        let store = session.open_store();
        assert_eq!(
            store.db_path(),
            session.index_path,
            "session must not resolve ASGREP_INDEX_PATH"
        );
        assert_ne!(store.db_path(), poison_db);
        assert!(session.index_path.is_file());
    });
    match prev {
        Some(v) => std::env::set_var("ASGREP_INDEX_PATH", v),
        None => std::env::remove_var("ASGREP_INDEX_PATH"),
    }
    result.expect("isolation assertion failed under ASGREP_INDEX_PATH");
}

#[test]
fn index_all_and_search_use_private_db() {
    let session = isolated_index_session();
    session.write("lib.rs", "fn isolated_marker_fn() {}\n");
    let _indexer = session.index_all(IndexOptions {
        force_reindex: true,
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    let resp = searcher.search("isolated_marker_fn").expect("search");
    assert!(
        resp.hits
            .iter()
            .any(|h| h.excerpt.contains("isolated_marker_fn")),
        "expected hit from private index: {:?}",
        resp.hits
    );
    assert!(session.index_path.is_file());
}
