//! Recovery fault injection: torn bytes, locks, concurrency (core).
//!
//! Consolidates the active-fault half of `durable_recovery_pass2` into 2
//! intent-grouped tests: torn-byte loudness (splice + truncation sweep) and
//! concurrent-access correctness (peer visibility, snapshot isolation, stale
//! and live locks). Path confusion, atomic publish, orphan tmp, and the
//! frozen store live in `recovery_contracts`; the SIGKILL crash merges with
//! its serve-parity drill in `recovery_drills`. SQLite is lazy: page faults
//! surface when touched, so torn fixtures "refuse loudly" either at open or
//! at `integrity_check`, but never verify as silently healthy.
use ast_sgrep_core::{IndexStore, StoreError};
use ast_sgrep_testkit::{
    assert_torn, build_and_quiet, home_names, isolated_index_session, truncate_file,
    upsert_test_file,
};
use std::path::Path;
use std::time::{Duration, Instant};

/// Outcome of opening a torn fixture: loud refusal, an open handle whose full
/// verification reports corruption, or a normalized-healthy open (the
/// writable-open DDL rebuild path, which must serve zero torn rows).
/// File-local one-use comparator: only the torn-matrix test probes opens this
/// way, so it stays here rather than in shared testkit.
#[derive(Debug, PartialEq, Eq)]
enum Probe {
    Refused(&'static str),
    Dirty(String),
    Healthy { file_count: usize },
}

fn discriminant_name(err: &StoreError) -> &'static str {
    match err {
        StoreError::Database(_) => "Database",
        StoreError::Io(_) => "Io",
        StoreError::Other(_) => "Other",
    }
}

fn probe_open(root: &Path, db: &Path, readonly: bool) -> Probe {
    let result = if readonly {
        IndexStore::open_readonly(root, Some(db))
    } else {
        IndexStore::open(root, Some(db))
    };
    match result {
        Err(err) => Probe::Refused(discriminant_name(&err)),
        Ok(store) => {
            let detail: String = store
                .connection()
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap();
            if detail == "ok" {
                Probe::Healthy {
                    file_count: store.status().unwrap().file_count,
                }
            } else {
                Probe::Dirty(detail)
            }
        }
    }
}

/// INTENT: any torn store image refuses loudly (at open) or detects loudly
/// (at verification) — deterministically, with no quarantine and no
/// fabrication from the failed opens. Never silently healthy, never torn
/// rows served as authoritative.
/// FACETS: A-prefix/B-suffix splice (read-only + writable determinism),
/// truncation sweep over 0/1/header/mid/near-end with exact discriminants
/// where SQLite is strict.
/// KILLS: silent-healthy, quarantine-on-refusal, serve-torn-rows mutants.
#[test]
fn torn_bytes_never_silently_healthy() {
    // Facet: prefix of store A spliced to suffix of store B (a torn page-1).
    {
        let big_body = format!("def big():\n{}\n    return 1\n", "# pad\n".repeat(5000));
        let session_a = isolated_index_session();
        for (rel, body) in [
            ("a0.py", "def a0():\n    return 0\n".to_string()),
            ("a1.py", "def a1():\n    return 1\n".to_string()),
            ("a2.py", "def a2():\n    return 2\n".to_string()),
            ("a3.py", "def a3():\n    return 3\n".to_string()),
            ("big.py", big_body),
        ] {
            session_a.write(rel, body.as_str());
        }
        build_and_quiet(&session_a.corpus_root, &session_a.index_path, false, false);
        let session_b = isolated_index_session();
        session_b.write("b.py", "x = 1\n");
        build_and_quiet(&session_b.corpus_root, &session_b.index_path, false, false);

        let a_bytes = std::fs::read(&session_a.index_path).unwrap();
        let b_bytes = std::fs::read(&session_b.index_path).unwrap();
        // Structurally divergent stores: near-identical stores splice into an
        // accidentally coherent db. Different sizes tear by construction.
        assert!(
            a_bytes.len() > b_bytes.len() + 4096,
            "splice sources must diverge in size"
        );
        let min = b_bytes.len();
        assert!(min > 4096, "fixtures must span several pages");
        let mut splice = a_bytes[..2000].to_vec();
        splice.extend_from_slice(&b_bytes[2000..min]);

        // Fresh session per probe: a writable open may normalize in place, so
        // each outcome is measured against pristine torn bytes.
        let probe_fresh = |readonly: bool| {
            let probe = isolated_index_session();
            std::fs::write(&probe.index_path, &splice).unwrap();
            assert_torn(&probe.index_path);
            let outcome = probe_open(&probe.corpus_root, &probe.index_path, readonly);
            (outcome, probe)
        };

        // Read-only opens never normalize: refuse or detect, deterministically.
        let (first, _) = probe_fresh(true);
        let (second, ro_kept) = probe_fresh(true);
        assert_eq!(
            first, second,
            "read-only splice outcome must be deterministic"
        );
        assert!(
            matches!(first, Probe::Refused(_) | Probe::Dirty(_)),
            "read-only splice must refuse or detect corruption, got {first:?}"
        );
        if let Probe::Refused(name) = &first {
            assert_eq!(*name, "Database");
        }
        assert_eq!(std::fs::read(&ro_kept.index_path).unwrap(), splice);

        // Writable opens refuse, detect, or normalize to an empty healthy
        // store — never serve torn rows as authoritative.
        let (first, _) = probe_fresh(false);
        let (second, rw_kept) = probe_fresh(false);
        assert_eq!(
            first, second,
            "writable splice outcome must be deterministic"
        );
        match &first {
            Probe::Refused(name) => assert_eq!(*name, "Database"),
            Probe::Dirty(_) => {}
            Probe::Healthy { file_count } => {
                assert_eq!(
                    *file_count, 0,
                    "normalized splice must serve zero torn rows"
                )
            }
        }

        for kept in [&ro_kept, &rw_kept] {
            let after = home_names(kept.index_path.parent().unwrap());
            assert!(
                after.iter().all(|n| !n.contains(".corrupt")),
                "failed splice opens must not quarantine: {after:?}"
            );
            assert!(
                after
                    .iter()
                    .all(|n| n != "lexical.db" && n != "semantic.ivf"),
                "failed splice opens must not fabricate sidecars: {after:?}"
            );
        }
    }

    // Facet: 0/1/header/mid/near-end tears refuse loudly, with exact
    // discriminants where SQLite is strict (0→Other, torn-header→Database).
    {
        let src = isolated_index_session();
        for (rel, body) in [
            ("one.py", "def one():\n    return 1\n"),
            ("two.py", "def two():\n    return 2\n"),
            ("three.py", "def three():\n    return 3\n"),
            ("four.py", "def four():\n    return 4\n"),
        ] {
            src.write(rel, body);
        }
        build_and_quiet(&src.corpus_root, &src.index_path, false, false);
        let full = std::fs::read(&src.index_path).unwrap();
        // Near-end tears a whole live page: a 1-byte-short tail zero-fills
        // and verifies clean (no page checksums), so that offset is not a
        // fault. With an empty freelist every page is live.
        let conn = rusqlite::Connection::open(&src.index_path).unwrap();
        let page_count: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap();
        let freelist: i64 = conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .unwrap();
        assert!(page_count >= 5, "fixture must span several pages");
        assert_eq!(freelist, 0, "fixture must have no dead pages");
        let page_size = full.len() / page_count as usize;
        let mid = full.len() / 2;
        let near_end = full.len() - page_size;

        // The zero-byte writable arm is contracts-owned (empty store
        // initializes in place) and is not re-pinned here.
        for (label, len) in [
            ("zero", 0usize),
            ("one-byte", 1usize),
            ("header", 64usize),
            ("mid", mid),
            ("near-end", near_end),
        ] {
            let probe = isolated_index_session();
            let lay_torn = || {
                std::fs::write(&probe.index_path, &full).unwrap();
                truncate_file(&probe.index_path, len as u64);
            };
            lay_torn();
            if len > 100 {
                assert_torn(&probe.index_path);
            }

            match (
                label,
                probe_open(&probe.corpus_root, &probe.index_path, true),
            ) {
                ("zero" | "one-byte", outcome) => assert_eq!(
                    outcome,
                    Probe::Refused("Other"),
                    "{label} read-only must refuse as Other"
                ),
                ("header", outcome) => assert_eq!(
                    outcome,
                    Probe::Refused("Database"),
                    "{label} read-only must refuse as Database"
                ),
                (_, Probe::Refused(name)) => assert_eq!(name, "Database"),
                (_, Probe::Dirty(_)) => {}
                (_, outcome) => panic!("{label} read-only must never verify healthy: {outcome:?}"),
            }
            assert_eq!(std::fs::read(&probe.index_path).unwrap(), full[..len]);

            // Fresh bytes: the read-only probe is side-effect free, but the
            // writable probe may normalize, so re-lay pristine torn bytes.
            lay_torn();
            match (
                label,
                probe_open(&probe.corpus_root, &probe.index_path, false),
            ) {
                ("zero", _) => {}
                ("one-byte", outcome) => assert_eq!(
                    outcome,
                    Probe::Healthy { file_count: 0 },
                    "one-byte writable must normalize to an empty store"
                ),
                ("header", outcome) => {
                    assert_eq!(outcome, Probe::Refused("Database"));
                    assert_eq!(std::fs::read(&probe.index_path).unwrap(), full[..len]);
                }
                (_, Probe::Refused(name)) => assert_eq!(name, "Database"),
                (_, Probe::Dirty(_)) => {}
                (_, Probe::Healthy { file_count }) => {
                    assert_eq!(file_count, 0, "{label} writable must serve zero torn rows")
                }
            }
            let after = home_names(probe.index_path.parent().unwrap());
            assert!(
                after.iter().all(|n| !n.contains(".corrupt")),
                "{label}: failed opens must not quarantine: {after:?}"
            );
        }
    }
}

/// INTENT: concurrent handles are peers and readers always see committed
/// snapshots — bidirectional visibility with no wedge, torn reads impossible,
/// stray lockfiles ignored, live locks block-then-proceed.
/// FACETS: two-writer peer visibility, read-during-bulk-tx isolation, stale
/// *.lock ignored unconsumed, live BEGIN IMMEDIATE blocks then proceeds.
/// KILLS: fork/wedge, torn-read, lockfile-honoring, fail-fast-on-locked.
#[test]
fn concurrent_access_and_snapshot_isolation() {
    let session = isolated_index_session();
    let temp = session.index_path.parent().unwrap().to_path_buf();

    // Facet: two writable handles observe each other's commits both ways.
    {
        let db = session.index_path.clone();
        drop(IndexStore::open(&session.corpus_root, Some(&db)).unwrap());
        let first = IndexStore::open(&session.corpus_root, Some(&db)).unwrap();
        let second = IndexStore::open(&session.corpus_root, Some(&db)).unwrap();
        upsert_test_file(&first, "from-first.py", "a = 1\n".to_string(), "c1");
        assert_eq!(
            second.file_hash("from-first.py").unwrap().as_deref(),
            Some("c1"),
            "second opener must see the first's commit"
        );
        upsert_test_file(&second, "from-second.py", "b = 2\n".to_string(), "c2");
        assert_eq!(
            first.file_hash("from-second.py").unwrap().as_deref(),
            Some("c2"),
            "first opener must see the second's commit"
        );
        assert_eq!(first.status().unwrap().file_count, 2);
        assert_eq!(second.status().unwrap().file_count, 2);
    }

    // Facet: a read-only open during another handle's uncommitted bulk write
    // serves the last committed snapshot; after commit the rows appear.
    {
        let db = temp.join("snap.db");
        let writer = IndexStore::open(&session.corpus_root, Some(&db)).unwrap();
        upsert_test_file(&writer, "committed.py", "c = 1\n".to_string(), "s1");
        writer.begin_bulk_tx().unwrap();
        upsert_test_file(&writer, "uncommitted.py", "u = 2\n".to_string(), "s2");

        let reader = IndexStore::open_readonly(&session.corpus_root, Some(&db)).unwrap();
        assert_eq!(reader.status().unwrap().file_count, 1);
        assert!(reader.file_hash("uncommitted.py").unwrap().is_none());
        drop(reader);

        writer.commit_bulk_tx().unwrap();
        let reader = IndexStore::open_readonly(&session.corpus_root, Some(&db)).unwrap();
        assert_eq!(reader.status().unwrap().file_count, 2);
        assert!(reader.file_hash("uncommitted.py").unwrap().is_some());
    }

    // Facet: stray *.lock files are not part of the contract — planted garbage
    // neither blocks nor is consumed — while a live SQLite write lock blocks
    // a second writer, which proceeds after rollback (never wedges).
    {
        let db = temp.join("locks.db");
        drop(IndexStore::open(&session.corpus_root, Some(&db)).unwrap());
        let lock_a = db.with_extension("db.lock");
        let lock_b = temp.join(".asgrep-index.lock");
        std::fs::write(&lock_a, b"pid 99999\n").unwrap();
        std::fs::write(&lock_b, b"stale\n").unwrap();
        let store = IndexStore::open(&session.corpus_root, Some(&db)).unwrap();
        assert_eq!(store.status().unwrap().file_count, 0);
        upsert_test_file(&store, "l.py", "x = 1\n".to_string(), "lock-h");
        assert_eq!(std::fs::read(&lock_a).unwrap(), b"pid 99999\n");
        assert_eq!(std::fs::read(&lock_b).unwrap(), b"stale\n");
        drop(store);

        let holder = rusqlite::Connection::open(&db).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        let started = Instant::now();
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                let second = IndexStore::open(&session.corpus_root, Some(&db)).unwrap();
                upsert_test_file(&second, "w.py", "y = 2\n".to_string(), "live-w");
            });
            // Bounded blocking probe (not a logic sleep): hold the lock past
            // the writer's arrival so elapsed time proves it blocked.
            std::thread::sleep(Duration::from_millis(300));
            holder.execute_batch("ROLLBACK").unwrap();
            writer.join().unwrap();
        });
        assert!(started.elapsed() >= Duration::from_millis(200));
        let store = IndexStore::open(&session.corpus_root, Some(&db)).unwrap();
        assert!(store.file_hash("w.py").unwrap().is_some());
    }
}
