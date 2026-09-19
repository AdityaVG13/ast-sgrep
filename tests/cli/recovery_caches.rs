//! Recovery CLI: derived-state faults (lexical cache, semantic IVF, stamp, wipes).
//!
//! Covers catalog `tests/catalog/recovery.md` CLI rows: lexical split contract
//! (garbage+torn) through rebuild and serve parity, IVF degrade+rebuild
//! transparency (garbage+torn), combined derived-state wipe, stamp cold-start,
//! no-op byte-idempotence, reindex observable idempotence, and recovered-vs-clean
//! parity. Discriminants only (exit code, envelope, shapes, bytes).

use ast_sgrep_testkit::{
    assert_failure_envelope, assert_serve_parity, assert_success, capture_baseline, run_in,
    run_index, run_outline_snapshot, run_reindex, run_search, run_status, search_answer_keys,
    seed_project, status_snapshot, truncate_file, OUTLINE_PATH, QUERY,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Exact binary path via the compile-time env cargo sets for THIS test target.
/// `env!` must expand here (it is unset inside the testkit dependency); the
/// resolved path is passed as `bin` to the testkit runners.
fn asgrep() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// INTENT: a corrupt lexical cache keeps its split contract (writer refuses exit
/// 2, readers degrade exit 0 with answers intact), delete+index rebuilds it, and
/// the rebuilt cache serves the pre-crash baseline identically — deterministically
/// across repeated fault cycles.
/// FACETS: garbage arm + torn arm (writer-2 / status-0 / search-0-with-hits /
/// rebuild-whole / status+search+outline parity); 3-cycle cache determinism.
/// KILLS: split-contract, cache-recovery, lossy-cache-recovery mutants.
/// COVERS: corrupt_lexical_cache_fails_writer_but_not_reader,
/// fault_torn_truncated_lexical_cache_split_contract,
/// relation_corrupt_recover_verify_roundtrip_lexical_cache,
/// drill_corrupt_lexical_cache_serve_parity,
/// relation_repeated_cache_fault_cycles_deterministic.
#[test]
fn lexical_cache_split_contract_recovers_and_serves() {
    let tantivy = [("ASGREP_TANTIVY", "1")];
    for shape in ["garbage", "torn"] {
        let (temp, root, index) = seed_project();
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
            &tantivy,
        );
        assert_success(&output, "index");
        let cache = temp.path().join("lexical.db");
        assert!(cache.is_file(), "forced tantivy must build the sidecar");
        let baseline = capture_baseline(&asgrep(), 
            &temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &["--no-embed"], &tantivy, 1,
        );

        if shape == "garbage" {
            fs::write(&cache, b"RECOVERY-CACHES-LEXICAL-GARBAGE-0001").unwrap();
        } else {
            truncate_file(&cache, 64);
        }

        // Split contract: the writer fails closed, readers degrade past the cache.
        let failed = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
            &tantivy,
        );
        assert_failure_envelope(&failed, "index", 2, "operational");
        let status = run_status(&asgrep(), &temp, &root_s, &index_s, &[]);
        assert_eq!(status["file_count"], 1, "{shape}: authoritative rows stay readable");
        let degraded = run_search(&asgrep(), &temp, &root_s, &index_s, QUERY, &["--no-embed"], &tantivy);
        assert!(
            degraded["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
            "{shape}: readers must degrade past a corrupt lexical cache: {degraded}"
        );

        // Recover: deleting the cache lets the next index rebuild it whole.
        fs::remove_file(&cache).unwrap();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
            &tantivy,
        );
        assert_success(&output, "index");
        assert!(
            cache.is_file() && fs::metadata(&cache).unwrap().len() > 64,
            "{shape}: the next index must rebuild a whole lexical cache"
        );
        assert_serve_parity(&asgrep(), 
            &temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &["--no-embed"], &tantivy,
            &baseline, &format!("post-{shape}-lexical"),
        );
    }

    // Cycle determinism: 3 corrupt→delete→rebuild cycles converge identically.
    {
        let (temp, root, index) = seed_project();
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
            &tantivy,
        );
        assert_success(&output, "index");
        let cache = temp.path().join("lexical.db");
        let baseline_status = status_snapshot(&run_status(&asgrep(), &temp, &root_s, &index_s, &[]));
        let baseline_answers = search_answer_keys(&run_search(&asgrep(), 
            &temp, &root_s, &index_s, QUERY, &["--no-embed"], &tantivy,
        ));
        assert!(!baseline_answers.is_empty());
        for cycle in 0..3u32 {
            fs::write(&cache, format!("RECOVERY-CACHES-LEXICAL-CYCLE-{cycle:04}")).unwrap();
            let failed = run_in(&asgrep(), 
                temp.path(),
                &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
                &tantivy,
            );
            assert_failure_envelope(&failed, "index", 2, "operational");
            fs::remove_file(&cache).unwrap();
            let output = run_in(&asgrep(), 
                temp.path(),
                &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
                &tantivy,
            );
            assert_success(&output, "index");
            assert!(
                cache.is_file() && fs::metadata(&cache).unwrap().len() > 0,
                "cycle {cycle}: recovery must rebuild a whole cache"
            );
            assert_eq!(
                status_snapshot(&run_status(&asgrep(), &temp, &root_s, &index_s, &[])),
                baseline_status,
                "cycle {cycle}: status must converge to baseline"
            );
            assert_eq!(
                search_answer_keys(&run_search(&asgrep(), &temp, &root_s, &index_s, QUERY, &["--no-embed"], &tantivy)),
                baseline_answers,
                "cycle {cycle}: answers must converge to baseline"
            );
        }
    }
}

/// INTENT: a corrupt semantic IVF sidecar degrades search to the non-IVF path
/// (exit 0, identical answers, corrupt bytes untouched), and the next mutation
/// or rebuild restores a valid sidecar — transparently in both serve beats.
/// FACETS: garbage arm + torn arm; degraded serve == healthy baseline (count +
/// answer set); delete+reindex rebuilds valid magic + serves baseline again;
/// source-mutation rebuild restores valid magic. ANN lane forced throughout so
/// the corruption is genuinely consulted.
/// KILLS: refuse-on-cache-corrupt, degraded-answer-loss mutants.
/// COVERS: corrupt_semantic_ivf_degrades_search_and_rebuilds_on_mutation,
/// fault_torn_truncated_semantic_ivf_degrades_then_rebuilds,
/// relation_degraded_ivf_search_answer_parity,
/// drill_corrupt_ivf_degraded_and_rebuilt_serve_parity.
#[test]
fn semantic_ivf_degrades_then_rebuilds_transparently() {
    let ann = [("ASGREP_ANN_THRESHOLD", "1")];
    for shape in ["garbage", "torn"] {
        let (temp, root, index) = seed_project();
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "index", &root_s],
            &ann,
        );
        assert_success(&output, "index");
        let ivf = temp.path().join("semantic.ivf");
        assert!(ivf.is_file(), "forced ANN must build the sidecar");
        let baseline = capture_baseline(&asgrep(), &temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &[], &ann, 1);

        let injected = if shape == "garbage" {
            b"RECOVERY-CACHES-IVF-GARBAGE-0002-NOT-IVF!!".to_vec()
        } else {
            b"ASI".to_vec() // torn magic: 3 bytes of a kill mid-write
        };
        fs::write(&ivf, &injected).unwrap();

        // Beat 1: full function THROUGH the degradation.
        assert_serve_parity(&asgrep(), 
            &temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &[], &ann,
            &baseline, &format!("degraded-{shape}-ivf"),
        );
        assert_eq!(
            fs::read(&ivf).unwrap(),
            injected,
            "{shape}: degraded readers must not rewrite the corrupt sidecar"
        );

        // Rebuild: delete + reindex restores a valid sidecar; beat 2 serves baseline.
        fs::remove_file(&ivf).unwrap();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "reindex", &root_s],
            &ann,
        );
        assert_success(&output, "reindex");
        assert!(
            fs::read(&ivf).unwrap().starts_with(b"ASIVF\0"),
            "{shape}: recovery must rebuild a valid IVF sidecar"
        );
        assert_serve_parity(&asgrep(), 
            &temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &[], &ann,
            &baseline, &format!("post-rebuilt-{shape}-ivf"),
        );

        // Mutation rebuild: a semantic change also restores a valid sidecar.
        fs::write(&ivf, &injected).unwrap();
        fs::write(root.join("src/lib.rs"), "fn probe_target() { run(2); }\nfn added_fn() {}\n").unwrap();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "reindex", &root_s],
            &ann,
        );
        assert_success(&output, "reindex");
        assert!(
            fs::read(&ivf).unwrap().starts_with(b"ASIVF\0"),
            "{shape}: a semantic mutation must rebuild a valid IVF sidecar"
        );
    }
}

/// INTENT: derived state is never load-bearing — wiping every derived file still
/// converges and serves, and the stamp is cold-start state, not a gate.
/// FACETS: combined wipe (lexical+IVF+stamp deleted → index converges, serve
/// stays alive, reindex restores all, serve parity); stamp corrupt + stamp
/// missing (index clean, numeric nonzero epoch, visible via status).
/// KILLS: derived-state-load-bearing, stamp-gating mutants.
/// COVERS: drill_deleted_caches_and_stamp_serve_parity,
/// writer_generation_stamp_missing_or_corrupt_is_cold_start_not_error.
#[test]
fn wiped_derived_state_converges_and_restores() {
    // Combined wipe: delete every derived file; index converges, reindex restores.
    {
        let (temp, root, index) = seed_project();
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let envs = [("ASGREP_TANTIVY", "1"), ("ASGREP_ANN_THRESHOLD", "1")];
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "index", &root_s],
            &envs,
        );
        assert_success(&output, "index");
        let cache = temp.path().join("lexical.db");
        let ivf = temp.path().join("semantic.ivf");
        let stamp = temp.path().join("writer_generation");
        assert!(cache.is_file() && ivf.is_file() && stamp.is_file());
        let baseline = capture_baseline(&asgrep(), 
            &temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &["--no-embed"], &[], 1,
        );
        fs::remove_file(&cache).unwrap();
        fs::remove_file(&ivf).unwrap();
        fs::remove_file(&stamp).unwrap();
        assert!(!cache.exists() && !ivf.exists() && !stamp.exists(), "the drill must really wipe all derived state");

        // The next index converges and the authoritative rows keep serving.
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "index", &root_s],
            &envs,
        );
        assert_success(&output, "index");
        let keys = search_answer_keys(&run_search(&asgrep(), &temp, &root_s, &index_s, QUERY, &["--no-embed"], &[]));
        assert!(!keys.is_empty(), "serve must stay alive through the wipe");
        let outline = run_outline_snapshot(&asgrep(), &temp, &root_s, &index_s, OUTLINE_PATH);
        assert_eq!(outline.1, 2, "outline must stay alive through the wipe");

        let healed = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "reindex", &root_s],
            &envs,
        );
        let healed = assert_success(&healed, "reindex");
        assert_eq!(healed["files_indexed"], 1);
        assert!(cache.is_file() && fs::metadata(&cache).unwrap().len() > 0, "recovery must restore the lexical cache");
        assert!(fs::read(&ivf).unwrap().starts_with(b"ASIVF\0"), "recovery must restore a valid IVF sidecar");
        let epoch: u64 = fs::read_to_string(&stamp).unwrap().trim().parse().expect("recovery must restore a numeric stamp epoch");
        assert!(epoch > 0, "restored epoch must be nonzero");
        assert_serve_parity(&asgrep(), 
            &temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &["--no-embed"], &[],
            &baseline, "post-wiped-caches",
        );
    }

    // Stamp cold-start: corrupt and missing stamps both index cleanly.
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let stamp = temp.path().join("writer_generation");
        assert!(stamp.is_file(), "index must advertise a stamp");
        let root_s = root.to_str().unwrap();
        let index_s = index.to_str().unwrap();

        fs::write(&stamp, "NOT-A-NUMBER{{{\n").unwrap();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", index_s, "--no-embed", "--json", "index", root_s],
            &[],
        );
        assert_success(&output, "index");
        let epoch: u64 = fs::read_to_string(&stamp).unwrap().trim().parse().expect("corrupt stamp must be replaced by a numeric epoch");
        assert!(epoch > 0, "replacement epoch must be nonzero");

        fs::remove_file(&stamp).unwrap();
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", index_s, "--no-embed", "--json", "index", root_s],
            &[],
        );
        assert_success(&output, "index");
        let epoch: u64 = fs::read_to_string(&stamp).unwrap().trim().parse().expect("missing stamp must be recreated as a numeric epoch");
        assert!(epoch > 0, "recreated epoch must be nonzero");
        let status = run_status(&asgrep(), &temp, root_s, index_s, &[]);
        assert_eq!(status["writer_generation"], epoch);
    }
}

/// INTENT: recovery is observationally transparent — no-op work changes no bytes,
/// repeated rebuilds change no observables, and healed state equals clean state.
/// FACETS: no-op incremental index leaves db+lexical+IVF byte-identical (stamp
/// re-stamped numeric-nonzero); double reindex identical over
/// status/files_indexed/answers; corrupted-then-healed twin serves identical
/// status/answers/doctor-health as a clean twin.
/// KILLS: gratuitous-rewrite, non-idempotent-rebuild, recovery-residue mutants.
/// COVERS: relation_noop_double_index_caches_byte_identical,
/// relation_double_reindex_observable_idempotence,
/// relation_recovered_state_cli_parity_vs_clean.
#[test]
fn recovery_preserves_observables() {
    // No-op byte-idempotence: an unchanged tree leaves every cache byte-identical.
    {
        let (temp, root, index) = seed_project();
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let envs = [("ASGREP_TANTIVY", "1"), ("ASGREP_ANN_THRESHOLD", "1")];
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "index", &root_s],
            &envs,
        );
        assert_success(&output, "index");
        let lexical = temp.path().join("lexical.db");
        let ivf = temp.path().join("semantic.ivf");
        let stamp = temp.path().join("writer_generation");
        assert!(lexical.is_file() && ivf.is_file() && stamp.is_file());
        let db_before = fs::read(&index).unwrap();
        let lexical_before = fs::read(&lexical).unwrap();
        let ivf_before = fs::read(&ivf).unwrap();
        let stamp_before: u64 = fs::read_to_string(&stamp).unwrap().trim().parse().expect("stamp must be a numeric epoch");
        assert!(stamp_before > 0);
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--json", "index", &root_s],
            &envs,
        );
        assert_success(&output, "index");
        assert_eq!(fs::read(&index).unwrap(), db_before, "a no-op index must leave the database byte-identical");
        assert_eq!(fs::read(&lexical).unwrap(), lexical_before, "a no-op index must leave the lexical cache byte-identical");
        assert_eq!(fs::read(&ivf).unwrap(), ivf_before, "a no-op index must leave the IVF sidecar byte-identical");
        let stamp_after: u64 = fs::read_to_string(&stamp).unwrap().trim().parse().expect("stamp must stay a numeric epoch");
        assert!(stamp_after > 0, "re-stamped epoch must stay nonzero");
    }

    // Rebuild idempotence: two consecutive reindex runs serve identical observables.
    {
        let (temp, root, index) = seed_project();
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let tantivy = [("ASGREP_TANTIVY", "1")];
        let output = run_in(&asgrep(), 
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
            &tantivy,
        );
        assert_success(&output, "index");
        let snap = |temp: &TempDir| -> (Value, u64, Vec<(String, u64, String)>) {
            let healed = run_in(&asgrep(), 
                temp.path(),
                &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
                &tantivy,
            );
            let healed = assert_success(&healed, "reindex");
            let files_indexed = healed["files_indexed"].as_u64().unwrap();
            let status = status_snapshot(&run_status(&asgrep(), temp, &root_s, &index_s, &[]));
            let answers = search_answer_keys(&run_search(&asgrep(), temp, &root_s, &index_s, QUERY, &["--no-embed"], &[]));
            (status, files_indexed, answers)
        };
        let first = snap(&temp);
        let second = snap(&temp);
        assert_eq!(first.0, second.0, "status must be idempotent across reindex");
        assert_eq!(first.1, second.1, "files_indexed must be idempotent across reindex");
        assert_eq!(first.2, second.2, "served answers must be idempotent across reindex");
        assert!(!first.2.is_empty(), "the answers must be non-vacuous");
    }

    // Twin parity: corrupted-then-healed serves exactly what clean serves.
    {
        let tantivy = [("ASGREP_TANTIVY", "1")];
        let clean = seed_project();
        run_index(&asgrep(), &clean.0, &clean.1, &clean.2);
        let recovered = seed_project();
        run_index(&asgrep(), &recovered.0, &recovered.1, &recovered.2);
        fs::write(&recovered.2, b"RECOVERY-CACHES-PARITY-0003-NOT-SQLITE!!").unwrap();
        run_reindex(&asgrep(), &recovered.0, &recovered.1, &recovered.2, &[]);
        let observe = |temp: &TempDir, root: &std::path::Path, index: &std::path::Path| {
            let root_s = root.to_str().unwrap().to_owned();
            let index_s = index.to_str().unwrap().to_owned();
            let status = status_snapshot(&run_status(&asgrep(), temp, &root_s, &index_s, &[]));
            let answers = search_answer_keys(&run_search(&asgrep(), temp, &root_s, &index_s, QUERY, &["--no-embed"], &tantivy));
            let doctor = run_in(&asgrep(), temp.path(), &["--index-path", &index_s, "--json", "doctor", &root_s], &[]);
            let doctor = assert_success(&doctor, "doctor");
            assert_eq!(doctor["healthy"], true);
            let issues = doctor["issues"].as_array().unwrap().len();
            (status, answers, issues)
        };
        let clean_obs = observe(&clean.0, &clean.1, &clean.2);
        let recovered_obs = observe(&recovered.0, &recovered.1, &recovered.2);
        assert!(!clean_obs.1.is_empty(), "the probe answers must be non-vacuous");
        assert_eq!(recovered_obs.0, clean_obs.0, "recovered status must equal clean status");
        assert_eq!(recovered_obs.1, clean_obs.1, "recovered answers must equal clean answers");
        assert_eq!(recovered_obs.2, clean_obs.2, "recovered doctor issues must equal clean doctor issues");
    }
}
