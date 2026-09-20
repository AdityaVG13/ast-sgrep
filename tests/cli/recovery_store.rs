//! Recovery CLI: authoritative-store faults (corrupt/torn/deleted/empty/locked/killed).
//!
//! Covers catalog `tests/catalog/recovery.md` CLI rows: readers fail-closed,
//! writer boundary (garbage+torn), deleted/zero-length cold-start, chained crash,
//! quarantine stability/uniqueness/cycle-determinism, all four lock shapes, and
//! SIGKILL mid-rewrite (next-run contract + serve parity + twin equality).
//! Every assertion keys on documented discriminants (exit code, machine envelope,
//! status/search/outline shapes, durable bytes) — never message text.

#[cfg(unix)]
use ast_sgrep_testkit::kill9;
use ast_sgrep_testkit::{
    assert_failure_envelope, assert_serve_parity, assert_success, capture_baseline, run_in,
    run_index, run_reindex, run_search, run_status, search_answer_keys, seed_big_project,
    seed_project, status_snapshot, truncate_file, KillOnDrop, OUTLINE_PATH, QUERY, RUN_TIMEOUT,
};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

/// Exact binary path via the compile-time env cargo sets for THIS test target.
/// `env!` must expand here (it is unset inside the testkit dependency); the
/// resolved path is passed as `bin` to the testkit runners.
fn asgrep() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// INTENT: a corrupt authoritative db (garbage or torn bytes) is refused loudly,
/// quarantined with its bytes preserved, healed by reindex, and serves the
/// pre-crash baseline identically.
/// FACETS: garbage arm + torn arm; refuse across status/outline/search/incremental
/// index (exit 2, inode untouched, no quarantine on refuse); reindex heals
/// (quarantine holds injected bytes); status+search+outline equal baseline.
/// KILLS: silent-empty/walk-downgrade, refuse-skip, quarantine-skip, lossy-recovery.
/// COVERS: readers_fail_closed_on_corrupt_index_db,
/// incremental_index_refuses_corrupt_db_while_reindex_heals,
/// fault_torn_truncated_index_db_refused_then_healed,
/// relation_corrupt_recover_verify_roundtrip_index,
/// drill_corrupt_index_db_serve_parity, drill_torn_truncated_db_serve_parity.
#[test]
fn corrupt_store_refuses_quarantines_heals_and_serves() {
    for shape in ["garbage", "torn"] {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let baseline = capture_baseline(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            1,
        );

        // CRASH: plant the fault; keep the injected bytes for evidence checks.
        let injected = if shape == "garbage" {
            fs::write(&index, b"RECOVERY-STORE-CORRUPT-GARBAGE-0001").unwrap();
            fs::read(&index).unwrap()
        } else {
            let len = fs::metadata(&index).unwrap().len();
            assert!(len > 1, "fixture db must be truncatable");
            truncate_file(&index, len / 2);
            assert!(fs::metadata(&index).unwrap().len() < len);
            fs::read(&index).unwrap()
        };
        let quarantine = index.with_file_name("idx.db.corrupt");

        // OBSERVE: every reader refuses; the refusing writer moves nothing.
        let status = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                &index_s,
                "--no-embed",
                "--json",
                "status",
                &root_s,
            ],
            &[],
        );
        assert_failure_envelope(&status, "status", 2, "operational");
        let outline = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                &index_s,
                "--no-embed",
                "--json",
                "outline",
                OUTLINE_PATH,
                &root_s,
            ],
            &[],
        );
        assert_failure_envelope(&outline, "outline", 2, "operational");
        let search = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                &index_s,
                "--no-embed",
                "--no-auto-index",
                "--json",
                "search",
                QUERY,
                &root_s,
            ],
            &[],
        );
        assert_failure_envelope(&search, "search", 2, "operational");
        let refused = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                &index_s,
                "--no-embed",
                "--json",
                "index",
                &root_s,
            ],
            &[],
        );
        assert_failure_envelope(&refused, "index", 2, "operational");
        assert!(
            !quarantine.exists(),
            "{shape}: the refusing writer must not move the corrupt inode"
        );
        assert_eq!(
            fs::read(&index).unwrap(),
            injected,
            "{shape}: the refused run must leave the corrupt inode untouched"
        );

        // RECOVER: reindex heals and quarantines the injected bytes.
        let healed = run_reindex(&asgrep(), &temp, &root, &index, &[]);
        assert_eq!(healed["files_indexed"], 1);
        assert_eq!(
            fs::read(&quarantine).unwrap(),
            injected,
            "{shape}: quarantine must preserve the injected bytes"
        );

        // SERVE: identical to the pre-crash baseline.
        assert_serve_parity(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            &baseline,
            &format!("post-{shape}-store"),
        );
    }
}

/// INTENT: absent or empty authoritative state cold-starts via plain `index`
/// (no quarantine: nothing corrupt to preserve), alone and chained after a
/// corrupt→heal cycle, and serves the ORIGINAL baseline.
/// FACETS: deleted-db arm (refuse→cold-start→serve), zero-length arm
/// (cold-start→serve), chained arm (corrupt→heal→delete→heal→serve with crash-1
/// evidence surviving and no new slot taken).
/// KILLS: cold-start, chain-residue, quarantine-on-empty mutants.
/// COVERS: fault_zero_length_index_db_rebuilt_as_cold_start,
/// drill_deleted_index_db_serve_parity, drill_chained_double_crash_serve_parity.
#[test]
fn deleted_or_empty_store_cold_starts_and_serves() {
    // Arm A: deleted db — refuse proves the crash landed, index cold-starts.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let baseline = capture_baseline(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            1,
        );
        fs::remove_file(&index).unwrap();
        assert!(!index.exists(), "the drill must really delete the database");
        let refused = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                &index_s,
                "--no-embed",
                "--json",
                "status",
                &root_s,
            ],
            &[],
        );
        assert_failure_envelope(&refused, "status", 2, "operational");
        let rebuilt = run_index(&asgrep(), &temp, &root, &index);
        assert_eq!(rebuilt["files_indexed"], 1);
        assert!(
            !index.with_file_name("idx.db.corrupt").exists(),
            "a cold-start rebuild must not take a quarantine"
        );
        assert_serve_parity(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            &baseline,
            "post-deleted-db",
        );
    }

    // Arm B: zero-length db — SQLite treats it as empty; index cold-starts.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let baseline = capture_baseline(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            1,
        );
        fs::write(&index, b"").unwrap();
        assert_eq!(fs::metadata(&index).unwrap().len(), 0);
        let rebuilt = run_index(&asgrep(), &temp, &root, &index);
        assert_eq!(rebuilt["files_indexed"], 1);
        assert!(
            !index.with_file_name("idx.db.corrupt").exists(),
            "a cold-start rebuild must not take a quarantine"
        );
        assert_serve_parity(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            &baseline,
            "post-zero-length-db",
        );
    }

    // Arm C: chained corrupt→heal→delete→heal serves the ORIGINAL baseline.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let quarantine = index.with_file_name("idx.db.corrupt");
        let baseline = capture_baseline(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            1,
        );
        let garbage1 = b"RECOVERY-STORE-CHAIN-CRASH1-0002".to_vec();
        fs::write(&index, &garbage1).unwrap();
        let refused = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                &index_s,
                "--no-embed",
                "--json",
                "status",
                &root_s,
            ],
            &[],
        );
        assert_failure_envelope(&refused, "status", 2, "operational");
        let healed = run_reindex(&asgrep(), &temp, &root, &index, &[]);
        assert_eq!(healed["files_indexed"], 1);
        assert_eq!(
            fs::read(&quarantine).unwrap(),
            garbage1,
            "crash #1 evidence must be quarantined"
        );
        fs::remove_file(&index).unwrap();
        let refused = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                &index_s,
                "--no-embed",
                "--json",
                "status",
                &root_s,
            ],
            &[],
        );
        assert_failure_envelope(&refused, "status", 2, "operational");
        let rebuilt = run_index(&asgrep(), &temp, &root, &index);
        assert_eq!(rebuilt["files_indexed"], 1);
        assert_eq!(
            fs::read(&quarantine).unwrap(),
            garbage1,
            "crash #1 evidence must survive crash #2"
        );
        assert!(
            !index.with_file_name("idx.db.corrupt.1").exists(),
            "the cold start must not take a new quarantine slot"
        );
        assert_serve_parity(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            OUTLINE_PATH,
            &["--no-embed"],
            &[],
            &baseline,
            "post-chained-crashes",
        );
    }
}

/// INTENT: quarantine evidence is complete (unique slot per fault, never
/// overwrites), stable (healthy rebuilds neither rewrite it nor take new slots),
/// and correctly paired across repeated fault cycles.
/// FACETS: stability (second rebuild changes nothing), uniqueness (occupied slot
/// forces `.corrupt.1`, sentinel intact), 3-cycle determinism (distinct garbage
/// per cycle converges identically; slot i holds garbage i).
/// KILLS: overwrite-evidence, rewrite-evidence, cycle-drift/slot-mixup mutants.
/// COVERS: relation_quarantine_bytes_stable_no_new_slot_on_healthy_rebuild,
/// fault_occupied_quarantine_slot_allocates_unique_quarantine,
/// relation_repeated_db_fault_cycles_deterministic.
#[test]
fn quarantine_evidence_stable_unique_and_deterministic() {
    // Stability: a healthy rebuild neither rewrites the quarantine nor takes a slot.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let quarantine = index.with_file_name("idx.db.corrupt");
        let garbage = b"RECOVERY-STORE-QUARANTINE-0003".to_vec();
        fs::write(&index, &garbage).unwrap();
        run_reindex(&asgrep(), &temp, &root, &index, &[]);
        assert_eq!(fs::read(&quarantine).unwrap(), garbage);
        let status_after_heal =
            status_snapshot(&run_status(&asgrep(), &temp, &root_s, &index_s, &[]));
        run_reindex(&asgrep(), &temp, &root, &index, &[]);
        assert_eq!(
            fs::read(&quarantine).unwrap(),
            garbage,
            "the second rebuild must not rewrite the quarantine"
        );
        assert!(
            !index.with_file_name("idx.db.corrupt.1").exists(),
            "a healthy rebuild must not take a new quarantine slot"
        );
        assert_eq!(
            status_snapshot(&run_status(&asgrep(), &temp, &root_s, &index_s, &[])),
            status_after_heal,
            "status must be stable across the second rebuild"
        );
    }

    // Uniqueness: an occupied slot forces a unique quarantine; sentinel intact.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let root_s = root.to_str().unwrap();
        let index_s = index.to_str().unwrap();
        let sentinel = b"SENTINEL-PRIOR-QUARANTINE".to_vec();
        fs::write(index.with_file_name("idx.db.corrupt"), &sentinel).unwrap();
        let garbage = b"RECOVERY-STORE-FRESH-CORRUPT-0004".to_vec();
        fs::write(&index, &garbage).unwrap();
        let healed = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                index_s,
                "--no-embed",
                "--json",
                "reindex",
                root_s,
            ],
            &[],
        );
        let healed = assert_success(&healed, "reindex");
        assert_eq!(healed["files_indexed"], 1);
        assert_eq!(
            fs::read(index.with_file_name("idx.db.corrupt")).unwrap(),
            sentinel,
            "the prior quarantine is evidence and must not be overwritten"
        );
        assert_eq!(
            fs::read(index.with_file_name("idx.db.corrupt.1")).unwrap(),
            garbage,
            "the fresh corruption must land in a unique quarantine"
        );
    }

    // Cycle determinism: 3 corrupt→reindex cycles converge identically.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let baseline_status =
            status_snapshot(&run_status(&asgrep(), &temp, &root_s, &index_s, &[]));
        let baseline_answers = search_answer_keys(&run_search(
            &asgrep(),
            &temp,
            &root_s,
            &index_s,
            QUERY,
            &["--no-embed"],
            &[],
        ));
        assert!(!baseline_answers.is_empty());
        for cycle in 0..3u32 {
            let garbage = format!("RECOVERY-STORE-CYCLE-{cycle:04}-DISTINCT").into_bytes();
            fs::write(&index, &garbage).unwrap();
            let healed = run_reindex(&asgrep(), &temp, &root, &index, &[]);
            assert_eq!(healed["files_indexed"], 1, "cycle {cycle}: files_indexed");
            assert_eq!(
                status_snapshot(&run_status(&asgrep(), &temp, &root_s, &index_s, &[])),
                baseline_status,
                "cycle {cycle}: status must converge to baseline"
            );
            assert_eq!(
                search_answer_keys(&run_search(
                    &asgrep(),
                    &temp,
                    &root_s,
                    &index_s,
                    QUERY,
                    &["--no-embed"],
                    &[]
                )),
                baseline_answers,
                "cycle {cycle}: answers must converge to baseline"
            );
            let slot = if cycle == 0 {
                index.with_file_name("idx.db.corrupt")
            } else {
                index.with_file_name(format!("idx.db.corrupt.{cycle}"))
            };
            assert_eq!(
                fs::read(&slot).unwrap(),
                garbage,
                "cycle {cycle}: slot {} must hold this cycle's bytes",
                slot.display()
            );
            assert!(
                !index
                    .with_file_name(format!("idx.db.corrupt.{}", cycle + 1))
                    .exists(),
                "cycle {cycle}: no slot beyond the current one may exist"
            );
        }
    }
}

/// INTENT: the recovery lock serializes rebuilds without ever corrupting or
/// wedge-blocking them: stale state is ignored, a live holder blocks (never a
/// fast verdict under it), and an unusable lock path fails closed.
/// FACETS: static debris (garbage lock + stale tmps), dead-pid record (unix),
/// live flock holder blocking then releasing (unix), lock path as directory.
/// KILLS: lock-content-honoring, pid-honoring, lock-ignoring,
/// proceed-without-lock mutants.
/// COVERS: stale_lock_and_temp_crash_debris_do_not_block_reindex,
/// fault_stale_dead_pid_lock_does_not_block_reindex,
/// fault_live_lock_holder_blocks_or_refuses_never_corrupts,
/// fault_lock_path_occupied_by_directory_fails_closed.
#[test]
fn recovery_locking_serializes_without_corruption() {
    // Static debris: garbage lock + stale tmps never block reindex.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        fs::write(&index, b"RECOVERY-STORE-LOCK-DEBRIS-0005").unwrap();
        fs::write(
            index.with_file_name("idx.db.reindex.lock"),
            "stale-lock-garbage",
        )
        .unwrap();
        fs::write(
            temp.path().join(".writer_generation.999999.0.tmp"),
            "stale-stamp-tmp",
        )
        .unwrap();
        fs::write(
            temp.path().join(".semantic.ivf.999999.tmp"),
            "stale-ivf-tmp",
        )
        .unwrap();
        let healed = run_reindex(&asgrep(), &temp, &root, &index, &[]);
        assert_eq!(healed["files_indexed"], 1);
        assert!(
            index.with_file_name("idx.db.corrupt").is_file(),
            "quarantine must still be taken past stale debris"
        );
    }

    // Obstruction: lock path as a directory fails closed, untouched; removal heals.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let garbage = b"RECOVERY-STORE-LOCK-OBSTRUCT-0006".to_vec();
        fs::write(&index, &garbage).unwrap();
        let lock_dir = index.with_file_name("idx.db.reindex.lock");
        let quarantine = index.with_file_name("idx.db.corrupt");
        fs::create_dir(&lock_dir).unwrap();
        let refused = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                index.to_str().unwrap(),
                "--no-embed",
                "--json",
                "reindex",
                root.to_str().unwrap(),
            ],
            &[],
        );
        assert_failure_envelope(&refused, "reindex", 2, "operational");
        assert_eq!(
            fs::read(&index).unwrap(),
            garbage,
            "the refused rebuild must leave the corrupt inode untouched"
        );
        assert!(
            !quarantine.exists(),
            "the refused rebuild must not take a quarantine"
        );
        fs::remove_dir(&lock_dir).unwrap();
        let healed = run_reindex(&asgrep(), &temp, &root, &index, &[]);
        assert_eq!(healed["files_indexed"], 1);
        assert!(quarantine.is_file());
    }

    #[cfg(unix)]
    {
        // Dead-pid record: locking is by flock, not pid — a stale record is ignored.
        {
            let (temp, root, index) = seed_project();
            run_index(&asgrep(), &temp, &root, &index);
            fs::write(&index, b"RECOVERY-STORE-LOCK-DEADPID-0007").unwrap();
            let mut probe = Command::new("sh")
                .arg("-c")
                .arg("exit 0")
                .spawn()
                .expect("spawn pid probe");
            let dead_pid = probe.id();
            assert!(probe.wait().expect("wait probe").success());
            fs::write(
                index.with_file_name("idx.db.reindex.lock"),
                format!("pid={dead_pid}\n"),
            )
            .unwrap();
            let healed = run_reindex(&asgrep(), &temp, &root, &index, &[]);
            assert_eq!(healed["files_indexed"], 1);
            assert!(
                index.with_file_name("idx.db.corrupt").is_file(),
                "quarantine must still be taken past the stale pid record"
            );
        }

        // Live holder: reindex blocks under the flock, then heals once released.
        {
            let (temp, root, index) = seed_project();
            run_index(&asgrep(), &temp, &root, &index);
            let garbage = b"RECOVERY-STORE-LOCK-LIVE-0008".to_vec();
            fs::write(&index, &garbage).unwrap();
            let root_s = root.to_str().unwrap().to_owned();
            let index_s = index.to_str().unwrap().to_owned();
            let lock_path = index.with_file_name("idx.db.reindex.lock");
            let ready = temp.path().join("holder-ready");
            let mut holder = KillOnDrop(Some(
                Command::new("python3")
                    .args([
                        "-c",
                        "import fcntl, sys, time; f = open(sys.argv[1], 'w'); \
                         fcntl.flock(f.fileno(), fcntl.LOCK_EX); \
                         open(sys.argv[2], 'w').write('locked'); time.sleep(60)",
                        lock_path.to_str().unwrap(),
                        ready.to_str().unwrap(),
                    ])
                    .current_dir(temp.path())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("spawn flock holder"),
            ));
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.is_file() {
                assert!(
                    Instant::now() < deadline,
                    "the flock holder never acquired the lock"
                );
                assert!(
                    holder
                        .child()
                        .try_wait()
                        .expect("try_wait holder")
                        .is_none(),
                    "the flock holder died before acquiring the lock"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
            let mut healing = KillOnDrop(Some(
                Command::new(asgrep())
                    .args([
                        "--index-path",
                        &index_s,
                        "--no-embed",
                        "--json",
                        "reindex",
                        &root_s,
                    ])
                    .env("NO_COLOR", "1")
                    .current_dir(temp.path())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("spawn blocked reindex"),
            ));
            std::thread::sleep(Duration::from_secs(3));
            assert!(
                healing
                    .child()
                    .try_wait()
                    .expect("try_wait reindex")
                    .is_none(),
                "reindex must block on the live holder, not exit under it"
            );
            let _ = holder.child().kill();
            let _ = holder.child().wait();
            let deadline = Instant::now() + RUN_TIMEOUT;
            loop {
                match healing.child().try_wait().expect("try_wait reindex") {
                    Some(_) => break,
                    None => {
                        assert!(
                            Instant::now() < deadline,
                            "reindex never completed after the holder died"
                        );
                        std::thread::sleep(Duration::from_millis(25));
                    }
                }
            }
            let output = healing.take().wait_with_output().expect("collect reindex");
            let value = assert_success(&output, "reindex");
            assert_eq!(value["files_indexed"], 1);
            assert_eq!(
                fs::read(index.with_file_name("idx.db.corrupt")).unwrap(),
                garbage,
                "the blocked-then-released reindex must quarantine the corrupt bytes"
            );
            let status = run_status(&asgrep(), &temp, &root_s, &index_s, &[]);
            assert_eq!(status["file_count"], 1);
        }
    }
}

/// INTENT: a SIGKILL mid-rewrite leaves either the intact commit (exit 0) or a
/// loud refusal (exit 2) — never a lying envelope — and the next reindex
/// converges to baseline-identical serve, equal to an uninterrupted twin.
/// FACETS: next-run 0-or-2 contract on the killed fixture; serve parity vs its
/// own pre-crash baseline (status+search+outline); status+answers equality vs an
/// uninterrupted twin. Every kill interleaving satisfies the same asserts.
/// KILLS: lying-envelope/exit-1, kill-residue, resume-divergence mutants.
/// COVERS: fault_sigkill_mid_cache_write_next_run_recovers_or_fails_closed,
/// drill_sigkill_mid_reindex_serve_parity,
/// relation_sigkill_interrupted_resume_equals_uninterrupted.
#[test]
#[cfg(unix)]
fn sigkill_mid_rewrite_recovers_and_serves() {
    const FILES: usize = 400;
    const PROBE: &str = "worker_0199";
    const PROBE_FILE: &str = "src/worker_0199.rs";
    let tantivy = [("ASGREP_TANTIVY", "1")];

    // Twin A: the uninterrupted reference run.
    let (temp_a, root_a, index_a) = seed_big_project(FILES);
    let output = run_in(
        &asgrep(),
        temp_a.path(),
        &[
            "--index-path",
            index_a.to_str().unwrap(),
            "--no-embed",
            "--json",
            "index",
            root_a.to_str().unwrap(),
        ],
        &tantivy,
    );
    assert_success(&output, "index");
    let root_as = root_a.to_str().unwrap().to_owned();
    let index_as = index_a.to_str().unwrap().to_owned();
    run_reindex(&asgrep(), &temp_a, &root_a, &index_a, &tantivy);
    let reference_status =
        status_snapshot(&run_status(&asgrep(), &temp_a, &root_as, &index_as, &[]));
    let reference_answers = search_answer_keys(&run_search(
        &asgrep(),
        &temp_a,
        &root_as,
        &index_as,
        PROBE,
        &["--no-embed"],
        &tantivy,
    ));
    assert_eq!(reference_status["file_count"], FILES);
    assert!(!reference_answers.is_empty());

    // Twin B: POPULATE + baseline, then the kill fault.
    let (temp_b, root_b, index_b) = seed_big_project(FILES);
    let output = run_in(
        &asgrep(),
        temp_b.path(),
        &[
            "--index-path",
            index_b.to_str().unwrap(),
            "--no-embed",
            "--json",
            "index",
            root_b.to_str().unwrap(),
        ],
        &tantivy,
    );
    assert_success(&output, "index");
    assert!(temp_b.path().join("lexical.db").is_file());
    let root_s = root_b.to_str().unwrap().to_owned();
    let index_s = index_b.to_str().unwrap().to_owned();
    let baseline = capture_baseline(
        &asgrep(),
        &temp_b,
        &root_s,
        &index_s,
        PROBE,
        PROBE_FILE,
        &["--no-embed"],
        &tantivy,
        FILES as u64,
    );
    for _ in 0..5 {
        let mut victim = KillOnDrop(Some(
            Command::new(asgrep())
                .args([
                    "--index-path",
                    &index_s,
                    "--no-embed",
                    "--json",
                    "reindex",
                    &root_s,
                ])
                .env("NO_COLOR", "1")
                .env("ASGREP_TANTIVY", "1")
                .current_dir(temp_b.path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn victim reindex"),
        ));
        std::thread::sleep(Duration::from_millis(5));
        let pid = victim.child().id();
        let _ = kill9(pid);
        let status = victim.child().wait().expect("wait victim");
        if status.code().is_none() {
            break; // the shot landed mid-write
        }
    }

    // Next-run contract: intact commit or loud refusal, never exit 1.
    let status = run_in(
        &asgrep(),
        temp_b.path(),
        &[
            "--index-path",
            &index_s,
            "--no-embed",
            "--json",
            "status",
            &root_s,
        ],
        &[],
    );
    match status.status.code() {
        Some(0) => {
            let value = assert_success(&status, "status");
            assert_eq!(value["file_count"], FILES);
        }
        Some(2) => {
            assert_failure_envelope(&status, "status", 2, "operational");
        }
        other => panic!("status after SIGKILL must exit 0 or 2, got {other:?}"),
    }

    // Resume equals uninterrupted AND serves the pre-crash baseline.
    let healed = run_reindex(&asgrep(), &temp_b, &root_b, &index_b, &tantivy);
    assert_eq!(healed["files_indexed"], FILES as u64);
    assert_eq!(
        status_snapshot(&run_status(&asgrep(), &temp_b, &root_s, &index_s, &[])),
        reference_status,
        "resumed status must equal uninterrupted status"
    );
    assert_eq!(
        search_answer_keys(&run_search(
            &asgrep(),
            &temp_b,
            &root_s,
            &index_s,
            PROBE,
            &["--no-embed"],
            &tantivy
        )),
        reference_answers,
        "resumed answers must equal uninterrupted answers"
    );
    assert_serve_parity(
        &asgrep(),
        &temp_b,
        &root_s,
        &index_s,
        PROBE,
        PROBE_FILE,
        &["--no-embed"],
        &tantivy,
        &baseline,
        "post-SIGKILL",
    );
}
