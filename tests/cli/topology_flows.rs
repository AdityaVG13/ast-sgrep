//! Topology flow drills for `asgrep`: one full end-to-end transcript per
//! feature-set cell, plus the rerank accept-side.
//!
//! The four cell drills run the SAME chained transcript (`index` -> `status`
//! -> `search` -> `keyword` -> `semantic` -> `outline` x2 -> `doctor`) on the
//! fixed 2-file tree and pin the SAME exact counts, key values, and hit
//! sequences — so green runs under all five sets prove the default flow is
//! IDENTICAL across sets. Each drill then pins its cell's unique
//! misuse/accept compositions (combined flags, `semantic --rerank`); the
//! fifth test pins the rerank degrade path on the 1-file tree.
//!
//! * `default_cell_full_flow_and_combined_misuse` — both halves off: every
//!   composition rejects.
//! * `neural_cell_full_flow_and_rerank_misuse` — rerank half rejects on
//!   combined forms; neural dry-run plan unchanged.
//! * `rerank_cell_full_flow_and_neural_misuse` — neural half rejects on
//!   combined forms; embed-off accepts; `semantic --rerank` order marker.
//! * `all_features_cell_full_flow_and_guarded_use` — nothing rejects;
//!   conjunction deep-equal + order marker + dry-run unchanged.
//! * `rerank_degrade_preserves_local_results` — accept-side deep-equality on
//!   all rerank entries (spans rerank-only + all-features).
//!
//! Download-safety: `--neural-embed` with embeddings on is never run against
//! a real root under `neural-embed`/`all-features`. Neural-on exposure is
//! limited to proven-safe forms: `index --dry-run` (returns before any
//! embedder exists) and `--no-embed` combinations (embed-off skips validation
//! and never constructs an embedder). Combined probes under neural builds fail
//! on the rerank half first (gate-first ordering: validation runs before root
//! canonicalization or embedder construction). All runs inherit testkit's
//! hermetic env (`HF_HUB_OFFLINE=1`, ambient `ASGREP_*` scrubbed).
//!
//! Assertions pin exit codes, JSON shapes, counts, and bytes only — never
//! message text.

use ast_sgrep_testkit::{asgrep_bin, run_env, run_json_full};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// Why file-local: fixed 2-file tree (four symbols, three callers; `b.rs`
// calls back into `a.rs`) yielding the exact cross-file transcript; the
// 1-file tree cannot pin cross-file sequences.
struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    index: PathBuf,
}

impl Fixture {
    fn two_file() -> Self {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("root");
        fs::create_dir(&root).expect("root dir");
        fs::write(
            root.join("a.rs"),
            "fn alpha_query_target() {}\nfn beta_helper() { alpha_query_target(); }\n",
        )
        .expect("source a.rs");
        fs::write(
            root.join("b.rs"),
            "fn gamma_extra() {}\nfn delta_caller() { gamma_extra(); alpha_query_target(); }\n",
        )
        .expect("source b.rs");
        let index = temp.path().join("index.db");
        Self {
            _temp: temp,
            root,
            index,
        }
    }

    // Why file-local: one-file tree for the degrade test (matches the T3
    // 1-file corpus the degrade relation was written against).
    // Gated to match its sole caller (cfg rerank).
    #[cfg(feature = "rerank")]
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
        assert_eq!(value["ok"], true);
        Self {
            _temp: temp,
            root,
            index,
        }
    }

    fn index_strs(&self) -> (String, String) {
        (
            self.index.to_str().unwrap().to_owned(),
            self.root.to_str().unwrap().to_owned(),
        )
    }

    fn search(&self, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
        self.channel("search", extra, envs)
    }

    fn channel(&self, cmd: &str, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
        let (index, root) = self.index_strs();
        let mut owned: Vec<String> = vec![
            "--index-path".to_owned(),
            index,
            "--json".to_owned(),
            cmd.to_owned(),
        ];
        owned.extend(extra.iter().map(ToString::to_string));
        owned.push("alpha_query_target".to_owned());
        owned.push(root);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let bin = asgrep_bin();
        run_json_full(&bin, &refs, envs)
    }
}

// Why file-local: shared full-flow transcript with the exact values every
// feature set must emit. Called by exactly one gated cell test per build, so
// green runs across all five sets prove cross-set identity of the default
// flow on the fixed tree.
fn assert_default_flow() -> Fixture {
    let fx = Fixture::two_file();
    let (index, root) = fx.index_strs();
    let bin = asgrep_bin();

    let (code, value, stderr) = run_json_full(
        &bin,
        &["--index-path", &index, "--json", "index", &root],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert_eq!(value["files_indexed"], 2);
    assert_eq!(value["symbols_extracted"], 4);
    assert_eq!(value["callers_extracted"], 3);
    assert_eq!(value["files_failed"], 0);
    assert_eq!(value["walk_errors"], false);

    let (code, status, stderr) = run_json_full(
        &bin,
        &["--index-path", &index, "--json", "status", &root],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(status["ok"], true);
    assert_eq!(status["embed_backend"], "semantic-v2");
    assert_eq!(status["embed_dim"], 256);
    assert_eq!(status["semantic_chunk_count"], 6);
    assert_eq!(status["symbol_count"], 4);
    assert_eq!(status["file_count"], 2);
    assert_eq!(status["caller_count"], 3);
    assert_eq!(status["line_count"], 6);

    let (code, value, stderr) = fx.search(&[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits array");
    let seq: Vec<(Option<&str>, Option<&str>, Option<&str>)> = hits
        .iter()
        .map(|h| (h["kind"].as_str(), h["symbol"].as_str(), h["file"].as_str()))
        .collect();
    // Three rows, not four: RRF fuses one row per (file, line) and the b.rs:2
    // exact evidence folds into the caller row (visible in its contributors).
    // The Sep 18 consolidation snapshot a fourth standalone asgrep row on
    // b.rs that line-fusion cannot emit; H1 + §41.3 pin n1/line at the CLI.
    assert_eq!(
        seq,
        [
            (Some("def"), Some("alpha_query_target"), Some("a.rs")),
            (Some("caller"), Some("alpha_query_target"), Some("a.rs")),
            (Some("caller"), Some("alpha_query_target"), Some("b.rs")),
        ],
        "value={value}"
    );

    let (code, value, stderr) = fx.channel("keyword", &[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits array");
    let seq: Vec<(Option<&str>, Option<&str>)> = hits
        .iter()
        .map(|h| (h["kind"].as_str(), h["file"].as_str()))
        .collect();
    assert_eq!(
        seq,
        [
            (Some("asgrep"), Some("a.rs")),
            (Some("asgrep"), Some("a.rs")),
            (Some("asgrep"), Some("b.rs")),
        ],
        "value={value}"
    );

    let (code, value, stderr) = fx.channel("semantic", &[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    let symbols: Vec<&str> = value["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol"].as_str().expect("symbol"))
        .collect();
    assert_eq!(
        symbols,
        ["alpha_query_target", "delta_caller", "beta_helper"]
    );

    for (file, names) in [
        ("a.rs", ["alpha_query_target", "beta_helper"]),
        ("b.rs", ["gamma_extra", "delta_caller"]),
    ] {
        let output = run_env(
            &bin,
            &["--index-path", &index, "--json", "outline", file, &root],
            &[],
        );
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout).expect("outline JSON");
        assert_eq!(value["ok"], true);
        assert_eq!(value["file"], file);
        assert_eq!(value["count"], 2);
        let symbols = value["symbols"].as_array().expect("symbols");
        let got: Vec<&str> = symbols
            .iter()
            .map(|s| s["name"].as_str().expect("name"))
            .collect();
        assert_eq!(got, names);
        assert!(symbols.iter().all(|s| s["kind"] == "function"));
    }

    let (code, value, stderr) = run_json_full(
        &bin,
        &["--index-path", &index, "--json", "doctor", &root],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    assert_eq!(value["healthy"], true);
    assert_eq!(value["issues"].as_array().map(Vec::len), Some(0));
    assert!(value["suggested_commands"]
        .as_array()
        .is_some_and(|c| !c.is_empty()));

    fx
}

// Why file-local: one-use rejection pin — exit 2, silent stderr, operational
// envelope — shared by all four cell drills.
fn assert_rejected(code: i32, value: &Value, stderr: &str) {
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(
        stderr.is_empty(),
        "machine mode stderr must be empty: {stderr}"
    );
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["error"]["kind"], "operational");
}

/// INTENT: default cell runs the shared transcript with identical values and
/// rejects every flag conjunction (both halves off) — flag form, env form,
/// the `semantic --rerank` entry point, and the `--no-embed` form (embed-off
/// skips the neural gate but the rerank gate still fires).
/// KILLS: gate-inversion, conjunction-error.
/// Absorbs: T4 default drill (kept verbatim; unique combined-misuse atoms).
#[test]
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
fn default_cell_full_flow_and_combined_misuse() {
    let fx = assert_default_flow();

    let (code, value, stderr) = fx.search(&["--rerank", "--neural-embed"], &[]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) =
        fx.search(&[], &[("ASGREP_RERANK", "1"), ("ASGREP_NEURAL_EMBED", "1")]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) = fx.channel("semantic", &["--rerank"], &[]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) = fx.search(&["--rerank", "--neural-embed", "--no-embed"], &[]);
    assert_rejected(code, &value, &stderr);
}

/// INTENT: neural-only cell runs the shared transcript with identical values;
/// combined compositions reject on the missing rerank half (gate runs before
/// any embedder construction, so no model load is reachable) and
/// `--neural-embed` leaves the dry-run plan byte-identical (parse-accept with
/// zero model exposure).
/// KILLS: gate-inversion, leak-across-sets.
/// Absorbs: T4 neural drill (kept verbatim) + T2 neural-only dry-run atoms
/// (DELETE; missing-root half covered by the identity exit taxonomy).
#[test]
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
fn neural_cell_full_flow_and_rerank_misuse() {
    let fx = assert_default_flow();

    let (code, value, stderr) = fx.search(&["--rerank", "--neural-embed"], &[]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) =
        fx.search(&[], &[("ASGREP_RERANK", "1"), ("ASGREP_NEURAL_EMBED", "1")]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) = fx.channel("semantic", &["--rerank"], &[]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) = fx.search(&["--rerank", "--neural-embed", "--no-embed"], &[]);
    assert_rejected(code, &value, &stderr);

    let (index, root) = fx.index_strs();
    let bin = asgrep_bin();
    let (code, plain, stderr) = run_json_full(
        &bin,
        &[
            "--index-path",
            &index,
            "--json",
            "index",
            "--dry-run",
            &root,
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    let (code, neural, stderr) = run_json_full(
        &bin,
        &[
            "--index-path",
            &index,
            "--json",
            "index",
            "--neural-embed",
            "--dry-run",
            &root,
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={neural}");
    assert_eq!(neural, plain, "neural flag changed the dry-run plan");
}

/// INTENT: rerank-only cell runs the shared transcript with identical values;
/// combined compositions reject on the missing neural half, while embed-off
/// clears both gates (deep-equal to plain embed-off) and `semantic --rerank`
/// is accepted with the documented order marker (same 3-hit set, ranks 2-3
/// swapped).
/// KILLS: gate-inversion, order-regression.
/// Absorbs: T4 rerank drill (kept verbatim; unique no-embed deep-equal +
/// `[alpha,beta,delta]` marker).
#[test]
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
fn rerank_cell_full_flow_and_neural_misuse() {
    let fx = assert_default_flow();

    let (code, value, stderr) = fx.search(&["--rerank", "--neural-embed"], &[]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) =
        fx.search(&[], &[("ASGREP_RERANK", "1"), ("ASGREP_NEURAL_EMBED", "1")]);
    assert_rejected(code, &value, &stderr);

    let (code, plain, stderr) = fx.search(&["--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    assert_eq!(plain["ok"], true);
    let (code, combined, _) = fx.search(&["--rerank", "--neural-embed", "--no-embed"], &[]);
    assert_eq!(code, 0, "value={combined}");
    assert_eq!(
        combined, plain,
        "combined no-embed diverged from plain no-embed"
    );

    let (code, ranked, _) = fx.channel("semantic", &["--rerank"], &[]);
    assert_eq!(code, 0, "value={ranked}");
    assert_eq!(ranked["ok"], true);
    let symbols: Vec<&str> = ranked["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol"].as_str().expect("symbol"))
        .collect();
    assert_eq!(
        symbols,
        ["alpha_query_target", "beta_helper", "delta_caller"]
    );
    let mut sorted = symbols.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        ["alpha_query_target", "beta_helper", "delta_caller"]
    );
}

/// INTENT: all-features cell runs the shared transcript with identical values;
/// both gates clear so nothing rejects — combined `--no-embed` deep-equals
/// plain embed-off, `semantic --rerank` carries the rerank-cell order marker,
/// and the neural dry-run plan is unchanged. No neural-on invocation touches
/// a real root.
/// KILLS: gate-inversion.
/// Absorbs: T4 all-features drill (kept verbatim; unique conjunction
/// accept-side) + T2 all-features atoms (DELETE; search/keyword/dry-run halves
/// subsumed by identity exact pins + this drill).
#[test]
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
fn all_features_cell_full_flow_and_guarded_use() {
    let fx = assert_default_flow();

    let (code, plain, stderr) = fx.search(&["--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    assert_eq!(plain["ok"], true);
    let (code, combined, _) = fx.search(&["--rerank", "--neural-embed", "--no-embed"], &[]);
    assert_eq!(code, 0, "value={combined}");
    assert_eq!(
        combined, plain,
        "combined no-embed diverged from plain no-embed"
    );

    let (code, ranked, _) = fx.channel("semantic", &["--rerank"], &[]);
    assert_eq!(code, 0, "value={ranked}");
    assert_eq!(ranked["ok"], true);
    let symbols: Vec<&str> = ranked["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol"].as_str().expect("symbol"))
        .collect();
    assert_eq!(
        symbols,
        ["alpha_query_target", "beta_helper", "delta_caller"]
    );

    let (index, root) = fx.index_strs();
    let bin = asgrep_bin();
    let (code, plain, _) = run_json_full(
        &bin,
        &[
            "--index-path",
            &index,
            "--json",
            "index",
            "--dry-run",
            &root,
        ],
        &[],
    );
    assert_eq!(code, 0, "value={plain}");
    let (code, neural, _) = run_json_full(
        &bin,
        &[
            "--index-path",
            &index,
            "--json",
            "index",
            "--neural-embed",
            "--dry-run",
            &root,
        ],
        &[],
    );
    assert_eq!(code, 0, "value={neural}");
    assert_eq!(neural, plain, "neural flag changed the dry-run plan");
}

/// INTENT: with `rerank` on and no local model, the degrade path preserves
/// local results exactly on every rerank entry point (spans rerank-only +
/// all-features cells). No stderr silence assertion: the degrade path may log.
/// KILLS: leak-across-sets.
/// Absorbs: T3 rerank degrade (kept verbatim) + T1 rerank accept (exit-0 half
/// subsumed by the deep-equality) + T2 rerank-only bare/env entries (widened
/// from rerank-only to all rerank builds).
#[test]
#[cfg(feature = "rerank")]
fn rerank_degrade_preserves_local_results() {
    let fx = Fixture::one_file();
    let bin = asgrep_bin();

    let (code, plain, stderr) = run_json_full(
        &bin,
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "keyword",
            "alpha_query_target",
            fx.root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    assert_eq!(plain["ok"], true);
    assert!(plain["hits"].as_array().is_some_and(|h| h.len() <= 16));
    let (code, ranked, _) = run_json_full(
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
    assert_eq!(code, 0, "value={ranked}");
    assert_eq!(ranked["ok"], true);
    assert_eq!(ranked, plain, "rerank degrade changed keyword results");

    let (code, plain, stderr) = fx.search(&[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    let (code, ranked, _) = fx.search(&["--rerank"], &[]);
    assert_eq!(code, 0, "value={ranked}");
    assert_eq!(
        ranked["hits"], plain["hits"],
        "rerank degrade changed search hits"
    );

    let (code, bare, _) = run_json_full(
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
    assert_eq!(code, 0, "value={bare}");
    assert_eq!(bare["ok"], true);
    assert!(bare["hits"].is_array());

    let (code, value, _) = fx.search(&[], &[("ASGREP_RERANK", "1")]);
    assert_eq!(code, 0, "value={value}");
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
}
