#![forbid(unsafe_code)]

//! Shared integration-test harness.
//!
//! The `lsp` feature (E17 B3) is the only path that prod-depends
//! `ast-sgrep-lsp`. Default Bill is core + lang only.

mod cli;
#[cfg(feature = "codemode")]
mod codemode;
mod fault;
mod fixture;
mod golden;
mod hit;
mod index;
mod isolation;
mod lang;
#[cfg(feature = "lsp")]
mod lsp;
mod mcp;
mod num;
mod scrub;
mod verdict;
pub use cli::{
    asgrep_bin, assert_failure_envelope, assert_success, parse_stdout, run, run_env, run_json,
    CliSession,
};
#[cfg(feature = "codemode")]
pub use codemode::{
    assert_json_byte_identical, batch_call, batch_request, catalog_call, config_at,
    config_at_indexed, serve_lines, serve_request_line, session_at, session_at_indexed,
    BUDGET_EXCEEDED, CALL_NOW_ONLY, DEFS_NEEDS_SYMBOL, MAX_QUERY_CHARS, SESSION_BUSY,
};
pub use fault::{err_of, flip_bytes, remove_sqlite_sidecars, truncate_file, write_garbage};
pub use fixture::{file_tree, sample_file, sample_root, set_mtime_secs, write_file};
pub use golden::{
    assert_golden, assert_golden_at, assert_golden_json, assert_golden_json_at,
    canonicalize_chain_response, canonicalize_extraction, canonicalize_text, updating_goldens,
};
pub use hit::{hit_keys, mk_hit, HitKey};
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
pub use mcp::{
    assert_ping_ok, assert_tool_error_shape, assert_tool_success, assert_tools_list_ok,
    big_tree, cancelled_notif, collect_responses, corrupt_index_db, expected_tool_names,
    index_tree, indexed_tree, init_payload, initialized_notif, is_error, mcp_bin, ping,
    response_by_id, rpc_pipeline, rpc_session, rpc_session_env, small_tree, tool_body,
    tool_call, tool_text, tools_list, LiveSession, TESTKIT_CLIENT_NAME,
};
pub use num::approx_eq;
pub use scrub::Scrubber;
pub use verdict::TestVerdict;
