#![forbid(unsafe_code)]

mod cli;
mod factory;
mod fixture;
mod hit;
mod index;
mod isolation;
mod lang;
mod lsp;
mod safety;
mod test_log;
pub use cli::CliSession;
pub use factory::{
    count_files_under, factory_corpus_basic_graph, factory_corpus_credential_theme,
    factory_default_index_options, factory_default_search_options, factory_index_and_searcher,
    factory_ready_basic_graph, factory_ready_credential_theme, FactoryIndexBundle,
    BASIC_GRAPH_FILES, CREDENTIAL_THEME_FILES,
};
pub use fixture::{sample_file, sample_root};
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
pub use lsp::{lsp_search_hit_keys, sample_backend};
pub use safety::{
    assert_api_key_not_live, assert_real_network_env_safe, assert_test_url_allowed,
    check_test_url, classify_real_env_gate, classify_real_gate, classify_real_network_gate,
    guard_live_embed_config, host_from_url, is_loopback_host, is_production_blocked_host,
    real_network_flag_from, real_network_tests_enabled, require_real_ready, RealGateStatus,
    SafetyError, PRODUCTION_HOST_BLOCKLIST, REAL_NETWORK_TESTS_ENV,
};
pub use test_log::{with_test_logging, IndexSnapshot, TestLogger, TestPhase};
