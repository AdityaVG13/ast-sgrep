#![forbid(unsafe_code)]

//! Shared integration-test harness.
//!
//! The `lsp` feature (E17 B3) is the only path that prod-depends
//! `ast-sgrep-lsp`. Default Bill is core + lang only.

mod cli;
mod fixture;
mod golden;
mod hit;
mod index;
mod isolation;
mod lang;
#[cfg(feature = "lsp")]
mod lsp;
mod scrub;
mod verdict;
pub use cli::CliSession;
pub use fixture::{sample_file, sample_root};
pub use golden::{
    assert_golden, assert_golden_at, assert_golden_json, assert_golden_json_at,
    canonicalize_chain_response, canonicalize_extraction, canonicalize_text, updating_goldens,
};
pub use hit::{hit_keys, HitKey};
pub use index::{
    core_search_hit_keys, index_sample, json_hit_keys, reopen_indexer, response_hit_keys,
    searcher_from, HitKey as SurfaceHitKey, IndexedFixture,
};
pub use isolation::{isolated_index_session, with_temp_index, IsolatedIndexSession};
pub use lang::{
    assert_has_callee, assert_has_symbol, assert_language_conformance, parse, ExpectedCall,
    ExpectedPattern, ExpectedSymbol, LanguageConformanceCase,
};
#[cfg(feature = "lsp")]
pub use lsp::{lsp_search_hit_keys, sample_backend};
pub use scrub::Scrubber;
pub use verdict::TestVerdict;
