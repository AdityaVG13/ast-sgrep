use thiserror::Error;

pub mod index;
pub mod output;
pub mod query;
pub mod rank;
pub mod search;
pub mod store;

pub use index::{IndexOptions, IndexStats, Indexer};
pub use output::format_hit_line;
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
