#![forbid(unsafe_code)]

use thiserror::Error;
pub mod bench_suite;
pub mod chain;
pub mod env_flag;
pub mod fusion;
pub mod gitignore;
pub mod index;
pub mod intent;
pub mod io_bounds;
pub mod limits;
pub mod pattern;
pub mod pipeline_parts;
pub mod query;
pub mod rank;
pub mod search;
pub mod semantic_ann;
pub mod semantic_chunk;
pub mod semantic_ivf;
pub mod store;
pub mod tantivy_index;
pub mod fts {
    pub fn escape_fts_term(term: &str) -> String {
        format!("\"{}\"", term.replace('"', "\"\""))
    }
    pub fn escape_fts_query(terms: &[String]) -> String {
        terms
            .iter()
            .map(|t| escape_fts_term(t))
            .collect::<Vec<_>>()
            .join(" OR ")
    }
}
pub use fusion::{
    analyze_weight_sensitivity, learn_fusion_weights, ChannelRanks, FusionCandidate, FusionChannel,
    FusionExample, LearnedFusionModel, WeightSensitivity,
};
pub use index::{EmbedBackend, FileIndexStats, IndexOptions, IndexStats, Indexer};
pub use io_bounds::{read_text_capped, MAX_INDEX_FILE_BYTES};
pub use limits::{
    clamp_agent_limit, clamp_output_limit, DEFAULT_AGENT_LIMIT, MAX_EXCERPT_LINES,
    MAX_OUTPUT_RESULTS,
};
pub use search::format_hit_line;
pub use pattern::search_pattern;
pub use query::{ParsedQuery, QueryMode};
pub use search::{HitSignal, SearchHit, SearchOptions, SearchResponse, Searcher};
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
impl From<String> for StoreError {
    fn from(s: String) -> Self {
        StoreError::Other(s)
    }
}
pub type Result<T> = std::result::Result<T, StoreError>;
