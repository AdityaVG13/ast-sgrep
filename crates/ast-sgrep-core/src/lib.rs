use thiserror::Error; pub mod bench_suite; pub mod chain; pub mod gitignore; pub mod index; pub mod intent; pub mod pattern; pub mod pipeline_parts; pub mod query; pub mod rank; pub mod search; pub mod semantic_ann; pub mod semantic_chunk;
pub mod semantic_ivf; pub mod store; pub mod tantivy_index;
pub mod fts {
    pub fn escape_fts_term(term: &str) -> String { format!("\"{}\"", term.replace('"', "\"\"")) } pub fn escape_fts_query(terms: &[String]) -> String { terms.iter().map(|t| escape_fts_term(t)).collect::<Vec<_>>().join(" OR ") }
} pub mod skip {
    pub use crate::gitignore:: { should_skip_dir, should_skip_file, DEFAULT_SKIP_DIR_NAMES, INDEXABLE_EXTENSIONS, };
} pub mod text { pub use crate::index::{split_content_lines, SplitLines}; } pub mod output {
    pub use crate::search::format_hit_line;
}
pub use index::{EmbedBackend, FileIndexStats, IndexOptions, IndexStats, Indexer}; pub use output::format_hit_line; pub use pattern::search_pattern; pub use query::{ParsedQuery, QueryMode};
pub use search::{SearchHit, SearchOptions, SearchResponse, Searcher}; pub use store::{index_db_path, IndexStatus, IndexStore};
#[derive(Debug, Error)] pub enum StoreError {
    #[error("database error: {0}")] Database(#[from] rusqlite::Error), #[error("io error: {0}")] Io(#[from] std::io::Error), #[error("{0}")] Other(String),
} impl From<String> for StoreError {
    fn from(s: String) -> Self { StoreError::Other(s) }
} pub type Result<T> = std::result::Result<T, StoreError>;
