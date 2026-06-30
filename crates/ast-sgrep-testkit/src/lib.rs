//! Shared helpers for ast-sgrep integration tests.

mod cli;
mod fixture;
mod index;
mod lang;
mod lsp;
mod plugins;
mod repo;
mod search;

pub use cli::{run_cli, CliSession};
pub use fixture::{sample_file, sample_path, sample_root};
pub use index::{
    hybrid_searcher, index_options_from, index_sample, reopen_indexer, semantic_searcher,
    searcher_from, IndexedFixture,
};
pub use lang::{
    assert_has_callee, assert_has_import, assert_has_symbol, assert_no_callee, parse,
    run_false_positive_case,
};
pub use lsp::sample_backend;
pub use plugins::{sample_embed_response, sample_search_response};
pub use repo::{index_repo, temp_repo};
pub use search::{
    assert_excerpt_contains, assert_has_embed_hits, assert_no_embed_hits, assert_query_finds,
    assert_semantic_finds, top_symbols,
};
