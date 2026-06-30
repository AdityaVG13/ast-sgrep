//! Shared helpers for ast-sgrep integration and regression tests.
//!
//! Use this crate as a dev-dependency from integration test targets. Typical flow:
//!
//! ```ignore
//! use ast_sgrep_testkit::{index_sample, searcher_from, assert_query_finds};
//! use ast_sgrep_core::{IndexOptions, SearchOptions};
//!
//! let indexed = index_sample(IndexOptions::default());
//! let searcher = searcher_from(&indexed, SearchOptions::default());
//! assert_query_finds(&searcher, "auth_refresh", |h| h.symbol.as_deref() == Some("auth_refresh"));
//! ```
//!
//! Modules:
//! - [`fixture`] — sample repo paths and file contents
//! - [`index`] — index the sample fixture, build searchers
//! - [`search`] — assertion helpers for query results
//! - [`repo`] — ephemeral temp repos for gitignore / hardening tests
//! - [`lang`] — parse inline snippets, symbol/callee/import assertions
//! - [`lsp`] — indexed LSP backend for protocol tests
//! - [`plugins`] — canned search/embed JSON for plugin unit tests

mod fixture;
mod index;
mod lang;
mod lsp;
mod plugins;
mod repo;
mod search;

pub use fixture::{sample_file, sample_path, sample_root};
pub use index::{hybrid_searcher, index_sample, semantic_searcher, searcher_from, IndexedFixture};
pub use lang::{
    assert_has_callee, assert_has_import, assert_has_symbol, assert_no_callee, parse,
    run_false_positive_case,
};
pub use lsp::sample_backend;
pub use plugins::{sample_embed_response, sample_search_response};
pub use repo::{index_repo, temp_repo};
pub use search::{
    assert_excerpt_contains, assert_has_embed_hits, assert_no_embed_hits, assert_query_finds,
    top_symbols,
};
