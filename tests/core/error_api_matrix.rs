//! Error-API matrix for `ast-sgrep-core`.
//!
//! F35 collapse of `error_api_pass{1,2,3,4}.rs` into one parametric target.
//! Pure collapse: every test below is unchanged from its pass-file original —
//! same triggers, same assertions, same INTENT/KILLS/ABSORBS audit lines. Only
//! the file layout changed (one import block, E1-E4 sections). Per-pass module
//! docs are kept as section banners; shared setup lives in
//! `tests/core/error_testkit.rs` (canonical beats promoted to `ast-sgrep-testkit`).
//!
//! Matrix (mission -> test):
//! - E1 taxonomy: `store_error_database_from_garbage_index_file`, `store_error_io_from_missing_file_read`, `schema_mismatch_helper_roundtrip_is_format_contract`, `search_rejects_oversize_query`, `io_bounds_reject_as_other`, `batch_shape_rejects_as_other`
//! - E2 propagation: `indexer_new_missing_root_surfaces_io`, `searcher_new_missing_root_converts_to_other`, `store_open_uncreatable_index_dir_converts_to_other`, `db_garbage_kind_preserved_through_store_open`, `db_dropped_table_through_search_pattern_stays_database`, `update_paths_over_max_rejects_before_side_effect`, `cancelled_index_all_rejects_before_side_effect`, `per_file_failure_is_counted_not_silent`, `missing_db_through_searcher_new_fails_closed`
//! - E3 relations: `garbage_db_same_discriminant_across_open_search_and_index`, `blocked_index_dir_same_discriminant_across_store_and_indexer`, `oversize_query_same_discriminant_across_search_lanes`, `dropped_table_same_discriminant_across_depth_and_search_lanes`, `repeated_triggers_yield_identical_discriminant_sequence`, `error_does_not_poison_later_success_on_same_handle`, `failed_opens_leave_filesystem_untouched`, `failed_writes_preserve_committed_rows`
//! - E4 drills: `drill_deleted_index_search_fails_closed_then_reindex_recovers`, `drill_corrupt_index_open_fails_database_then_force_reindex_recovers`, `drill_unreadable_index_search_fails_database_then_perm_restore_recovers`, `drill_oversize_query_search_rejects_then_clean_query_succeeds`, `drill_dropped_tables_search_and_status_fail_database_then_rebuild_recovers`, `drill_missing_root_flows_fail_then_restore_recovers`, `drill_chained_double_fault_query_precedence_then_schema_fault_then_recovery`
//!
//! No asserted behavior changed in this collapse (see F35: keep every distinct
//! behavior assertion, drop pure duplication — no pure duplication remained;
//! net test count is unchanged).
#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_core::{
    call_path::{find_call_path, CallPathConfig},
    query::ParsedQuery,
    read_text_capped,
    search::passes::regex::regex_pass,
    search_pattern, IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError,
    MAX_FILE_FILTER_CHARS, MAX_INCREMENTAL_PATHS, MAX_QUERY_CHARS, MAX_REGEX_PATTERN_CHARS,
};
use ast_sgrep_testkit::{err_of, truncate_file, write_garbage};
use error_testkit::{
    assert_clean_state, discriminant, indexer_at, is_corrupt_kind, mem_searcher, populate,
    remove_db_files, searcher_at, sqlite_code, write_garbage_db,
};
use rusqlite::ErrorCode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tempfile::TempDir;

// ── E1 taxonomy inventory (from error_api_pass1.rs) ──
// E1 error-taxonomy inventory for ast-sgrep-core.
//
// Grouped taxonomy rows: the Database/Io discriminants and the
// schema-mismatch format contract stay pinned solo; the 10 `Other` ingress
// rows fold into 3 group tests (io-bounds, search-ingress anchored at
// `search_rejects_oversize_query`, batch/shape). Discriminants only
// (`matches!`); no message-text asserts except the `parse_schema_mismatch`
// roundtrip, which is a format contract. Already-covered-elsewhere pins
// re-assert the discriminant (the original suites mostly assert `is_err` or
// message substrings).
//

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

// ── E2 propagation oracles (from error_api_pass2.rs) ──
// E2 error-propagation oracles for ast-sgrep-core.
//
// E1 pins taxonomy (each `StoreError` variant is reachable). E2 pins
// PROPAGATION: a depth error must surface through every layer with the
// discriminant (and sqlite/io kind) the layer contract promises — never
// `Ok`, never a panic, never a silent downgrade. Discriminants and
// downcast kinds only (`matches!`); no message-text asserts.
//
// Site inventory (from `map_err` / `.ok()` / `?` grep over
// `crates/ast-sgrep-core/src`); each row names its pinning test:
//
// - `io_bounds::RootDir::open` rustix `map_err(io::Error)` + `?` (Io
//   preserved) → `indexer_new_missing_root_surfaces_io` (missing-root +
//   file-as-root trigger legs).
// - `Indexer::new` `canonicalize().unwrap_or(...)` swallow (harmless:
//   `RootDir::open` fails after) → `indexer_new_missing_root_surfaces_io`.
// - `Searcher::new` `canonicalize().map_err(... Other ...)` (deliberate
//   Io→Other with context) → `searcher_new_missing_root_converts_to_other`.
// - `store::sqlite::open_inner` `create_dir_all.map_err(... Other ...)`
//   (deliberate Io→Other) → `store_open_uncreatable_index_dir_converts_to_other`.
// - `store::sqlite::open_inner` `Connection::open ?` (`#[from]` Database,
//   kind preserved) + `init_schema ?` →
//   `db_garbage_kind_preserved_through_store_open` (store/searcher/indexer
//   layer legs + byte-identical rejection proof).
// - `open_inner` read-only missing-db guard (Other, fail-closed) →
//   `missing_db_through_searcher_new_fails_closed`.
// - `pattern::search_pattern` `store.pattern_node_count()?` →
//   `db_dropped_table_through_search_pattern_stays_database`.
// - `Indexer::update_paths` over-max gate before `ignore.clear()`/indexing
//   → `update_paths_over_max_rejects_before_side_effect`.
// - `Indexer::check_cancel ?` at `index_all` entry and per update path → the
//   cancel anchor `cancelled_index_all_rejects_before_side_effect`
//   (entry×assert over both write entries; absorbs the E3 cancel relation).
// - `update_paths` per-file `index_failure` counted isolation
//   (`files_failed += 1`, batch stays `Ok`) → `per_file_failure_is_counted`.
//
// Deliberately NOT pinned here: `Searcher::index_gen` / `cached_stamp_parts`
// `.ok()?` sites (fail-open to recompute by documented design, hdwh — no
// deterministic public trigger for a PRAGMA failure), `pattern.rs` walk
// `File::open(path).ok()?` skip-unreadable (walk-level file skip, covered
// by walk-behavior suites), `Durability::from_env` `.ok().unwrap_or_default`
// (env parsing, not error propagation).
//

/// INTENT: `RootDir::open` failures propagate through `Indexer::new` as `Io` —
/// missing root (NotFound kind, db not created) plus file-as-root (`O_DIRECTORY`) legs.
/// KILLS: Io→Other-swap, side-effect-before-check, variant-swap.
/// ABSORBS: indexer_new_file_as_root_surfaces_io (same RootDir::open path, second trigger).
#[test]
fn indexer_new_missing_root_surfaces_io() {
    // Anchor leg: missing root. The `canonicalize().unwrap_or` above the open
    // swallows nothing observable: the open fails after. The db path must not
    // be created: the root check precedes any store side effect.
    {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("nosuch-root");
        let db = temp.path().join("index.db");
        let err = err_of(Indexer::new(IndexOptions {
            root: missing,
            index_path: Some(db.clone()),
            ..IndexOptions::default()
        }));
        match &err {
            StoreError::Io(io) => assert_eq!(
                io.kind(),
                std::io::ErrorKind::NotFound,
                "missing root must surface NotFound kind, got {err:?}"
            ),
            other => panic!("missing root must fail as Io, got {other:?}"),
        }
        assert!(
            !db.exists(),
            "failed root open must not create the index file"
        );
    }
    // Absorbed leg: a regular file as root fails `RootDir::open`
    // (`O_DIRECTORY`) and still surfaces as `Io` — not `Other`, not `Ok`,
    // not a panic.
    {
        let temp = TempDir::new().unwrap();
        let file_root = temp.path().join("file.rs");
        std::fs::write(&file_root, b"fn main() {}\n").unwrap();
        let err = err_of(Indexer::new(IndexOptions {
            root: file_root,
            index_path: Some(temp.path().join("index.db")),
            ..IndexOptions::default()
        }));
        assert!(
            matches!(err, StoreError::Io(_)),
            "file-as-root must fail as Io, got {err:?}"
        );
    }
}

/// INTENT: same depth error through the read layer — `Searcher::new`
/// deliberately converts the `canonicalize` Io into `Other` with context
/// (deliberate conversion contract). Fail-closed, never silent `Ok`.
/// KILLS: conversion-drop(raw-Io-leak).
/// ABSORBS: none (kept solo).
#[test]
fn searcher_new_missing_root_converts_to_other() {
    let temp = TempDir::new().unwrap();
    let err = err_of(Searcher::new(SearchOptions {
        root: temp.path().join("nosuch-root"),
        ..SearchOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Other(_)),
        "Searcher::new missing root must fail as Other, got {err:?}"
    );
}

/// INTENT: `open_inner` maps `create_dir_all` Io into `Other` (deliberate
/// map-site contract) — a file blocking the index directory fails as `Other`,
/// deterministically, instead of leaking a raw Io or panicking.
/// KILLS: conversion-drop.
/// ABSORBS: none (kept solo).
#[test]
fn store_open_uncreatable_index_dir_converts_to_other() {
    let temp = TempDir::new().unwrap();
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
    let err = err_of(IndexStore::open(
        temp.path(),
        Some(&blocker.join("index.db")),
    ));
    assert!(
        matches!(err, StoreError::Other(_)),
        "blocked index dir must fail as Other, got {err:?}"
    );
}

/// INTENT: garbage bytes stay Database + NotADatabase kind +
/// corrupt-detectable through the store open, the read-layer constructor, and
/// the write-layer constructor without force — and rejection precedes any
/// repair side effect (byte-identical).
/// KILLS: kind-drop, variant-swap, read-layer-conversion, silent-recovery,
/// repair-before-reject.
/// ABSORBS: db_garbage_through_searcher_new_stays_database,
/// db_garbage_through_indexer_new_without_force_stays_database (byte-identical
/// assertion carried).
/// OVERLAP: E3 garbage_db_same_discriminant asserts the same cross-layer relation.
#[test]
fn db_garbage_kind_preserved_through_store_open() {
    // Anchor leg: store open pins the kind + corrupt-detectability (E1 pins
    // the discriminant; E2 pins the kind preservation).
    {
        let temp = TempDir::new().unwrap();
        let db = write_garbage_db(temp.path(), "index.db");
        let err = err_of(IndexStore::open(temp.path(), Some(&db)));
        assert_eq!(
            sqlite_code(&err),
            Some(ErrorCode::NotADatabase),
            "garbage db must preserve NotADatabase kind, got {err:?}"
        );
        assert!(
            is_corrupt_kind(&err),
            "garbage db must stay corruption-detectable, got {err:?}"
        );
    }
    // Absorbed leg: the same depth failure through `Searcher::new` (readonly
    // open + readonly configure + readonly schema probe) — the read layer
    // adds no conversion.
    {
        let temp = TempDir::new().unwrap();
        let db = write_garbage_db(temp.path(), "index.db");
        let err = err_of(searcher_at(temp.path(), &db));
        assert!(
            matches!(err, StoreError::Database(_)),
            "garbage db through Searcher::new must stay Database, got {err:?}"
        );
        assert!(
            is_corrupt_kind(&err),
            "garbage db kind must survive the read layer, got {err:?}"
        );
    }
    // Absorbed leg: without `force_reindex`, `Indexer::new` propagates the
    // open `Database` error untouched (no recovery, no `Ok`) and leaves the
    // file byte-identical.
    {
        let temp = TempDir::new().unwrap();
        let db = write_garbage_db(temp.path(), "index.db");
        let before = std::fs::read(&db).unwrap();
        let err = err_of(Indexer::new(IndexOptions {
            root: temp.path().to_path_buf(),
            index_path: Some(db.clone()),
            ..IndexOptions::default()
        }));
        assert!(
            matches!(err, StoreError::Database(_)),
            "garbage db through Indexer::new must stay Database, got {err:?}"
        );
        assert!(
            is_corrupt_kind(&err),
            "garbage db kind must survive Indexer::new, got {err:?}"
        );
        assert_eq!(
            std::fs::read(&db).unwrap(),
            before,
            "rejected open must not mutate the db file"
        );
    }
}

/// INTENT: a sqlite failure below `search_pattern` (`pattern_node_count()?`)
/// propagates as `Database` — the search lane never degrades it to `Ok` empty
/// or `Other`. Control first: the same call on the healthy store is `Ok`.
/// KILLS: lane-degrade-to-Ok-empty, degrade-to-Other.
/// ABSORBS: none (kept solo).
#[test]
fn db_dropped_table_through_search_pattern_stays_database() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    assert_eq!(
        store.pattern_node_count().expect("table exists"),
        0,
        "fresh store must have an empty pattern_nodes table"
    );
    assert!(
        search_pattern("greet_user", &store, temp.path(), None, 10).is_ok(),
        "control: healthy store must answer Ok"
    );
    store
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    assert!(
        matches!(store.pattern_node_count(), Err(StoreError::Database(_))),
        "dropped table must fail direct count as Database"
    );
    let err = err_of(search_pattern("greet_user", &store, temp.path(), None, 10));
    assert!(
        matches!(
            err,
            StoreError::Database(rusqlite::Error::SqliteFailure(_, _))
        ),
        "dropped table through search_pattern must stay Database(SqliteFailure), got {err:?}"
    );
}

/// INTENT: the over-max gate fires before `ignore.clear()` and before any file
/// is touched — `Other` plus a store that never saw the batch.
/// KILLS: gate-after-side-effect.
/// ABSORBS: none (kept solo).
/// OVERLAP: E3 failed_writes (E2 pins never-saw-batch via file_hash).
#[test]
fn update_paths_over_max_rejects_before_side_effect() {
    let temp = TempDir::new().unwrap();
    let rel = temp.path().join("a.rs");
    std::fs::write(&rel, b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let paths = vec![rel; MAX_INCREMENTAL_PATHS + 1];
    let err = err_of(indexer.update_paths(&paths).map(|_| ()));
    assert!(
        matches!(err, StoreError::Other(_)),
        "over-max batch must fail as Other, got {err:?}"
    );
    assert_eq!(
        indexer.store().file_hash("a.rs").expect("store readable"),
        None,
        "rejected batch must not index its first valid path"
    );
}

/// INTENT: a set cancel flag fails every write entry (`index_all`,
/// `update_paths`) as `Other` with nothing indexed and no Ok stats — one
/// entry×assert matrix over the same `check_cancel` contract.
/// KILLS: cancel-check-drop, partial-index-on-cancel, entry-divergence-on-cancel.
/// ABSORBS: cancelled_update_paths_rejects_before_side_effect (same
/// check_cancel contract, second entry); E3
/// cancelled_writes_fail_closed_with_same_discriminant (entry-agreement
/// relation folds here).
#[test]
fn cancelled_index_all_rejects_before_side_effect() {
    let temp = TempDir::new().unwrap();
    let rel = temp.path().join("a.rs");
    std::fs::write(&rel, b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let cancel = Arc::new(AtomicBool::new(false));
    indexer.set_cancel(Arc::clone(&cancel));
    cancel.store(true, Ordering::SeqCst);
    // Entry×assert: both write entries fail as Other (neither reports Ok stats
    // for work it did not do — the E3 relation leg).
    let via_index_all = err_of(indexer.index_all().map(|_| ()));
    let via_update = err_of(indexer.update_paths(std::slice::from_ref(&rel)).map(|_| ()));
    assert_eq!(
        (
            discriminant(&via_index_all),
            discriminant(&via_update)
        ),
        (2, 2),
        "cancelled writes must fail as Other through both entries, got {via_index_all:?} / {via_update:?}"
    );
    // Side-effect legs: neither entry indexed anything.
    assert_eq!(
        indexer.store().file_hash("a.rs").expect("store readable"),
        None,
        "cancelled writes must index nothing"
    );
}

/// INTENT: per-file isolation degrades loudly, not silently — a binary `.rs`
/// fails its own path while its healthy sibling indexes; the batch stays `Ok`
/// with the failure COUNTED in `files_failed` (plus a stderr line).
/// KILLS: silent-skip, batch-Err-on-single-failure.
/// ABSORBS: none (kept solo; only loud-degradation pin).
#[test]
fn per_file_failure_is_counted_not_silent() {
    let temp = TempDir::new().unwrap();
    let good = temp.path().join("good.rs");
    std::fs::write(&good, b"fn greet() {}\n").unwrap();
    let blob = temp.path().join("blob.rs");
    std::fs::write(&blob, [0xffu8, 0xfe, 0x00, 0x61]).unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let stats = indexer
        .update_paths(&[good, blob])
        .expect("per-file failure must not fail the batch");
    assert_eq!(stats.files_indexed, 1, "healthy sibling must index");
    assert_eq!(stats.files_failed, 1, "binary file must be counted failed");
    assert!(
        indexer
            .store()
            .file_hash("good.rs")
            .expect("store readable")
            .is_some(),
        "healthy sibling must be indexed"
    );
    assert_eq!(
        indexer
            .store()
            .file_hash("blob.rs")
            .expect("store readable"),
        None,
        "failed file must leave no row"
    );
}

/// INTENT: search-before-index fails closed as discriminant 2 (`Other` or
/// `EmptyIndex` — testkit's single source of truth projects both as the
/// fail-closed non-sqlite refusal; `missing_index_error` names EmptyIndex
/// since the split) through `Searcher::new` (readonly missing-db guard) —
/// never an `Ok` searcher silently answering empty. Control: after indexing,
/// the same constructor is `Ok`.
/// KILLS: fail-open-to-empty-searcher.
/// ABSORBS: none (kept solo).
#[test]
fn missing_db_through_searcher_new_fails_closed() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let err = err_of(Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db.clone()),
        ..SearchOptions::default()
    }));
    assert_eq!(
        discriminant(&err),
        2,
        "missing db through Searcher::new must fail closed (discriminant 2), got {err:?}"
    );
    let mut indexer = indexer_at(temp.path(), &db);
    indexer.index_all().expect("index the fixture");
    assert!(
        Searcher::new(SearchOptions {
            root: temp.path().to_path_buf(),
            index_path: Some(db),
            ..SearchOptions::default()
        })
        .is_ok(),
        "control: Searcher::new must succeed once the db exists"
    );
}

// ── E3 metamorphic relations (from error_api_pass3.rs) ──
// E3 negative-path metamorphic tests for ast-sgrep-core errors.
//
// E1 pins taxonomy (each `StoreError` discriminant is reachable). E2 pins
// propagation (a depth error surfaces with the promised discriminant at each
// layer). E3 pins RELATIONS over failures — properties that hold across
// entry points, repetitions, and surrounding state:
//
// - MR1 same-fault-same-discriminant: one underlying fault observed through
//   different entry points yields the same discriminant (and sqlite kind).
// - MR2 error determinism: the same trigger repeated yields the identical
//   discriminant sequence; a failure never poisons a later success.
// - MR3 fail-closed: a poisoned store returns `Err` through every lane —
//   never `Ok` with partial or empty results misreported as success. (The
//   cancelled-writes entry×assert relation folds into the E2 cancel anchor;
//   this file keeps the poisoned-store legs.)
// - MR4 error-before-side-effect: a failed open/write leaves the filesystem
//   and all committed store rows exactly as they were.
//
// Discriminants (and sqlite `ErrorCode`) only; no message-text asserts.
//

/// INTENT: MR1 — the same corrupt-bytes fault through the store open, the
/// read-layer constructor, and the write-layer constructor yields the same
/// discriminant AND the same sqlite kind (MR1 anchor).
/// KILLS: layer-conversion, kind-divergence.
/// ABSORBS: none (kept solo).
/// OVERLAP: E2 garbage legs (this is the relation statement).
#[test]
fn garbage_db_same_discriminant_across_open_search_and_index() {
    let temp = TempDir::new().unwrap();
    let db = write_garbage_db(temp.path(), "index.db");
    let via_store = err_of(IndexStore::open(temp.path(), Some(&db)));
    let via_searcher = err_of(searcher_at(temp.path(), &db));
    let via_indexer = err_of(Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db),
        ..IndexOptions::default()
    }));
    for (name, err) in [
        ("store open", &via_store),
        ("searcher new", &via_searcher),
        ("indexer new", &via_indexer),
    ] {
        assert_eq!(
            discriminant(err),
            0,
            "garbage db via {name} must stay Database, got {err:?}"
        );
    }
    assert_eq!(
        (
            sqlite_code(&via_store),
            sqlite_code(&via_searcher),
            sqlite_code(&via_indexer)
        ),
        (
            Some(ErrorCode::NotADatabase),
            Some(ErrorCode::NotADatabase),
            Some(ErrorCode::NotADatabase),
        ),
        "sqlite kind must survive every layer identically"
    );
}

/// INTENT: MR1 — a file blocking the index directory fails identically through
/// the store open and the write-layer constructor (both `Other` by the
/// deliberate `create_dir_all` Io→Other map).
/// KILLS: layer-divergence.
/// ABSORBS: none (kept solo; distinct map site from garbage).
#[test]
fn blocked_index_dir_same_discriminant_across_store_and_indexer() {
    let temp = TempDir::new().unwrap();
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
    let target = blocker.join("index.db");
    let via_store = err_of(IndexStore::open(temp.path(), Some(&target)));
    let via_indexer = err_of(Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(target),
        ..IndexOptions::default()
    }));
    assert_eq!(
        (discriminant(&via_store), discriminant(&via_indexer)),
        (2, 2),
        "blocked index dir must fail as Other through both layers, got {via_store:?} / {via_indexer:?}"
    );
}

/// INTENT: MR1 — one oversize query through every lexical search lane,
/// including the multi-pattern fan-in, yields `Other` everywhere. No lane
/// answers, truncates, or converts.
/// KILLS: lane-truncate-and-answer, lane-conversion.
/// ABSORBS: none (kept solo).
#[test]
fn oversize_query_same_discriminant_across_search_lanes() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let searcher = mem_searcher(temp.path());
    let query = "a".repeat(MAX_QUERY_CHARS + 1);
    let errs = [
        ("search", err_of(searcher.search(&query))),
        ("search_literal", err_of(searcher.search_literal(&query))),
        ("search_word", err_of(searcher.search_word(&query))),
        ("search_regex", err_of(searcher.search_regex(&query))),
        (
            "search_multi_pattern",
            err_of(searcher.search_multi_pattern(std::slice::from_ref(&query))),
        ),
    ];
    for (lane, err) in &errs {
        assert_eq!(
            discriminant(err),
            2,
            "oversize query via {lane} must fail as Other, got {err:?}"
        );
    }
}

/// INTENT: MR1+MR3 — one dropped-table fault fails as Database with the same
/// sqlite kind through the depth query, the free function, the prefixed search
/// lane, the multi-pattern fan-in, AND the status probe. Controls answer Ok
/// first on the healthy stores, so the post-drop Errs are the poison.
/// KILLS: lane-degrade, kind-divergence, status-Ok-over-broken-schema,
/// Ok-empty-over-poison, cache-masks-poison.
/// ABSORBS: dropped_table_fails_status_with_database_discriminant (status leg),
/// poisoned_store_fails_closed_across_pattern_lanes (control-first + cache-off
/// rationale).
/// OVERLAP: E4 dropped-tables drill pins the status leg too (end-to-end + rebuild).
#[test]
fn dropped_table_same_discriminant_across_depth_and_search_lanes() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    // Response cache OFF: otherwise the control queries below would populate the
    // cache and the post-drop lanes would answer cached `Ok` without touching
    // the poisoned table — a legitimate cache layer, not the fail-closed
    // relation under test.
    let searcher = Searcher::with_store(
        IndexStore::open_in_memory(temp.path()).expect("second store"),
        SearchOptions {
            root: temp.path().to_path_buf(),
            limit: 10,
            use_embed: false,
            ..SearchOptions::default()
        },
    )
    .with_response_cache(false);
    // Control first: every lane answers Ok on the healthy stores.
    assert_eq!(
        store.pattern_node_count().expect("table exists"),
        0,
        "control: fresh store must have an empty pattern_nodes table"
    );
    assert!(
        search_pattern("greet_user", &store, temp.path(), None, 10).is_ok(),
        "control: free function on healthy store must be Ok"
    );
    assert!(
        searcher.search("pattern:greet_user").is_ok(),
        "control: prefixed search on healthy store must be Ok"
    );
    assert!(
        searcher
            .search_multi_pattern(&["greet_user".to_string()])
            .is_ok(),
        "control: multi-pattern on healthy store must be Ok"
    );
    assert!(
        store.status().is_ok(),
        "control: status on a healthy store must be Ok"
    );
    store
        .connection()
        .execute_batch("DROP TABLE pattern_nodes; DROP TABLE symbols")
        .unwrap();
    searcher
        .store()
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    let via_depth = err_of(store.pattern_node_count().map(|_| ()));
    let via_free_fn =
        err_of(search_pattern("greet_user", &store, temp.path(), None, 10).map(|_| ()));
    let via_search = err_of(searcher.search("pattern:greet_user").map(|_| ()));
    let via_multi = err_of(
        searcher
            .search_multi_pattern(&["greet_user".to_string()])
            .map(|_| ()),
    );
    let via_status = err_of(store.status().map(|_| ()));
    for (lane, err) in [
        ("depth query", &via_depth),
        ("free function", &via_free_fn),
        ("prefixed search", &via_search),
        ("multi-pattern", &via_multi),
        ("status probe", &via_status),
    ] {
        assert_eq!(
            discriminant(err),
            0,
            "dropped table via {lane} must stay Database, got {err:?}"
        );
    }
    assert_eq!(
        sqlite_code(&via_depth),
        sqlite_code(&via_free_fn),
        "depth and free-function sqlite kinds must match"
    );
    assert_eq!(
        sqlite_code(&via_search),
        sqlite_code(&via_multi),
        "search and multi-pattern sqlite kinds must match"
    );
    assert!(
        sqlite_code(&via_status).is_some(),
        "status failure must carry a sqlite kind, got {via_status:?}"
    );
}

/// INTENT: MR2 — the same four triggers, run twice against fresh fixtures,
/// yield the identical discriminant sequence (and identical sqlite kinds).
/// Error identity is a pure function of the trigger.
/// KILLS: run-to-run-drift.
/// ABSORBS: none (kept solo).
#[test]
fn repeated_triggers_yield_identical_discriminant_sequence() {
    fn run_once() -> (Vec<u8>, Vec<Option<ErrorCode>>) {
        let temp = TempDir::new().unwrap();
        let garbage = write_garbage_db(temp.path(), "garbage.db");
        let blocker = temp.path().join("blocker");
        std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
        let e1 = err_of(IndexStore::open(temp.path(), Some(&garbage)));
        let e2 = err_of(IndexStore::open(
            temp.path(),
            Some(&blocker.join("index.db")),
        ));
        let e3 = err_of(Searcher::new(SearchOptions {
            root: temp.path().join("nosuch-root"),
            ..SearchOptions::default()
        }));
        let searcher = mem_searcher(temp.path());
        let e4 = err_of(searcher.search(&"q".repeat(MAX_QUERY_CHARS + 1)));
        let errs = [e1, e2, e3, e4];
        (
            errs.iter().map(discriminant).collect(),
            errs.iter().map(sqlite_code).collect(),
        )
    }
    let (discs_a, codes_a) = run_once();
    let (discs_b, codes_b) = run_once();
    assert_eq!(discs_a, vec![0, 2, 2, 2], "first run must pin the sequence");
    assert_eq!(
        discs_a, discs_b,
        "repeated triggers must yield identical discriminants"
    );
    assert_eq!(
        codes_a, codes_b,
        "repeated triggers must yield identical sqlite kinds"
    );
}

/// INTENT: MR2 — a failure never poisons later success through the same handle:
/// an `Err` followed by a valid call is `Ok`, and the `Err` repeated is the
/// same discriminant. Errors are stateless rejections, not handle poison.
/// KILLS: handle-poison-on-error.
/// ABSORBS: none (kept solo).
/// OVERLAP: E4 oversize drill (E3 is ingress-level).
#[test]
fn error_does_not_poison_later_success_on_same_handle() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let searcher = mem_searcher(temp.path());
    let oversize = "q".repeat(MAX_QUERY_CHARS + 1);
    assert_eq!(
        discriminant(&err_of(searcher.search(&oversize))),
        2,
        "oversize query must fail as Other"
    );
    assert!(
        searcher.search("alpha").is_ok(),
        "valid search after a failure must still succeed"
    );
    assert_eq!(
        discriminant(&err_of(searcher.search_regex("(["))),
        2,
        "invalid regex must fail as Other"
    );
    assert!(
        searcher.search_regex("alp.*").is_ok(),
        "valid regex after a failure must still succeed"
    );
    assert_eq!(
        discriminant(&err_of(searcher.search(&oversize))),
        2,
        "repeated oversize query must fail identically"
    );
}

/// INTENT: MR4 — failed opens leave the filesystem byte-identical: no
/// half-created index file, no repaired garbage, no directory entries added or
/// removed. Snapshot before, trigger, snapshot after; the maps must match.
/// KILLS: half-created-index, garbage-repair-on-reject.
/// ABSORBS: none (kept solo).
#[test]
fn failed_opens_leave_filesystem_untouched() {
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut map = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let key = path.strip_prefix(dir).unwrap().to_path_buf();
                    map.insert(key, std::fs::read(&path).unwrap());
                }
            }
        }
        map
    }
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let garbage = write_garbage_db(temp.path(), "garbage.db");
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
    let before = snapshot(temp.path());
    let _ = err_of(Indexer::new(IndexOptions {
        root: temp.path().join("nosuch-root"),
        index_path: Some(temp.path().join("index.db")),
        ..IndexOptions::default()
    }));
    assert_eq!(
        snapshot(temp.path()),
        before,
        "missing-root open must not touch the filesystem"
    );
    let _ = err_of(Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(garbage),
        ..IndexOptions::default()
    }));
    assert_eq!(
        snapshot(temp.path()),
        before,
        "garbage-db open must not touch the filesystem"
    );
    let _ = err_of(IndexStore::open(
        temp.path(),
        Some(&blocker.join("index.db")),
    ));
    assert_eq!(
        snapshot(temp.path()),
        before,
        "blocked-dir open must not touch the filesystem"
    );
}

/// INTENT: MR4 — failed writes leave committed store state intact: a previously
/// indexed file keeps its exact hash and the line count is unchanged after an
/// over-max batch, a cancelled update, and a cancelled full index, all of which
/// report the same `Other` discriminant.
/// KILLS: committed-row-clobber-on-failure.
/// ABSORBS: none (kept solo).
#[test]
fn failed_writes_preserve_committed_rows() {
    let temp = TempDir::new().unwrap();
    let good = temp.path().join("good.rs");
    std::fs::write(&good, b"fn greet() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    indexer.index_all().expect("commit the good file");
    let committed_hash = indexer
        .store()
        .file_hash("good.rs")
        .expect("store readable");
    assert!(
        committed_hash.is_some(),
        "control: good file must be committed"
    );
    let committed_lines = indexer
        .store()
        .indexed_line_count()
        .expect("line count readable");
    let over_max = vec![good.clone(); MAX_INCREMENTAL_PATHS + 1];
    let e1 = err_of(indexer.update_paths(&over_max).map(|_| ()));
    let cancel = Arc::new(AtomicBool::new(true));
    indexer.set_cancel(Arc::clone(&cancel));
    let e2 = err_of(
        indexer
            .update_paths(std::slice::from_ref(&good))
            .map(|_| ()),
    );
    let e3 = err_of(indexer.index_all().map(|_| ()));
    for (op, err) in [
        ("over-max batch", &e1),
        ("cancelled update", &e2),
        ("cancelled index_all", &e3),
    ] {
        assert_eq!(discriminant(err), 2, "{op} must fail as Other, got {err:?}");
    }
    assert_eq!(
        indexer
            .store()
            .file_hash("good.rs")
            .expect("store readable"),
        committed_hash,
        "failed writes must preserve the committed hash"
    );
    assert_eq!(
        indexer
            .store()
            .indexed_line_count()
            .expect("line count readable"),
        committed_lines,
        "failed writes must preserve the committed line count"
    );
}

// ── E4 end-to-end drills (from error_api_pass4.rs) ──
// E4 end-to-end error drills for ast-sgrep-core.
//
// E1 pins taxonomy (each `StoreError` discriminant is reachable). E2 pins
// propagation (depth errors surface with the promised discriminant per
// layer). E3 pins relations over failures (metamorphic properties). E4 pins
// FULL FLOWS: populate a real file-backed store, inject a fault, run the
// complete search/open/status flow, assert the end-to-end discriminant —
// then prove clean state afterward (no half-writes: the committed hash is
// intact or the rebuild reproduces it, and a subsequent clean run
// succeeds).
//
// One drill per fault class (delete / corrupt × {garbage, truncated stump} /
// revoked perms / oversize query / dropped tables / missing root) plus one
// chained double-fault drill. Discriminants only (`matches!`); no
// message-text asserts.
//

/// INTENT: deleted db fails Other, reindex restores search+status+exact hash.
/// KILLS: heal-hash-divergence, fail-open-on-missing-db.
/// ABSORBS: none (kept solo).
#[test]
fn drill_deleted_index_search_fails_closed_then_reindex_recovers() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("index.db");
    let committed = populate(temp.path(), &db);
    remove_db_files(&db);
    assert!(!db.exists(), "fault injection must delete the index");

    let err = err_of(searcher_at(temp.path(), &db));
    assert_eq!(
        discriminant(&err),
        2,
        "deleted index through Searcher::new must fail as Other, got {err:?}"
    );

    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db.clone()),
        ..IndexOptions::default()
    })
    .expect("recovery indexer");
    indexer.index_all().expect("recovery index_all");
    drop(indexer);
    assert_clean_state(temp.path(), &db, &committed);
}

/// INTENT: corrupt index — garbage-overwrite AND 32-byte-stump triggers — fails
/// the full open flow as Database; the `force_reindex` recovery path
/// quarantines + rebuilds to clean state for both triggers (corrupt-drill anchor).
/// KILLS: quarantine-regression, heal-hash-divergence, truncation-tolerated.
/// ABSORBS: drill_truncated_index_search_fails_database_then_rebuild_recovers
/// (the stump is a second corruption trigger with identical discriminant +
/// recovery; folded as a trigger leg).
/// OVERLAP: recovery.
#[test]
fn drill_corrupt_index_open_fails_database_then_force_reindex_recovers() {
    for trigger in ["garbage-overwrite", "32-byte-stump"] {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("index.db");
        let committed = populate(temp.path(), &db);
        match trigger {
            "garbage-overwrite" => {
                write_garbage(&db);
            }
            _ => truncate_file(&db, 32),
        }

        let err = err_of(searcher_at(temp.path(), &db));
        assert_eq!(
            discriminant(&err),
            0,
            "{trigger} through Searcher::new must fail as Database, got {err:?}"
        );

        let mut indexer = Indexer::new(IndexOptions {
            root: temp.path().to_path_buf(),
            index_path: Some(db.clone()),
            force_reindex: true,
            ..IndexOptions::default()
        })
        .unwrap_or_else(|err| {
            panic!("{trigger}: force_reindex must quarantine and recover, got {err:?}")
        });
        indexer
            .index_all()
            .unwrap_or_else(|err| panic!("{trigger}: recovery index_all failed, got {err:?}"));
        drop(indexer);
        assert_clean_state(temp.path(), &db, &committed);
    }
}

/// INTENT: unreadable index (unix 000) fails the full open flow as Database
/// (sqlite open failure); restoring perms restores the live store
/// byte-identical — no rebuild, no half-writes.
/// KILLS: perm-error-misclassified, rebuild-on-transient.
/// ABSORBS: none (kept solo; distinct fault + no-rebuild recovery).
#[cfg(unix)]
#[test]
fn drill_unreadable_index_search_fails_database_then_perm_restore_recovers() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let db = temp.path().join("index.db");
    let committed = populate(temp.path(), &db);
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o000)).unwrap();

    let err = err_of(searcher_at(temp.path(), &db));
    assert_eq!(
        discriminant(&err),
        0,
        "unreadable index through Searcher::new must fail as Database, got {err:?}"
    );

    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_clean_state(temp.path(), &db, &committed);
}

/// INTENT: oversize query end-to-end — the full search flow rejects as `Other`
/// without touching store state; the same handle then serves a valid search,
/// status, and the unchanged committed hash.
/// KILLS: rejection-side-effect, handle-poison.
/// ABSORBS: none (kept solo).
/// OVERLAP: E3 no-poison (adds status+hash+repeat).
#[test]
fn drill_oversize_query_search_rejects_then_clean_query_succeeds() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("index.db");
    let committed = populate(temp.path(), &db);
    let searcher = searcher_at(temp.path(), &db).expect("searcher");

    let oversize = "q".repeat(MAX_QUERY_CHARS + 1);
    let err = err_of(searcher.search(&oversize));
    assert_eq!(
        discriminant(&err),
        2,
        "oversize query through full search must fail as Other, got {err:?}"
    );

    assert!(
        searcher.search("alpha").is_ok(),
        "valid search on the same handle must succeed after the rejection"
    );
    assert!(
        searcher.store().status().is_ok(),
        "status must succeed after the rejection"
    );
    assert_eq!(
        searcher.store().file_hash("a.rs").expect("hash readable"),
        committed,
        "rejected query must leave committed rows untouched"
    );
    assert_eq!(
        discriminant(&err_of(searcher.search(&oversize))),
        2,
        "repeated oversize query must reject identically"
    );
}

/// INTENT: dropped tables end-to-end — with `pattern_nodes` + `callers` gone
/// (both outside the readonly core-table guard, so the open itself succeeds),
/// the full pattern-search flow AND the status flow both fail as `Database`
/// (never `Ok`-empty); delete + rebuild restores clean state.
/// KILLS: Ok-empty-over-poison, status-Ok-over-poison.
/// ABSORBS: none (kept solo).
/// OVERLAP: E3 dropped-table legs (end-to-end + rebuild).
#[test]
fn drill_dropped_tables_search_and_status_fail_database_then_rebuild_recovers() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("index.db");
    let committed = populate(temp.path(), &db);
    // Response cache OFF: otherwise a cached `Ok` could answer without
    // touching the poisoned table — a legitimate cache layer, not the
    // fail-closed flow under test.
    let control = searcher_at(temp.path(), &db)
        .expect("searcher")
        .with_response_cache(false);
    assert!(
        control.search("pattern:greet_user").is_ok(),
        "control: pattern search on the healthy store must be Ok"
    );
    drop(control);
    // Fault injection needs a read-write handle: the searcher opens readonly.
    {
        let rw = IndexStore::open(temp.path(), Some(&db)).expect("rw fault-injection store");
        rw.connection()
            .execute_batch("DROP TABLE pattern_nodes; DROP TABLE callers")
            .unwrap();
        // Bundle-era total poison: defeat the read-bundle materialization too
        // (it legitimately serves correct rows past SQLite schema loss).
        ast_sgrep_testkit::remove_read_bundle(&db);
    }
    let searcher = searcher_at(temp.path(), &db)
        .expect("searcher")
        .with_response_cache(false);

    let via_search = err_of(searcher.search("pattern:greet_user").map(|_| ()));
    assert_eq!(
        discriminant(&via_search),
        0,
        "dropped tables through full search must fail as Database, got {via_search:?}"
    );
    let via_status = err_of(searcher.store().status().map(|_| ()));
    assert_eq!(
        discriminant(&via_status),
        0,
        "dropped tables through full status must fail as Database, got {via_status:?}"
    );
    drop(searcher);

    remove_db_files(&db);
    let committed_rebuilt = populate(temp.path(), &db);
    assert_eq!(
        committed_rebuilt, committed,
        "rebuilt store must reproduce the committed hash"
    );
    assert_clean_state(temp.path(), &db, &committed);
}

/// INTENT: missing root end-to-end — with the populated root renamed away, the
/// full write flow fails as `Io` (NotFound) and the full read flow fails closed
/// as `Other`; restoring the directory restores both flows byte-identical.
/// KILLS: flow-confusion(Io↔Other), restore-divergence.
/// ABSORBS: none (kept solo).
#[test]
fn drill_missing_root_flows_fail_then_restore_recovers() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let db = root.join("index.db");
    let committed = populate(&root, &db);
    let hidden = temp.path().join("root_hidden");
    std::fs::rename(&root, &hidden).unwrap();

    let via_indexer = err_of(Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(db.clone()),
        ..IndexOptions::default()
    }));
    match &via_indexer {
        StoreError::Io(io) => assert_eq!(
            io.kind(),
            std::io::ErrorKind::NotFound,
            "missing root through Indexer::new must surface NotFound, got {via_indexer:?}"
        ),
        other => panic!("missing root through Indexer::new must fail as Io, got {other:?}"),
    }
    let via_searcher = err_of(searcher_at(&root, &db));
    assert_eq!(
        discriminant(&via_searcher),
        2,
        "missing root through Searcher::new must fail as Other, got {via_searcher:?}"
    );

    std::fs::rename(&hidden, &root).unwrap();
    assert!(
        Indexer::new(IndexOptions {
            root: root.clone(),
            index_path: Some(db.clone()),
            ..IndexOptions::default()
        })
        .is_ok(),
        "Indexer::new must succeed once the root is restored"
    );
    assert_clean_state(&root, &db, &committed);
}

/// INTENT: chained double fault — a dropped `pattern_nodes` table PLUS an
/// oversize query in one flow. Query ingress fires first (`Other` — the depth
/// fault cannot mask input validation), the valid query then surfaces the
/// schema fault (`Database`); delete + rebuild restores clean state where the
/// oversize query still rejects deterministically (only precedence pin).
/// KILLS: ingress-after-store-touch, precedence-swap.
/// ABSORBS: none (kept solo).
#[test]
fn drill_chained_double_fault_query_precedence_then_schema_fault_then_recovery() {
    let temp = TempDir::new().unwrap();
    let db = temp.path().join("index.db");
    let committed = populate(temp.path(), &db);
    // Fault injection needs a read-write handle: the searcher opens readonly.
    {
        let rw = IndexStore::open(temp.path(), Some(&db)).expect("rw fault-injection store");
        rw.connection()
            .execute_batch("DROP TABLE pattern_nodes")
            .unwrap();
        // Bundle-era total poison: defeat the read-bundle materialization too.
        ast_sgrep_testkit::remove_read_bundle(&db);
    }
    let searcher = searcher_at(temp.path(), &db)
        .expect("searcher")
        .with_response_cache(false);

    // Fault 1 observed: ingress validation precedes any store touch.
    let oversize = format!("pattern:{}", "q".repeat(MAX_QUERY_CHARS));
    assert!(oversize.len() > MAX_QUERY_CHARS);
    let via_oversize = err_of(searcher.search(&oversize));
    assert_eq!(
        discriminant(&via_oversize),
        2,
        "oversize query over a poisoned store must still fail as Other, got {via_oversize:?}"
    );
    // Fault 2 observed: the valid query reaches the poisoned depth table.
    let via_valid = err_of(searcher.search("pattern:greet_user").map(|_| ()));
    assert_eq!(
        discriminant(&via_valid),
        0,
        "valid query over a poisoned store must fail as Database, got {via_valid:?}"
    );
    drop(searcher);

    remove_db_files(&db);
    let committed_rebuilt = populate(temp.path(), &db);
    assert_eq!(
        committed_rebuilt, committed,
        "rebuilt store must reproduce the committed hash"
    );
    let searcher = searcher_at(temp.path(), &db).expect("clean searcher");
    assert!(
        searcher.search("pattern:greet_user").is_ok(),
        "valid pattern search must succeed after recovery"
    );
    assert_eq!(
        discriminant(&err_of(searcher.search(&oversize))),
        2,
        "oversize query must still reject as Other after recovery"
    );
    assert_clean_state(temp.path(), &db, &committed);
}
