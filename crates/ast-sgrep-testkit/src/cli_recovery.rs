//! CLI crash-recovery harness: seed, run, baseline, serve-parity, kill.
//!
//! # Contract
//!
//! - One canonical copy of the former `tests/cli/recovery_harness.rs` `#[path]`
//!   module, shared by the `recovery_store` / `recovery_caches` / `recovery_aux`
//!   suites.
//! - Every CLI invocation goes through [`run_in`] (60s hard timeout, hermetic
//!   env) so a wedged binary can never hang the suite.
//! - `bin` is an explicit first parameter on every runner: the exact binary
//!   path comes from `env!("CARGO_BIN_EXE_asgrep")`, which expands only in
//!   the test target — inside this dependency that env is unset, so the
//!   caller resolves it once (`fn asgrep()`) and passes it down.
//! - Runners panic (never `Result`) on spawn/timeout failure; exit status is
//!   the caller's verdict (see [`crate::assert_success`] /
//!   [`crate::assert_failure_envelope`]).

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// INTENT: two-function seed source so both the search baseline
/// (`probe_target`) and the outline baseline (count 2) are non-vacuous.
pub const SOURCE: &str = "fn probe_target() { run(1); }\nfn helper_alpha() { run(2); }\n";
/// INTENT: the seed probe query; must hit exactly the first seed function.
pub const QUERY: &str = "probe_target";
/// INTENT: the seeded source path that outline snapshots are taken over.
pub const OUTLINE_PATH: &str = "src/lib.rs";
/// INTENT: hard per-invocation bound for [`run_in`]: a wedged binary panics
/// the test instead of hanging the suite.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// INTENT: hermetic run with a hard timeout. Scrubs inherited `ASGREP_*` (the
/// parent env must not shift flag defaults), pins `NO_COLOR=1` +
/// `HF_HUB_OFFLINE=1` (an accidental model load fails fast instead of
/// downloading), runs in `dir`. Panics past `timeout`, killing the child.
pub fn run_timeout(
    bin: &Path,
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    timeout: Duration,
) -> Output {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("NO_COLOR", "1")
        .env("HF_HUB_OFFLINE", "1")
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let scrub: Vec<String> = std::env::vars()
        .map(|(key, _)| key)
        .filter(|key| key.starts_with("ASGREP_"))
        .collect();
    for var in &scrub {
        cmd.env_remove(var);
    }
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

/// INTENT: [`run_timeout`] with the suite-standard [`RUN_TIMEOUT`]: the only
/// raw invocation entry point recovery tests use.
pub fn run_in(bin: &Path, dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    run_timeout(bin, dir, args, envs, RUN_TIMEOUT)
}

/// INTENT: seed `<temp>/proj/src/lib.rs` from [`SOURCE`]; returns
/// (tempdir, root, index db path): the writable starting point every CLI
/// recovery test builds from. Planting reuses `file_tree`.
pub fn seed_project() -> (TempDir, PathBuf, PathBuf) {
    let temp = crate::fixture::file_tree(&[("proj/src/lib.rs", SOURCE)]);
    let root = temp.path().join("proj");
    let index = temp.path().join("idx.db");
    (temp, root, index)
}

/// INTENT: seed `<temp>/proj/src/worker_<i>.rs` x `count`: the wide
/// kill-window fixture for SIGKILL fault injection (generated names need
/// owned strings, so this writes directly instead of via `file_tree`).
pub fn seed_big_project(count: usize) -> (TempDir, PathBuf, PathBuf) {
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

/// INTENT: plain `--no-embed` index; returns the success envelope (for
/// `files_indexed`): the canonical "healthy build" beat.
pub fn run_index(bin: &Path, temp: &TempDir, root: &Path, index: &Path) -> Value {
    let output = run_in(
        bin,
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
    crate::assert_success(&output, "index")
}

/// INTENT: plain `--no-embed` reindex; returns the success envelope: the
/// canonical "documented recovery" beat.
pub fn run_reindex(
    bin: &Path,
    temp: &TempDir,
    root: &Path,
    index: &Path,
    envs: &[(&str, &str)],
) -> Value {
    let output = run_in(
        bin,
        temp.path(),
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--no-embed",
            "--json",
            "reindex",
            root.to_str().unwrap(),
        ],
        envs,
    );
    crate::assert_success(&output, "reindex")
}

/// INTENT: doctor unhealthy envelope — exit 2, ok:false, healthy:false, null
/// status, and an `index_open` issue. The unhealthy path carries no `error`
/// object, so `assert_failure_envelope` (which requires `error.kind`) rejects
/// it; this pins the shape instead. Returns the parsed body.
pub fn assert_doctor_unhealthy(output: &Output, db_desc: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(2),
        "doctor must exit 2 for {db_desc}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = crate::parse_stdout(output);
    assert_eq!(value["schema_version"], "1.0.0");
    assert_eq!(value["tool"], "asgrep");
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
        "doctor must report kind index_open for {db_desc}: {value}"
    );
    value
}

/// INTENT: stable `status` observables recovery must preserve. Excludes
/// `writer_generation`, `embed_cache_*`, and paths (vary run to run by design).
pub fn status_snapshot(status: &Value) -> Value {
    serde_json::json!({
        "file_count": status["file_count"],
        "symbol_count": status["symbol_count"],
        "line_count": status["line_count"],
        "semantic_chunk_count": status["semantic_chunk_count"],
        "caller_count": status["caller_count"],
        "import_count": status["import_count"],
        "schema_version": status["schema_version"],
        "semantic_ivf_present": status["semantic_ivf_present"],
    })
}

/// INTENT: plain `status` run; returns the success envelope.
pub fn run_status(
    bin: &Path,
    temp: &TempDir,
    root: &str,
    index: &str,
    envs: &[(&str, &str)],
) -> Value {
    let output = run_in(
        bin,
        temp.path(),
        &["--index-path", index, "--json", "status", root],
        envs,
    );
    crate::assert_success(&output, "status")
}

/// INTENT: sorted (file, line_start, symbol) answer keys. Scores, excerpts,
/// and ordering are excluded: only the served answer SET is compared across
/// recovery.
pub fn search_answer_keys(search: &Value) -> Vec<(String, u64, String)> {
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

/// INTENT: `search` with `--no-auto-index` (serve must not heal under the
/// probe); returns the success envelope.
pub fn run_search(
    bin: &Path,
    temp: &TempDir,
    root: &str,
    index: &str,
    query: &str,
    extra: &[&str],
    envs: &[(&str, &str)],
) -> Value {
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
    let output = run_in(bin, temp.path(), &args, envs);
    crate::assert_success(&output, "search")
}

/// INTENT: outline serve snapshot — (file, count, (name, kind, line_start,
/// line_end) per symbol in served order). Compared whole between baseline
/// and recovery.
pub fn run_outline_snapshot(
    bin: &Path,
    temp: &TempDir,
    root: &str,
    index: &str,
    path: &str,
) -> (String, u64, Vec<(String, String, u64, u64)>) {
    let output = run_in(
        bin,
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
    let value = crate::assert_success(&output, "outline");
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

/// INTENT: the pre-crash SERVE baseline: everything post-recovery serve must
/// reproduce (status + search answers + outline).
pub struct ServeBaseline {
    pub status: Value,
    pub search_keys: Vec<(String, u64, String)>,
    pub outline: (String, u64, Vec<(String, String, u64, u64)>),
}

/// INTENT: capture the pre-crash SERVE baseline, asserting it is non-vacuous
/// (non-empty answers, non-empty outline, expected file count): parity
/// against an empty baseline would prove nothing.
pub fn capture_baseline(
    bin: &Path,
    temp: &TempDir,
    root: &str,
    index: &str,
    query: &str,
    outline_path: &str,
    search_extra: &[&str],
    search_envs: &[(&str, &str)],
    expect_files: u64,
) -> ServeBaseline {
    let status = status_snapshot(&run_status(bin, temp, root, index, &[]));
    let baseline = ServeBaseline {
        search_keys: search_answer_keys(&run_search(
            bin,
            temp,
            root,
            index,
            query,
            search_extra,
            search_envs,
        )),
        outline: run_outline_snapshot(bin, temp, root, index, outline_path),
        status,
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
    assert_eq!(
        baseline.status["file_count"], expect_files,
        "baseline file_count"
    );
    baseline
}

/// INTENT: SERVE beat — re-serve status + search + outline and require
/// observable identity with the pre-crash baseline.
#[allow(clippy::too_many_arguments)]
pub fn assert_serve_parity(
    bin: &Path,
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
        status_snapshot(&run_status(bin, temp, root, index, &[])),
        baseline.status,
        "{stage}: recovered status must equal the pre-crash baseline"
    );
    assert_eq!(
        search_answer_keys(&run_search(bin, temp, root, index, query, search_extra, search_envs)),
        baseline.search_keys,
        "{stage}: recovered search answers must equal the pre-crash baseline"
    );
    assert_eq!(
        run_outline_snapshot(bin, temp, root, index, outline_path),
        baseline.outline,
        "{stage}: recovered outline must equal the pre-crash baseline"
    );
}

/// INTENT: best-effort reaper so fault-injection children never leak past a
/// test: on drop, SIGKILL (`Child::kill`) + reap (`wait`). Explicit waits in
/// the test body take precedence; this is the leak backstop.
pub struct KillOnDrop(pub Option<Child>);

impl KillOnDrop {
    pub fn child(&mut self) -> &mut Child {
        self.0.as_mut().expect("child present")
    }

    pub fn take(&mut self) -> Child {
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

/// INTENT: deliver the SIGKILL fault (`/bin/kill -9`) to a live child pid.
/// Returns whether the signal dispatched; the caller asserts the death.
#[cfg(unix)]
pub fn kill9(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-9", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
