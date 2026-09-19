//! Recovery relations: metamorphic convergence over crash histories (core).
//!
//! Consolidates `durable_recovery_pass3` (11 tests) into 5 intent-grouped
//! tests: rebuild idempotence, repair-then-verify, recovery determinism (+
//! order independence), interruption without residue, and recovered-vs-twin
//! search parity. The quarantine-monotonicity relation is pinned by the
//! chained crash drill in `recovery_drills` (fresh slot, first intact,
//! reconvergence across fault shapes) rather than duplicated here. All
//! assertions match discriminants, row counts, byte equality, or sorted hit
//! keys — never message text.
use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{compute_ann_fingerprint, load_semantic_ivf, save_semantic_ivf};
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, StoreError};
use ast_sgrep_testkit::{
    assert_torn, build_and_quiet, corpus_session, err_of, flip_bytes, home_names,
    isolated_index_session, quarantine_path, quiesced_db_bytes, search_parity_key,
    search_parity_keys, store_snapshot, truncate_file, IsolatedIndexSession,
};
use std::path::{Path, PathBuf};

const QUERIES: &[&str] = &["alpha_needle", "beta_needle", "gamma_caller"];
const UNKNOWN_QUERY: &str = "needle_no_such_token_xyz";

/// Sibling db path under the session temp root, sharing one corpus root.
/// File-local: a one-line path join, not worth a shared helper.
fn sibling_db(session: &IsolatedIndexSession, name: &str) -> PathBuf {
    session.index_path.parent().unwrap().join(name)
}

fn integrity(root: &Path, db: &Path) -> String {
    let store = IndexStore::open_readonly(root, Some(db)).unwrap();
    store.connection().query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap()
}

fn quick_check(root: &Path, db: &Path) -> String {
    let store = IndexStore::open_readonly(root, Some(db)).unwrap();
    store.connection().query_row("PRAGMA quick_check(1)", [], |r| r.get(0)).unwrap()
}

/// INTENT: rebuilding unchanged state converges — an incremental second pass
/// is a byte-identical no-op, forced rebuilds converge logically, and IVF
/// sidecar saves are byte-stable.
/// FACETS: incremental noop (0 files, identical bytes/snapshot/keys, no
/// quarantine), force-twice logical equality, IVF sibling+overwrite identity.
/// KILLS: gratuitous-rewrite, non-idempotent-rebuild,
/// nondeterministic-serialize mutants.
#[test]
fn rebuilds_are_idempotent() {
    // Facet: a second incremental pass over an unchanged corpus is a no-op.
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let db = session.index_path.clone();
        build_and_quiet(&root, &db, false, false);
        let bytes_once = quiesced_db_bytes(&db);
        let snap_once = store_snapshot(&root, &db);
        let hits_once = search_parity_keys(&root, &db, QUERIES, false);

        let mut indexer = Indexer::new(IndexOptions {
            root: root.clone(),
            index_path: Some(db.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap();
        let stats = indexer.index_all().unwrap();
        assert_eq!(stats.files_indexed, 0, "unchanged corpus must index nothing");
        assert_eq!(stats.files_failed, 0);
        indexer.store().checkpoint_wal().unwrap();
        drop(indexer);

        assert_eq!(quiesced_db_bytes(&db), bytes_once, "second incremental pass must be byte-identical");
        assert_eq!(store_snapshot(&root, &db), snap_once);
        assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_once);
        assert_eq!(integrity(&root, &db), "ok");
        assert!(home_names(db.parent().unwrap()).iter().all(|n| !n.contains(".corrupt")));
    }

    // Facet: two forced rebuilds converge logically (raw bytes excluded by
    // design: page layout varies run to run).
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let db = session.index_path.clone();
        build_and_quiet(&root, &db, true, false);
        let snap_once = store_snapshot(&root, &db);
        let hits_once = search_parity_keys(&root, &db, QUERIES, false);
        assert_eq!(integrity(&root, &db), "ok");

        build_and_quiet(&root, &db, true, false);
        assert_eq!(store_snapshot(&root, &db), snap_once);
        assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_once);
        assert_eq!(integrity(&root, &db), "ok");
    }

    // Facet: the same IVF payload saved twice is byte-identical and loads
    // back the same vectors from every copy.
    {
        let session = isolated_index_session();
        let dir = session.index_path.parent().unwrap().to_path_buf();
        let dim = 4usize;
        let vectors: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
        let fp = compute_ann_fingerprint(4, 4, dim, Some("test"), 1);

        let first = dir.join("first.ivf");
        let second = dir.join("second.ivf");
        save_semantic_ivf(&first, fp, dim, &vectors, &index).unwrap();
        save_semantic_ivf(&second, fp, dim, &vectors, &index).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());

        save_semantic_ivf(&first, fp, dim, &vectors, &index).unwrap();
        assert_eq!(
            std::fs::read(&first).unwrap(),
            std::fs::read(&second).unwrap(),
            "IVF overwrite must be byte-stable"
        );
        for path in [&first, &second] {
            assert_eq!(load_semantic_ivf(path, fp).unwrap().unwrap().vectors, vectors);
        }
    }
}

/// INTENT: torn images repair through forced rebuild to clean verification
/// with the pre-fault state fully restored and the torn inode preserved in
/// quarantine — identically for bit-flip and truncation fault shapes.
/// FACETS: mid-db page flip, half-truncation (same recovery path + asserts).
/// KILLS: lossy-repair mutants.
#[test]
fn repair_then_verify_restores_healthy_state() {
    for shape in ["page-flip", "truncate"] {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let db = session.index_path.clone();
        build_and_quiet(&root, &db, false, false);
        let snap_healthy = store_snapshot(&root, &db);
        let hits_healthy = search_parity_keys(&root, &db, QUERIES, false);

        match shape {
            "page-flip" => {
                let mut torn = quiesced_db_bytes(&db);
                let mid = torn.len() / 2;
                flip_bytes(&mut torn, mid, 64);
                std::fs::write(&db, &torn).unwrap();
            }
            _ => {
                let len = std::fs::metadata(&db).unwrap().len();
                truncate_file(&db, len / 2);
            }
        }
        assert_torn(&db);

        build_and_quiet(&root, &db, true, false);

        assert!(quarantine_path(&db, ".corrupt").is_file(), "{shape}: recovery must preserve the torn inode");
        assert_eq!(integrity(&root, &db), "ok");
        assert_eq!(quick_check(&root, &db), "ok");
        assert_eq!(integrity(&root, &db), "ok", "verification must be stable");
        assert_eq!(store_snapshot(&root, &db), snap_healthy, "{shape}: repair must restore the snapshot");
        assert_eq!(search_parity_keys(&root, &db, QUERIES, false), hits_healthy, "{shape}: repair must restore search keys");
    }
}

/// INTENT: identical faults recover identically, regardless of which copy
/// they strike or the order disjoint faults arrive in.
/// FACETS: same 64B flip on two copies (identical tear/quarantine/recovery),
/// disjoint flips F1;F2 vs F2;F1 (identical quarantine/recovery).
/// The byte-layer commutativity assert is deliberately NOT pinned: XOR flips
/// at disjoint offsets commute by construction, so asserting it would test
/// the fixture, not the product (catalog TAUTOLOGY-RISK).
/// KILLS: nondeterministic-recovery, order-dependent-recovery mutants.
#[test]
fn recovery_is_deterministic_and_order_independent() {
    // Facet: the same fault on two copies tears, quarantines, and recovers
    // identically, with the quarantine preserving the torn image exactly.
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        build_and_quiet(&root, &session.index_path, false, false);
        let base_bytes = quiesced_db_bytes(&session.index_path);

        let mut snaps = Vec::new();
        let mut hits = Vec::new();
        let mut quarantines = Vec::new();
        let mut torn_images = Vec::new();
        for tag in ["copy-a.db", "copy-b.db"] {
            let db = sibling_db(&session, tag);
            let mut torn = base_bytes.clone();
            let mid = torn.len() / 2;
            flip_bytes(&mut torn, mid, 64);
            std::fs::write(&db, &torn).unwrap();
            assert_torn(&db);
            torn_images.push(torn);

            build_and_quiet(&root, &db, true, false);
            quarantines.push(std::fs::read(quarantine_path(&db, ".corrupt")).unwrap());
            assert_eq!(integrity(&root, &db), "ok");
            snaps.push(store_snapshot(&root, &db));
            hits.push(search_parity_keys(&root, &db, QUERIES, false));
        }
        assert_eq!(torn_images[0], torn_images[1]);
        assert_eq!(quarantines[0], quarantines[1]);
        assert_eq!(quarantines[0], torn_images[0]);
        assert_eq!(snaps[0], snaps[1]);
        assert_eq!(hits[0], hits[1]);
    }

    // Facet: disjoint faults commute at recovery — both orders quarantine
    // identically and converge to identical state.
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        build_and_quiet(&root, &session.index_path, false, false);
        let base_bytes = quiesced_db_bytes(&session.index_path);
        let offsets = [base_bytes.len() / 3, 2 * base_bytes.len() / 3];

        let mut quarantines = Vec::new();
        let mut snaps = Vec::new();
        let mut hits = Vec::new();
        for (tag, order) in [("order-12.db", [0usize, 1usize]), ("order-21.db", [1usize, 0usize])] {
            let db = sibling_db(&session, tag);
            let mut torn = base_bytes.clone();
            for k in order {
                flip_bytes(&mut torn, offsets[k], 32);
            }
            std::fs::write(&db, &torn).unwrap();
            assert_torn(&db);

            build_and_quiet(&root, &db, true, false);
            quarantines.push(std::fs::read(quarantine_path(&db, ".corrupt")).unwrap());
            assert_eq!(integrity(&root, &db), "ok");
            snaps.push(store_snapshot(&root, &db));
            hits.push(search_parity_keys(&root, &db, QUERIES, false));
        }
        assert_eq!(quarantines[0], quarantines[1]);
        assert_eq!(snaps[0], snaps[1], "recovery must converge regardless of fault order");
        assert_eq!(hits[0], hits[1]);
    }
}

/// INTENT: work that never committed leaves no residue — resuming over the
/// interrupted state converges to exactly a fresh single-pass build.
/// FACETS: pre-cancelled index_all (Other + zero rows) then resume,
/// abandoned bulk tx (partial row rolled back) then build.
/// KILLS: residue, partial-commit-survives mutants.
#[test]
fn interrupted_work_resumes_to_fresh_state() {
    // Facet: a cancelled build commits nothing; resume equals fresh.
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let resumed_db = sibling_db(&session, "resumed.db");
        let fresh_db = sibling_db(&session, "fresh.db");

        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let indexer = Indexer::new(IndexOptions {
            root: root.clone(),
            index_path: Some(resumed_db.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap();
        let mut indexer = indexer;
        indexer.set_cancel(cancel.clone());
        let err = err_of(indexer.index_all());
        assert!(matches!(err, StoreError::Other(_)), "cancelled build must fail as Other, got {err:?}");
        assert_eq!(indexer.store().status().unwrap().file_count, 0);
        drop(indexer);

        cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        build_and_quiet(&root, &resumed_db, false, false);
        build_and_quiet(&root, &fresh_db, false, false);

        assert_eq!(store_snapshot(&root, &resumed_db), store_snapshot(&root, &fresh_db));
        assert_eq!(search_parity_keys(&root, &resumed_db, QUERIES, false), search_parity_keys(&root, &fresh_db, QUERIES, false));
        assert_eq!(integrity(&root, &resumed_db), "ok");
    }

    // Facet: an abandoned bulk tx rolls back; the next build equals fresh.
    {
        let session = corpus_session();
        let root = session.corpus_root.clone();
        let resumed_db = sibling_db(&session, "resumed.db");
        let fresh_db = sibling_db(&session, "fresh.db");

        {
            let store = IndexStore::open(&root, Some(&resumed_db)).unwrap();
            store.begin_bulk_tx().unwrap();
            let lines = [(1u32, "partial = 1\n".to_string())];
            store
                .upsert_file(UpsertFileInput {
                    rel_path: "partial.py",
                    language: Some("python"),
                    mtime_secs: 1,
                    mtime_nanos: 0,
                    content_hash: "rel-partial",
                    lines: &lines,
                    eol: "\n",
                    symbols: &[],
                    callers: &[],
                    imports: &[],
                    pattern_nodes: &[],
                    depth_truncated: false,
                    semantic_chunks: &[],
                    embed_semantic: false,
                    embed_backend: ast_sgrep_embed::EmbedPreference::Auto,
                })
                .unwrap();
            // Abandoned: dropped without commit, must roll back.
            drop(store);
        }

        build_and_quiet(&root, &resumed_db, false, false);
        build_and_quiet(&root, &fresh_db, false, false);

        let store = IndexStore::open_readonly(&root, Some(&resumed_db)).unwrap();
        assert!(store.file_hash("partial.py").unwrap().is_none(), "abandoned row must not survive");
        drop(store);
        assert_eq!(store_snapshot(&root, &resumed_db), store_snapshot(&root, &fresh_db));
        assert_eq!(search_parity_keys(&root, &resumed_db, QUERIES, false), search_parity_keys(&root, &fresh_db, QUERIES, false));
    }
}

/// INTENT: a corrupt-then-recovered index serves exactly what a
/// never-crashed twin serves — per-query keys identical, including whatever
/// the engine answers for an unknown token, pinned over real hits.
/// KILLS: recovery-residue, degraded-answer-loss mutants.
#[test]
fn recovered_index_matches_never_crashed_twin() {
    let session = corpus_session();
    let root = session.corpus_root.clone();
    let healthy_db = sibling_db(&session, "healthy.db");
    let recovered_db = sibling_db(&session, "recovered.db");

    build_and_quiet(&root, &healthy_db, false, false);
    build_and_quiet(&root, &recovered_db, false, false);

    let mut torn = quiesced_db_bytes(&recovered_db);
    let mid = torn.len() / 2;
    flip_bytes(&mut torn, mid, 64);
    std::fs::write(&recovered_db, &torn).unwrap();
    assert_torn(&recovered_db);
    build_and_quiet(&root, &recovered_db, true, false);

    assert_eq!(store_snapshot(&root, &recovered_db), store_snapshot(&root, &healthy_db));
    assert_eq!(search_parity_keys(&root, &recovered_db, QUERIES, false), search_parity_keys(&root, &healthy_db, QUERIES, false));
    assert_eq!(
        search_parity_key(&root, &recovered_db, UNKNOWN_QUERY, false),
        search_parity_key(&root, &healthy_db, UNKNOWN_QUERY, false),
        "unknown-token answers must agree across arms"
    );
    assert!(
        !search_parity_key(&root, &healthy_db, QUERIES[0], false).is_empty(),
        "parity must be over real hits, not mutual emptiness"
    );
}
