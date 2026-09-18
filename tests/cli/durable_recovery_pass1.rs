//! R1 recovery-contract oracles for `ast-sgrep-cli` durable state.
//!
//! Every assertion keys on a DOCUMENTED discriminant — process exit code
//! (0 success / 1 usage / 2 operational, see `capabilities --json` →
//! `exit_codes`), the machine envelope (`ok`, `exit_code`, `error.kind`),
//! the doctor envelope (`healthy`, `issues[].kind`, `status`), bench
//! history fields (`bench_history.verdict`, `ratchet_ok`), status fields
//! (`file_count`, `writer_generation`, `semantic_ivf_present`), or durable
//! file state (existence + bytes). No test matches on message text.
//!
//! Non-overlap with prior art:
//! - `codemod_crash_windows`: codemod swap windows, orphan-backup healing,
//!   plan/apply preview agreement, read-only codemod targets. Not repeated.
//! - `cpu_limit_orphan_reap`: limiter payload-group reaping. Not repeated.
//! - `watch_daemon_e2e` / `watch_incremental`: live watch reindex and
//!   `update_paths` semantics. Not repeated.
//! - `bench_history_integrity`: corrupt aggregate `.bench-history.json`
//!   fails loud (message-text based). Here: missing history rebuilds,
//!   read-only history dir fails with exit-2/operational discriminants,
//!   and the corrupt *committed prior* fails open to `establish_baseline`
//!   (the deliberate contrast, with a keep-verdict control arm).
//! - `machine_contracts`: envelope shapes, doctor `missing_root`, SCIP
//!   degradation. Here: doctor `index_open` / `empty_index` /
//!   `semantic_ivf_missing` kinds, corrupt-index reader/writer split,
//!   quarantine + stale-lock resume boundaries, sidecar cache contracts,
//!   install config recovery.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const SOURCE: &str = "fn probe_target() { run(1); }\n";

fn asgrep() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// Seed `<temp>/proj/src/lib.rs`; returns (tempdir, root, index db path).
fn seed_project() -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), SOURCE).unwrap();
    let index = temp.path().join("idx.db");
    (temp, root, index)
}

fn run_in(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(asgrep());
    cmd.args(args).env("NO_COLOR", "1").current_dir(dir);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("run asgrep")
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// Assert the operational-failure discriminant triple: exit 2, ok:false,
/// exit_code:2, error.kind == "operational".
fn assert_operational_failure(output: &Output, command: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2 for {command}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(output);
    assert_eq!(value["command"], command);
    assert_eq!(value["ok"], false, "ok must be false: {value}");
    assert_eq!(value["exit_code"], 2, "exit_code must be 2: {value}");
    assert_eq!(
        value["error"]["kind"], "operational",
        "error.kind must be operational: {value}"
    );
    value
}

fn assert_success(output: &Output, command: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0 for {command}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(output);
    assert_eq!(value["command"], command);
    assert_eq!(value["ok"], true, "ok must be true: {value}");
    assert_eq!(value["exit_code"], 0, "exit_code must be 0: {value}");
    value
}

fn run_index(temp: &TempDir, root: &Path, index: &Path) {
    let output = run_in(
        temp.path(),
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--no-embed",
            "--json",
            "index",
            root.to_str().unwrap(),
        ],
        &[],
    );
    assert_success(&output, "index");
}

fn corrupt_db(index: &Path) -> Vec<u8> {
    let garbage = b"THIS IS NOT A SQLITE DATABASE FILE !!!!".to_vec();
    fs::write(index, &garbage).unwrap();
    garbage
}

/// Index readers fail closed on a corrupt database: no silent empty, no
/// walk-only downgrade — exit 2 + operational across status, outline, and
/// passive search.
#[test]
fn readers_fail_closed_on_corrupt_index_db() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    corrupt_db(&index);
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();

    let status = run_in(
        temp.path(),
        &[
            "--index-path",
            index_s,
            "--no-embed",
            "--json",
            "status",
            root_s,
        ],
        &[],
    );
    assert_operational_failure(&status, "status");

    let outline = run_in(
        temp.path(),
        &[
            "--index-path",
            index_s,
            "--no-embed",
            "--json",
            "outline",
            "src/lib.rs",
            root_s,
        ],
        &[],
    );
    assert_operational_failure(&outline, "outline");

    let search = run_in(
        temp.path(),
        &[
            "--index-path",
            index_s,
            "--no-embed",
            "--no-auto-index",
            "--json",
            "search",
            "probe_target",
            root_s,
        ],
        &[],
    );
    assert_operational_failure(&search, "search");
}

/// Doctor reports a corrupt or missing database as `index_open` with
/// healthy:false, a null status, and exit 2 — never ok:true.
#[test]
fn doctor_reports_corrupt_and_missing_index_as_index_open() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    corrupt_db(&index);
    let root_s = root.to_str().unwrap();

    for db in [index.clone(), temp.path().join("nonexistent.db")] {
        let output = run_in(
            temp.path(),
            &[
                "--index-path",
                db.to_str().unwrap(),
                "--json",
                "doctor",
                root_s,
            ],
            &[],
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "doctor must exit 2 for {}",
            db.display()
        );
        let value = stdout_json(&output);
        assert_eq!(value["command"], "doctor");
        assert_eq!(value["ok"], false);
        assert_eq!(value["exit_code"], 2);
        assert_eq!(value["healthy"], false);
        assert_eq!(value["status"], Value::Null);
        let kinds: Vec<&str> = value["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|issue| issue["kind"].as_str())
            .collect();
        assert!(
            kinds.contains(&"index_open"),
            "doctor must report kind index_open for {}: {value}",
            db.display()
        );
    }
}

/// Crash-window resume boundary for the authoritative store: the incremental
/// writer refuses a corrupt database (exit 2, no quarantine taken), while the
/// full-rebuild writer heals it (exit 0, corrupt inode quarantined beside the
/// path, rows rebuilt and searchable).
#[test]
fn incremental_index_refuses_corrupt_db_while_reindex_heals() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let garbage = corrupt_db(&index);
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();
    let quarantine = index.with_file_name("idx.db.corrupt");

    let refused = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "index", root_s],
        &[],
    );
    assert_operational_failure(&refused, "index");
    assert!(
        !quarantine.exists(),
        "the refusing writer must not move the corrupt inode"
    );

    let healed = run_in(
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
    let value = assert_success(&healed, "reindex");
    assert_eq!(value["files_indexed"], 1);
    assert!(
        quarantine.is_file(),
        "reindex must quarantine the corrupt database at {}",
        quarantine.display()
    );
    assert_eq!(
        fs::read(&quarantine).unwrap(),
        garbage,
        "quarantine must preserve the corrupt inode's bytes"
    );

    let status = run_in(
        temp.path(),
        &[
            "--index-path",
            index_s,
            "--no-embed",
            "--json",
            "status",
            root_s,
        ],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);

    let search = run_in(
        temp.path(),
        &[
            "--index-path",
            index_s,
            "--no-embed",
            "--no-auto-index",
            "--json",
            "search",
            "probe_target",
            root_s,
        ],
        &[],
    );
    let search = assert_success(&search, "search");
    assert!(
        search["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
        "the rebuilt index must answer: {search}"
    );
}

/// Stale daemon/lock state from a dead process never refuses a fresh rebuild:
/// a garbage `.reindex.lock`, stale writer-generation and IVF temp files sit
/// beside a corrupt database and reindex still heals with exit 0.
#[test]
fn stale_lock_and_temp_crash_debris_do_not_block_reindex() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    corrupt_db(&index);
    fs::write(index.with_file_name("idx.db.reindex.lock"), "stale-lock-garbage").unwrap();
    fs::write(
        temp.path().join(".writer_generation.999999.0.tmp"),
        "stale-stamp-tmp",
    )
    .unwrap();
    fs::write(temp.path().join(".semantic.ivf.999999.tmp"), "stale-ivf-tmp").unwrap();
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();

    let healed = run_in(
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
    let value = assert_success(&healed, "reindex");
    assert_eq!(value["files_indexed"], 1);
    assert!(
        index.with_file_name("idx.db.corrupt").is_file(),
        "quarantine must still be taken past stale debris"
    );
}

/// The writer-generation stamp is cold-start state, not load-bearing: a
/// corrupt stamp and a missing stamp both index cleanly, and the stamp is
/// recreated as a numeric nonzero epoch (also visible via status).
#[test]
fn writer_generation_stamp_missing_or_corrupt_is_cold_start_not_error() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let stamp = temp.path().join("writer_generation");
    assert!(stamp.is_file(), "index must advertise a stamp");
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();

    fs::write(&stamp, "NOT-A-NUMBER{{{\n").unwrap();
    let output = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "index", root_s],
        &[],
    );
    assert_success(&output, "index");
    let epoch: u64 = fs::read_to_string(&stamp)
        .unwrap()
        .trim()
        .parse()
        .expect("corrupt stamp must be replaced by a numeric epoch");
    assert!(epoch > 0, "replacement epoch must be nonzero");

    fs::remove_file(&stamp).unwrap();
    let output = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "index", root_s],
        &[],
    );
    assert_success(&output, "index");
    let epoch: u64 = fs::read_to_string(&stamp)
        .unwrap()
        .trim()
        .parse()
        .expect("missing stamp must be recreated as a numeric epoch");
    assert!(epoch > 0, "recreated epoch must be nonzero");

    let status = run_in(
        temp.path(),
        &[
            "--index-path",
            index_s,
            "--no-embed",
            "--json",
            "status",
            root_s,
        ],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["writer_generation"], epoch);
}

/// The semantic IVF sidecar is a derived cache: garbage bytes degrade search
/// to the non-IVF path (exit 0, hits intact) instead of refusing, and the
/// next semantic mutation rebuilds a valid sidecar (magic `ASIVF\0`).
/// `ASGREP_ANN_THRESHOLD=1` forces the ANN lane on the tiny fixture so the
/// corruption is genuinely consulted rather than vacuously unloaded.
#[test]
fn corrupt_semantic_ivf_degrades_search_and_rebuilds_on_mutation() {
    let (temp, root, index) = seed_project();
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let ann = [("ASGREP_ANN_THRESHOLD", "1")];

    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "index", &root_s],
        &ann,
    );
    assert_success(&output, "index");
    let ivf = temp.path().join("semantic.ivf");
    assert!(ivf.is_file(), "forced ANN must build the sidecar");
    let status = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "status", &root_s],
        &ann,
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["semantic_ivf_present"], true);

    fs::write(&ivf, b"GARBAGE-NOT-IVF-BYTES!!!!").unwrap();
    let search = run_in(
        temp.path(),
        &[
            "--index-path",
            &index_s,
            "--no-auto-index",
            "--json",
            "search",
            "probe_target",
            &root_s,
        ],
        &ann,
    );
    let search = assert_success(&search, "search");
    assert!(
        search["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
        "corrupt IVF must degrade, not lose hits: {search}"
    );

    fs::write(root.join("src/lib.rs"), "fn probe_target() { run(2); }\nfn added_fn() {}\n")
        .unwrap();
    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "reindex", &root_s],
        &ann,
    );
    assert_success(&output, "reindex");
    let rebuilt = fs::read(&ivf).unwrap();
    assert!(
        rebuilt.starts_with(b"ASIVF\0"),
        "a semantic mutation must rebuild a valid IVF sidecar"
    );
}

/// The lexical sidecar is a derived cache with a split contract: a corrupt
/// `lexical.db` fails the tantivy writer closed (exit 2 operational) while
/// the authoritative rows stay readable (status exit 0, file_count intact)
/// and readers degrade past the cache (search exit 0).
#[test]
fn corrupt_lexical_cache_fails_writer_but_not_reader() {
    let (temp, root, index) = seed_project();
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let tantivy = [("ASGREP_TANTIVY", "1")];

    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_success(&output, "index");
    assert!(
        temp.path().join("lexical.db").is_file(),
        "forced tantivy must build the sidecar"
    );

    fs::write(temp.path().join("lexical.db"), b"GARBAGE-NOT-SQLITE!!").unwrap();
    let failed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_operational_failure(&failed, "index");

    let status = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);

    let search = run_in(
        temp.path(),
        &[
            "--index-path",
            &index_s,
            "--no-embed",
            "--no-auto-index",
            "--json",
            "search",
            "probe_target",
            &root_s,
        ],
        &tantivy,
    );
    let search = assert_success(&search, "search");
    assert!(
        search["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
        "readers must degrade past a corrupt lexical cache: {search}"
    );
}

/// A missing bench history file — including missing parent directories — is
/// rebuilt, not refused: exit 0, verdict `establish_baseline`, schema v1
/// document with an entry for the label. (Complement to
/// `bench_history_integrity`: corrupt aggregate history fails loud.)
#[test]
fn bench_history_missing_file_and_dir_are_rebuilt() {
    let (temp, root, index) = seed_project();
    let history = temp.path().join("histdir").join("sub").join("hist.json");
    assert!(!history.parent().unwrap().exists());
    let hist_dir = temp.path().join("hist").to_str().unwrap().to_owned();

    let output = run_in(
        temp.path(),
        &[
            "--no-embed",
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "bench",
            "--query",
            "probe_target",
            "--iterations",
            "1",
            root.to_str().unwrap(),
        ],
        &[
            ("ASGREP_BENCH_HISTORY_PATH", history.to_str().unwrap()),
            ("ASGREP_BENCH_HISTORY_DIR", &hist_dir),
        ],
    );
    let value = assert_success(&output, "bench");
    assert_eq!(value["bench_history"]["verdict"], "establish_baseline");
    assert_eq!(value["bench_history"]["ratchet_ok"], true);

    let doc: Value = serde_json::from_str(&fs::read_to_string(&history).unwrap()).unwrap();
    assert_eq!(doc["schema_version"], "1");
    assert!(
        doc["entries"]["query:probe_target"]["avg_search_ms"]
            .as_f64()
            .is_some(),
        "rebuilt history must carry the label entry: {doc}"
    );
}

/// Unwritable and missing directories error cleanly with the operational
/// discriminant: bench into a read-only history dir exits 2, and status
/// against a missing project root exits 2.
#[test]
fn unwritable_and_missing_dirs_error_cleanly() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ro = temp.path().join("ro");
        fs::create_dir(&ro).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
        let hist_dir = temp.path().join("hist").to_str().unwrap().to_owned();
        let ro_history = ro.join("hist.json").to_str().unwrap().to_owned();
        let output = run_in(
            temp.path(),
            &[
                "--no-embed",
                "--index-path",
                index.to_str().unwrap(),
                "--json",
                "bench",
                "--query",
                "probe_target",
                "--iterations",
                "1",
                root.to_str().unwrap(),
            ],
            &[
                ("ASGREP_BENCH_HISTORY_PATH", ro_history.as_str()),
                ("ASGREP_BENCH_HISTORY_DIR", hist_dir.as_str()),
            ],
        );
        assert_operational_failure(&output, "bench");
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    {
        let _ = (&temp, &root, &index);
    }

    let missing_root = temp.path().join("no-such-root");
    let output = run_in(
        temp.path(),
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--no-embed",
            "--json",
            "status",
            missing_root.to_str().unwrap(),
        ],
        &[],
    );
    assert_operational_failure(&output, "status");
}

/// Keep-gate committed priors fail open where aggregate history fails closed:
/// a valid generous prior yields `keep` (control arm proving the file is
/// consulted), corrupting the same file flips the verdict to
/// `establish_baseline` with exit 0, the corrupt bytes are preserved, and the
/// run snapshot is still written.
#[test]
fn corrupt_committed_prior_fails_open_to_baseline() {
    let (temp, root, index) = seed_project();
    let hist_dir = temp.path().join("hist");
    fs::create_dir(&hist_dir).unwrap();
    let prior = hist_dir.join("query-probe-target.latest.json");
    let history = temp.path().join("hist.json");
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let history_s = history.to_str().unwrap().to_owned();
    let hist_dir_s = hist_dir.to_str().unwrap().to_owned();

    let run_bench = |temp: &TempDir| {
        run_in(
            temp.path(),
            &[
                "--no-embed",
                "--index-path",
                &index_s,
                "--json",
                "bench",
                "--query",
                "probe_target",
                "--iterations",
                "1",
                &root_s,
            ],
            &[
                ("ASGREP_BENCH_HISTORY_PATH", history_s.as_str()),
                ("ASGREP_BENCH_HISTORY_DIR", hist_dir_s.as_str()),
            ],
        )
    };

    fs::write(
        &prior,
        serde_json::json!({
            "schema_version": "1",
            "label": "query:probe_target",
            "avg_search_ms": 1_000_000_000.0,
            "geomean_search_ms": null,
            "cv_pct": 0.0,
            "host": "oracle",
            "git_sha": "oracle",
            "profile": "oracle",
            "verdict": "keep",
            "updated_unix_ms": 0,
            "placeholder": false,
            "keep_eligible": true
        })
        .to_string(),
    )
    .unwrap();
    let value = assert_success(&run_bench(&temp), "bench");
    assert_eq!(
        value["bench_history"]["verdict"], "keep",
        "the generous prior must be consulted (control arm): {value}"
    );

    fs::write(&prior, "{ corrupt json").unwrap();
    let value = assert_success(&run_bench(&temp), "bench");
    assert_eq!(
        value["bench_history"]["verdict"], "establish_baseline",
        "a corrupt prior must fail open to a fresh baseline: {value}"
    );
    assert_eq!(
        fs::read_to_string(&prior).unwrap(),
        "{ corrupt json",
        "the corrupt prior is evidence and must not be rewritten"
    );
    let run_snapshot: Value = serde_json::from_str(
        &fs::read_to_string(hist_dir.join("query-probe-target.run.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(run_snapshot["label"], "query:probe_target");
    assert_eq!(run_snapshot["verdict"], "establish_baseline");
}

/// Install config recovery: a missing agent config is created with the
/// managed entry (exit 0), while a corrupt config fails closed (exit 2 +
/// operational) and keeps its bytes for the human to repair.
#[test]
fn install_config_missing_rebuilt_corrupt_refused() {
    let temp = TempDir::new().unwrap();
    let cursor_dir = temp.path().join("cursor").to_str().unwrap().to_owned();
    let fake_bin = temp.path().join("fake-asgrep-mcp");
    fs::write(&fake_bin, "#!/bin/sh\n").unwrap();
    let fake_bin_s = fake_bin.to_str().unwrap().to_owned();
    let config = temp.path().join("cursor").join("mcp.json");

    let output = run_in(
        temp.path(),
        &["install", "--target", "cursor", "--yes"],
        &[
            ("CURSOR_CONFIG_DIR", cursor_dir.as_str()),
            ("ASGREP_MCP_BIN", fake_bin_s.as_str()),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "missing config must be created: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(doc["mcpServers"]["asgrep"]["command"], fake_bin_s);

    fs::write(&config, "{ corrupt json").unwrap();
    let output = run_in(
        temp.path(),
        &["install", "--target", "cursor", "--yes", "--force", "--json"],
        &[
            ("CURSOR_CONFIG_DIR", cursor_dir.as_str()),
            ("ASGREP_MCP_BIN", fake_bin_s.as_str()),
        ],
    );
    assert_operational_failure(&output, "install");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "{ corrupt json",
        "a corrupt config must not be overwritten"
    );
}
