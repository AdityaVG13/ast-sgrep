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
    root.join(INDEX_DIR).join(INDEX_DB)
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
