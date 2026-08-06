mod embed_support;
mod module_resolve;
pub(crate) mod sql;
mod sqlite;
pub use sql::integrity_check;
pub use sqlite::{
    CallerRow, ImportRow, IndexStore, IndexedLineRow, RefreshLinesInput, SymbolLocationRow,
    SymbolRow, UpsertFileInput,
};
use std::path::{Path, PathBuf};
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
}
