//! R4 end-to-end crash drills for `ast-sgrep-cli` durable state.
//!
//! R1 (`durable_recovery_pass1`) plants static post-crash states and checks the
//! verdict. R2 (`durable_recovery_pass2`) injects active faults and checks the
//! next run recovers or fails closed. R3 (`durable_recovery_pass3`) asserts
//! relations OVER recovery (idempotence, roundtrip, determinism, parity).
//!
//! R4 runs FULL crash→recover→serve drills through the real binary. Every
//! drill has the same four beats, and every beat runs a real CLI invocation:
//!
//! 1. POPULATE: `index` (+ `search`) builds durable state; the drill captures
//!    a pre-crash SERVE baseline — `search` answer keys, `outline` symbols,
//!    and `status` file_count.
//! 2. CRASH: SIGKILL mid-run, corrupt cache/state bytes, torn (truncated)
//!    state, or deleted state. The drill asserts the crash is OBSERVABLE
//!    (a refusal, a degraded serve, or missing files) so the drill cannot be
//!    vacuous.
//! 3. RECOVER: the next CLI invocation (`reindex` for corrupt/torn
//!    authoritative state, `index` for cold starts and rebuilt caches)
//!    converges with exit 0.
//! 4. SERVE: `search` + `outline` outputs are IDENTICAL to the pre-crash
//!    baseline — same hit keys, same symbols, same counts. Recovery must be
//!    observationally transparent to every serve surface.
//!
//! Every assertion keys on a DOCUMENTED discriminant — process exit code
//! (0 success / 1 usage / 2 operational), the machine envelope (`ok`,
//! `exit_code`, `error.kind`), envelope shapes (`file_count`,
//! `files_indexed`, `hits`, `symbols`, `count`), and durable file state
//! (existence + bytes). No test matches on message text.
//!
//! Non-overlap with prior art:
//! - `durable_recovery_pass1`: single-run verdicts on planted states; outline
//!   appears only as a fail-closed reader, never as a serve-parity surface.
//!   R4 never asserts a lone verdict: every drill ends in baseline-identical
//!   SERVE.
//! - `durable_recovery_pass2`: fault injection ending in verdicts or row-set
//!   convergence; no pre-crash baselines, no outline comparisons, no chained
//!   crashes. R4's SIGKILL drill adds baseline capture plus search+outline
//!   parity; R4's torn/corrupt drills add outline answer-set identity.
//! - `durable_recovery_pass3`: relations over search+status only — outline is
//!   never compared, caches are never wiped in combination, crashes are never
//!   chained. R4 compares the outline serve surface in EVERY drill, wipes all
//!   derived state at once, and chains corrupt→heal→delete→heal.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const SOURCE: &str = "fn probe_target() { run(1); }\nfn helper_alpha() { run(2); }\n";
const QUERY: &str = "probe_target";
const OUTLINE_PATH: &str = "src/lib.rs";
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

fn asgrep() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// Seed `<temp>/proj/src/lib.rs` (two functions, so outline count is 2 and the
/// serve baseline is non-vacuous); returns (tempdir, root, index db path).
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

/// Hermetic run with a hard timeout so a wedged binary never hangs the suite.
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

/// Sorted (file, line_start, symbol) served-answer keys for a search. Scores,
/// excerpts, and ordering are excluded: only the served answer SET is compared
/// between the pre-crash baseline and the post-recovery serve.
fn search_answer_keys(search: &Value) -> Vec<(String, u64, String)> {
    let mut keys: Vec<(String, u64, String)> = search["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("search must carry a hits array: {search}"))
        .iter()
        .map(|hit| {
            (
                hit["file"].as_str().unwrap_or_default().to_owned(),
                hit["line_start"].as_u64().unwrap_or(0),
                hit["symbol"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    keys.sort();
    keys
}

fn run_search_keys(
    temp: &TempDir,
    root: &str,
    index: &str,
    query: &str,
    extra: &[&str],
    envs: &[(&str, &str)],
) -> Vec<(String, u64, String)> {
    let mut args = vec![
        "--index-path",
        index,
        "--no-auto-index",
        "--json",
        "search",
        query,
        root,
    ];
    args.extend_from_slice(extra);
    let output = run_in(temp.path(), &args, envs);
    search_answer_keys(&assert_success(&output, "search"))
}

/// Outline serve snapshot: (file, count, (name, kind, line_start, line_end)
/// per symbol in served order). Compared whole between baseline and recovery.
fn run_outline_snapshot(
    temp: &TempDir,
    root: &str,
    index: &str,
    path: &str,
) -> (String, u64, Vec<(String, String, u64, u64)>) {
    let output = run_in(
        temp.path(),
        &[
            "--index-path",
            index,
            "--no-embed",
            "--json",
            "outline",
            path,
            root,
        ],
        &[],
    );
    let value = assert_success(&output, "outline");
    let symbols: Vec<(String, String, u64, u64)> = value["symbols"]
        .as_array()
        .unwrap_or_else(|| panic!("outline must carry a symbols array: {value}"))
        .iter()
        .map(|symbol| {
            (
                symbol["name"].as_str().unwrap_or_default().to_owned(),
                symbol["kind"].as_str().unwrap_or_default().to_owned(),
                symbol["line_start"].as_u64().unwrap_or(0),
                symbol["line_end"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    (
        value["file"].as_str().unwrap_or_default().to_owned(),
        value["count"].as_u64().unwrap_or(0),
        symbols,
    )
}

fn run_file_count(temp: &TempDir, root: &str, index: &str) -> u64 {
    let output = run_in(
        temp.path(),
        &["--index-path", index, "--no-embed", "--json", "status", root],
        &[],
    );
    assert_success(&output, "status")["file_count"]
        .as_u64()
        .expect("status must carry file_count")
}

/// The pre-crash SERVE baseline: everything the post-recovery serve must
/// reproduce exactly.
struct ServeBaseline {
    search_keys: Vec<(String, u64, String)>,
    outline: (String, u64, Vec<(String, String, u64, u64)>),
    file_count: u64,
}

fn capture_baseline(
    temp: &TempDir,
    root: &str,
    index: &str,
    query: &str,
    outline_path: &str,
    search_extra: &[&str],
    search_envs: &[(&str, &str)],
    expect_files: u64,
) -> ServeBaseline {
    let baseline = ServeBaseline {
        search_keys: run_search_keys(temp, root, index, query, search_extra, search_envs),
        outline: run_outline_snapshot(temp, root, index, outline_path),
        file_count: run_file_count(temp, root, index),
    };
    assert!(
        !baseline.search_keys.is_empty(),
        "the search baseline must be non-vacuous"
    );
    assert!(
        baseline.outline.1 > 0 && !baseline.outline.2.is_empty(),
        "the outline baseline must be non-vacuous: {:?}",
        baseline.outline
    );
    assert_eq!(baseline.file_count, expect_files, "baseline file_count");
    baseline
}

/// SERVE beat: re-serve search + outline + status and require byte-level
/// observable identity with the pre-crash baseline.
#[allow(clippy::too_many_arguments)]
fn assert_serve_parity(
    temp: &TempDir,
    root: &str,
    index: &str,
    query: &str,
    outline_path: &str,
    search_extra: &[&str],
    search_envs: &[(&str, &str)],
    baseline: &ServeBaseline,
    stage: &str,
) {
    assert_eq!(
        run_search_keys(temp, root, index, query, search_extra, search_envs),
        baseline.search_keys,
        "{stage}: recovered search answers must equal the pre-crash baseline"
    );
    assert_eq!(
        run_outline_snapshot(temp, root, index, outline_path),
        baseline.outline,
        "{stage}: recovered outline must equal the pre-crash baseline"
    );
    assert_eq!(
        run_file_count(temp, root, index),
        baseline.file_count,
        "{stage}: recovered file_count must equal the pre-crash baseline"
    );
}

/// Best-effort reaper so fault-injection children never leak past a test.
struct KillOnDrop(Option<Child>);

impl KillOnDrop {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().expect("child present")
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

/// Drill: SIGKILL mid-`reindex` on a 300-file index with its lexical cache.
/// POPULATE and capture the serve baseline; shoot the rewrite with `kill -9`
/// (every interleaving — shot landed mid-write vs. rewrite already done —
/// satisfies the same drill); RECOVER with `reindex`; SERVE search + outline
/// identical to the pre-crash baseline.
#[test]
#[cfg(unix)]
fn drill_sigkill_mid_reindex_serve_parity() {
    const FILES: usize = 300;
    const PROBE: &str = "worker_0199";
    const PROBE_FILE: &str = "src/worker_0199.rs";
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

    let baseline = capture_baseline(
        &temp,
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

    let healed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
        &tantivy,
    );
    let healed = assert_success(&healed, "reindex");
    assert_eq!(healed["files_indexed"], FILES as u64);

    assert_serve_parity(
        &temp,
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

/// Drill: corrupt authoritative database. POPULATE and capture the baseline;
/// CRASH by overwriting the database with garbage (the next read refuses with
/// exit 2, proving the crash landed); RECOVER with `reindex` (quarantine
/// preserves the injected bytes); SERVE identical to baseline.
#[test]
fn drill_corrupt_index_db_serve_parity() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();

    let baseline = capture_baseline(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &[],
        1,
    );

    let garbage = b"R4-DRILL-CORRUPT-DB-GARBAGE-0001".to_vec();
    fs::write(&index, &garbage).unwrap();
    let refused = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    assert_operational_failure(&refused, "status");

    let healed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
        &[],
    );
    let healed = assert_success(&healed, "reindex");
    assert_eq!(healed["files_indexed"], 1);
    assert_eq!(
        fs::read(index.with_file_name("idx.db.corrupt")).unwrap(),
        garbage,
        "quarantine must preserve the injected crash bytes"
    );

    assert_serve_parity(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &[],
        &baseline,
        "post-corrupt-db",
    );
}

/// Drill: deleted authoritative database. POPULATE and capture the baseline;
/// CRASH by deleting the database file (the next read refuses with exit 2,
/// proving the crash landed); RECOVER with `index` as a cold start (exit 0,
/// full row set, no quarantine: nothing corrupt to preserve); SERVE identical
/// to baseline.
#[test]
fn drill_deleted_index_db_serve_parity() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();

    let baseline = capture_baseline(
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
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    assert_operational_failure(&refused, "status");

    let rebuilt = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &[],
    );
    let rebuilt = assert_success(&rebuilt, "index");
    assert_eq!(rebuilt["files_indexed"], 1);
    assert!(
        !index.with_file_name("idx.db.corrupt").exists(),
        "a cold-start rebuild must not take a quarantine"
    );

    assert_serve_parity(
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

/// Drill: torn (truncated) authoritative database — the on-disk shape of a
/// write cut in half. POPULATE and capture the baseline; CRASH by truncating
/// the database to half its bytes (readers refuse with exit 2, proving the
/// crash landed); RECOVER with `reindex` (quarantine preserves the torn
/// bytes); SERVE identical to baseline.
#[test]
fn drill_torn_truncated_db_serve_parity() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();

    let baseline = capture_baseline(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &[],
        1,
    );

    let original_len = fs::metadata(&index).unwrap().len();
    let mut torn = fs::read(&index).unwrap();
    torn.truncate(original_len as usize / 2);
    assert!(!torn.is_empty(), "fixture db must be truncatable");
    fs::write(&index, &torn).unwrap();
    assert!(fs::metadata(&index).unwrap().len() < original_len);
    let refused = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    assert_operational_failure(&refused, "status");

    let healed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
        &[],
    );
    let healed = assert_success(&healed, "reindex");
    assert_eq!(healed["files_indexed"], 1);
    assert_eq!(
        fs::read(index.with_file_name("idx.db.corrupt")).unwrap(),
        torn,
        "quarantine must preserve the torn crash bytes"
    );

    assert_serve_parity(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &[],
        &baseline,
        "post-torn-db",
    );
}

/// Drill: corrupt lexical cache. POPULATE with the tantivy sidecar and capture
/// the baseline; CRASH by overwriting `lexical.db` with garbage (the writer
/// refuses the next `index` with exit 2, proving the crash landed, while the
/// authoritative rows stay intact); RECOVER by deleting the torn cache and
/// running `index`, which rebuilds a whole cache; SERVE identical to baseline.
#[test]
fn drill_corrupt_lexical_cache_serve_parity() {
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

    let baseline = capture_baseline(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &tantivy,
        1,
    );

    fs::write(&cache, b"R4-DRILL-LEXICAL-GARBAGE-0002").unwrap();
    let failed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_operational_failure(&failed, "index");

    fs::remove_file(&cache).unwrap();
    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_success(&output, "index");
    assert!(
        cache.is_file() && fs::metadata(&cache).unwrap().len() > 0,
        "recovery must rebuild a whole lexical cache"
    );

    assert_serve_parity(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &tantivy,
        &baseline,
        "post-corrupt-lexical",
    );
}

/// Drill: combined wipe of ALL derived state — `lexical.db`, `semantic.ivf`,
/// and the `writer_generation` stamp. POPULATE with both sidecars and capture
/// the baseline; CRASH by deleting all three (the deletions are directly
/// observable); the next `index` still converges with exit 0 and serves
/// (derived state is never load-bearing); RECOVER fully with `reindex`, which
/// restores every sidecar; SERVE identical to baseline.
#[test]
fn drill_deleted_caches_and_stamp_serve_parity() {
    let (temp, root, index) = seed_project();
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let envs = [("ASGREP_TANTIVY", "1"), ("ASGREP_ANN_THRESHOLD", "1")];

    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "index", &root_s],
        &envs,
    );
    assert_success(&output, "index");
    let cache = temp.path().join("lexical.db");
    let ivf = temp.path().join("semantic.ivf");
    let stamp = temp.path().join("writer_generation");
    assert!(cache.is_file() && ivf.is_file() && stamp.is_file());

    let baseline = capture_baseline(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &[],
        1,
    );

    fs::remove_file(&cache).unwrap();
    fs::remove_file(&ivf).unwrap();
    fs::remove_file(&stamp).unwrap();
    assert!(
        !cache.exists() && !ivf.exists() && !stamp.exists(),
        "the drill must really wipe all derived state"
    );

    // Derived state is never load-bearing: the next index converges and the
    // authoritative rows keep serving through the wipe.
    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "index", &root_s],
        &envs,
    );
    assert_success(&output, "index");
    let keys = run_search_keys(&temp, &root_s, &index_s, QUERY, &["--no-embed"], &[]);
    assert!(!keys.is_empty(), "serve must stay alive through the wipe");
    let outline = run_outline_snapshot(&temp, &root_s, &index_s, OUTLINE_PATH);
    assert_eq!(outline.1, 2, "outline must stay alive through the wipe");

    let healed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "reindex", &root_s],
        &envs,
    );
    let healed = assert_success(&healed, "reindex");
    assert_eq!(healed["files_indexed"], 1);
    assert!(
        cache.is_file() && fs::metadata(&cache).unwrap().len() > 0,
        "recovery must restore the lexical cache"
    );
    assert!(
        fs::read(&ivf).unwrap().starts_with(b"ASIVF\0"),
        "recovery must restore a valid IVF sidecar"
    );
    let epoch: u64 = fs::read_to_string(&stamp)
        .unwrap()
        .trim()
        .parse()
        .expect("recovery must restore a numeric stamp epoch");
    assert!(epoch > 0, "restored epoch must be nonzero");

    assert_serve_parity(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &[],
        &baseline,
        "post-wiped-caches",
    );
}

/// Drill: corrupt semantic IVF sidecar, in two serve beats. POPULATE with the
/// forced ANN lane and capture the baseline; CRASH by overwriting
/// `semantic.ivf` with garbage — SERVE #1 proves full function THROUGH the
/// degradation (exit 0, answers identical to baseline, torn bytes untouched);
/// RECOVER by deleting the sidecar and running `reindex`, which rebuilds a
/// valid sidecar (magic `ASIVF\0`); SERVE #2 proves identity again.
/// `ASGREP_ANN_THRESHOLD=1` forces the ANN lane so the corruption is genuinely
/// consulted rather than vacuously unloaded.
#[test]
fn drill_corrupt_ivf_degraded_and_rebuilt_serve_parity() {
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

    let baseline = capture_baseline(&temp, &root_s, &index_s, QUERY, OUTLINE_PATH, &[], &ann, 1);

    let garbage = b"R4-DRILL-IVF-GARBAGE-0003-NOT-IVF!!".to_vec();
    fs::write(&ivf, &garbage).unwrap();
    assert_serve_parity(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &[],
        &ann,
        &baseline,
        "degraded-ivf",
    );
    assert_eq!(
        fs::read(&ivf).unwrap(),
        garbage,
        "degraded readers must not rewrite the corrupt sidecar"
    );

    fs::remove_file(&ivf).unwrap();
    let healed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "reindex", &root_s],
        &ann,
    );
    assert_success(&healed, "reindex");
    assert!(
        fs::read(&ivf).unwrap().starts_with(b"ASIVF\0"),
        "recovery must rebuild a valid IVF sidecar"
    );

    assert_serve_parity(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &[],
        &ann,
        &baseline,
        "post-rebuilt-ivf",
    );
}

/// Drill: chained double crash — corrupt→heal→delete→heal. POPULATE and
/// capture the baseline; CRASH #1 corrupts the database (refused with exit 2),
/// RECOVER #1 heals it with `reindex` (quarantine holds crash #1's bytes);
/// CRASH #2 deletes the healed database (refused with exit 2), RECOVER #2
/// cold-starts it with `index`; SERVE is identical to the ORIGINAL pre-crash
/// baseline, and crash #1's quarantine evidence survives crash #2 (no new slot
/// taken by the cold start).
#[test]
fn drill_chained_double_crash_serve_parity() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();
    let quarantine = index.with_file_name("idx.db.corrupt");

    let baseline = capture_baseline(
        &temp,
        &root_s,
        &index_s,
        QUERY,
        OUTLINE_PATH,
        &["--no-embed"],
        &[],
        1,
    );

    // Crash #1: corrupt → refused → reindex heals.
    let garbage1 = b"R4-DRILL-CHAIN-CRASH1-GARBAGE-0004".to_vec();
    fs::write(&index, &garbage1).unwrap();
    let refused = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    assert_operational_failure(&refused, "status");
    let healed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
        &[],
    );
    let healed = assert_success(&healed, "reindex");
    assert_eq!(healed["files_indexed"], 1);
    assert_eq!(
        fs::read(&quarantine).unwrap(),
        garbage1,
        "crash #1 evidence must be quarantined"
    );

    // Crash #2: delete the healed database → refused → index cold-starts.
    fs::remove_file(&index).unwrap();
    assert!(!index.exists());
    let refused = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "status", &root_s],
        &[],
    );
    assert_operational_failure(&refused, "status");
    let rebuilt = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &[],
    );
    let rebuilt = assert_success(&rebuilt, "index");
    assert_eq!(rebuilt["files_indexed"], 1);

    // Crash #1's evidence survives crash #2; the cold start takes no new slot.
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
