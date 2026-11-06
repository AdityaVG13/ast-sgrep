//! CLI oracle harness: draining runs, envelope runners, and the indexed
//! hand-corpus fixture shared by the `tests/cli/oracle_foundry_*` suites.
//!
//! # Contract
//!
//! - One canonical copy of the former `tests/cli/oracle_foundry_common.rs`
//!   harness plus the e2e seed builders and surface-key sort.
//! - `bin` is an explicit first parameter on every entry point: the exact
//!   binary path comes from `env!("CARGO_BIN_EXE_asgrep")`, which expands only
//!   in the test target — inside this dependency that env is unset, so the
//!   caller resolves it once (`fn asgrep()`) and passes it down.
//! - Every spawn goes through [`run_drain_timeout`] (hermetic env, pipes
//!   drained while waiting, 60s hard timeout): a wedged binary panics the
//!   test instead of hanging the suite, and large outputs can never deadlock
//!   past the 64KB pipe buffer (the `try_wait`-without-drain shape deadlocks
//!   on the ~300KB huge-line hit; proven >300s vs 0.016s direct).
//! - Arg vectors are byte-identical to the pass3/pass4 originals, including
//!   flag order and the trailing `--files-with-matches` after the root
//!   positional; human-face runs take flags AFTER the root positional.
//! - Runners panic (never `Result`) on spawn/timeout failure or non-JSON
//!   stdout; exit status is the caller's verdict.

use crate::cli_recovery::RUN_TIMEOUT;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;
use tempfile::TempDir;

/// INTENT: hermetic run that drains stdout/stderr while waiting, with a hard
/// timeout, via [`crate::spawn_hermetic`] (`Drain` + bound, no `cwd`
/// override). Scrubs inherited `ASGREP_*` (the parent env must not shift flag
/// defaults), pins `NO_COLOR=1` + `HF_HUB_OFFLINE=1`. Unlike
/// [`crate::cli_recovery::run_timeout`] the pipes are pumped by reader threads
/// during the wait, so outputs larger than the pipe buffer cannot deadlock the
/// child. No `current_dir` override: oracle invocations pass absolute paths.
/// Panics past `timeout`, killing the child. See [`crate::spawn::SpawnMode`]
/// for the deadlock facet this shape exists for.
pub fn run_drain_timeout(
    bin: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    timeout: Duration,
) -> Output {
    crate::spawn::spawn_hermetic(
        bin,
        args,
        envs,
        None,
        crate::spawn::SpawnMode::Drain,
        Some(timeout),
    )
}

/// INTENT: oracle spawn point — [`run_drain_timeout`] with [`RUN_TIMEOUT`].
fn spawn(bin: &Path, args: &[&str]) -> Output {
    run_drain_timeout(bin, args, &[], RUN_TIMEOUT)
}

/// INTENT: raw-bytes run returning (exit code, stdout bytes, lossy stderr):
/// byte-stability facets compare raw stdout across runs, and the JSON runners
/// drop the bytes. Panics when the child has no exit code.
pub fn oracle_run_raw(bin: &Path, args: &[&str]) -> (i32, Vec<u8>, String) {
    let output = spawn(bin, args);
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (code, output.stdout, stderr)
}

/// INTENT: full-envelope run (exit code, parsed body, raw stdout bytes, lossy
/// stderr) in one assertion block; [`crate::run_json_full`] drops the raw
/// bytes. Panics when stdout is not JSON or the child has no exit code.
pub fn oracle_run_json(bin: &Path, args: &[&str]) -> (i32, Value, Vec<u8>, String) {
    let output = spawn(bin, args);
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let value = crate::parse_stdout(&output);
    (code, value, output.stdout, stderr)
}

/// INTENT: verb-parametrized build runner returning the raw envelope: `index`
/// and `reindex` share one arg shape, and the verb parameter keeps the two
/// beats from drifting apart. [`crate::cli_recovery::run_index`] /
/// `run_reindex` are verb-fixed and assert-success, while oracles pin count
/// facets off the envelope. Panics when stdout is not JSON.
pub fn oracle_build_json(
    bin: &Path,
    verb: &str,
    root: &Path,
    index: &Path,
) -> (i32, Value, Vec<u8>, String) {
    oracle_run_json(
        bin,
        &[
            "--json",
            "--no-embed",
            "--index-path",
            index.to_str().unwrap(),
            verb,
            root.to_str().unwrap(),
        ],
    )
}

/// INTENT: indexed hand-corpus fixture — owns the TempDir lifetime plus the
/// absolute root/index paths and the target-resolved binary path every oracle
/// beat needs. (Cf. [`crate::CliSession::sample`], which is sample-root-fixed
/// instead of hand-fixture.)
pub struct OracleCorpus {
    /// Owns the corpus/index tree; must outlive `root`/`index` use.
    pub temp: TempDir,
    /// Hand-fixture corpus root.
    pub root: PathBuf,
    /// Explicit on-disk index path.
    pub index: PathBuf,
    bin: PathBuf,
}

impl OracleCorpus {
    /// INTENT: plant-then-index in one beat over `(rel, bytes)` files;
    /// returns the index envelope so count facets (files/symbols/failed)
    /// assert off the same run. Panics when indexing fails.
    pub fn index_files(bin: &Path, files: &[(&str, &[u8])]) -> (Self, Value) {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("corpus");
        std::fs::create_dir_all(&root).expect("mkdirs");
        for (rel, bytes) in files {
            crate::write_file(&root.join(rel), bytes);
        }
        let index = temp.path().join("index.db");
        let (code, value, _, stderr) = oracle_build_json(bin, "index", &root, &index);
        assert_eq!(code, 0, "index must succeed: {stderr}");
        (
            Self {
                temp,
                root,
                index,
                bin: bin.to_path_buf(),
            },
            value,
        )
    }

    /// INTENT: raw-bytes face for ad-hoc beats: exit code, raw stdout bytes,
    /// and scrubbed stderr text (no JSON parsing).
    pub fn run_raw(&self, args: &[&str]) -> (i32, Vec<u8>, String) {
        oracle_run_raw(&self.bin, args)
    }

    /// INTENT: JSON face for ad-hoc beats: parses stdout as JSON and also
    /// returns the raw bytes for byte-identity asserts.
    pub fn run_json(&self, args: &[&str]) -> (i32, Value, Vec<u8>, String) {
        oracle_run_json(&self.bin, args)
    }

    /// INTENT: channel-parametrized query runner (`search`/`keyword`/`chain`
    /// share one JSON arg shape at limit 50); the channel parameter is what
    /// makes the no-hits cross-channel loop possible.
    pub fn query_json(
        &self,
        channel: &str,
        query: &str,
        extra: &[&str],
    ) -> (i32, Value, Vec<u8>, String) {
        self.query_json_limit(channel, query, "50", extra)
    }

    /// INTENT: channel-parametrized query runner with an explicit limit: the
    /// limit-growth facet needs a non-default limit without rebuilding the arg
    /// vector by hand.
    pub fn query_json_limit(
        &self,
        channel: &str,
        query: &str,
        limit: &str,
        extra: &[&str],
    ) -> (i32, Value, Vec<u8>, String) {
        let mut owned: Vec<String> = vec![
            "--index-path".into(),
            self.index.to_str().unwrap().into(),
            "--no-embed".into(),
            "--no-auto-index".into(),
            "--json".into(),
            "--limit".into(),
            limit.into(),
        ];
        owned.extend(extra.iter().map(|s| s.to_string()));
        owned.push(channel.into());
        owned.push(query.into());
        owned.push(self.root.to_str().unwrap().into());
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.run_json(&refs)
    }

    /// INTENT: human-face runner with trailing-flag order: human-face runs
    /// take flags AFTER the root positional, exactly as the pass3 originals did.
    pub fn query_human(
        &self,
        channel: &str,
        query: &str,
        extra_after_root: &[&str],
    ) -> (i32, Vec<u8>, String) {
        let mut owned: Vec<String> = vec![
            "--index-path".into(),
            self.index.to_str().unwrap().into(),
            "--no-embed".into(),
            "--no-auto-index".into(),
            "--limit".into(),
            "50".into(),
            channel.into(),
            query.into(),
            self.root.to_str().unwrap().into(),
        ];
        owned.extend(extra_after_root.iter().map(|s| s.to_string()));
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.run_raw(&refs)
    }

    /// INTENT: outline JSON-face runner: outline takes `--root` + a
    /// root-relative path (no `--no-embed`), exactly as the pass3/pass4
    /// originals; parses stdout as JSON.
    pub fn outline_json(&self, rel: &str) -> (i32, Value, Vec<u8>, String) {
        self.run_json(&[
            "--index-path",
            self.index.to_str().unwrap(),
            "--no-auto-index",
            "--root",
            self.root.to_str().unwrap(),
            "outline",
            rel,
            "--json",
        ])
    }

    /// INTENT: outline human-face runner: same `--root` + relative-path shape
    /// as [`Self::outline_json`] but returns raw text via [`Self::run_raw`].
    pub fn outline_human(&self, rel: &str) -> (i32, Vec<u8>, String) {
        self.run_raw(&[
            "--index-path",
            self.index.to_str().unwrap(),
            "--no-auto-index",
            "--root",
            self.root.to_str().unwrap(),
            "outline",
            rel,
        ])
    }

    /// INTENT: status runner returning the raw envelope: status has its own
    /// flag shape (no `--no-embed`/`--no-auto-index` in the pass4 original),
    /// and [`crate::cli_recovery::run_status`] asserts success while oracles
    /// pin fail-closed codes too.
    pub fn status_json(&self) -> (i32, Value, Vec<u8>, String) {
        self.run_json(&[
            "--index-path",
            self.index.to_str().unwrap(),
            "--json",
            "status",
            self.root.to_str().unwrap(),
        ])
    }

    /// INTENT: reindex beat returning the raw envelope for the files_indexed facet.
    pub fn reindex(&self) -> (i32, Value, Vec<u8>, String) {
        oracle_build_json(&self.bin, "reindex", &self.root, &self.index)
    }
}

/// INTENT: order-free surface-key sort for the limit/filter/differential
/// facets; the backend's served order is pinned elsewhere (prefix-stability
/// facet). Pure.
pub fn sorted_surface_keys(mut keys: Vec<crate::HitKey>) -> Vec<crate::HitKey> {
    keys.sort();
    keys
}

/// INTENT: hand-corpus seed for the keyword face — one corpus whose file set
/// ({a.rs, b.rs}) and sentinel lines are fixed for every facet. Panics when
/// indexing fails.
pub fn oracle_keyword_corpus(bin: &Path) -> OracleCorpus {
    let (corpus, _) = OracleCorpus::index_files(bin, &[
        (
            "a.rs",
            b"fn alpha_one() {\n    let sentinel_alpha = 1;\n    println!(\"{sentinel_alpha}\");\n}\n\nfn alpha_two() {\n    let sentinel_alpha = 2;\n    println!(\"{sentinel_alpha}\");\n}\n\nfn plain_alpha() {}\n".as_slice(),
        ),
        (
            "b.rs",
            b"fn beta_one() {\n    let sentinel_alpha = 3;\n    println!(\"{sentinel_alpha}\");\n}\n\nfn plain_beta() {}\n".as_slice(),
        ),
        ("c.rs", b"fn plain_gamma() {}\n".as_slice()),
    ]);
    corpus
}

/// INTENT: hand-corpus seed for the search face — one corpus with a known call
/// graph (beta calls alpha; gamma isolated): 2 files, 3 symbols, 1 caller
/// edge. Panics when indexing fails or reports `ok: false`.
pub fn oracle_search_corpus(bin: &Path) -> OracleCorpus {
    let (corpus, value) = OracleCorpus::index_files(
        bin,
        &[
            (
                "a.rs",
                b"fn pass4_alpha() {}\nfn pass4_beta() { pass4_alpha(); }\n".as_slice(),
            ),
            ("b.rs", b"fn pass4_gamma() {}\n".as_slice()),
        ],
    );
    assert_eq!(value["ok"], true);
    corpus
}

/// INTENT: hand-corpus seed for the outline-agreement facet — exactly three
/// one-line fns in line order. Panics when indexing fails.
pub fn oracle_outline_corpus(bin: &Path) -> OracleCorpus {
    let (corpus, _) = OracleCorpus::index_files(
        bin,
        &[(
            "src/main.rs",
            b"fn alpha() {}\nfn beta() {}\nfn gamma() {}\n".as_slice(),
        )],
    );
    corpus
}
