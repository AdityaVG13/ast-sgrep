//! E1 error-taxonomy inventory for ast-sgrep-core.
//!
//! Grouped taxonomy rows: the Database/Io discriminants and the
//! schema-mismatch format contract stay pinned solo; the 10 `Other` ingress
//! rows fold into 3 group tests (io-bounds, search-ingress anchored at
//! `search_rejects_oversize_query`, batch/shape). Discriminants only
//! (`matches!`); no message-text asserts except the `parse_schema_mismatch`
//! roundtrip, which is a format contract. Already-covered-elsewhere pins
//! re-assert the discriminant (the original suites mostly assert `is_err` or
//! message substrings).

#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_core::call_path::{find_call_path, CallPathConfig};
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::passes::regex::regex_pass;
use ast_sgrep_core::{
    read_text_capped, search_pattern, IndexOptions, IndexStore, Indexer, SearchOptions, Searcher,
    StoreError, MAX_FILE_FILTER_CHARS, MAX_INCREMENTAL_PATHS, MAX_QUERY_CHARS,
    MAX_REGEX_PATTERN_CHARS,
};
use ast_sgrep_testkit::err_of;
use error_testkit::{mem_searcher, write_garbage_db};
use std::path::PathBuf;
use tempfile::TempDir;

/// INTENT: garbage bytes at the index path open as Database (sole Database row).
/// KILLS: variant-swap(Database→Other), garbage-tolerated.
/// ABSORBS: none (kept solo).
/// OVERLAP: durable_recovery_pass1 torn db (this pins the minimal trigger).
#[test]
fn store_error_database_from_garbage_index_file() {
    let temp = TempDir::new().unwrap();
    let db = write_garbage_db(temp.path(), "index.db");
    let err = err_of(IndexStore::open(temp.path(), Some(&db)));
    assert!(
        matches!(err, StoreError::Database(_)),
        "garbage index file must fail as Database, got {err:?}"
    );
}

/// INTENT: missing file read surfaces Io (sole Io row; NEW).
/// KILLS: variant-swap(Io→Other).
/// ABSORBS: none (kept solo).
#[test]
fn store_error_io_from_missing_file_read() {
    let temp = TempDir::new().unwrap();
    let err = err_of(read_text_capped(&temp.path().join("nosuch.rs"), 64));
    assert!(
        matches!(err, StoreError::Io(_)),
        "missing file must fail as Io, got {err:?}"
    );
}

/// INTENT: schema-mismatch helpers are a format contract — the constructor
/// message round-trips through the parser (only format-contract pin).
/// KILLS: BEHAVIOR-ONLY (self-roundtrip; kills format drift).
/// ABSORBS: none (kept solo).
/// OVERLAP: oracle_foundry_pass1/2.
#[test]
fn schema_mismatch_helper_roundtrip_is_format_contract() {
    let err = StoreError::schema_newer_than_binary(7, 5);
    assert!(matches!(err, StoreError::Other(_)));
    let message = err.to_string();
    assert_eq!(StoreError::parse_schema_mismatch(&message), Some((7, 5)));
    assert_eq!(StoreError::parse_schema_mismatch("garbage"), None);
    assert_eq!(StoreError::parse_schema_mismatch(""), None);
}

/// INTENT: search-ingress Other group — oversize query (anchor), empty pattern,
/// invalid regex, oversize regex (direct gate), oversize file_filter, and empty
/// file_filter all refuse as Other before touching store state.
/// KILLS: ingress-wiring-drop, empty-pattern-accepted, regex-compile-error-swallow,
/// gate-drop (regex-length, filter-length), empty-filter-accepted.
/// ABSORBS: store_error_other_from_empty_pattern_ingress,
/// search_regex_rejects_invalid_pattern, regex_pass_rejects_oversize_pattern,
/// searcher_new_rejects_oversize_file_filter, search_rejects_empty_file_filter.
/// OVERLAP: pattern_routing message-level (empty-pattern discriminant new);
/// oracle unit bounds (search wiring NEW).
#[test]
fn search_rejects_oversize_query() {
    // Anchor leg: search ingress wires `validate_query_len`.
    {
        let temp = TempDir::new().unwrap();
        let searcher = mem_searcher(temp.path());
        let query = "a".repeat(MAX_QUERY_CHARS + 1);
        let err = err_of(searcher.search(&query));
        assert!(
            matches!(err, StoreError::Other(_)),
            "oversize query must fail as Other, got {err:?}"
        );
    }
    // Absorbed: store_error_other_from_empty_pattern_ingress.
    {
        let temp = TempDir::new().unwrap();
        let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
        let err = err_of(search_pattern("", &store, temp.path(), None, 10));
        assert!(
            matches!(err, StoreError::Other(_)),
            "empty pattern must fail as Other, got {err:?}"
        );
    }
    // Absorbed: search_regex_rejects_invalid_pattern.
    {
        let temp = TempDir::new().unwrap();
        let searcher = mem_searcher(temp.path());
        let err = err_of(searcher.search_regex("(["));
        assert!(
            matches!(err, StoreError::Other(_)),
            "invalid regex must fail as Other, got {err:?}"
        );
    }
    // Absorbed: regex_pass_rejects_oversize_pattern — the regex-length gate
    // (unreachable via `search_regex`, whose query gate fires first at the same
    // bound) refuses when hit directly.
    {
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
    // Absorbed: searcher_new_rejects_oversize_file_filter — rejected before
    // opening the store.
    {
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
    // Absorbed: search_rejects_empty_file_filter — refuses at finish (distinct
    // `compile_glob` site from the NUL-filter row in search_correctness_epics
    // iva9.2).
    {
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
}

/// INTENT: io-bounds Other group — over-cap, binary, and directory reads refuse
/// as Other.
/// KILLS: cap-drop, binary-sniff-drop, dir-read-accepted.
/// ABSORBS: io_bounds_over_cap_rejects_as_other,
/// io_bounds_binary_and_directory_reject_as_other.
/// OVERLAP: oracle_foundry_pass3 is_err-only (discriminants new).
#[test]
fn io_bounds_reject_as_other() {
    // Absorbed: io_bounds_over_cap_rejects_as_other.
    {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("big.rs");
        std::fs::write(&path, b"123456789").unwrap();
        let err = err_of(read_text_capped(&path, 8));
        assert!(
            matches!(err, StoreError::Other(_)),
            "over-cap file must fail as Other, got {err:?}"
        );
    }
    // Absorbed: io_bounds_binary_and_directory_reject_as_other.
    {
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
}

/// INTENT: batch/shape Other group — over-max incremental batch, zero-pattern
/// multi-pattern batch, and oversize call-path endpoints (both sides) refuse as
/// Other.
/// KILLS: max-batch-gate-drop, empty-batch-accepted, endpoint-gate-drop(one-side).
/// ABSORBS: update_paths_rejects_over_max_batch,
/// search_multi_pattern_rejects_empty_batch, call_path_rejects_oversize_endpoints.
#[test]
fn batch_shape_rejects_as_other() {
    // Absorbed: update_paths_rejects_over_max_batch.
    {
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
    // Absorbed: search_multi_pattern_rejects_empty_batch.
    {
        let temp = TempDir::new().unwrap();
        let searcher = mem_searcher(temp.path());
        let err = err_of(searcher.search_multi_pattern(&[]));
        assert!(
            matches!(err, StoreError::Other(_)),
            "empty pattern batch must fail as Other, got {err:?}"
        );
    }
    // Absorbed: call_path_rejects_oversize_endpoints — both sides validated.
    {
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
}
