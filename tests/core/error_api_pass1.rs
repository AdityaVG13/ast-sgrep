//! E1 error-taxonomy inventory for ast-sgrep-core.
//!
//! One test per error variant/site proving it is reachable with a hand-built
//! trigger. Discriminants only (`matches!`); no message-text asserts except
//! the `parse_schema_mismatch` roundtrip, which is a format contract.
//! Already-covered-elsewhere pins re-assert the discriminant (the original
//! suites mostly assert `is_err` or message substrings).

use ast_sgrep_core::call_path::{find_call_path, CallPathConfig};
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::passes::regex::regex_pass;
use ast_sgrep_core::{
    read_text_capped, search_pattern, IndexOptions, IndexStore, Indexer, SearchOptions, Searcher,
    StoreError, MAX_FILE_FILTER_CHARS, MAX_INCREMENTAL_PATHS, MAX_QUERY_CHARS,
    MAX_REGEX_PATTERN_CHARS,
};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn mem_searcher(root: &Path) -> Searcher {
    let store = IndexStore::open_in_memory(root).expect("in-memory store");
    Searcher::with_store(
        store,
        SearchOptions {
            root: root.to_path_buf(),
            limit: 10,
            use_embed: false,
            ..SearchOptions::default()
        },
    )
}

fn err_of<T>(result: Result<T, StoreError>) -> StoreError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(err) => err,
    }
}

/// StoreError::Database — garbage bytes at the index path fail the open with
/// the Database discriminant. Class already covered by
/// durable_recovery_pass1 (torn db); this pins the minimal trigger.
#[test]
fn store_error_database_from_garbage_index_file() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("index.db");
    std::fs::write(&db, b"this is not a sqlite database file; garbage").unwrap();
    let err = err_of(IndexStore::open(temp.path(), Some(&db)));
    assert!(
        matches!(err, StoreError::Database(_)),
        "garbage index file must fail as Database, got {err:?}"
    );
}

/// StoreError::Io — reading a missing file surfaces the Io discriminant.
/// NEW (no suite pins Io).
#[test]
fn store_error_io_from_missing_file_read() {
    let temp = TempDir::new().unwrap();
    let err = err_of(read_text_capped(&temp.path().join("nosuch.rs"), 64));
    assert!(
        matches!(err, StoreError::Io(_)),
        "missing file must fail as Io, got {err:?}"
    );
}

/// StoreError::Other — empty pattern ingress refuses. Already covered by
/// pattern_routing (message-level); this pins the discriminant.
#[test]
fn store_error_other_from_empty_pattern_ingress() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    let err = err_of(search_pattern("", &store, temp.path(), None, 10));
    assert!(
        matches!(err, StoreError::Other(_)),
        "empty pattern must fail as Other, got {err:?}"
    );
}

/// Schema-mismatch helpers are a format contract: the constructor message
/// round-trips through the parser. Already covered by oracle_foundry_pass1/2;
/// pinned here as the taxonomy row.
#[test]
fn schema_mismatch_helper_roundtrip_is_format_contract() {
    let err = StoreError::schema_newer_than_binary(7, 5);
    assert!(matches!(err, StoreError::Other(_)));
    let message = err.to_string();
    assert_eq!(
        StoreError::parse_schema_mismatch(&message),
        Some((7, 5))
    );
    assert_eq!(StoreError::parse_schema_mismatch("garbage"), None);
    assert_eq!(StoreError::parse_schema_mismatch(""), None);
}

/// io_bounds over-cap rejection. Already covered by oracle_foundry_pass3
/// (`is_err`-only); this pins the Other discriminant.
#[test]
fn io_bounds_over_cap_rejects_as_other() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("big.rs");
    std::fs::write(&path, b"123456789").unwrap();
    let err = err_of(read_text_capped(&path, 8));
    assert!(
        matches!(err, StoreError::Other(_)),
        "over-cap file must fail as Other, got {err:?}"
    );
}

/// io_bounds binary + non-regular-file rejections. Already covered by
/// oracle_foundry_pass3 (`is_err`-only); this pins both Other discriminants.
#[test]
fn io_bounds_binary_and_directory_reject_as_other() {
    let temp = TempDir::new().unwrap();
    let binary = temp.path().join("blob.bin");
    std::fs::write(&binary, [0xffu8, 0xfe, 0x00, 0x61]).unwrap();
    let err = err_of(read_text_capped(&binary, 64));
    assert!(
        matches!(err, StoreError::Other(_)),
        "binary file must fail as Other, got {err:?}"
    );
    let err = err_of(read_text_capped(temp.path(), 64));
    assert!(
        matches!(err, StoreError::Other(_)),
        "directory must fail as Other, got {err:?}"
    );
}

/// Search ingress wires `validate_query_len`: an oversize query refuses.
/// Unit bounds covered by oracle_foundry_pass1/2; the search wiring is NEW.
#[test]
fn search_rejects_oversize_query() {
    let temp = TempDir::new().unwrap();
    let searcher = mem_searcher(temp.path());
    let query = "a".repeat(MAX_QUERY_CHARS + 1);
    let err = err_of(searcher.search(&query));
    assert!(
        matches!(err, StoreError::Other(_)),
        "oversize query must fail as Other, got {err:?}"
    );
}

/// `regex:` with an uncompilable pattern refuses. NEW.
#[test]
fn search_regex_rejects_invalid_pattern() {
    let temp = TempDir::new().unwrap();
    let searcher = mem_searcher(temp.path());
    let err = err_of(searcher.search_regex("(["));
    assert!(
        matches!(err, StoreError::Other(_)),
        "invalid regex must fail as Other, got {err:?}"
    );
}

/// The regex-length gate (unreachable via `search_regex`, whose query gate
/// fires first at the same bound) refuses when hit directly. NEW.
#[test]
fn regex_pass_rejects_oversize_pattern() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    };
    let pattern = "x".repeat(MAX_REGEX_PATTERN_CHARS + 1);
    let parsed = ParsedQuery::regex(&pattern);
    let err = err_of(regex_pass(&store, &options, &parsed));
    assert!(
        matches!(err, StoreError::Other(_)),
        "oversize regex must fail as Other, got {err:?}"
    );
}

/// `Searcher::new` rejects an oversize file_filter before opening the store.
/// NEW.
#[test]
fn searcher_new_rejects_oversize_file_filter() {
    let temp = TempDir::new().unwrap();
    let err = err_of(Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        file_filter: Some("f".repeat(MAX_FILE_FILTER_CHARS + 1)),
        ..SearchOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Other(_)),
        "oversize file_filter must fail as Other, got {err:?}"
    );
}

/// An empty file_filter refuses at finish (distinct `compile_glob` site from
/// the NUL-filter row in search_correctness_epics iva9.2). NEW.
#[test]
fn search_rejects_empty_file_filter() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    let searcher = Searcher::with_store(
        store,
        SearchOptions {
            root: temp.path().to_path_buf(),
            limit: 10,
            file_filter: Some(String::new()),
            use_embed: false,
            ..SearchOptions::default()
        },
    );
    let err = err_of(searcher.search_literal("alpha"));
    assert!(
        matches!(err, StoreError::Other(_)),
        "empty file_filter must fail as Other, got {err:?}"
    );
}

/// Incremental updates over `MAX_INCREMENTAL_PATHS` refuse. NEW.
#[test]
fn update_paths_rejects_over_max_batch() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db),
        ..IndexOptions::default()
    })
    .expect("indexer");
    let paths = vec![PathBuf::from("a.rs"); MAX_INCREMENTAL_PATHS + 1];
    let err = err_of(indexer.update_paths(&paths).map(|_| ()));
    assert!(
        matches!(err, StoreError::Other(_)),
        "over-max batch must fail as Other, got {err:?}"
    );
}

/// Multi-pattern ingress with zero patterns refuses. NEW.
#[test]
fn search_multi_pattern_rejects_empty_batch() {
    let temp = TempDir::new().unwrap();
    let searcher = mem_searcher(temp.path());
    let err = err_of(searcher.search_multi_pattern(&[]));
    assert!(
        matches!(err, StoreError::Other(_)),
        "empty pattern batch must fail as Other, got {err:?}"
    );
}

/// `find_call_path` validates both endpoint lengths. NEW.
#[test]
fn call_path_rejects_oversize_endpoints() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    let long = "a".repeat(MAX_QUERY_CHARS + 1);
    let config = CallPathConfig::default();
    let err = err_of(find_call_path(&store, &long, "sink", &config).map(|_| ()));
    assert!(
        matches!(err, StoreError::Other(_)),
        "oversize source must fail as Other, got {err:?}"
    );
    let err = err_of(find_call_path(&store, "source", &long, &config).map(|_| ()));
    assert!(
        matches!(err, StoreError::Other(_)),
        "oversize sink must fail as Other, got {err:?}"
    );
}
