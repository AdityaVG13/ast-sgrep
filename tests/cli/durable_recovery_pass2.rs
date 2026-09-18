//! R2 fault-injection oracles for `ast-sgrep-cli` durable state.
//!
//! R1 (`durable_recovery_pass1`) plants static post-crash states and checks
//! the verdict. R2 injects ACTIVE faults — a live `SIGKILL` mid-write, a
//! live lock holder, torn (truncated) files, occupied paths, read-only
//! homes, a killed watch loop — and checks the next run recovers or fails
//! closed. Every test names its fault class in its name and asserts only
//! DOCUMENTED discriminants: process exit code (0 success / 1 usage / 2
//! operational), the machine envelope (`ok`, `exit_code`, `error.kind`),
//! envelope shapes (`file_count`, `files_indexed`, `hits`,
//! `bench_history.verdict`), and durable file state (existence + bytes).
//! No test matches on message text.
//!
//! Non-overlap with prior art:
//! - `durable_recovery_pass1`: static garbage/missing states (corrupt db,
//!   garbage lock content, corrupt-or-missing stamps/sidecars, missing or
//!   read-only bench history, corrupt prior, install config). R2 never
//!   re-plants those exact states: truncation (torn writes) replaces
//!   garbage, a dead-PID lock record and a LIVE holder replace garbage
//!   lock content, and every verdict is reached through an injected fault.
//! - `codemod_crash_windows`: codemod swap windows. Not repeated.
//! - `cpu_limit_orphan_reap`: limiter payload-group reaping. The `/bin/kill`
//!   and polling idioms are reused; the product surface is disjoint.
//! - `watch_daemon_e2e` / `watch_incremental`: live watch reindex and
//!   `update_paths` semantics. R2 kills the watch loop and checks resume
//!   through the next CLI run, never the loop's own output.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const SOURCE: &str = "fn probe_target() { run(1); }\n";
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

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

/// Seed `<temp>/proj/src/worker_<i>.rs` x `count` for a wide kill window.
fn seed_big_project(count: usize) -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    fs::create_dir_all(root.join("src")).unwrap();
    for i in 0..count {
        fs::write(
            root.join("src").join(format!("worker_{i:04}.rs")),
            format!("fn worker_{i:04}() {{ run({i}); }}\n"),
        )
        .unwrap();
    }
    let index = temp.path().join("idx.db");
    (temp, root, index)
}

/// Hermetic child run with a hard timeout: every CLI invocation in R2 goes
/// through here so a wedged binary can never hang the suite.
fn run_timeout(dir: &Path, args: &[&str], envs: &[(&str, &str)], timeout: Duration) -> Output {
    let mut cmd = Command::new(asgrep());
    cmd.args(args)
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn asgrep");
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => return child.wait_with_output().expect("collect output"),
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let output = child.wait_with_output().expect("collect output after kill");
                    panic!(
                        "asgrep {args:?} exceeded {timeout:?}; stderr: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn run_in(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    run_timeout(dir, args, envs, RUN_TIMEOUT)
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

/// Best-effort reaper so fault-injection children never leak past a test.
struct KillOnDrop(Option<Child>);

impl KillOnDrop {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().expect("child present")
    }

    fn take(&mut self) -> Child {
        self.0.take().expect("child present")
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(unix)]
fn kill9(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-9", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Fault class: SIGKILL delivered mid-cache-write. A `reindex` rewriting the
/// 400-file index plus its lexical cache is shot with `kill -9`; the next
/// run must either serve the intact commit (exit 0) or refuse the torn one
/// (exit 2 + operational) — never exit 1, never a lying envelope — and a
/// fresh `reindex` must converge back to the full row set.
#[test]
#[cfg(unix)]
fn fault_sigkill_mid_cache_write_next_run_recovers_or_fails_closed() {
    const FILES: usize = 400;
    const PROBE: &str = "worker_0199";
    let (temp, root, index) = seed_big_project(FILES);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let tantivy = [("ASGREP_TANTIVY", "1")];

    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_success(&output, "index");
    assert!(temp.path().join("lexical.db").is_file());

    // Fire the fault at a full cache rewrite. Several attempts maximize the
    // chance the shot lands mid-write; every interleaving (shot landed vs.
    // rewrite already done) satisfies the same next-run assertions.
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
                .current_dir(temp.path())
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

    let status = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    match status.status.code() {
        Some(0) => {
            let value = assert_success(&status, "status");
            assert_eq!(value["file_count"], FILES);
        }
        Some(2) => {
            assert_operational_failure(&status, "status");
        }
        other => panic!("status after SIGKILL must exit 0 or 2, got {other:?}"),
    }

    let healed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
        &tantivy,
    );
    assert_success(&healed, "reindex");

    let status = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], FILES);

    let search = run_in(
        temp.path(),
        &[
            "--index-path",
            &index_s,
            "--no-embed",
            "--no-auto-index",
            "--json",
            "search",
            PROBE,
            &root_s,
        ],
        &tantivy,
    );
    let search = assert_success(&search, "search");
    assert!(
        search["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
        "the converged index must answer: {search}"
    );
}

/// Fault class: torn (truncated) authoritative database — the on-disk shape
/// of a write cut in half. Readers and the incremental writer refuse it
/// (exit 2 + operational, inode untouched), and `reindex` heals it with the
/// torn bytes preserved in quarantine.
#[test]
fn fault_torn_truncated_index_db_refused_then_healed() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();

    let original_len = fs::metadata(&index).unwrap().len();
    let torn_len = original_len / 2;
    assert!(torn_len > 0, "fixture db must be truncatable");
    let mut torn = fs::read(&index).unwrap();
    torn.truncate(torn_len as usize);
    fs::write(&index, &torn).unwrap();
    assert!(fs::metadata(&index).unwrap().len() < original_len);
    let quarantine = index.with_file_name("idx.db.corrupt");

    let status = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "status", root_s],
        &[],
    );
    assert_operational_failure(&status, "status");

    let refused = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "index", root_s],
        &[],
    );
    assert_operational_failure(&refused, "index");
    assert!(
        !quarantine.exists(),
        "the refusing writer must not move the torn inode"
    );

    let healed = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "reindex", root_s],
        &[],
    );
    let value = assert_success(&healed, "reindex");
    assert_eq!(value["files_indexed"], 1);
    assert!(
        quarantine.is_file(),
        "reindex must quarantine the torn database"
    );
    assert_eq!(
        fs::read(&quarantine).unwrap(),
        torn,
        "quarantine must preserve the torn bytes"
    );

    let status = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "status", root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);
}

/// Fault class: zero-length database — the shape of a create that never got
/// its first page. SQLite treats it as an empty database, so the next
/// `index` cold-starts and rebuilds in place (exit 0, no quarantine taken:
/// nothing corrupt to preserve), converging to the full row set.
#[test]
fn fault_zero_length_index_db_rebuilt_as_cold_start() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();

    fs::write(&index, b"").unwrap();
    assert_eq!(fs::metadata(&index).unwrap().len(), 0);

    let output = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "index", root_s],
        &[],
    );
    assert_success(&output, "index");
    assert!(
        !index.with_file_name("idx.db.corrupt").exists(),
        "a cold-start rebuild must not take a quarantine"
    );

    let status = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "status", root_s],
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

/// Fault class: torn (truncated) `lexical.db` cache. The split contract from
/// R1 holds for torn bytes too: the tantivy writer fails closed (exit 2 +
/// operational) while the authoritative rows stay readable (status exit 0)
/// and readers degrade past the cache (search exit 0 with hits). Deleting
/// the torn cache lets the next `index` rebuild it.
#[test]
fn fault_torn_truncated_lexical_cache_split_contract() {
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
    let cache = temp.path().join("lexical.db");
    assert!(cache.is_file(), "forced tantivy must build the sidecar");

    let mut torn = fs::read(&cache).unwrap();
    torn.truncate(64);
    fs::write(&cache, &torn).unwrap();

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
        "readers must degrade past a torn lexical cache: {search}"
    );

    fs::remove_file(&cache).unwrap();
    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_success(&output, "index");
    assert!(
        cache.is_file() && fs::metadata(&cache).unwrap().len() > 64,
        "the next index must rebuild a whole lexical cache"
    );
}

/// Fault class: torn (truncated) `semantic.ivf` sidecar — the shape of a
/// kill landing mid-sidecar-write. Search degrades to the non-IVF path
/// (exit 0, hits intact, torn bytes left alone), and the next semantic
/// mutation rebuilds a valid sidecar (magic `ASIVF\0`).
/// `ASGREP_ANN_THRESHOLD=1` forces the ANN lane so the torn file is
/// genuinely consulted rather than vacuously unloaded.
#[test]
fn fault_torn_truncated_semantic_ivf_degrades_then_rebuilds() {
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

    fs::write(&ivf, b"ASI").unwrap(); // torn magic: 3 bytes of a kill mid-write
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
        "torn IVF must degrade, not lose hits: {search}"
    );
    assert_eq!(
        fs::metadata(&ivf).unwrap().len(),
        3,
        "readers must refuse the torn sidecar without rewriting it"
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

/// Fault class: torn (truncated) aggregate bench history. The keep-gate
/// aggregate fails loud (exit 2 + operational), the torn bytes are preserved
/// as evidence (never reset into a fresh baseline), and removing the torn
/// file lets the next run re-establish the baseline.
#[test]
fn fault_torn_truncated_bench_history_fails_loud_preserves_bytes() {
    let (temp, root, index) = seed_project();
    let history = temp.path().join("hist.json");
    let hist_dir = temp.path().join("hist").to_str().unwrap().to_owned();
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let history_s = history.to_str().unwrap().to_owned();

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
                ("ASGREP_BENCH_HISTORY_DIR", hist_dir.as_str()),
            ],
        )
    };

    let value = assert_success(&run_bench(&temp), "bench");
    assert_eq!(value["bench_history"]["verdict"], "establish_baseline");

    let mut torn = fs::read(&history).unwrap();
    torn.truncate(torn.len() / 2);
    assert!(!torn.is_empty());
    assert!(
        serde_json::from_slice::<Value>(&torn).is_err(),
        "the harness must inject genuinely torn JSON"
    );
    fs::write(&history, &torn).unwrap();

    assert_operational_failure(&run_bench(&temp), "bench");
    assert_eq!(
        fs::read(&history).unwrap(),
        torn,
        "the torn aggregate is evidence and must not be reset"
    );

    fs::remove_file(&history).unwrap();
    let value = assert_success(&run_bench(&temp), "bench");
    assert_eq!(value["bench_history"]["verdict"], "establish_baseline");
}

/// Fault class: stale daemon/pid record in the recovery lock. The lock file
/// carries the pid of a provably dead process; file locking is by flock, not
/// by pid, so the stale record must not refuse a fresh rebuild.
#[test]
#[cfg(unix)]
fn fault_stale_dead_pid_lock_does_not_block_reindex() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    corrupt_db(&index);

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

    let healed = run_in(
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
    let value = assert_success(&healed, "reindex");
    assert_eq!(value["files_indexed"], 1);
    assert!(
        index.with_file_name("idx.db.corrupt").is_file(),
        "quarantine must still be taken past the stale pid record"
    );
}

/// Fault class: LIVE lock holder on the recovery lock. While another process
/// holds the exclusive flock, a `reindex` of a corrupt database must block
/// (still running at the deadline — never a fast success, never a refusal)
/// and, once the holder dies, must complete and heal without corruption.
#[test]
#[cfg(unix)]
fn fault_live_lock_holder_blocks_or_refuses_never_corrupts() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let garbage = corrupt_db(&index);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let lock_path = index.with_file_name("idx.db.reindex.lock");
    let ready = temp.path().join("holder-ready");

    // Holder: acquire the real flock, signal readiness, sleep. Killed below.
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
            holder.child().try_wait().expect("try_wait holder").is_none(),
            "the flock holder died before acquiring the lock"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    let mut healing = KillOnDrop(Some(
        Command::new(asgrep())
            .args(["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s])
            .env("NO_COLOR", "1")
            .current_dir(temp.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn blocked reindex"),
    ));
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        healing.child().try_wait().expect("try_wait reindex").is_none(),
        "reindex must block on the live holder, not exit under it"
    );

    // The holder dies; the blocked reindex must now complete and heal.
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

    let status = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);
}

/// Fault class: recovery-lock path occupied by a directory. The lock cannot
/// be acquired, so `reindex` of a corrupt database fails closed (exit 2 +
/// operational) with the corrupt inode untouched and no quarantine taken;
/// removing the obstruction lets the next `reindex` heal.
#[test]
fn fault_lock_path_occupied_by_directory_fails_closed() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let garbage = corrupt_db(&index);
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();
    let lock_dir = index.with_file_name("idx.db.reindex.lock");
    let quarantine = index.with_file_name("idx.db.corrupt");

    fs::create_dir(&lock_dir).unwrap();
    let refused = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "reindex", root_s],
        &[],
    );
    assert_operational_failure(&refused, "reindex");
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
    let healed = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "reindex", root_s],
        &[],
    );
    let value = assert_success(&healed, "reindex");
    assert_eq!(value["files_indexed"], 1);
    assert!(quarantine.is_file());
}

/// Fault class: read-only index home. Writes fail closed with the
/// operational discriminant and no torn artifact is left behind. The
/// read-only open path also fails closed against a read-only HOME (the
/// WAL-mode database needs its directory for the `-shm` wal-index even for
/// reads), while a read-only db FILE in a writable home still serves —
/// that file/dir split is the discriminant. Restoring writability
/// recovers fully: the fault leaves no damage.
#[test]
#[cfg(unix)]
fn fault_readonly_index_home_fails_closed_exit_2() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), SOURCE).unwrap();
    let state = temp.path().join("state");
    fs::create_dir(&state).unwrap();
    let db = state.join("idx.db");
    let root_s = root.to_str().unwrap();
    let db_s = db.to_str().unwrap();

    // Arm A: a fresh index into a read-only home is refused, nothing created.
    fs::set_permissions(&state, fs::Permissions::from_mode(0o555)).unwrap();
    let output = run_in(
        temp.path(),
        &["--index-path", db_s, "--no-embed", "--json", "index", root_s],
        &[],
    );
    assert_operational_failure(&output, "index");
    assert!(
        !db.exists(),
        "the refused index must not leave a torn database behind"
    );
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();

    // Arm B: with a committed index, a read-only HOME refuses even readers,
    // then recovers without damage once writability returns.
    let output = run_in(
        temp.path(),
        &["--index-path", db_s, "--no-embed", "--json", "index", root_s],
        &[],
    );
    assert_success(&output, "index");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o555)).unwrap();
    let status = run_in(
        temp.path(),
        &["--index-path", db_s, "--no-embed", "--json", "status", root_s],
        &[],
    );
    assert_operational_failure(&status, "status");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    let status = run_in(
        temp.path(),
        &["--index-path", db_s, "--no-embed", "--json", "status", root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);

    // Arm C: a read-only db FILE in a writable home still serves readers.
    fs::set_permissions(&db, fs::Permissions::from_mode(0o444)).unwrap();
    let status = run_in(
        temp.path(),
        &["--index-path", db_s, "--no-embed", "--json", "status", root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);
    fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
}

/// Fault class: `watch` killed mid-incremental-loop. The loop is shot with
/// `kill -9` after its initial commit (an edit lands in the kill race, so it
/// may or may not have been picked up); the resume boundary is the next
/// plain `index`, which must converge without a full `reindex`: exit 0,
/// status intact, and the raced edit searchable.
#[test]
#[cfg(unix)]
fn fault_killed_watch_resumes_via_next_index() {
    use std::io::{BufRead, BufReader};
    use std::sync::{Arc, Mutex};

    let (temp, root, index) = seed_project();
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();

    let watch = Command::new(asgrep())
        .args([
            "--index-path",
            &index_s,
            "--no-embed",
            "watch",
            "--debounce-ms",
            "50",
            &root_s,
        ])
        .env("NO_COLOR", "1")
        .current_dir(temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    let mut watch = KillOnDrop(Some(watch));
    let stderr = watch.child().stderr.take().expect("piped watch stderr");
    let log = Arc::new(Mutex::new(String::new()));
    let writer = Arc::clone(&log);
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if let Ok(mut held) = writer.lock() {
                held.push_str(&line);
                held.push('\n');
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = log.lock().map(|held| held.clone()).unwrap_or_default();
        if text.contains("initial index") {
            break;
        }
        assert!(Instant::now() < deadline, "watch never finished its initial index: {text}");
        assert!(
            watch.child().try_wait().expect("try_wait watch").is_none(),
            "watch died before its initial index: {text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The initial commit is durable and visible cross-process while the
    // loop is still alive.
    let status = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);

    // The raced edit: it may or may not reach the loop before the shot.
    fs::write(root.join("src/lib.rs"), "fn probe_target() { run(1); }\nfn raced_edit() {}\n").unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let pid = watch.child().id();
    assert!(kill9(pid), "the SIGKILL fault must land on a live watch loop");
    let death = watch.child().wait().expect("wait watch");
    assert!(
        death.code().is_none(),
        "watch must die by signal, got {death:?}"
    );

    // Resume boundary: the next plain index converges, no reindex needed.
    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &[],
    );
    assert_success(&output, "index");

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
            "raced_edit",
            &root_s,
        ],
        &[],
    );
    let search = assert_success(&search, "search");
    assert!(
        search["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
        "the resumed index must answer the raced edit: {search}"
    );
}

/// Fault class: quarantine slot already occupied by a prior recovery. The
/// new corruption must not overwrite the earlier evidence: the sentinel
/// stays byte-identical and the fresh bytes land in a unique `.corrupt.1`
/// quarantine while the rebuild still converges.
#[test]
fn fault_occupied_quarantine_slot_allocates_unique_quarantine() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();

    let sentinel = b"SENTINEL-PRIOR-QUARANTINE".to_vec();
    fs::write(index.with_file_name("idx.db.corrupt"), &sentinel).unwrap();
    let garbage = corrupt_db(&index);

    let healed = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "reindex", root_s],
        &[],
    );
    let value = assert_success(&healed, "reindex");
    assert_eq!(value["files_indexed"], 1);
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

    let status = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "status", root_s],
        &[],
    );
    let status = assert_success(&status, "status");
    assert_eq!(status["file_count"], 1);
}
