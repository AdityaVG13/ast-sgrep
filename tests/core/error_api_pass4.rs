//! E4 end-to-end error drills for ast-sgrep-core.
//!
//! E1 pins taxonomy (each `StoreError` discriminant is reachable). E2 pins
//! propagation (depth errors surface with the promised discriminant per
//! layer). E3 pins relations over failures (metamorphic properties). E4 pins
//! FULL FLOWS: populate a real file-backed store, inject a fault, run the
//! complete search/open/status flow, assert the end-to-end discriminant —
//! then prove clean state afterward (no half-writes: the committed hash is
//! intact or the rebuild reproduces it, and a subsequent clean run
//! succeeds).
//!
//! One drill per fault class (delete / corrupt × {garbage, truncated stump} /
//! revoked perms / oversize query / dropped tables / missing root) plus one
//! chained double-fault drill. Discriminants only (`matches!`); no
//! message-text asserts.

#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, StoreError, MAX_QUERY_CHARS};
use ast_sgrep_testkit::{err_of, truncate_file, write_garbage};
use error_testkit::{assert_clean_state, discriminant, populate, remove_db_files, searcher_at};
use tempfile::TempDir;

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
