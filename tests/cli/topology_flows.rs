//! Topology flow drill for `asgrep`: one full end-to-end transcript
//! (`index` -> `status` -> `search` -> `keyword` -> `semantic` ->
//! `outline` x2 -> `doctor`) on a fixed 2-file tree, plus retired-flag
//! conjunction rejections. Phase 2 (br-6oh) deleted the feature cells this
//! file used to drill per-set; the transcript and misuse pins are now
//! universal.
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
    assert_eq!(status["embed_backend"], "static");
    assert_eq!(status["embed_dim"], 256);
    assert_eq!(status["semantic_chunk_count"], 10);
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
    // The three structural rows fuse one per (file, line): the b.rs:2 exact
    // evidence folds into the caller row (visible in its contributors).
    // A fourth, embed-only row (gamma_extra) trails: the page is thin, so
    // the learned embed channel generates candidates, and its name-field
    // cosine clears admission. Under the retired hashed default the same
    // fallback ran but hashed cosines never admitted a row here. The row
    // is marked semantic_uncorroborated and ranks last by a wide margin.
    assert_eq!(
        seq,
        [
            (Some("def"), Some("alpha_query_target"), Some("a.rs")),
            (Some("caller"), Some("alpha_query_target"), Some("a.rs")),
            (Some("caller"), Some("alpha_query_target"), Some("b.rs")),
            (Some("embed"), Some("gamma_extra"), Some("b.rs")),
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
    // Semantic channel ranking under the native static engine (Phase 2A
    // default in every feature cell): exact symbol first, then the
    // remaining symbols by learned name/body similarity. Callers rank
    // above the unrelated gamma_extra because the whole-body parent
    // chunk embeds the callee name; fragment-only chunking ranked them
    // on scraps and let gamma's name cosine win.
    assert_eq!(
        symbols,
        [
            "alpha_query_target",
            "beta_helper",
            "delta_caller",
            "gamma_extra"
        ]
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

/// INTENT: the shared transcript runs with identical values and every
/// retired-flag conjunction rejects — flag form, env form, the
/// `semantic --rerank` entry point, and the `--no-embed` form.
/// KILLS: gate-inversion, conjunction-error.
#[test]
fn full_flow_and_retired_conjunctions_reject() {
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
