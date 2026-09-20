#![forbid(unsafe_code)]

//! Shared integration-test harness.
//!
//! The `lsp` feature (E17 B3) is the only path that prod-depends
//! `ast-sgrep-lsp`. Default Bill is core + lang only.

mod cli;
mod cli_oracle;
mod cli_recovery;
#[cfg(feature = "codemode")]
mod codemode;
#[cfg(feature = "codemode")]
mod codemode_invalidation;
#[cfg(feature = "codemode")]
mod codemode_recovery;
mod core_invalidation;
mod core_oracle;
mod core_recovery;
mod fault;
mod fixture;
mod golden;
mod hit;
mod index;
mod isolation;
mod lang;
mod lang_pipeline;
#[cfg(feature = "lsp")]
mod lsp;
mod mcp;
mod num;
mod scrub;
mod verdict;
pub use cli::{
    asgrep_bin, assert_failure_envelope, assert_finite_unit, assert_fixture_hits,
    assert_human_error, assert_human_success, assert_no_success_shape, assert_operational_envelope,
    assert_success, assert_usage_envelope, envelope_shape, eval_project, f64_at, fixture_root,
    indexed_project, indexed_project_n, parse_human_row, parse_human_summary, parse_stdout, run,
    run_env, run_eval_ok, run_eval_raw, run_index_default, run_index_json_noembed, run_json,
    run_json_full, run_search_json, usage_message, write_fixture, write_gold, CliSession,
};
pub use cli_oracle::{
    oracle_build_json, oracle_keyword_corpus, oracle_outline_corpus, oracle_run_json,
    oracle_run_raw, oracle_search_corpus, run_drain_timeout, sorted_surface_keys, write_root_bytes,
    OracleCorpus,
};
#[cfg(unix)]
pub use cli_recovery::kill9;
pub use cli_recovery::{
    assert_doctor_unhealthy, assert_serve_parity, capture_baseline, run_in, run_index,
    run_outline_snapshot, run_reindex, run_search, run_status, run_timeout, search_answer_keys,
    seed_big_project, seed_project, status_snapshot, KillOnDrop, ServeBaseline, OUTLINE_PATH,
    QUERY, RUN_TIMEOUT, SOURCE,
};
#[cfg(feature = "codemode")]
pub use codemode::{
    assert_json_byte_identical, assert_other_preserves_cause, batch_call, batch_request,
    batch_request_with_mode, call_error_discriminant, catalog_call, config_at, config_at_indexed,
    hits_fixture, indexed_session_at, sample_search_hit, scored_hits5, scored_hits6,
    search_select_plan, serve_lines, serve_request_line, session_at, session_at_indexed,
    BUDGET_EXCEEDED, CALL_NOW_ONLY, DEFS_NEEDS_SYMBOL, MAX_QUERY_CHARS, SESSION_BUSY,
};
#[cfg(feature = "codemode")]
pub use codemode_invalidation::{
    external_reindex, find_limit8, hit_bytes, hit_count, hit_file_set, hit_files, hit_name_set,
    hits_file, index_dir_db_path, search_limit8, seeded_py_repo, session_at_limit8,
    status_file_count, status_writer_generation, targeted_refresh, write_py, SEEDED_ALPHA_TOKEN,
};
#[cfg(feature = "codemode")]
pub use codemode_recovery::{
    db_user_version, indexed_codemode_repo, indexed_repo_files, load_batch_file, load_plan_file,
    remove_db_with_sidecars, serve_transcript_indexed, set_db_user_version, status_counts,
    twin_repos,
};
pub use core_invalidation::{hermetic_indexer, HitTuple, InvalidationFixture};
pub use core_oracle::{
    build_core_index, chain_store, core_index_options, core_search_options, core_searcher,
    crossover_corpus, finish_options, fused_keys, hit_files_in_order, mixed_hits, pair_examples,
    parsed_query, rank_fused, response_hit_keys_with_scores, route_fuse_pipeline, searcher_at_root,
    set_rank, set_weight, single_rank, sorted_hit_files, tie_corpus, unit_channel_weights,
    write_core_fixture, CorePipelineFixture,
};
pub use core_recovery::{
    assert_torn, build_and_quiet, corpus_session, home_names, quarantine_path, quiesced_db_bytes,
    search_parity_key, search_parity_keys, store_snapshot, upsert_test_file, RECOVERY_CORPUS,
};
pub use fault::{
    corrupt_db_total, err_of, flip_bytes, is_corrupt_kind, remove_sqlite_sidecars, sqlite_code,
    store_error_discriminant, truncate_file, write_garbage,
};
pub use fixture::{
    dir_listing, file_tree, index_db_path, sample_file, sample_root, set_mtime_secs, write_file,
    write_temp,
};
pub use golden::{
    assert_golden, assert_golden_at, assert_golden_json, assert_golden_json_at,
    canonicalize_chain_response, canonicalize_extraction, canonicalize_text, golden_text_pretty,
    updating_goldens,
};
pub use hit::{fused_key, hit_key_bits, hit_keys, mk_hit, sorted_contributors, HitKey};
pub use index::{
    core_search_hit_keys, index_sample, json_hit_keys, reopen_indexer, response_hit_keys,
    searcher_from, HitKey as SurfaceHitKey, IndexedFixture,
};
pub use isolation::{isolated_index_session, with_temp_index, IsolatedIndexSession};
pub use lang::{
    assert_has_callee, assert_has_symbol, assert_language_conformance, match_lines, parse,
    run_in_fresh_thread, ExpectedCall, ExpectedPattern, ExpectedSymbol, LanguageConformanceCase,
};
pub use lang_pipeline::{
    assert_rank_sound, assert_spans_in_source, rank_hits, run_pipeline, score_hit, PipelineOutcome,
};
#[cfg(feature = "lsp")]
pub use lsp::{
    edit_full_replace, edit_ranged, edit_ranged_len, lsp_search_hit_keys, sample_backend,
};
pub use mcp::{
    assert_hit_envelope, assert_jsonrpc_error, assert_miss_envelope, assert_ping_ok,
    assert_tool_error_shape, assert_tool_success, assert_tool_success_shape, assert_tools_list_ok,
    big_tree, cancelled_notif, collect_responses, corrupt_index_db, distinct_hit_paths,
    expected_tool_names, index_tree, indexed_tree, init_payload, initialized_notif, is_error,
    mcp_bin, multi_hit_tree, ping, response_by_id, rpc_at, rpc_pipeline, rpc_session,
    rpc_session_env, rpc_session_raw, search_call, small_tree, snippet_bytes,
    spawn_raw_no_handshake, tool_body, tool_call, tool_error_discriminant, tool_text, tools_list,
    CallSession, LiveSession, TESTKIT_CLIENT_NAME,
};
#[cfg(feature = "plugins")]
pub use mcp::{assert_hit_path_set, hit_path_set};
pub use num::{approx_eq, chunk_row, fold_rank_scored, lcg_vec, ranked_indices};
pub use scrub::Scrubber;
pub use verdict::TestVerdict;
