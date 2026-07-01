use thiserror::Error;

pub mod bench_suite;
pub mod fts;
pub mod gitignore;
pub mod index;
pub mod output;
pub mod pattern;
pub mod query;
pub mod rank;
pub mod search;
pub mod semantic_ann;
pub mod semantic_chunk;
pub mod semantic_ivf;
pub mod skip;
pub mod store;
pub mod tantivy_index;
pub mod text;

pub use index::{EmbedBackend, FileIndexStats, IndexOptions, IndexStats, Indexer};
pub use output::format_hit_line;
pub use pattern::search_pattern;
pub use query::{ParsedQuery, QueryMode};
pub use search::{SearchHit, SearchOptions, SearchResponse, Searcher};
pub use store::{index_db_path, IndexStatus, IndexStore};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;
