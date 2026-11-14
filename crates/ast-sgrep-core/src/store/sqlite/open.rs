use super::super::embed_support::init_cache_seq;
use super::super::sql::configure_connection_with;
use super::super::try_index_db_path;
use super::IndexStore;
use crate::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

impl IndexStore {
    pub fn open(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        Self::open_with_durability(root, index_path, crate::store::Durability::from_env())
    }

    /// Open with an explicit durability profile.
    pub fn open_with_durability(
        root: &Path,
        index_path: Option<&Path>,
        durability: crate::store::Durability,
    ) -> Result<Self> {
        Self::open_inner(root, index_path, durability, false)
    }

    /// Search/status/doctor: SQLITE_OPEN_READ_ONLY, no schema migration, no WAL.
    pub fn open_readonly(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        Self::open_inner(root, index_path, crate::store::Durability::from_env(), true)
    }

    /// An EMPTY in-memory store standing in for an index bound to a
    /// different project root. Same schema, zero rows, so
    /// every serving lane (pattern signatures, candidate narrowing, literal
    /// line corpus, embeddings) degrades through its existing empty-index
    /// gates and the native walk alone answers from the query root — the
    /// foreign tree's rows can never be mixed into results. Nothing is
    /// persisted; `:memory:` has no sidecar surface, and the version-0
    /// legacy migration's semantic-IVF invalidation is an explicit no-op
    /// for in-memory targets instead of degenerating to a CWD-relative
    /// `semantic.ivf` deletion (data loss).
    pub fn open_in_memory(root: &Path) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let durability = crate::store::Durability::from_env();
        configure_connection_with(&conn, durability)?;
        Self::from_configured_connection(conn, root, PathBuf::from(":memory:"), durability, false)
    }

    /// Peek `PRAGMA user_version` without creating or migrating the database.
    pub fn peek_schema_version(root: &Path, index_path: Option<&Path>) -> Result<i64> {
        let db_path = try_index_db_path(root, index_path).map_err(|e| {
            crate::StoreError::Other(format!(
                "failed to resolve index path for root {}: {e}",
                root.display()
            ))
        })?;
        if !db_path.is_file() {
            return Err(crate::StoreError::Other(format!(
                "index does not exist at {} (root {}); run: asgrep index {} --json",
                db_path.display(),
                root.display(),
                root.display()
            )));
        }
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(Into::into)
    }

    fn open_inner(
        root: &Path,
        index_path: Option<&Path>,
        durability: crate::store::Durability,
        read_only: bool,
    ) -> Result<Self> {
        let db_path = try_index_db_path(root, index_path).map_err(|e| {
            crate::StoreError::Other(format!(
                "failed to resolve index path for root {}: {e}",
                root.display()
            ))
        })?;
        if read_only {
            if !db_path.is_file() {
                return Err(missing_index_error(root, &db_path));
            }
        } else {
            // Default layout, writable: never grow a second `.asgrep` inside a
            // checkout that already owns one (the root's own index is fine).
            if index_path.is_none()
                && std::env::var_os("ASGREP_INDEX_PATH").is_none()
                && !db_path.is_file()
            {
                if let Some(enclosing) = enclosing_index_db(root) {
                    return Err(second_index_error(root, &db_path, &enclosing));
                }
            }
            if let Some(p) = db_path.parent() {
                std::fs::create_dir_all(p).map_err(|e| {
                    crate::StoreError::Other(format!(
                        "failed to create index directory {} (root {}): {e}",
                        p.display(),
                        root.display()
                    ))
                })?;
            }
        }
        // Preserve rusqlite's error code so explicit reindex can distinguish a
        // corrupt/non-database file from permission, locking, and IO failures.
        let t_conn = crate::env_flag::profile_clock();
        let conn = if read_only {
            Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?
        } else {
            Connection::open(&db_path)?
        };
        crate::env_flag::emit_profile_stage_since("sqlite_conn_open", t_conn);
        let t_config = crate::env_flag::profile_clock();
        if read_only {
            super::super::sql::configure_connection_readonly(&conn)?;
        } else {
            configure_connection_with(&conn, durability)?;
        }
        crate::env_flag::emit_profile_stage_since("sqlite_configure", t_config);
        Self::from_configured_connection(conn, root, db_path, durability, read_only)
    }

    /// Disk and in-memory stores start with identical transaction/cache state.
    /// Their callers retain ownership of connection configuration and path policy.
    fn from_configured_connection(
        conn: Connection,
        root: &Path,
        db_path: PathBuf,
        durability: crate::store::Durability,
        read_only: bool,
    ) -> Result<Self> {
        let store = Self {
            conn,
            root: root.to_path_buf(),
            db_path,
            file_tx_depth: std::cell::Cell::new(0),
            file_tx_owns: std::cell::Cell::new(false),
            file_tx_poisoned: std::cell::Cell::new(false),
            bulk_tx_active: std::cell::Cell::new(false),
            bulk_tx_owns: std::cell::Cell::new(false),
            cache_seq: std::cell::Cell::new(0),
            durability,
            line_corpus: std::cell::RefCell::new(None),
            bundle: std::cell::RefCell::new(None),
            bundle_file_stamp: std::cell::Cell::new(None),
            line_corpus_disabled: std::cell::Cell::new(false),
            build_trigram_index: std::cell::Cell::new(true),
            trigram_populated: std::cell::Cell::new(None),
            names_sidecar: std::cell::RefCell::new(None),
            lazy_ivf: std::cell::RefCell::new(None),
            read_only,
        };
        let t_schema = crate::env_flag::profile_clock();
        store.init_schema()?;
        crate::env_flag::emit_profile_stage_since("sqlite_schema_probes", t_schema);
        let t_seq = crate::env_flag::profile_clock();
        init_cache_seq(&store.conn, &store.cache_seq)?;
        crate::env_flag::emit_profile_stage_since("sqlite_cache_seq", t_seq);
        Ok(store)
    }
}

/// Nearest ancestor (never the root itself) holding the default-layout index,
/// searched only up to and including the git work tree root: a parent that
/// merely happens to hold an index (a home directory, a shared scratch tree) is
/// not this project's checkout and must not capture a new project. The home
/// directory and anything above it never capture, git or no git — a git-less
/// tree (benchmark corpus, tarball extract) below an indexed home is still
/// its own project.
/// Silent when an env override moves indexes elsewhere.
fn enclosing_index_db(root: &Path) -> Option<PathBuf> {
    if std::env::var_os("ASGREP_INDEX_PATH").is_some()
        || std::env::var_os("ASGREP_USE_CACHE").is_some()
    {
        return None;
    }
    let start = if root.is_file() { root.parent()? } else { root };
    let work_tree = start.ancestors().find(|dir| dir.join(".git").exists());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    start
        .ancestors()
        .skip(1)
        .take_while(|dir| work_tree.is_none_or(|tree| dir.starts_with(tree)))
        .take_while(|dir| home.as_deref().is_none_or(|h| !h.starts_with(dir)))
        .map(|dir| dir.join(super::super::INDEX_DIR).join(super::super::INDEX_DB))
        .find(|candidate| candidate.is_file())
}

/// No index file at this root. Say which file is missing and, when the root sits
/// inside an indexed checkout, name that index: a nested directory answering
/// "index is empty" reads as a broken index instead of a missing one. Keeps the
/// documented `index is empty` fail-closed phrase (docs/validation/negative-ledgers.md).
fn missing_index_error(root: &Path, db_path: &Path) -> crate::StoreError {
    if let Some(enclosing) = enclosing_index_db(root) {
        return crate::StoreError::EmptyIndex(format!(
            "index is empty for {}: no index file at {}; this checkout is indexed at {} -- run the command at that root, pass --index-path for a separate index, or rerun with --auto-index to index this checkout in place",
            root.display(),
            db_path.display(),
            enclosing.display()
        ));
    }
    crate::StoreError::EmptyIndex(format!(
        "index is empty for {}: no index file at {}; run: asgrep index {} --json, or rerun with --auto-index to index this checkout in place",
        root.display(),
        db_path.display(),
        root.display()
    ))
}

/// Writable open with the default layout inside an indexed checkout: refuse to
/// create a second `.asgrep` under the tree. The caller either works at the
/// checkout root or asks for a deliberate separate index path.
fn second_index_error(root: &Path, db_path: &Path, enclosing: &Path) -> crate::StoreError {
    let enclosing_root = enclosing
        .parent()
        .and_then(|dir| dir.parent())
        .unwrap_or(enclosing);
    crate::StoreError::Other(format!(
        "refusing to create a second index at {} (root {}): the enclosing checkout is already indexed at {} -- run: asgrep index {} --json, or pass --index-path for a separate index",
        db_path.display(),
        root.display(),
        enclosing.display(),
        enclosing_root.display()
    ))
}