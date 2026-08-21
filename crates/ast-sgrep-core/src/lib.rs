#![forbid(unsafe_code)]

//! L1 search kernel. L2/L3 consumers should prefer crate-root reexports
//! (`HitKind`, `SearchHit`, `Durability`, `SymbolRow`, `hit_why`, and the
//! other `pub use` items below). Intentional module paths remain: `search`,
//! `store`, `query`, `intent`, `chain`, `call_path`, `env_flag`, `semantic_ann`, `pattern`.
//! Do not split this crate to lower fan_in.

use thiserror::Error;
pub mod bench_suite;
pub mod call_path;
pub mod chain;
pub mod codemod;
pub mod env_flag;
pub mod fusion;
pub mod gitignore;
pub mod index;
mod index_prepare;
mod index_recovery;
mod index_watch;
pub mod intent;
pub mod io_bounds;
pub mod limits;
pub mod pattern;
pub mod perf_profile;
pub mod pipeline_parts;
pub mod query;
pub mod rank;
pub mod search;
pub mod semantic_ann;
pub mod semantic_chunk;
pub mod semantic_ivf;
pub mod store;
pub mod tantivy_index;
/// Compatibility re-exports for callers using the pre-1.3 module paths.
pub mod skip {
    pub use crate::gitignore::{
        should_skip_dir, should_skip_file, DEFAULT_SKIP_DIR_NAMES, INDEXABLE_EXTENSIONS,
    };
}

pub mod text {
    pub use crate::index::{split_content_lines, SplitLines};
}

pub mod output {
    pub use crate::search::format_hit_line;
}

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
pub use index::{
    canonicalize_affected_path, indexed_rel_path, EmbedBackend, FileIndexStats, IndexOptions,
    IndexStats, Indexer, INDEX_CANCELLED, MAX_INCREMENTAL_PATHS,
};
pub use io_bounds::{read_text_capped, MAX_INDEX_FILE_BYTES};
pub use limits::{
    clamp_agent_limit, clamp_output_limit, validate_query_len, DEFAULT_AGENT_LIMIT,
    MAX_EXCERPT_LINES, MAX_FILE_FILTER_CHARS, MAX_OUTPUT_RESULTS, MAX_QUERY_CHARS,
    MAX_REGEX_PATTERN_CHARS, MAX_SEARCH_HIT_EXCERPT_BYTES, MAX_STDIN_LINE_BYTES,
};
pub mod lexicon;
pub mod resolution;
pub mod scip;
pub use pattern::{run_external_ast_grep, search_pattern, ExternalAstGrepMatch};
pub use query::{ParsedQuery, QueryMode};
pub use search::{
    follow_ups_for_hit, format_hit_line, hit_why, margin_is_decisive, plan_suggested_next,
    CriticNote, EmbedFieldScores, HitKind, HitSignal, SearchHit, SearchOptions, SearchResponse,
    Searcher,
};
pub use store::{
    bump_writer_generation, index_db_path, read_writer_generation, try_index_db_path,
    writer_generation_path, Durability, IndexStatus, IndexStore, SymbolRow, WRITER_GENERATION_FILE,
};
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}
impl StoreError {
    pub(crate) fn is_corrupt_database(&self) -> bool {
        matches!(
            self,
            Self::Database(rusqlite::Error::SqliteFailure(error, _))
                if matches!(
                    error.code,
                    rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
                )
        )
    }

    pub(crate) fn is_binary_file(&self) -> bool {
        matches!(self, Self::Other(message) if message.starts_with("binary file: "))
    }
}
impl From<String> for StoreError {
    fn from(s: String) -> Self {
        StoreError::Other(s)
    }
}
pub type Result<T> = std::result::Result<T, StoreError>;
