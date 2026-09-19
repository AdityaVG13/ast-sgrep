//! Core crash-recovery fixtures: corpus, builds, comparators, fault gates.
//!
//! # Contract
//!
//! - One canonical copy of the file-local helpers triplicated across the
//!   `tests/core/recovery_*` suites (contracts/drills/faults/relations).
//! - Builders index the [`RECOVERY_CORPUS`] fixture and quiesce (checkpoint +
//!   sidecar sweep) so raw bytes are stable for comparison.
//! - Comparators pin logical state (snapshot rows, excerpt-pinned hit keys),
//!   never message text. [`search_parity_key`] deliberately pins
//!   `excerpt`/`line_end`, NOT the `kind` shape of
//!   [`crate::core_search_hit_keys`]: content drift must fail the comparison,
//!   not hide behind a coarser key.
//! - Helpers panic (never `Result`) on IO/index failure, matching suite
//!   convention: a broken fixture is a test failure, not a fallible op.

use crate::fault::remove_sqlite_sidecars;
use crate::isolation::{IsolatedIndexSession, isolated_index_session};
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
use std::path::{Path, PathBuf};

/// Shared needle corpus: three files, one distinct searchable token each
/// (`alpha_needle`, `beta_needle`, `gamma_caller`), so per-query attribution
/// is exact and parity is pinned over real hits, not mutual emptiness.
pub const RECOVERY_CORPUS: &[(&str, &str)] = &[
    ("src/a.py", "def alpha_needle():\n    return 1\n"),
    ("src/b.py", "def beta_needle():\n    return 2\n"),
    ("src/c.py", "import os\n\ndef gamma_caller():\n    return os.getcwd()\n"),
];

/// INTENT: the writable starting point every core recovery test builds from —
/// an isolated session with [`RECOVERY_CORPUS`] written, ready to index.
pub fn corpus_session() -> IsolatedIndexSession {
    let session = isolated_index_session();
    for (rel, body) in RECOVERY_CORPUS {
        session.write(rel, body);
    }
    session
}

/// INTENT: build (or force-rebuild) the corpus index, then checkpoint WAL
/// content into the main db and drop all handles so raw bytes are stable for
/// comparison. The strictest of the three suite copies: `force` selects
/// `reindex_all` vs `index_all`, `use_tantivy` the sidecar lane.
pub fn build_and_quiet(root: &Path, db: &Path, force: bool, use_tantivy: bool) {
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        embed_semantic: false,
        force_reindex: force,
        use_tantivy,
        ..IndexOptions::default()
    })
    .unwrap();
    if force {
        indexer.reindex_all().unwrap();
    } else {
        indexer.index_all().unwrap();
    }
    indexer.store().checkpoint_wal().unwrap();
    drop(indexer);
    remove_sqlite_sidecars(db);
}

/// INTENT: quiesced main-db bytes for byte-equality asserts. Sidecars are
/// removed first: WAL presence is state-dependent, so only the main db is
/// byte-stable across builds.
pub fn quiesced_db_bytes(db: &Path) -> Vec<u8> {
    remove_sqlite_sidecars(db);
    std::fs::read(db).unwrap()
}

/// INTENT: deterministic logical snapshot — status counts, per-file content
/// hashes, and every indexed line row. Volatile meta is excluded by
/// construction, so equality means the recovered state converged.
pub fn store_snapshot(root: &Path, db: &Path) -> String {
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
        parts.push(format!("file:{path}={:?}", store.file_hash(&path).unwrap()));
    }
    for row in store.all_indexed_lines().unwrap() {
        parts.push(format!("line:{}:{}:{}", row.0, row.1, row.2));
    }
    parts.join("\n")
}

/// INTENT: sorted, float-free hit keys for ONE query — file, span, symbol,
/// excerpt. Scores are f64 and excluded; excerpt/line_end are pinned (see
/// module docs) so served-content drift fails loudly. Ranking breadth is
/// pinned by key-vector equality.
pub fn search_parity_key(root: &Path, db: &Path, query: &str, use_tantivy: bool) -> Vec<String> {
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

/// INTENT: [`search_parity_key`] over a query set: the per-query answer vector
/// a recovered index must reproduce to prove serve parity with its baseline.
pub fn search_parity_keys(
    root: &Path,
    db: &Path,
    queries: &[&str],
    use_tantivy: bool,
) -> Vec<(String, Vec<String>)> {
    queries
        .iter()
        .map(|q| ((*q).to_string(), search_parity_key(root, db, q, use_tantivy)))
        .collect()
}

/// INTENT: fixture-validity gate — torn bytes must actually be torn under a
/// raw SQLite open, otherwise the "fault" is accidentally coherent and the
/// test it guards is vacuous.
pub fn assert_torn(db: &Path) {
    match rusqlite::Connection::open(db) {
        Err(_) => {}
        Ok(conn) => match conn.query_row("PRAGMA integrity_check", [], |r| {
            r.get::<_, String>(0)
        }) {
            Err(_) => {}
            Ok(detail) => assert_ne!(detail, "ok", "fault fixture is not torn: {}", db.display()),
        },
    }
}

/// INTENT: quarantine slot path for `db` (`index.db` + `.corrupt[.N]`): the
/// evidence address every recovery path must preserve the torn image at.
pub fn quarantine_path(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.file_name().unwrap().to_os_string();
    name.push(suffix);
    db.with_file_name(name)
}

/// INTENT: sorted directory listing for quarantine/sidecar-absence asserts.
/// Takes the directory itself (not the db): the directory-as-db facet lists a
/// path that IS a directory, which a db-parent form cannot express.
pub fn home_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// INTENT: single-row upsert with fixed test metadata (python, mtime 1, LF):
/// the minimal committed-state probe for concurrency and snapshot-isolation
/// tests. Panics when the upsert fails.
pub fn upsert_test_file(store: &IndexStore, rel: &str, body: String, hash: &str) {
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
