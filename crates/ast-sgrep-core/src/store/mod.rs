mod embed_support;
pub(crate) mod pragmas; pub(crate) mod sql; mod sqlite; pub use pragmas::integrity_check; pub use sqlite::{
    CallerRow, ImportRow, IndexStore, IndexedLineRow, SymbolLocationRow, SymbolRow, UpsertFileInput,
}; use std::path::{Path, PathBuf};

pub const INDEX_DIR: &str = ".asgrep"; pub const INDEX_DB: &str = "index.db";

fn as_db_path(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e == "db") { path } else { path.join(INDEX_DB) }
} pub fn index_db_path(root: &Path, index_path: Option<&Path>) -> PathBuf {
    if let Some(path) = index_path { return as_db_path(path.to_path_buf()); } if let Ok(env_path) = std::env::var("ASGREP_INDEX_PATH") { return as_db_path(PathBuf::from(env_path)); } let local = root.join(INDEX_DIR).join(INDEX_DB);
    if local.exists() { return local; } if std::env::var("ASGREP_USE_CACHE").ok().as_deref() == Some("1") { return cache_index_path(root); } local
} fn cache_index_path(root: &Path) -> PathBuf {
    let hash = blake3::hash(root.to_string_lossy().as_bytes()); let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".cache/asgrep").join(hash.to_hex().to_string()).join(INDEX_DB)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] pub struct IndexStatus {
    pub root: String, pub index_path: String, pub file_count: usize, pub line_count: usize, pub symbol_count: usize, pub caller_count: usize, pub import_count: usize, pub semantic_chunk_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")] pub embed_backend: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] pub embed_dim: Option<usize>, pub embed_cache_entries: usize,
    pub embed_cache_capacity: usize, pub embed_cache_hits: u64, pub embed_cache_misses: u64, pub semantic_ivf_present: bool,
}
