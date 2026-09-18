//! T1 default-feature-surface inventory for `asgrep`.
//!
//! Pins the DEFAULT-build (`default = []`) CLI contract for the
//! `neural-embed` / `rerank` feature surface, which the CLI crate forwards to
//! `ast-sgrep-core`:
//!
//! * The tuning flags exist in clap surface (help/capabilities) on every build.
//! * Default search/index runs fully offline on local hashed embeddings and
//!   stamps `semantic-v2`.
//! * `search --rerank` / `search --neural-embed` fail closed (exit 2,
//!   operational) on a default build; `--rerank-top-k` alone is a no-op and
//!   `--neural-embed --no-embed` still succeeds (validation only fires when
//!   embeddings are on).
//! * `index --neural-embed` degrades to local embeddings (exit 0, `semantic-v2`
//!   stamp) instead of failing.
//!
//! Assertions pin exit codes, JSON shapes, and counts only — never message text.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn run(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(asgrep_bin());
    cmd.args(args).env("NO_COLOR", "1");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("run asgrep")
}

fn run_json(args: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
    let output = run(args, envs);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let value = serde_json::from_slice::<Value>(output.stdout.as_slice())
        .unwrap_or_else(|error| panic!("stdout is not JSON: {error}\nstdout: {stdout}\nstderr: {stderr}"));
    (output.status.code().expect("exit code"), value, stderr)
}

/// Temp fixture: one Rust file with a caller/callee pair, indexed with default flags.
struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    index: PathBuf,
}

impl Fixture {
    fn indexed() -> Self {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("root");
        fs::create_dir(&root).expect("root dir");
        fs::write(
            root.join("a.rs"),
            "fn alpha_query_target() {}\nfn beta_helper() { alpha_query_target(); }\n",
        )
        .expect("source");
        let index = temp.path().join("index.db");
        let (code, value, stderr) = run_json(
            &[
                "--index-path",
                index.to_str().unwrap(),
                "--json",
                "index",
                root.to_str().unwrap(),
            ],
            &[],
        );
        assert_eq!(code, 0, "stderr={stderr} value={value}");
        assert!(stderr.is_empty(), "machine mode stderr must be empty: {stderr}");
        assert_eq!(value["ok"], true);
        assert!(value["files_indexed"].as_u64().unwrap_or(0) >= 1);
        Self { _temp: temp, root, index }
    }

    fn search(&self, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
        let mut args: Vec<&str> = vec![
            "--index-path",
            self.index.to_str().unwrap(),
            "--json",
            "search",
        ];
        args.extend_from_slice(extra);
        args.push("alpha_query_target");
        args.push(self.root.to_str().unwrap());
        run_json(&args, envs)
    }
}

#[test]
fn version_json_shape() {
    let (code, value, stderr) = run_json(&["version", "--json"], &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "version");
    assert_eq!(value["tool"], "asgrep");
    assert!(value["version"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(value["index_schema_version"].is_u64());
    assert!(value["machine_schema_version"].is_string());
}

#[test]
fn version_text_and_clap_version_shapes() {
    let output = run(&["version"], &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.lines().count(), 3);
    assert!(stdout.lines().all(|line| !line.is_empty()));

    let output = run(&["--version"], &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.lines().count(), 1);
    assert!(!stdout.trim().is_empty());
}

#[test]
fn help_exits_zero_and_unknown_flag_is_usage_error() {
    let output = run(&["search", "--help"], &[]);
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());

    let output = run(&["--help"], &[]);
    assert!(output.status.success());

    // Discriminator for the surface tests below: unknown flags are usage
    // errors (exit 1), so a non-1 exit proves a flag exists in clap surface.
    let output = run(&["search", "--definitely-not-a-flag", "q", "."], &[]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn default_search_runs_offline_on_local_embeddings() {
    let fx = Fixture::indexed();
    let (code, value, stderr) = fx.search(&[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode stderr must be empty: {stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty());

    let (code, status, stderr) = run_json(
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "status",
            fx.root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(status["ok"], true);
    assert_eq!(status["embed_backend"], "semantic-v2");
    assert!(status["embed_dim"].as_u64().unwrap_or(0) > 0);
    assert!(status["semantic_chunk_count"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn semantic_only_search_runs_offline() {
    let fx = Fixture::indexed();
    let (code, value, stderr) = fx.search(&["--semantic-only"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());

    let (code, value, stderr) = run_json(
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "semantic",
            "alpha_query_target",
            fx.root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
}

/// Default build: every `--rerank` entry point fails closed as operational (exit 2).
#[test]
#[cfg(not(feature = "rerank"))]
fn rerank_flag_fails_closed_on_default_build() {
    let fx = Fixture::indexed();
    for extra in [&["--rerank"][..], &["--rerank", "--rerank-top-k", "5"][..]] {
        let (code, value, stderr) = fx.search(extra, &[]);
        assert_eq!(code, 2, "stderr={stderr} value={value}");
        assert!(stderr.is_empty());
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["kind"], "operational");
    }
    // Keyword channel entry point.
    let (code, value, stderr) = run_json(
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "keyword",
            "--rerank",
            "alpha_query_target",
            fx.root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    // Bare-search and env entry points.
    let output = run(
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "--rerank",
            "alpha_query_target",
            fx.root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(output.status.code(), Some(2));
    let (code, value, _) = fx.search(&[], &[("ASGREP_RERANK", "1")]);
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
}

/// With the `rerank` feature the flag is accepted; a missing local model
/// degrades to unranked hits (still exit 0), so this stays offline-safe.
#[test]
#[cfg(feature = "rerank")]
fn rerank_flag_accepted_with_feature() {
    let fx = Fixture::indexed();
    let (code, value, _) = fx.search(&["--rerank"], &[]);
    assert_eq!(code, 0, "value={value}");
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
}

/// Default build: `--neural-embed` search fails closed as operational (exit 2).
#[test]
#[cfg(not(feature = "neural-embed"))]
fn neural_embed_search_fails_closed_on_default_build() {
    let fx = Fixture::indexed();
    let (code, value, stderr) = fx.search(&["--neural-embed"], &[]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["kind"], "operational");
}

/// `--no-embed` wins over `--neural-embed`: with embeddings off there is no
/// neural path to gate, so search succeeds on every build.
#[test]
fn neural_embed_with_no_embed_still_succeeds() {
    let fx = Fixture::indexed();
    let (code, value, stderr) = fx.search(&["--neural-embed", "--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
}

/// `--rerank-top-k` without `--rerank` is accepted and inert on every build.
#[test]
fn rerank_top_k_alone_is_accepted_noop() {
    let fx = Fixture::indexed();
    let (code, value, stderr) = fx.search(&["--rerank-top-k", "5"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
}

/// Default build: `index --neural-embed` degrades to local embeddings
/// (exit 0) and stamps the resolved `semantic-v2` backend.
#[test]
#[cfg(not(feature = "neural-embed"))]
fn index_neural_embed_degrades_to_local_on_default_build() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("root dir");
    fs::write(root.join("a.rs"), "fn alpha_query_target() {}\n").expect("source");
    let index = temp.path().join("index.db");
    let (code, value, stderr) = run_json(
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "index",
            "--neural-embed",
            root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    let (code, status, _) = run_json(
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "status",
            root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0);
    assert_eq!(status["embed_backend"], "semantic-v2");
    assert!(status["semantic_chunk_count"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn capabilities_advertises_tuning_surface() {
    let (code, value, stderr) = run_json(&["capabilities", "--json"], &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    let flags = value["search_tuning_flags"].as_array().expect("tuning flags");
    for flag in ["--no-embed", "--neural-embed", "--semantic-only", "--rerank", "--rerank-top-k"] {
        assert!(flags.iter().any(|f| f == flag), "missing {flag} in {flags:?}");
    }
    let env = value["environment"].as_array().expect("environment");
    for var in ["ASGREP_NEURAL_EMBED", "ASGREP_RERANK", "ASGREP_RERANK_TOP_K"] {
        assert!(env.iter().any(|e| e == var), "missing {var} in {env:?}");
    }
}
