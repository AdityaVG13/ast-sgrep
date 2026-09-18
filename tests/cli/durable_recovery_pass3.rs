//! R3 metamorphic-recovery relations for `ast-sgrep-cli` durable state.
//!
//! R1 (`durable_recovery_pass1`) plants static post-crash states and checks the
//! verdict. R2 (`durable_recovery_pass2`) injects active faults and checks the
//! next run recovers or fails closed. R3 asserts RELATIONS OVER recovery —
//! properties that must hold between two recovery executions, not single-run
//! verdicts:
//!
//! - idempotence: `recover(recover(s))` ≡ `recover(s)` (bytes where the
//!   product is byte-stable, observables elsewhere — full `reindex` always
//!   rewrites SQLite headers and the versioned IVF fingerprint, so rebuilt
//!   caches are observably but not byte-identical; the no-op incremental
//!   `index` IS byte-identical, and that split is the discriminant);
//! - roundtrip: `recover(corrupt(s))` ≡ `s` on documented observables, with
//!   the injected bytes preserved as evidence;
//! - determinism: repeated fault+recover cycles converge to the same
//!   observables every cycle, with per-cycle evidence correctly paired;
//! - resume equivalence: an interrupted run finished via recovery serves the
//!   same answers as an uninterrupted run;
//! - parity: recovered state serves the same CLI outputs as clean state.
//!
//! Every assertion keys on a DOCUMENTED discriminant — process exit code
//! (0 success / 1 usage / 2 operational), the machine envelope (`ok`,
//! `exit_code`, `error.kind`), envelope shapes (`file_count`, `files_indexed`,
//! `hits`, `bench_history.verdict`, `healthy`, `issues[].kind`), and durable
//! file state (existence + bytes). No test matches on message text. Timing,
//! score, cache-counter, and `writer_generation` fields are excluded from
//! parity comparisons: they vary run to run by design.

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

/// Stable `status` observables: counts and schema flags that recovery must
/// preserve. Excludes `writer_generation`, `embed_cache_*`, and paths.
fn status_snapshot(status: &Value) -> Value {
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

fn run_status(temp: &TempDir, root: &str, index: &str, envs: &[(&str, &str)]) -> Value {
    let output = run_in(
        temp.path(),
        &["--index-path", index, "--json", "status", root],
        envs,
    );
    assert_success(&output, "status")
}

/// Sorted (file, line_start, symbol) answer keys for a search. Scores,
/// excerpts, and ordering are excluded: only the served answer SET is
/// compared across recovery relations.
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

fn run_search(
    temp: &TempDir,
    root: &str,
    index: &str,
    query: &str,
    extra: &[&str],
    envs: &[(&str, &str)],
) -> Value {
    let mut args = vec!["--index-path", index, "--no-auto-index", "--json", "search", query, root];
    args.extend_from_slice(extra);
    let output = run_in(temp.path(), &args, envs);
    assert_success(&output, "search")
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

/// Idempotence, byte-exact arm: a no-op incremental `index` (no source
/// changes) leaves every durable cache byte-identical — the authoritative
/// database, the lexical cache, and the semantic IVF sidecar. The
/// `writer_generation` stamp is the documented exception: it is re-stamped
/// every run, so the relation asserts numeric-nonzero shape on both sides
/// rather than byte equality.
#[test]
fn relation_noop_double_index_caches_byte_identical() {
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
    let lexical = temp.path().join("lexical.db");
    let ivf = temp.path().join("semantic.ivf");
    let stamp = temp.path().join("writer_generation");
    assert!(lexical.is_file() && ivf.is_file() && stamp.is_file());

    let db_before = fs::read(&index).unwrap();
    let lexical_before = fs::read(&lexical).unwrap();
    let ivf_before = fs::read(&ivf).unwrap();
    let stamp_before: u64 = fs::read_to_string(&stamp)
        .unwrap()
        .trim()
        .parse()
        .expect("stamp must be a numeric epoch");
    assert!(stamp_before > 0);

    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--json", "index", &root_s],
        &envs,
    );
    assert_success(&output, "index");

    assert_eq!(
        fs::read(&index).unwrap(),
        db_before,
        "a no-op index must leave the database byte-identical"
    );
    assert_eq!(
        fs::read(&lexical).unwrap(),
        lexical_before,
        "a no-op index must leave the lexical cache byte-identical"
    );
    assert_eq!(
        fs::read(&ivf).unwrap(),
        ivf_before,
        "a no-op index must leave the IVF sidecar byte-identical"
    );
    let stamp_after: u64 = fs::read_to_string(&stamp)
        .unwrap()
        .trim()
        .parse()
        .expect("stamp must stay a numeric epoch");
    assert!(stamp_after > 0, "re-stamped epoch must stay nonzero");
}

/// Idempotence, observable arm: a full `reindex` always rewrites store bytes
/// (SQLite headers, versioned IVF fingerprint), so the double-rebuild
/// relation is over CLI observables, not bytes — `status` snapshot,
/// `files_indexed`, and served answer keys are identical across two
/// consecutive rebuilds: recover(recover(s)) ≡ recover(s).
#[test]
fn relation_double_reindex_observable_idempotence() {
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

    let snap = |temp: &TempDir| -> (Value, u64, Vec<(String, u64, String)>) {
        let healed = run_in(
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
            &tantivy,
        );
        let healed = assert_success(&healed, "reindex");
        let files_indexed = healed["files_indexed"].as_u64().unwrap();
        let status = status_snapshot(&run_status(temp, &root_s, &index_s, &[]));
        let answers = search_answer_keys(&run_search(temp, &root_s, &index_s, "probe_target", &["--no-embed"], &[]));
        (status, files_indexed, answers)
    };

    let first = snap(&temp);
    let second = snap(&temp);
    assert_eq!(first.0, second.0, "status must be idempotent across reindex");
    assert_eq!(first.1, second.1, "files_indexed must be idempotent across reindex");
    assert_eq!(first.2, second.2, "served answers must be idempotent across reindex");
    assert!(!first.2.is_empty(), "the answers must be non-vacuous");
}

/// Idempotence, evidence arm: the quarantine taken by a healing rebuild is
/// byte-stable under a second (healthy) rebuild — no rewrite, no extra slot.
/// recover(recover(corrupt(s))) adds no new evidence: exactly one quarantine
/// holding exactly the injected bytes.
#[test]
fn relation_quarantine_bytes_stable_no_new_slot_on_healthy_rebuild() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap();
    let index_s = index.to_str().unwrap();
    let quarantine = index.with_file_name("idx.db.corrupt");

    let garbage = b"R3-QUARANTINE-RELATION-GARBAGE-0001".to_vec();
    fs::write(&index, &garbage).unwrap();

    let healed = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "reindex", root_s],
        &[],
    );
    assert_success(&healed, "reindex");
    assert_eq!(fs::read(&quarantine).unwrap(), garbage);
    let status_after_heal = status_snapshot(&run_status(&temp, root_s, index_s, &[]));

    let again = run_in(
        temp.path(),
        &["--index-path", index_s, "--no-embed", "--json", "reindex", root_s],
        &[],
    );
    assert_success(&again, "reindex");

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
        status_snapshot(&run_status(&temp, root_s, index_s, &[])),
        status_after_heal,
        "status must be stable across the second rebuild"
    );
}

/// Roundtrip over the authoritative store: snapshot clean observables, corrupt
/// the database, watch readers refuse it (exit 2), heal with `reindex`, then
/// verify recover(corrupt(s)) ≡ s — same status snapshot, same answer keys —
/// with the injected bytes preserved in quarantine.
#[test]
fn relation_corrupt_recover_verify_roundtrip_index() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();

    let clean_status = status_snapshot(&run_status(&temp, &root_s, &index_s, &[]));
    let clean_answers = search_answer_keys(&run_search(
        &temp,
        &root_s,
        &index_s,
        "probe_target",
        &["--no-embed"],
        &[],
    ));
    assert!(!clean_answers.is_empty());

    let garbage = b"R3-ROUNDTRIP-INDEX-GARBAGE-0002".to_vec();
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
        "quarantine must preserve the injected bytes"
    );
    assert_eq!(
        status_snapshot(&run_status(&temp, &root_s, &index_s, &[])),
        clean_status,
        "recovered status must equal pre-corruption status"
    );
    assert_eq!(
        search_answer_keys(&run_search(&temp, &root_s, &index_s, "probe_target", &["--no-embed"], &[])),
        clean_answers,
        "recovered answers must equal pre-corruption answers"
    );
}

/// Roundtrip over the lexical cache: snapshot clean observables, corrupt
/// `lexical.db`, watch the split contract (writer refuses exit 2, readers
/// degrade exit 0), delete the cache, rebuild with `index`, then verify
/// recover(corrupt(s)) ≡ s on status and answers.
#[test]
fn relation_corrupt_recover_verify_roundtrip_lexical_cache() {
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

    let clean_status = status_snapshot(&run_status(&temp, &root_s, &index_s, &[]));
    let clean_answers = search_answer_keys(&run_search(
        &temp, &root_s, &index_s, "probe_target", &["--no-embed"], &tantivy,
    ));
    assert!(!clean_answers.is_empty());

    fs::write(&cache, b"R3-ROUNDTRIP-LEXICAL-GARBAGE-0003").unwrap();
    let failed = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_operational_failure(&failed, "index");
    // Readers degrade past the corrupt cache (exit 0) mid-roundtrip.
    let degraded = run_in(
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
    assert_success(&degraded, "search");

    fs::remove_file(&cache).unwrap();
    let output = run_in(
        temp.path(),
        &["--index-path", &index_s, "--no-embed", "--json", "index", &root_s],
        &tantivy,
    );
    assert_success(&output, "index");
    assert!(cache.is_file(), "recovery must rebuild the cache");

    assert_eq!(
        status_snapshot(&run_status(&temp, &root_s, &index_s, &[])),
        clean_status,
        "recovered status must equal pre-corruption status"
    );
    assert_eq!(
        search_answer_keys(&run_search(&temp, &root_s, &index_s, "probe_target", &["--no-embed"], &tantivy)),
        clean_answers,
        "recovered answers must equal pre-corruption answers"
    );
}

/// Roundtrip over bench history: a fresh run establishes the baseline verdict;
/// tearing the aggregate makes the next run fail loud (exit 2, torn bytes
/// preserved as evidence); removing the torn file lets recovery re-establish
/// the IDENTICAL verdict and schema shape — recover(torn(h)) ≡ fresh(h).
#[test]
fn relation_corrupt_recover_verify_roundtrip_bench_history() {
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

    let fresh = assert_success(&run_bench(&temp), "bench");
    assert_eq!(fresh["bench_history"]["verdict"], "establish_baseline");

    let mut torn = fs::read(&history).unwrap();
    torn.truncate(torn.len() / 2);
    assert!(serde_json::from_slice::<Value>(&torn).is_err());
    fs::write(&history, &torn).unwrap();

    assert_operational_failure(&run_bench(&temp), "bench");
    assert_eq!(
        fs::read(&history).unwrap(),
        torn,
        "the torn aggregate is evidence and must not be reset"
    );

    fs::remove_file(&history).unwrap();
    let recovered = assert_success(&run_bench(&temp), "bench");
    assert_eq!(
        recovered["bench_history"]["verdict"], fresh["bench_history"]["verdict"],
        "recovered history must re-establish the fresh-run verdict"
    );
    let doc: Value =
        serde_json::from_str(&fs::read_to_string(&history).unwrap()).unwrap();
    assert_eq!(doc["schema_version"], "1");
    assert!(
        doc["entries"]["query:probe_target"]["avg_search_ms"]
            .as_f64()
            .is_some(),
        "recovered history must carry the label entry: {doc}"
    );
}

/// Determinism across repeated fault+recover cycles on the authoritative
/// store: three corrupt→reindex cycles with DISTINCT garbage each converge to
/// the same observables every cycle (files_indexed, status snapshot, answer
/// keys), while the quarantine chain pairs each cycle's evidence correctly —
/// slot i holds garbage i, and the slot count grows by exactly one per cycle.
#[test]
fn relation_repeated_db_fault_cycles_deterministic() {
    let (temp, root, index) = seed_project();
    run_index(&temp, &root, &index);
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();

    let baseline_status = status_snapshot(&run_status(&temp, &root_s, &index_s, &[]));
    let baseline_answers = search_answer_keys(&run_search(
        &temp,
        &root_s,
        &index_s,
        "probe_target",
        &["--no-embed"],
        &[],
    ));
    assert!(!baseline_answers.is_empty());

    for cycle in 0..3u32 {
        let garbage = format!("R3-CYCLE-GARBAGE-{cycle:04}-DISTINCT-BYTES").into_bytes();
        fs::write(&index, &garbage).unwrap();

        let healed = run_in(
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
            &[],
        );
        let healed = assert_success(&healed, "reindex");
        assert_eq!(healed["files_indexed"], 1, "cycle {cycle}: files_indexed");
        assert_eq!(
            status_snapshot(&run_status(&temp, &root_s, &index_s, &[])),
            baseline_status,
            "cycle {cycle}: status must converge to baseline"
        );
        assert_eq!(
            search_answer_keys(&run_search(&temp, &root_s, &index_s, "probe_target", &["--no-embed"], &[])),
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

/// Determinism across repeated fault+recover cycles on the lexical cache:
/// three corrupt→delete→rebuild cycles converge to the same status snapshot
/// and answer keys every cycle, and each cycle rebuilds a whole cache.
#[test]
fn relation_repeated_cache_fault_cycles_deterministic() {
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
    assert!(cache.is_file());

    let baseline_status = status_snapshot(&run_status(&temp, &root_s, &index_s, &[]));
    let baseline_answers = search_answer_keys(&run_search(
        &temp, &root_s, &index_s, "probe_target", &["--no-embed"], &tantivy,
    ));
    assert!(!baseline_answers.is_empty());

    for cycle in 0..3u32 {
        fs::write(&cache, format!("R3-CACHE-CYCLE-{cycle:04}-GARBAGE")).unwrap();
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
            "cycle {cycle}: recovery must rebuild a whole cache"
        );
        assert_eq!(
            status_snapshot(&run_status(&temp, &root_s, &index_s, &[])),
            baseline_status,
            "cycle {cycle}: status must converge to baseline"
        );
        assert_eq!(
            search_answer_keys(&run_search(&temp, &root_s, &index_s, "probe_target", &["--no-embed"], &tantivy)),
            baseline_answers,
            "cycle {cycle}: answers must converge to baseline"
        );
    }
}

/// Degradation preserves the served answer set: search with a corrupt IVF
/// sidecar (degraded path) returns the same hit COUNT and the same
/// (file, line_start, symbol) answer keys as search with the healthy sidecar.
/// `ASGREP_ANN_THRESHOLD=1` forces the ANN lane so the corruption is genuinely
/// consulted rather than vacuously unloaded.
#[test]
fn relation_degraded_ivf_search_answer_parity() {
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

    let healthy = run_search(&temp, &root_s, &index_s, "probe_target", &[], &ann);
    let healthy_keys = search_answer_keys(&healthy);
    assert!(!healthy_keys.is_empty());

    fs::write(&ivf, b"R3-DEGRADED-IVF-GARBAGE-BYTES!!!").unwrap();
    let degraded = run_search(&temp, &root_s, &index_s, "probe_target", &[], &ann);
    let degraded_keys = search_answer_keys(&degraded);

    assert_eq!(
        degraded_keys.len(),
        healthy_keys.len(),
        "degraded search must serve the same hit count"
    );
    assert_eq!(
        degraded_keys, healthy_keys,
        "degraded search must serve the same answer set"
    );
}

/// Resume equivalence across a kill boundary: twin fixtures A (uninterrupted
/// `reindex`) and B (`reindex` shot with SIGKILL, then `reindex` to resume)
/// converge to the same status snapshot and the same probe answer keys.
/// Every interleaving (shot landed mid-write vs. rewrite already done)
/// satisfies the same relation — the resume must equal the uninterrupted run.
#[test]
#[cfg(unix)]
fn relation_sigkill_interrupted_resume_equals_uninterrupted() {
    const FILES: usize = 400;
    const PROBE: &str = "worker_0199";
    let tantivy = [("ASGREP_TANTIVY", "1")];

    let setup = |count: usize| {
        let (temp, root, index) = seed_big_project(count);
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
            &tantivy,
        );
        assert_success(&output, "index");
        (temp, root, index)
    };
    let snapshot = |temp: &TempDir, root: &Path, index: &Path| {
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let healed = run_in(
            temp.path(),
            &["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s],
            &tantivy,
        );
        assert_success(&healed, "reindex");
        let status = status_snapshot(&run_status(temp, &root_s, &index_s, &[]));
        let answers = search_answer_keys(&run_search(temp, &root_s, &index_s, PROBE, &["--no-embed"], &tantivy));
        (status, answers)
    };

    // Twin A: the uninterrupted reference run.
    let (temp_a, root_a, index_a) = setup(FILES);
    let reference = snapshot(&temp_a, &root_a, &index_a);
    assert_eq!(reference.0["file_count"], FILES);
    assert!(!reference.1.is_empty());

    // Twin B: the interrupted run — shot mid-rewrite, then resumed.
    let (temp_b, root_b, index_b) = setup(FILES);
    let root_s = root_b.to_str().unwrap().to_owned();
    let index_s = index_b.to_str().unwrap().to_owned();
    for _ in 0..5 {
        let mut victim = KillOnDrop(Some(
            Command::new(asgrep())
                .args(["--index-path", &index_s, "--no-embed", "--json", "reindex", &root_s])
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
    let resumed = snapshot(&temp_b, &root_b, &index_b);

    assert_eq!(
        resumed.0, reference.0,
        "resumed status must equal uninterrupted status"
    );
    assert_eq!(
        resumed.1, reference.1,
        "resumed answers must equal uninterrupted answers"
    );
}

/// Recovered-state CLI parity vs clean state: twin fixtures — one indexed
/// cleanly, one corrupted then healed — serve identical `status` snapshots,
/// identical probe answer keys, and identical `doctor` health verdicts
/// (exit 0, healthy:true, zero issues). Recovery must be observationally
/// transparent to every reader surface.
#[test]
fn relation_recovered_state_cli_parity_vs_clean() {
    let tantivy = [("ASGREP_TANTIVY", "1")];

    let clean = seed_project();
    run_index(&clean.0, &clean.1, &clean.2);

    let recovered = seed_project();
    run_index(&recovered.0, &recovered.1, &recovered.2);
    fs::write(&recovered.2, b"R3-PARITY-GARBAGE-0004-NOT-SQLITE!!").unwrap();
    let healed = run_in(
        recovered.0.path(),
        &[
            "--index-path",
            recovered.2.to_str().unwrap(),
            "--no-embed",
            "--json",
            "reindex",
            recovered.1.to_str().unwrap(),
        ],
        &[],
    );
    assert_success(&healed, "reindex");

    let observe = |temp: &TempDir, root: &Path, index: &Path| {
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let status = status_snapshot(&run_status(temp, &root_s, &index_s, &[]));
        let answers = search_answer_keys(&run_search(
            temp, &root_s, &index_s, "probe_target", &["--no-embed"], &tantivy,
        ));
        let doctor = run_in(
            temp.path(),
            &["--index-path", &index_s, "--json", "doctor", &root_s],
            &[],
        );
        assert_eq!(doctor.status.code(), Some(0));
        let doctor = stdout_json(&doctor);
        assert_eq!(doctor["ok"], true);
        assert_eq!(doctor["healthy"], true);
        let issues = doctor["issues"].as_array().unwrap().len();
        (status, answers, issues)
    };

    let clean_obs = observe(&clean.0, &clean.1, &clean.2);
    let recovered_obs = observe(&recovered.0, &recovered.1, &recovered.2);
    assert!(!clean_obs.1.is_empty(), "the probe answers must be non-vacuous");
    assert_eq!(
        recovered_obs.0, clean_obs.0,
        "recovered status must equal clean status"
    );
    assert_eq!(
        recovered_obs.1, clean_obs.1,
        "recovered answers must equal clean answers"
    );
    assert_eq!(
        recovered_obs.2, clean_obs.2,
        "recovered doctor issues must equal clean doctor issues"
    );
}
