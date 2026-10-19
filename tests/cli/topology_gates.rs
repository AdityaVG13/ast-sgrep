//! Retired-surface pins for `asgrep`: the removed neural/rerank entry
//! points fail closed with the canonical operational envelope.
//!
//! Phase 2 (br-6oh) deleted the `neural-embed`/`rerank` features entirely;
//! their flags and env vars remain parseable solely so they can be rejected
//! loudly (exit 2, operational envelope) instead of silently no-op'ing.
//!
//! Assertions pin exit codes, JSON shapes, key sets, and counts only — never
//! message text.

use ast_sgrep_testkit::{asgrep_bin, run_json_full};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// Why file-local: one-use key-set comparator for the cross-cause envelope
// equivalence pins; testkit has no key-set helper.
fn object_keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("envelope is an object")
        .keys()
        .cloned()
        .collect()
}

// Why file-local: fixed 1-file caller/callee tree; `CliSession::sample` uses a
// different corpus whose counts this suite must not depend on.
struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    index: PathBuf,
}

impl Fixture {
    fn one_file() -> Self {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("root");
        fs::create_dir(&root).expect("root dir");
        fs::write(
            root.join("a.rs"),
            "fn alpha_query_target() {}\nfn beta_helper() { alpha_query_target(); }\n",
        )
        .expect("source");
        let index = temp.path().join("index.db");
        let bin = asgrep_bin();
        let (code, value, stderr) = run_json_full(
            &bin,
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
        assert!(
            stderr.is_empty(),
            "machine mode stderr must be empty: {stderr}"
        );
        assert_eq!(value["ok"], true);
        assert!(value["files_indexed"].as_u64().unwrap_or(0) >= 1);
        Self {
            _temp: temp,
            root,
            index,
        }
    }

    fn search(&self, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
        self.channel("search", extra, envs)
    }

    fn channel(&self, cmd: &str, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
        let mut args: Vec<&str> = vec!["--index-path", self.index.to_str().unwrap(), "--json", cmd];
        args.extend_from_slice(extra);
        args.push("alpha_query_target");
        args.push(self.root.to_str().unwrap());
        let bin = asgrep_bin();
        run_json_full(&bin, &args, envs)
    }
}

// Why file-local: one-use canonical operational reference; every gate
// rejection envelope must be key-identical to it (cross-cause equivalence).
fn missing_root_envelope() -> (i32, Value) {
    let temp = TempDir::new().expect("tempdir");
    let index = temp.path().join("index.db");
    let missing = temp.path().join("missing");
    let bin = asgrep_bin();
    let (code, value, stderr) = run_json_full(
        &bin,
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "search",
            "alpha_query_target",
            missing.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["error"]["kind"], "operational");
    (code, value)
}

// Why file-local: one-use envelope+error key-equality assertion pairing with
// `object_keys` above; used only by this file's gate pins.
fn assert_key_identical(value: &Value, reference: &Value, what: &str) {
    assert_eq!(
        object_keys(value),
        object_keys(reference),
        "{what} keys diverged"
    );
    assert_eq!(
        object_keys(&value["error"]),
        object_keys(&reference["error"]),
        "{what} error keys diverged"
    );
}

/// INTENT: every retired rerank entry point fails closed with the canonical
/// operational envelope.
/// KILLS: gate-inversion, entry-point-leak, envelope-shape-leak.
#[test]
fn rerank_gates_fail_closed_with_canonical_envelope() {
    let fx = Fixture::one_file();
    let (_, reference) = missing_root_envelope();
    let bin = asgrep_bin();

    for extra in [&["--rerank"][..], &["--rerank", "--rerank-top-k", "5"][..]] {
        let (code, value, stderr) = fx.search(extra, &[]);
        assert_eq!(code, 2, "stderr={stderr} value={value}");
        assert!(stderr.is_empty());
        assert_eq!(value["ok"], false);
        assert_eq!(value["exit_code"], 2);
        assert_eq!(value["error"]["kind"], "operational");
        assert_key_identical(&value, &reference, "search --rerank");
    }

    let (code, value, stderr) = run_json_full(
        &bin,
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
    assert_key_identical(&value, &reference, "keyword --rerank");

    let (code, value, stderr) = run_json_full(
        &bin,
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
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_key_identical(&value, &reference, "bare --rerank");

    let (code, value, _) = fx.search(&[], &[("ASGREP_RERANK", "1")]);
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_key_identical(&value, &reference, "ASGREP_RERANK env");
}

/// INTENT: every retired neural entry point fails closed with the canonical
/// operational envelope.
/// KILLS: gate-inversion, envelope-shape-leak.
#[test]
fn neural_gates_fail_closed_with_canonical_envelope() {
    let fx = Fixture::one_file();
    let (_, reference) = missing_root_envelope();

    let (code, value, stderr) = fx.search(&["--neural-embed"], &[]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["error"]["kind"], "operational");
    assert_key_identical(&value, &reference, "search --neural-embed");

    let (code, value, _) = fx.search(&[], &[("ASGREP_NEURAL_EMBED", "1")]);
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_key_identical(&value, &reference, "ASGREP_NEURAL_EMBED env");
}

/// INTENT: retired flags fail on the index path too (no silent degrade),
/// and a plain index stamps the single static identity.
/// KILLS: gate-inversion, stamp-regression.
#[test]
fn index_rejects_retired_neural_flag_and_stamps_static() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("root dir");
    fs::write(root.join("a.rs"), "fn alpha_query_target() {}\n").expect("source");
    let index = temp.path().join("index.db");
    let bin = asgrep_bin();
    let (code, value, stderr) = run_json_full(
        &bin,
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
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], false);

    let (code, value, stderr) = run_json_full(
        &bin,
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
    assert_eq!(value["ok"], true);
    let (code, status, _) = run_json_full(
        &bin,
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
    assert_eq!(status["embed_backend"], "static");
    assert!(status["semantic_chunk_count"].as_u64().unwrap_or(0) > 0);
}
