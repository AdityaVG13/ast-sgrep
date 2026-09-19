//! Topology gate pins for `asgrep`: every optional-feature entry point fails
//! closed (or degrades, exactly once) with the canonical operational envelope.
//!
//! Covers the 2x2 matrix gate halves in four intent-grouped tests instead of
//! the nine scattered T1/T2/T3 pins:
//!
//! * `rerank_gates_fail_closed_with_canonical_envelope` — all five rerank
//!   entries reject whenever `rerank` is off (spans the default + neural-only
//!   cells).
//! * `neural_gates_fail_closed_with_canonical_envelope` — both neural entries
//!   reject whenever `neural-embed` is off (spans the default + rerank-only
//!   cells).
//! * `index_neural_flag_degrades_to_local` — the single documented asymmetry:
//!   `index --neural-embed` degrades to local embeddings where search rejects.
//! * `gates_open_on_offline_safe_probes_under_all_features` — both gates
//!   clear on every offline-safe probe when both features are on.
//!
//! Conjunction (`--rerank --neural-embed`) verdicts live in the per-cell flow
//! drills (`topology_flows.rs`), which pin the combined misuse/accept side per
//! cell; the lib-level truth table lives in core topology tests (one home).
//!
//! Download-safety: every test here is gated to a feature-OFF cell, so no run
//! can construct the neural embedder. All runs additionally inherit testkit's
//! hermetic env (`HF_HUB_OFFLINE=1`, ambient `ASGREP_*` scrubbed).
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
        assert!(stderr.is_empty(), "machine mode stderr must be empty: {stderr}");
        assert_eq!(value["ok"], true);
        assert!(value["files_indexed"].as_u64().unwrap_or(0) >= 1);
        Self { _temp: temp, root, index }
    }

    fn search(&self, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
        self.channel("search", extra, envs)
    }

    fn channel(&self, cmd: &str, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
        let mut args: Vec<&str> = vec![
            "--index-path",
            self.index.to_str().unwrap(),
            "--json",
            cmd,
        ];
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
    assert_eq!(object_keys(value), object_keys(reference), "{what} keys diverged");
    assert_eq!(
        object_keys(&value["error"]),
        object_keys(&reference["error"]),
        "{what} error keys diverged"
    );
}

/// INTENT: every rerank entry point fails closed with the canonical
/// operational envelope whenever `rerank` is off (spans default + neural-only
/// cells, so neural can never leak into the rerank gate outcome).
/// KILLS: gate-inversion, entry-point-leak, envelope-shape-leak.
/// Absorbs: T1 rerank 5-entry reject, T3 default-cell rerank half + neural-cell
/// rerank-unchanged, T2 neural-only rerank atom (DELETE).
#[test]
#[cfg(not(feature = "rerank"))]
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

/// INTENT: every neural entry point fails closed with the canonical
/// operational envelope whenever `neural-embed` is off (spans default +
/// rerank-only cells, so rerank can never leak into the neural gate outcome).
/// KILLS: gate-inversion, leak-across-sets, envelope-shape-leak.
/// Absorbs: T1 neural flag reject, T3 default-cell neural half + rerank-cell
/// neural-unchanged, T2 rerank-only binary atoms (DELETE).
#[test]
#[cfg(not(feature = "neural-embed"))]
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

/// INTENT: the single search/index gate asymmetry — `index --neural-embed`
/// degrades to local embeddings (exit 0, `semantic-v2` stamp) where `search
/// --neural-embed` rejects (see the test above).
/// KILLS: gate-inversion, stamp-regression.
/// Absorbs: T1 index degrade pin (only index-side pin; kept verbatim).
#[test]
#[cfg(not(feature = "neural-embed"))]
fn index_neural_flag_degrades_to_local() {
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
    assert_eq!(status["embed_backend"], "semantic-v2");
    assert!(status["semantic_chunk_count"].as_u64().unwrap_or(0) > 0);
}

/// INTENT: under all-features both gates clear on every offline-safe probe —
/// conjunctions accept when embed is off (flag + env forms), the neural flag
/// parses on the index path without loading a model, and `semantic --rerank`
/// preserves the hit set. No probe here can construct the neural embedder
/// (dry-run returns before any embedder exists; embed-off skips validation).
/// KILLS: gate-inversion, conjunction-error, load-on-safe-form (an accidental
/// model load fails fast under HF_HUB_OFFLINE instead of downloading).
/// Absorbs: new cell test closing the all-features hole in this file —
/// complements the F4 2-file conjunction/dry-run pins on the 1-file corpus;
/// env-conjunction accept is unpinned anywhere else.
#[test]
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
fn gates_open_on_offline_safe_probes_under_all_features() {
    let fx = Fixture::one_file();
    let bin = asgrep_bin();

    // Conjunction clears when embed is off: neural is skipped, rerank degrades
    // to the local result (deep-equal; no stderr silence pin — degrade may log).
    let (code, plain, stderr) = fx.search(&["--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    assert_eq!(plain["ok"], true);
    let (code, flag, _) = fx.search(&["--rerank", "--neural-embed", "--no-embed"], &[]);
    assert_eq!(code, 0, "value={flag}");
    assert_eq!(flag, plain, "flag conjunction diverged from plain no-embed");
    let (code, env, _) = fx.search(
        &["--no-embed"],
        &[("ASGREP_RERANK", "1"), ("ASGREP_NEURAL_EMBED", "1")],
    );
    assert_eq!(code, 0, "value={env}");
    assert_eq!(env, plain, "env conjunction diverged from plain no-embed");

    // Neural parses on the index path without loading: dry-run plan unchanged.
    let (code, dry_plain, _) = run_json_full(
        &bin,
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "index",
            "--dry-run",
            fx.root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "value={dry_plain}");
    let (code, dry_neural, _) = run_json_full(
        &bin,
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "index",
            "--neural-embed",
            "--dry-run",
            fx.root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "value={dry_neural}");
    assert_eq!(dry_neural, dry_plain, "neural flag changed the dry-run plan");

    // `semantic --rerank` accepted with the hit set preserved (order-insensitive:
    // the 2-file drill pins the order marker; here only set preservation).
    let (code, via_semantic, stderr) = fx.channel("semantic", &[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={via_semantic}");
    let (code, ranked, _) = fx.channel("semantic", &["--rerank"], &[]);
    assert_eq!(code, 0, "value={ranked}");
    assert_eq!(ranked["ok"], true);
    let mut got: Vec<&str> = ranked["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol"].as_str().expect("symbol"))
        .collect();
    assert!(!got.is_empty());
    let mut want: Vec<&str> = via_semantic["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol"].as_str().expect("symbol"))
        .collect();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want, "ranked hit set diverged");
}
