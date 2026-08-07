mod embed_support;
mod module_resolve;
pub(crate) mod sql;
mod sqlite;
pub use sql::integrity_check;
pub use sql::{
    assert_sql_ident, CALLER_COLUMN_ALLOWLIST, COUNT_TABLE_ALLOWLIST, FILE_CHILD_TABLE_ALLOWLIST,
};
pub use sqlite::{
    CallerRow, ImportRow, IndexStore, IndexedLineRow, RefreshLinesInput, SymbolLocationRow,
    SymbolRow, UpsertFileInput,
};
use std::path::{Path, PathBuf};

/// Write-durability profile for the index database (0obi).
///
/// Indexing used to force `PRAGMA synchronous = OFF` for every bulk and
/// per-file transaction. SQLite documents that this permits write reordering
/// and can corrupt the database after a power failure or hard reset. A search
/// index is rebuildable, but corruption still produces failed searches, silent
/// stale results, broken editor navigation, and forced full rebuilds -- so the
/// unsafe mode is now an explicit opt-in rather than the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Durability {
    /// `WAL` + `synchronous = FULL`. Survives power loss; slowest to index.
    Strict,
    /// `WAL` + `synchronous = NORMAL`. Survives process crash. Default.
    #[default]
    Balanced,
    /// `synchronous = OFF`. Fastest, and can corrupt the index on power loss.
    FastUnsafe,
}

impl Durability {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(Self::Strict),
            "balanced" | "default" => Some(Self::Balanced),
            "fast-unsafe" | "fast_unsafe" | "unsafe" => Some(Self::FastUnsafe),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Balanced => "balanced",
            Self::FastUnsafe => "fast-unsafe",
        }
    }

    /// `PRAGMA synchronous` value held outside write transactions.
    pub fn steady_pragma(self) -> &'static str {
        match self {
            Self::Strict => "FULL",
            // A crashed process must not leave `OFF` behind, so even the unsafe
            // profile returns to NORMAL between write batches.
            Self::Balanced | Self::FastUnsafe => "NORMAL",
        }
    }

    /// `PRAGMA synchronous` value used inside bulk and per-file write batches.
    pub fn write_pragma(self) -> &'static str {
        match self {
            Self::Strict => "FULL",
            Self::Balanced => "NORMAL",
            Self::FastUnsafe => "OFF",
        }
    }

    /// Resolve from `ASGREP_DURABILITY`, falling back to the safe default.
    /// An unrecognized value must not silently downgrade durability.
    pub fn from_env() -> Self {
        std::env::var("ASGREP_DURABILITY")
            .ok()
            .and_then(|value| Self::parse(&value))
            .unwrap_or_default()
    }
}

pub const INDEX_DIR: &str = ".asgrep";
pub const INDEX_DB: &str = "index.db";
fn as_db_path(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e == "db") {
        path
    } else {
        path.join(INDEX_DB)
    }
}
pub fn index_db_path(root: &Path, index_path: Option<&Path>) -> PathBuf {
    try_index_db_path(root, index_path)
        .unwrap_or_else(|_| root.join(INDEX_DIR).join(INDEX_DB))
}

pub fn try_index_db_path(root: &Path, index_path: Option<&Path>) -> crate::Result<PathBuf> {
    if let Some(path) = index_path {
        return Ok(as_db_path(path.to_path_buf()));
    }
    if let Ok(env_path) = std::env::var("ASGREP_INDEX_PATH") {
        return Ok(as_db_path(PathBuf::from(env_path)));
    }
    let local = root.join(INDEX_DIR).join(INDEX_DB);
    if local.exists() {
        return Ok(local);
    }
    if std::env::var("ASGREP_USE_CACHE")
        .ok()
        .as_deref()
        .is_some_and(crate::env_flag::is_boolish_true)
    {
        return cache_index_path(root);
    }
    Ok(local)
}
/// Resolve a private cache index path. Refuses shared `/tmp` when HOME/XDG_CACHE_HOME unset (i5ef).
fn cache_index_path(root: &Path) -> crate::Result<PathBuf> {
    let hash = blake3::hash(root.to_string_lossy().as_bytes());
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })
        .ok_or_else(|| {
            crate::StoreError::Other(
                "ASGREP_USE_CACHE requires HOME or XDG_CACHE_HOME; refusing shared /tmp fallback"
                    .into(),
            )
        })?;
    Ok(base
        .join("asgrep")
        .join(hash.to_hex().to_string())
        .join(INDEX_DB))
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexStatus {
    pub root: String,
    pub index_path: String,
    pub file_count: usize,
    pub line_count: usize,
    pub symbol_count: usize,
    pub caller_count: usize,
    pub import_count: usize,
    pub semantic_chunk_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_dim: Option<usize>,
    pub embed_cache_entries: usize,
    pub embed_cache_capacity: usize,
    pub embed_cache_hits: u64,
    pub embed_cache_misses: u64,
    pub semantic_ivf_present: bool,
    /// Active write-durability profile (0obi).
    pub durability: String,
}
