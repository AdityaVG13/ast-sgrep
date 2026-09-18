//! T3 behavioral-delta tests for `asgrep`.
//!
//! Where T1 pins the DEFAULT-build contract and T2 pins the 2x2
//! (`neural-embed` x `rerank`) per-cell matrix, T3 pins CROSS-SET RELATIONS:
//! what stays identical across feature sets, what differs, and that each
//! feature never leaks into the other's surface. Nothing here duplicates
//! T1/T2 pins (per-cell exit codes, lib validation truth table); every test
//! below asserts a relation BETWEEN sets, channels, or envelopes:
//!
//! * Default-flow equivalence: ungated tests run the same EXACT assertions
//!   (counts, key sets, deep-equal envelopes/bytes) under all five sets
//!   (`default`, `rerank`, `neural-embed`, `all-features`,
//!   `--no-default-features`), so green-everywhere proves local
//!   index/search/outline flows are IDENTICAL regardless of features.
//! * Version/help/capabilities deltas: the exact delta set is EMPTY -- no
//!   `cfg(feature)` exists in the CLI crate, so these surfaces are pinned
//!   byte/shape-identical in every build (exact key sets, exact flag/env
//!   lists, exact help-marker occurrence counts).
//! * Flag behavior per set: the PARSE layer is feature-independent (every
//!   flag parses in every build; invalid values are usage errors everywhere)
//!   while the GATE layer varies per cell; each gated test carries its cell's
//!   cfg guard (with negative `not(...)` components) and the negative side of
//!   the rerank accept-pin lives in T1 plus tests 9-10 here.
//! * No-leak: rejection envelopes in feature cells are key-identical to the
//!   missing-root operational envelope, and enabling a feature never changes
//!   the other feature's gate outcome.
//!
//! Download-safety: no test runs `--neural-embed` (flag or env) against a
//! real root with embeddings on under `neural-embed`/`all-features`. The only
//! neural-on invocations are `--no-embed` combinations (embed-off path skips
//! validation and never constructs an embedder; the flag form is T1-proven in
//! all sets) and `ASGREP_NEURAL_EMBED=1` rejections confined to
//! `not(neural-embed)` cells. All runs additionally set `HF_HUB_OFFLINE=1` so
//! an accidental model load fails fast instead of downloading, and scrub
//! ambient `ASGREP_*` vars so the parent environment cannot shift the child.
//!
//! Assertions pin exit codes, JSON shapes, counts, and bytes only -- never
//! message text.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// Ambient `ASGREP_*` vars leak into the child through inherited env and would
/// shift flag defaults, so every run starts from a scrubbed hermetic env.
fn run(args: &[&str], envs: &[(&str, &str)]) -> Output {
    const SCRUB: &[&str] = &[
        "ASGREP_LIMIT",
        "ASGREP_INDEX_PATH",
        "ASGREP_DURABILITY",
        "ASGREP_NO_EMBED",
        "ASGREP_NO_AUTO_INDEX",
        "ASGREP_AUTO_INDEX",
        "ASGREP_NEURAL_EMBED",
        "ASGREP_NEURAL_FALLBACK",
        "ASGREP_SEMANTIC_ONLY",
        "ASGREP_TANTIVY",
        "ASGREP_ANN_THRESHOLD",
        "ASGREP_ANN_PROBES",
        "ASGREP_RERANK",
        "ASGREP_RERANK_TOP_K",
        "ASGREP_ALLOW_AST_GREP",
        "ASGREP_ALLOW_EXTERNAL_INDEX",
        "ASGREP_AST_GREP",
        "ASGREP_LEDGER_PATH",
        "ASGREP_USE_CACHE",
    ];
    let mut cmd = Command::new(asgrep_bin());
    cmd.args(args).env("NO_COLOR", "1").env("HF_HUB_OFFLINE", "1");
    for var in SCRUB {
        cmd.env_remove(var);
    }
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

fn object_keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("envelope is an object")
        .keys()
        .cloned()
        .collect()
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

/// Operational error envelope for a missing root: the canonical reference that
/// every feature-gate rejection envelope must be key-identical to.
fn missing_root_envelope() -> (i32, Value) {
    let temp = TempDir::new().expect("tempdir");
    let index = temp.path().join("index.db");
    let missing = temp.path().join("missing");
    let (code, value, stderr) = run_json(
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
    assert_eq!(value["error"]["kind"], "operational");
    (code, value)
}

// ---------------------------------------------------------------------------
// Cross-set identity: ungated tests pinning EXACT values in every build, so
// green runs under all five sets prove the delta set is empty.
// ---------------------------------------------------------------------------

/// `version --json` carries no feature markers: exact key set plus exact scalar
/// values (crate version, core index-schema const) in every build, and the
/// text form cross-checks the JSON form line by line.
#[test]
fn version_envelope_values_identical_across_sets() {
    let (code, value, stderr) = run_json(&["version", "--json"], &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.is_empty());
    let keys: BTreeSet<&str> = value
        .as_object()
        .expect("version envelope is an object")
        .keys()
        .map(String::as_str)
        .collect();
    let expected: BTreeSet<&str> = [
        "command",
        "exit_code",
        "index_schema_version",
        "machine_schema_version",
        "ok",
        "schema_version",
        "tool",
        "version",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected, "value={value}");
    assert_eq!(value["command"], "version");
    assert_eq!(value["tool"], "asgrep");
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["index_schema_version"], ast_sgrep_core::INDEX_SCHEMA_VERSION);
    // Machine schema tracks the envelope schema (relation, not a hardcoded copy).
    assert_eq!(value["machine_schema_version"], value["schema_version"]);
    assert!(value["machine_schema_version"].as_str().is_some_and(|v| !v.is_empty()));

    // JSON<->text relation: the 3 text lines restate the JSON scalars exactly.
    let output = run(&["version"], &[]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "text={text}");
    assert_eq!(lines[0], format!("asgrep {}", value["version"].as_str().unwrap()));
    assert_eq!(lines[1], format!("index_schema {}", value["index_schema_version"]));
    assert_eq!(
        lines[2],
        format!("machine_schema {}", value["machine_schema_version"].as_str().unwrap())
    );
}

/// `capabilities --json` carries no feature markers: the exact tuning-flag and
/// environment lists are pinned element-for-element in every build (T1 pins a
/// subset; this pins the full lists, so any per-feature drift fails).
#[test]
fn capabilities_exact_surface_identical_across_sets() {
    let (code, value, stderr) = run_json(&["capabilities", "--json"], &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    let tuning: Vec<&str> = value["search_tuning_flags"]
        .as_array()
        .expect("tuning flags")
        .iter()
        .map(|f| f.as_str().expect("flag str"))
        .collect();
    assert_eq!(
        tuning,
        [
            "--ann-probes",
            "--ann-threshold",
            "--budget-tokens",
            "--excerpt-lines",
            "--format",
            "--neural-embed",
            "--no-embed",
            "--rerank",
            "--rerank-top-k",
            "--response-snippet-tokens",
            "--semantic-only",
            "--snippet-tokens",
            "--tantivy",
        ],
        "tuning surface drifted"
    );
    let env: Vec<&str> = value["environment"]
        .as_array()
        .expect("environment")
        .iter()
        .map(|e| e.as_str().expect("env str"))
        .collect();
    assert_eq!(
        env,
        [
            "ASGREP_LIMIT",
            "ASGREP_INDEX_PATH",
            "ASGREP_DURABILITY",
            "ASGREP_NO_EMBED",
            "ASGREP_NO_AUTO_INDEX",
            "ASGREP_AUTO_INDEX",
            "ASGREP_NEURAL_EMBED",
            "ASGREP_NEURAL_FALLBACK",
            "ASGREP_SEMANTIC_ONLY",
            "ASGREP_TANTIVY",
            "ASGREP_ANN_THRESHOLD",
            "ASGREP_ANN_PROBES",
            "ASGREP_RERANK",
            "ASGREP_RERANK_TOP_K",
            "ASGREP_ALLOW_AST_GREP",
            "ASGREP_ALLOW_EXTERNAL_INDEX",
            "ASGREP_AST_GREP",
            "ASGREP_LEDGER_PATH",
            "ASGREP_USE_CACHE",
            "XDG_CACHE_HOME",
            "NO_COLOR",
            "CI",
            "TERM",
            "SOURCE_DATE_EPOCH",
        ],
        "environment surface drifted"
    );
}

/// Help carries exactly the documented feature markers in every build: both
/// optional flags plus the single `(needs neural-embed feature)` marker, with
/// exact occurrence counts (substring counts are clap-wrap independent).
#[test]
fn help_feature_markers_pinned_in_every_build() {
    let output = run(&["search", "--help"], &[]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(!help.is_empty());
    // Flag line + "(needs neural-embed feature)" marker line.
    assert_eq!(help.matches("neural-embed").count(), 2, "help={help}");
    // `--rerank` flag line, `--rerank-top-k` flag line, and its help text.
    assert_eq!(help.matches("--rerank").count(), 3, "help={help}");
    assert_eq!(help.matches("--rerank-top-k").count(), 1, "help={help}");
    assert_eq!(help.matches("(needs neural-embed feature)").count(), 1, "help={help}");

    let output = run(&["--help"], &[]);
    assert!(output.status.success());
    let top = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(top.contains("--neural-embed"), "top help lost neural marker");
    assert!(top.contains("--rerank"), "top help lost rerank marker");
}

/// Default index/status/search flows are exactly identical in every build:
/// exact extraction counts, exact status counters, exact hit count with the
/// expected def/caller channel mix -- and `--rerank-top-k` alone yields a
/// full-envelope deep-equal (inert-flag equivalence, stronger than T1's shape pin).
#[test]
fn default_flows_exact_and_topk_inert_equivalence() {
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
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert_eq!(value["files_indexed"], 1);
    assert_eq!(value["symbols_extracted"], 2);
    assert_eq!(value["callers_extracted"], 1);
    assert_eq!(value["files_failed"], 0);
    assert_eq!(value["walk_errors"], false);

    let (code, status, stderr) = run_json(
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "status",
            root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(status["ok"], true);
    assert_eq!(status["embed_backend"], "semantic-v2");
    assert_eq!(status["embed_dim"], 256);
    assert_eq!(status["semantic_chunk_count"], 3);
    assert_eq!(status["symbol_count"], 2);
    assert_eq!(status["file_count"], 1);
    assert_eq!(status["caller_count"], 1);

    let fx = Fixture { _temp: temp, root, index };
    let (code, base, stderr) = fx.search(&[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={base}");
    assert!(stderr.is_empty());
    assert_eq!(base["ok"], true);
    let hits = base["hits"].as_array().expect("hits array");
    assert_eq!(hits.len(), 2, "value={base}");
    let mut kinds: Vec<&str> = hits.iter().map(|h| h["kind"].as_str().expect("kind")).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["caller", "def"]);
    assert!(hits.iter().all(|h| h["symbol"] == "alpha_query_target"));

    let (code, inert, stderr) = fx.search(&["--rerank-top-k", "5"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={inert}");
    assert!(stderr.is_empty());
    assert_eq!(inert, base, "inert flag changed the envelope");
}

/// `--neural-embed` (flag and env forms) is fully ignored when embeddings are
/// off: full-envelope deep-equal across all three spellings in every build.
#[test]
fn neural_ignored_when_embed_off_equivalence() {
    let fx = Fixture::indexed();
    let (code, base, stderr) = fx.search(&["--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={base}");
    assert!(stderr.is_empty());
    assert_eq!(base["ok"], true);

    let (code, flag, stderr) = fx.search(&["--neural-embed", "--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={flag}");
    assert!(stderr.is_empty());
    assert_eq!(flag, base, "flag form diverged");

    let (code, env, stderr) = fx.search(&["--no-embed"], &[("ASGREP_NEURAL_EMBED", "1")]);
    assert_eq!(code, 0, "stderr={stderr} value={env}");
    assert!(stderr.is_empty());
    assert_eq!(env, base, "env form diverged");
}

/// The `semantic` channel and `search --semantic-only` return deep-equal hit
/// arrays in every build (cross-entry-point equivalence).
#[test]
fn semantic_channel_equivalence() {
    let fx = Fixture::indexed();
    let (code, via_search, stderr) = fx.search(&["--semantic-only"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={via_search}");
    assert!(stderr.is_empty());
    assert_eq!(via_search["ok"], true);
    let (code, via_semantic, stderr) = run_json(
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
    assert_eq!(code, 0, "stderr={stderr} value={via_semantic}");
    assert!(stderr.is_empty());
    assert_eq!(via_semantic["ok"], true);
    assert_eq!(via_semantic["hits"], via_search["hits"], "channel hits diverged");
    assert!(!via_semantic["hits"].as_array().expect("hits").is_empty());
}

/// Dry-run envelope and outline output are exactly identical in every build;
/// outline stdout is additionally byte-identical across two independent
/// indexes of the same root (determinism relation).
#[test]
fn index_dry_run_and_outline_exact_across_sets() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("root dir");
    fs::write(
        root.join("a.rs"),
        "fn alpha_query_target() {}\nfn beta_helper() { alpha_query_target(); }\n",
    )
    .expect("source");

    let dry_index = temp.path().join("dry.db");
    let (code, dry, stderr) = run_json(
        &[
            "--index-path",
            dry_index.to_str().unwrap(),
            "--json",
            "index",
            "--dry-run",
            root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={dry}");
    assert!(stderr.is_empty());
    assert_eq!(dry["ok"], true);
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["mutates_index"], false);
    assert_eq!(dry["walk_errors"], false);
    assert_eq!(dry["files_would_index"], 1);
    assert_eq!(dry["files_skipped"], 0);
    assert!(!dry_index.exists(), "dry-run must not create the index file");

    let mut outlines = Vec::new();
    for name in ["first.db", "second.db"] {
        let index = temp.path().join(name);
        let (code, value, _) = run_json(
            &[
                "--index-path",
                index.to_str().unwrap(),
                "--json",
                "index",
                root.to_str().unwrap(),
            ],
            &[],
        );
        assert_eq!(code, 0, "value={value}");
        let output = run(
            &[
                "--index-path",
                index.to_str().unwrap(),
                "--json",
                "outline",
                "a.rs",
                root.to_str().unwrap(),
            ],
            &[],
        );
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout).expect("outline JSON");
        assert_eq!(value["ok"], true);
        assert_eq!(value["command"], "outline");
        assert_eq!(value["file"], "a.rs");
        assert_eq!(value["count"], 2);
        let symbols = value["symbols"].as_array().expect("symbols");
        assert_eq!(symbols.len(), 2);
        let names: Vec<&str> = symbols.iter().map(|s| s["name"].as_str().expect("name")).collect();
        assert_eq!(names, ["alpha_query_target", "beta_helper"]);
        assert!(symbols.iter().all(|s| s["kind"] == "function"));
        outlines.push(output.stdout);
    }
    assert_eq!(outlines[0], outlines[1], "outline bytes diverged across indexes");
}

/// The parse layer is feature-independent in every build: unknown flags and
/// invalid flag values are usage errors (exit 1), a missing query is a usage
/// error, and a missing root is operational (exit 2) -- with usage envelopes
/// key-identical to each other.
#[test]
fn exit_taxonomy_and_parse_layer_invariant() {
    let temp = TempDir::new().expect("tempdir");
    let index = temp.path().join("index.db");

    let output = run(&["search", "--definitely-not-a-flag", "q", "."], &[]);
    assert_eq!(output.status.code(), Some(1));

    // `--rerank-top-k` is parsed by clap in every build: an invalid value is a
    // usage error (exit 1), never an unknown-flag or gate error.
    let (code, bad_value, stderr) = run_json(
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "search",
            "--rerank-top-k",
            "notanumber",
            "alpha_query_target",
            temp.path().to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 1, "stderr={stderr} value={bad_value}");
    assert_eq!(bad_value["ok"], false);
    assert_eq!(bad_value["error"]["kind"], "usage");
    assert_eq!(bad_value["exit_code"], 1);

    let (code, missing_query, _) = run_json(
        &["--index-path", index.to_str().unwrap(), "--json", "search"],
        &[],
    );
    assert_eq!(code, 1, "value={missing_query}");
    assert_eq!(missing_query["error"]["kind"], "usage");
    assert_eq!(object_keys(&missing_query), object_keys(&bad_value));
    assert_eq!(object_keys(&missing_query["error"]), object_keys(&bad_value["error"]));

    let (code, missing_root) = missing_root_envelope();
    assert_eq!(code, 2);
    assert_eq!(missing_root["ok"], false);
    assert_eq!(missing_root["exit_code"], 2);
}

// ---------------------------------------------------------------------------
// Per-cell gate relations: each gated test pins its cell's relations and
// carries negative cfg guards so it cannot run (and pass vacuously) under the
// wrong feature set. Complements: the rerank accept-side negative (rejection
// under `not(rerank)`) is T1's `rerank_flag_fails_closed_on_default_build`
// plus tests 9-10 here.
// ---------------------------------------------------------------------------

/// Neither cell: both optional flags (plus both env entries) are rejected as
/// operational, and every rejection envelope is key-identical to the canonical
/// missing-root operational envelope (cross-cause equivalence).
#[test]
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
fn default_cell_both_rejections_share_operational_envelope() {
    let fx = Fixture::indexed();
    let (_, reference) = missing_root_envelope();
    let ref_keys = object_keys(&reference);
    let ref_err_keys = object_keys(&reference["error"]);

    let (code, value, stderr) = fx.search(&["--rerank"], &[]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(value["exit_code"], 2);
    assert_eq!(object_keys(&value), ref_keys, "rerank envelope diverged");
    assert_eq!(object_keys(&value["error"]), ref_err_keys);

    let (code, value, stderr) = fx.search(&["--neural-embed"], &[]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(object_keys(&value), ref_keys, "neural envelope diverged");
    assert_eq!(object_keys(&value["error"]), ref_err_keys);

    // Env entries reject identically to their flag forms (no-leak: the gate
    // sees flag and env as one input).
    let (code, value, _) = fx.search(&[], &[("ASGREP_RERANK", "1")]);
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(object_keys(&value), ref_keys, "rerank env envelope diverged");
    let (code, value, _) = fx.search(&[], &[("ASGREP_NEURAL_EMBED", "1")]);
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(object_keys(&value), ref_keys, "neural env envelope diverged");
}

/// Neural-only cell: enabling `neural-embed` leaves the rerank gate untouched
/// (no-leak) -- `search` and `keyword` `--rerank` rejections stay operational
/// with envelopes key-identical to the canonical missing-root envelope.
#[test]
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
fn neural_cell_rerank_gate_unchanged_no_leak() {
    let fx = Fixture::indexed();
    let (_, reference) = missing_root_envelope();
    let ref_keys = object_keys(&reference);
    let ref_err_keys = object_keys(&reference["error"]);

    let (code, value, stderr) = fx.search(&["--rerank"], &[]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(value["exit_code"], 2);
    assert_eq!(object_keys(&value), ref_keys, "rerank envelope diverged");
    assert_eq!(object_keys(&value["error"]), ref_err_keys);

    let (code, value, _) = run_json(
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
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(object_keys(&value), ref_keys, "keyword rerank envelope diverged");

    let (code, value, _) = fx.search(&[], &[("ASGREP_RERANK", "1")]);
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(object_keys(&value), ref_keys, "rerank env envelope diverged");
}

/// Rerank-only cell: enabling `rerank` leaves the neural gate untouched
/// (no-leak) -- the flag and env neural entries stay operational rejections
/// with envelopes key-identical to the canonical missing-root envelope.
#[test]
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
fn rerank_cell_neural_gate_unchanged_no_leak() {
    let fx = Fixture::indexed();
    let (_, reference) = missing_root_envelope();
    let ref_keys = object_keys(&reference);
    let ref_err_keys = object_keys(&reference["error"]);

    let (code, value, stderr) = fx.search(&["--neural-embed"], &[]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(value["exit_code"], 2);
    assert_eq!(object_keys(&value), ref_keys, "neural envelope diverged");
    assert_eq!(object_keys(&value["error"]), ref_err_keys);

    let (code, value, _) = fx.search(&[], &[("ASGREP_NEURAL_EMBED", "1")]);
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
    assert_eq!(object_keys(&value), ref_keys, "neural env envelope diverged");
}

/// Rerank accept-side (rerank-only + all-features cells): with no local model
/// the degrade path preserves local results exactly -- `--rerank` envelopes
/// deep-equal their plain counterparts (accept-side no-leak). No stderr
/// silence assertion: the degrade path may log. Never run under `not(rerank)`
/// (negative side: T1 rejection pin plus tests 9-10 here).
#[test]
#[cfg(feature = "rerank")]
fn rerank_degrade_preserves_local_hits() {
    let fx = Fixture::indexed();

    let (code, plain, stderr) = run_json(
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
    let (code, ranked, _) = run_json(
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
    assert_eq!(ranked["hits"], plain["hits"], "rerank degrade changed search hits");
}
