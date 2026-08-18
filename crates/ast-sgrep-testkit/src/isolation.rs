//! Real SQLite isolation harness for tests.
//!
//! # Contract
//!
//! - Every [`IsolatedIndexSession`] owns a private [`tempfile::TempDir`].
//! - The database is a **real on-disk** SQLite file (`index.db`), not an
//!   in-memory mock and not a shared process-wide path.
//! - `index_path` is always set **explicitly**, so ambient `ASGREP_INDEX_PATH`
//!   and `ASGREP_USE_CACHE` / XDG shared cache cannot leak across tests.
//! - Dropping the session (end of test / end of `with_temp_index`) removes
//!   corpus files and the DB (and SQLite sidecars under the temp root).
//!
//! Prefer this over ad-hoc `TempDir` + `IndexStore::open(root, None)` when the
//! test only needs a private store or a private corpus+index pair.

use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Private corpus root + real on-disk SQLite for a single test.
///
/// Holds the [`TempDir`] so cleanup happens on drop even if the caller only
/// keeps paths/handles derived from this session.
pub struct IsolatedIndexSession {
    /// Owns the on-disk tree; must outlive corpus/db use.
    _temp: TempDir,
    /// Writable corpus directory under the private temp root.
    pub corpus_root: PathBuf,
    /// Explicit path to the real on-disk SQLite database file.
    pub index_path: PathBuf,
}

impl IsolatedIndexSession {
    /// Create a fresh private corpus directory and `index.db` path.
    ///
    /// Does not open SQLite until [`Self::open_store`] / [`Self::index_all`].
    pub fn new() -> Self {
        let temp = TempDir::new().expect("isolated index tempdir");
        let corpus_root = temp.path().join("corpus");
        fs::create_dir_all(&corpus_root).expect("create isolated corpus dir");
        // Explicit file path under temp -- never env/XDG resolved.
        let index_path = temp.path().join("index.db");
        Self {
            _temp: temp,
            corpus_root,
            index_path,
        }
    }

    /// Write a relative file under the private corpus root.
    pub fn write(&self, rel: impl AsRef<Path>, body: impl AsRef<[u8]>) {
        let path = self.corpus_root.join(rel.as_ref());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create corpus parent");
        }
        fs::write(&path, body.as_ref()).unwrap_or_else(|e| {
            panic!("write corpus file {}: {e}", path.display());
        });
    }

    /// [`IndexOptions`] with isolation-safe `root` and `index_path` filled in.
    ///
    /// Callers may override other fields via struct update; `root` / `index_path`
    /// should stay as set here (or re-applied via [`Self::index_all`]).
    pub fn index_options(&self) -> IndexOptions {
        IndexOptions {
            root: self.corpus_root.clone(),
            index_path: Some(self.index_path.clone()),
            ..IndexOptions::default()
        }
    }

    /// [`SearchOptions`] with isolation-safe `root` and `index_path` filled in.
    pub fn search_options(&self) -> SearchOptions {
        SearchOptions {
            root: self.corpus_root.clone(),
            index_path: Some(self.index_path.clone()),
            ..SearchOptions::default()
        }
    }

    /// Open a real on-disk [`IndexStore`] at this session's explicit `index_path`.
    pub fn open_store(&self) -> IndexStore {
        IndexStore::open(&self.corpus_root, Some(&self.index_path))
            .expect("open isolated on-disk IndexStore")
    }

    /// Open the private store under an explicit durability profile (0obi).
    pub fn open_store_with_durability(&self, durability: ast_sgrep_core::Durability) -> IndexStore {
        IndexStore::open_with_durability(&self.corpus_root, Some(&self.index_path), durability)
            .expect("open isolated on-disk IndexStore")
    }

    /// Build an [`Indexer`] with isolation-safe paths, without indexing yet.
    pub fn indexer(&self, mut opts: IndexOptions) -> Indexer {
        opts.root = self.corpus_root.clone();
        opts.index_path = Some(self.index_path.clone());
        Indexer::new(opts).expect("isolated indexer")
    }

    /// Index the private corpus; forces isolation-safe paths on `opts`.
    pub fn index_all(&self, opts: IndexOptions) -> Indexer {
        let mut indexer = self.indexer(opts);
        indexer.index_all().expect("isolated index_all");
        indexer
    }

    /// Build a [`Searcher`] against this session's real on-disk index.
    pub fn searcher(&self, mut opts: SearchOptions) -> Searcher {
        opts.root = self.corpus_root.clone();
        opts.index_path = Some(self.index_path.clone());
        Searcher::new(opts).expect("isolated searcher")
    }
}

impl Default for IsolatedIndexSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a private corpus + real on-disk SQLite session (cleaned on drop).
pub fn isolated_index_session() -> IsolatedIndexSession {
    IsolatedIndexSession::new()
}

/// Run `f` with a private real-SQLite session; temp tree cleaned when `f` returns.
pub fn with_temp_index<R>(f: impl FnOnce(&IsolatedIndexSession) -> R) -> R {
    let session = IsolatedIndexSession::new();
    f(&session)
}

