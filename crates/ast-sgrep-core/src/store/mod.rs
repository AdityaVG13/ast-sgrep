mod sqlite;

pub use sqlite::{CallerRow, ImportRow, IndexStore, SymbolRow};

use std::path::Path;

/// Default index directory name inside a project root.
pub const INDEX_DIR: &str = ".asgrep";

/// Default SQLite database filename.
pub const INDEX_DB: &str = "index.db";

/// Resolve the index database path for a project root.
pub fn index_db_path(root: &Path, index_path: Option<&Path>) -> std::path::PathBuf {
    if let Some(path) = index_path {
        if path.extension().is_some_and(|e| e == "db") {
            return path.to_path_buf();
        }
        return path.join(INDEX_DB);
    }
    if let Ok(env_path) = std::env::var("ASGREP_INDEX_PATH") {
        let p = std::path::PathBuf::from(env_path);
        if p.extension().is_some_and(|e| e == "db") {
            return p;
        }
        return p.join(INDEX_DB);
    }
    let local = root.join(INDEX_DIR).join(INDEX_DB);
    if local.exists() {
        return local;
    }
    if std::env::var("ASGREP_USE_CACHE").ok().as_deref() == Some("1") {
        return cache_index_path(root);
    }
    local
}

fn cache_index_path(root: &Path) -> std::path::PathBuf {
    let hash = blake3::hash(root.to_string_lossy().as_bytes());
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home)
        .join(".cache")
        .join("asgrep")
        .join(hash.to_hex().to_string())
        .join(INDEX_DB)
}

/// Index status summary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexStatus {
    pub root: String,
    pub index_path: String,
    pub file_count: usize,
    pub line_count: usize,
    pub symbol_count: usize,
    pub caller_count: usize,
    pub import_count: usize,
}
