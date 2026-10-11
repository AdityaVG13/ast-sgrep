//! Keep-gate history integrity (PASS 56, H-AUDIT-55 keep-gate row).
//!
//! Contract: a CORRUPT `.bench-history.json` must fail the bench command
//! loudly (non-zero exit, message naming the file) — never be silently reset
//! to an empty baseline. The keep-gate ratchet compares against committed
//! history; silently re-baselining on a parse error would launder a real
//! regression (or a real corruption) into a fresh baseline.
//!
//! Failure-first: pre-fix the run exited 0 and rewrote the corrupt file with
//! a single fresh entry (silent reset).

use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn committed_first_baseline_is_usable_by_the_next_keep_gate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "fn baseline_target() {}\n").unwrap();
    let history = temp.path().join("history.json");
    let priors = temp.path().join("priors");
    let output = ast_sgrep_testkit::run_env(
        std::path::Path::new(env!("CARGO_BIN_EXE_asgrep")),
        &[
            "--json",
            "--no-embed",
            "bench",
            root.to_str().unwrap(),
            "--query",
            "baseline_target",
            "--iterations",
            "1",
        ],
        &[
            ("ASGREP_BENCH_HISTORY_PATH", history.to_str().unwrap()),
            ("ASGREP_BENCH_HISTORY_DIR", priors.to_str().unwrap()),
            ("ASGREP_BENCH_HISTORY_COMMIT", "1"),
        ],
    );
    let value = ast_sgrep_testkit::assert_success(&output, "bench");
    assert_eq!(value["bench_history"]["verdict"], "establish_baseline");
    let prior = ast_sgrep_cli::keep_gate::load_prior_file(std::path::Path::new(
        value["bench_history"]["committed_prior"].as_str().unwrap(),
    ));
    assert!(
        prior.avg_ms.is_some_and(|ms| ms > 0.0),
        "baseline must contain a measurement"
    );
    assert!(
        !prior.placeholder,
        "committing the first baseline must enable later comparisons"
    );
    let verdict = ast_sgrep_cli::keep_gate::evaluate_keep(
        ast_sgrep_cli::keep_gate::KeepSample {
            avg_ms: prior.avg_ms.unwrap() * 2.0,
            cv_pct: 0.0,
            geomean_ms: None,
        },
        prior,
        ast_sgrep_cli::keep_gate::KeepThresholds::from_repo(),
    );
    assert!(matches!(
        verdict,
        ast_sgrep_cli::keep_gate::KeepVerdict::RejectRegression { .. }
    ));
}

#[test]
fn distinct_query_and_batch_surfaces_keep_independent_priors() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "fn alpha_target() {}\n").unwrap();
    let history = temp.path().join("history.json");
    let priors = temp.path().join("priors");
    let mut observations = Vec::new();
    for (option, values) in [
        (
            "--query",
            vec!["alpha_target".to_owned(), "alpha-target".to_owned()],
        ),
        (
            "--queries-file",
            ["left", "right"]
                .into_iter()
                .zip(["alpha_target", "alpha-target"])
                .map(|(dir, query)| {
                    let dir = temp.path().join(dir);
                    fs::create_dir(&dir).unwrap();
                    let path = dir.join("queries.txt");
                    fs::write(&path, format!("{query}\n")).unwrap();
                    path.to_str().unwrap().to_owned()
                })
                .collect(),
        ),
    ] {
        for value in values {
            let run = |commit| {
                let output = ast_sgrep_testkit::run_env(
                    std::path::Path::new(env!("CARGO_BIN_EXE_asgrep")),
                    &[
                        "--json",
                        "--no-embed",
                        "bench",
                        root.to_str().unwrap(),
                        option,
                        &value,
                        "--iterations",
                        "1",
                    ],
                    &[
                        ("ASGREP_BENCH_HISTORY_PATH", history.to_str().unwrap()),
                        ("ASGREP_BENCH_HISTORY_DIR", priors.to_str().unwrap()),
                        ("ASGREP_BENCH_HISTORY_COMMIT", commit),
                        ("ASGREP_BENCH_RATCHET", "0"),
                    ],
                );
                ast_sgrep_testkit::assert_success(&output, "bench")
            };
            let doc = run("1");
            let meta = &doc["bench_history"];
            assert_eq!(
                meta["verdict"], "establish_baseline",
                "a distinct measured surface must not borrow another prior"
            );
            let path = std::path::PathBuf::from(meta["committed_prior"].as_str().unwrap());
            let run_path = std::path::PathBuf::from(meta["run_path"].as_str().unwrap());
            let bytes = fs::read(&path).unwrap();
            for (previous_path, previous_bytes, _) in &observations {
                assert_ne!(&path, previous_path);
                assert_eq!(
                    fs::read(previous_path).unwrap(),
                    *previous_bytes,
                    "new surface must preserve existing baselines"
                );
            }
            let repeat = run("0");
            assert_ne!(
                repeat["bench_history"]["verdict"], "establish_baseline",
                "an identical surface must still use its prior"
            );
            // A copied or stale foreign document must not establish comparability.
            if observations.len() == 3 {
                let mut foreign: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                foreign["label"] = serde_json::json!("query:foreign");
                fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
                assert_eq!(run("0")["bench_history"]["verdict"], "establish_baseline");
                fs::write(&path, &bytes).unwrap();
            }
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&fs::read(&run_path).unwrap()).unwrap()
                    ["label"],
                meta["label"]
            );
            observations.push((path, bytes, run_path));
        }
    }
    assert_eq!(observations.len(), 4);
}

/// INTENT: zero samples are invalid usage in every bench lane, before any
/// indexing or keep-history effects. A successful empty timing is not a baseline.
#[test]
fn zero_iteration_bench_is_usage_failure_without_side_effects() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "fn zero_target() {}\n").unwrap();
    let queries = temp.path().join("queries.txt");
    fs::write(&queries, "zero_target\n").unwrap();
    let mut outcomes = Vec::new();
    for (lane, extra) in [
        ("query", vec!["--query", "zero_target"]),
        ("suite", vec!["--suite", "default", "--fixture", "sample"]),
        ("batch", vec!["--queries-file", queries.to_str().unwrap()]),
    ] {
        let index = temp.path().join(format!("{lane}.sqlite"));
        let history = temp.path().join(format!("{lane}-history.json"));
        let priors = temp.path().join(format!("{lane}-priors"));
        let output = Command::new(env!("CARGO_BIN_EXE_asgrep"))
            .args(["--json", "--no-embed", "--index-path"])
            .arg(&index)
            .arg("bench")
            .arg(&root)
            .args(["--iterations", "0"])
            .args(extra)
            .env("ASGREP_BENCH_HISTORY", "1")
            .env("ASGREP_BENCH_HISTORY_PATH", &history)
            .env("ASGREP_BENCH_HISTORY_DIR", &priors)
            .env("ASGREP_BENCH_HISTORY_COMMIT", "1")
            .env("ASGREP_BENCH_RATCHET", "0")
            .output()
            .unwrap();
        let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        outcomes.push((
            lane,
            output.status.code(),
            reply["ok"].as_bool(),
            index.exists() || history.exists() || priors.exists(),
        ));
    }
    assert_eq!(
        outcomes,
        vec![
            ("query", Some(1), Some(false), false),
            ("suite", Some(1), Some(false), false),
            ("batch", Some(1), Some(false), false),
        ],
        "every dispatch lane must refuse zero samples before opening the index or writing history"
    );
}

#[cfg(unix)]
#[test]
fn nonregular_bench_text_inputs_fail_without_blocking_or_history_rewrite() {
    use std::os::unix::fs::FileTypeExt;
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "fn fifo_target() {}\n").unwrap();
    for lane in ["queries", "history"] {
        let fifo = temp.path().join(format!("{lane}.fifo"));
        assert!(Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let index = temp.path().join(format!("{lane}.db"));
        let regular_history = temp.path().join("unused-history.json");
        let history = if lane == "history" {
            &fifo
        } else {
            &regular_history
        };
        let priors = temp.path().join(format!("{lane}-priors"));
        let option = if lane == "history" {
            "--query"
        } else {
            "--queries-file"
        };
        let value = if lane == "history" {
            "fifo_target"
        } else {
            fifo.to_str().unwrap()
        };
        let output = ast_sgrep_testkit::run_drain_timeout(
            &ast_sgrep_testkit::asgrep_bin(),
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index.to_str().unwrap(),
                "bench",
                root.to_str().unwrap(),
                "--iterations",
                "1",
                option,
                value,
            ],
            &[
                ("ASGREP_BENCH_HISTORY", "1"),
                ("ASGREP_BENCH_HISTORY_PATH", history.to_str().unwrap()),
                ("ASGREP_BENCH_HISTORY_DIR", priors.to_str().unwrap()),
                ("ASGREP_BENCH_HISTORY_COMMIT", "1"),
                ("ASGREP_BENCH_RATCHET", "0"),
            ],
            std::time::Duration::from_secs(5),
        );
        ast_sgrep_testkit::assert_operational_envelope(&output);
        assert!(
            fs::metadata(&fifo).unwrap().file_type().is_fifo(),
            "input must not be replaced"
        );
        assert!(
            !priors.exists(),
            "refused input must not write a keep prior"
        );
        if lane == "queries" {
            assert!(!index.exists(), "query ingress must refuse before indexing");
        }
    }
}

#[test]
fn bench_fails_loudly_on_corrupt_history_file() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("lib.rs"), "fn bench_target() { run(1); }\n").unwrap();

    let history = temp.path().join("bench-history.json");
    fs::write(&history, "{ this is not json ]").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_asgrep"))
        .env("ASGREP_BENCH_HISTORY_PATH", &history)
        .args([
            "--no-embed",
            "bench",
            "--query",
            "bench_target",
            "--iterations",
            "1",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("run asgrep bench");

    assert!(
        !output.status.success(),
        "bench must exit non-zero when the keep-gate history file is corrupt; \
         stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        combined.contains("corrupt"),
        "the failure must tell the human the history file is corrupt: {combined}"
    );
    assert!(
        combined.contains(history.to_str().unwrap()),
        "the failure must name the corrupt history file: {combined}"
    );

    // The corrupt file is the human's evidence: bench must NOT auto-delete or
    // auto-reset it. Only a successful rewrite with a VALID prior would be
    // acceptable, and that path is unreachable from a corrupt parse.
    let after = fs::read_to_string(&history).unwrap();
    assert_eq!(
        after, "{ this is not json ]",
        "bench must never silently overwrite a corrupt history file"
    );
}
