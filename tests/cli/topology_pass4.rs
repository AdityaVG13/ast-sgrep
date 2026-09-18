//! T4 end-to-end topology drills for `asgrep`.
//!
//! Where T1 pins the DEFAULT-build contract, T2 pins the per-cell 2x2 matrix,
//! and T3 pins CROSS-SET RELATIONS on the 1-file tree, T4 runs FULL CLI FLOWS
//! per feature set through the real binary built under each set, on a fixed
//! 2-file tree (`a.rs`: caller/callee pair, `b.rs`: second pair calling back
//! into `a.rs`). Every drill below is new surface T1/T2/T3 never probe:
//!
//! * Full-flow cell drills (tests 1-4): each of the four cfg-gated cells runs
//!   the same chained transcript -- `index` -> `status` -> `search` ->
//!   `keyword` -> `semantic` -> `outline` (both files) -> `doctor` -- and pins
//!   the SAME exact counts, key values, and hit sequences. Green runs under
//!   all five sets (`default`, `rerank`, `neural-embed`, `all-features`,
//!   `--no-default-features`) therefore prove the default flow is IDENTICAL
//!   across sets. The only documented cross-set deltas are the gate exits
//!   below and the `semantic --rerank` order marker.
//! * Misuse/accept compositions (folded into tests 1-4): combined
//!   `--rerank --neural-embed` (flag form, env form, and `--no-embed` form)
//!   plus the `semantic --rerank` entry point. T1/T2/T3 probe each flag alone;
//!   no earlier pass combines them or touches `semantic --rerank`.
//! * Help/machine marker drills (tests 5-8): per-subcommand help marker
//!   counts for `index`/`keyword`/`semantic` (T3 pins `search` only), exact
//!   top-level `--help` counts (T3 pins `contains` only), zero-marker pins
//!   for `outline`/`status`/`doctor`, the clap `--version` <-> JSON cross
//!   check, and the full `capabilities` envelope (key set, exit-code deque,
//!   command roster, global flags, formats). Each drill is split into a
//!   negative-guard pair (`not(feature)` / `feature`) with identical bodies,
//!   so green-everywhere proves the per-set delta set is EMPTY along both
//!   feature dimensions.
//!
//! Download-safety: `--neural-embed` with embeddings on is never run against
//! a real root under `neural-embed`/`all-features`. Neural-on exposure is
//! limited to T2/T3-proven-safe forms: `index --dry-run` (returns before any
//! embedder exists) and `--no-embed` combinations (embed-off skips validation
//! and never constructs an embedder). Combined `--rerank --neural-embed`
//! probes under neural builds fail on the rerank half: `Searcher::new` runs
//! `validate_search_feature_flags` BEFORE root canonicalization or embedder
//! construction (gate-first ordering), and every run sets `HF_HUB_OFFLINE=1`
//! so an accidental model load fails fast instead of downloading. Ambient
//! `ASGREP_*` vars are scrubbed so the parent environment cannot shift the
//! child.
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

/// Hermetic run: scrub ambient `ASGREP_*` (they leak into the child and shift
/// flag defaults) and force the HF hub offline so a model load fails fast.
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

/// T4 fixed tree: two files, four symbols, three callers. `b.rs` calls back
/// into `a.rs` so the `alpha_query_target` hit set spans both files.
struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    index: PathBuf,
}

impl Fixture {
    fn build() -> Self {
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
        Self { _temp: temp, root, index }
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
        run_json(&refs, envs)
    }
}

/// Shared full-flow transcript: `index` -> `status` -> `search` -> `keyword`
/// -> `semantic` -> `outline` x2 -> `doctor`, with the exact values every
/// feature set must emit. Called by exactly one gated cell test per build, so
/// green runs across all five sets prove cross-set identity of the default
/// flow on the fixed tree.
fn assert_default_flow() -> Fixture {
    let fx = Fixture::build();
    let (index, root) = fx.index_strs();

    // index: exact extraction counts on the fixed tree.
    let (code, value, stderr) = run_json(
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

    // status: exact counters plus the resolved local-embedding backend.
    let (code, status, stderr) = run_json(
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

    // search: exact 4-hit cross-file sequence.
    let (code, value, stderr) = fx.search(&[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits array");
    let seq: Vec<(Option<&str>, Option<&str>, Option<&str>)> = hits
        .iter()
        .map(|h| (h["kind"].as_str(), h["symbol"].as_str(), h["file"].as_str()))
        .collect();
    assert_eq!(
        seq,
        [
            (Some("def"), Some("alpha_query_target"), Some("a.rs")),
            (Some("caller"), Some("alpha_query_target"), Some("a.rs")),
            (Some("caller"), Some("alpha_query_target"), Some("b.rs")),
            (Some("asgrep"), None, Some("b.rs")),
        ],
        "value={value}"
    );

    // keyword: exact 3-hit lexical sequence.
    let (code, value, stderr) = fx.channel("keyword", &[], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits array");
    let seq: Vec<(Option<&str>, Option<&str>)> =
        hits.iter().map(|h| (h["kind"].as_str(), h["file"].as_str())).collect();
    assert_eq!(
        seq,
        [
            (Some("asgrep"), Some("a.rs")),
            (Some("asgrep"), Some("a.rs")),
            (Some("asgrep"), Some("b.rs")),
        ],
        "value={value}"
    );

    // semantic: exact 3-hit symbol sequence in rank order.
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
    assert_eq!(symbols, ["alpha_query_target", "delta_caller", "beta_helper"]);

    // outline: exact per-file symbol rosters.
    for (file, names) in [
        ("a.rs", ["alpha_query_target", "beta_helper"]),
        ("b.rs", ["gamma_extra", "delta_caller"]),
    ] {
        let output = run(&["--index-path", &index, "--json", "outline", file, &root], &[]);
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout).expect("outline JSON");
        assert_eq!(value["ok"], true);
        assert_eq!(value["file"], file);
        assert_eq!(value["count"], 2);
        let symbols = value["symbols"].as_array().expect("symbols");
        let got: Vec<&str> =
            symbols.iter().map(|s| s["name"].as_str().expect("name")).collect();
        assert_eq!(got, names);
        assert!(symbols.iter().all(|s| s["kind"] == "function"));
    }

    // doctor: healthy envelope on the freshly indexed root.
    let (code, value, stderr) = run_json(
        &["--index-path", &index, "--json", "doctor", &root],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    assert_eq!(value["healthy"], true);
    assert_eq!(value["issues"].as_array().map(Vec::len), Some(0));
    assert!(value["suggested_commands"].as_array().is_some_and(|c| !c.is_empty()));

    fx
}

/// Rejection pin: exit 2, silent stderr, operational envelope.
fn assert_rejected(code: i32, value: &Value, stderr: &str) {
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode stderr must be empty: {stderr}");
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["error"]["kind"], "operational");
}

// ---------------------------------------------------------------------------
// Tests 1-4: full-flow cell drills. Each cell runs the shared transcript
// (identical values => cross-set identity) plus its cell-specific
// misuse/accept compositions. No earlier pass combines the flags or probes
// `semantic --rerank`.
// ---------------------------------------------------------------------------

/// Default cell: shared flow plus combined-flag misuse. Both optional paths
/// are off, so every composition -- flag form, env form, the unprobed
/// `semantic --rerank` entry point, and the `--no-embed` form (embed-off
/// skips the neural gate but the rerank gate still fires) -- rejects.
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

/// Neural-only cell: shared flow (identical values) plus rerank-half misuse.
/// Combined compositions reject on the missing rerank half -- the gate runs
/// before any embedder construction, so no model load is reachable -- and
/// `--neural-embed` parses on the index path without changing the dry-run
/// plan (deep-equal envelopes).
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

    // Dry-run is embedder-free: the neural flag parses yet leaves the plan
    // byte-identical (proves parse-accept with zero model exposure).
    let (index, root) = fx.index_strs();
    let (code, plain, stderr) = run_json(
        &["--index-path", &index, "--json", "index", "--dry-run", &root],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    let (code, neural, stderr) = run_json(
        &["--index-path", &index, "--json", "index", "--neural-embed", "--dry-run", &root],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={neural}");
    assert_eq!(neural, plain, "neural flag changed the dry-run plan");
}

/// Rerank-only cell: shared flow (identical values) plus neural-half misuse
/// and the rerank accept-side. Combined compositions reject on the missing
/// neural half; with `--no-embed` both gates clear and the result deep-equals
/// the plain embed-off search. `semantic --rerank` is accepted with the
/// documented order marker: same 3-hit set as the plain channel, ranks 2-3
/// swapped (`[alpha, beta, delta]` vs `[alpha, delta, beta]`).
#[test]
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
fn rerank_cell_full_flow_and_neural_misuse() {
    let fx = assert_default_flow();

    let (code, value, stderr) = fx.search(&["--rerank", "--neural-embed"], &[]);
    assert_rejected(code, &value, &stderr);

    let (code, value, stderr) =
        fx.search(&[], &[("ASGREP_RERANK", "1"), ("ASGREP_NEURAL_EMBED", "1")]);
    assert_rejected(code, &value, &stderr);

    // Embed-off clears both gates: neural is skipped, rerank degrades to the
    // local result (deep-equal; no stderr silence pin -- degrade may log).
    let (code, plain, stderr) = fx.search(&["--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    assert_eq!(plain["ok"], true);
    let (code, combined, _) = fx.search(&["--rerank", "--neural-embed", "--no-embed"], &[]);
    assert_eq!(code, 0, "value={combined}");
    assert_eq!(combined, plain, "combined no-embed diverged from plain no-embed");

    // `semantic --rerank`: accepted, same hit set, documented order marker.
    let (code, ranked, _) = fx.channel("semantic", &["--rerank"], &[]);
    assert_eq!(code, 0, "value={ranked}");
    assert_eq!(ranked["ok"], true);
    let symbols: Vec<&str> = ranked["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol"].as_str().expect("symbol"))
        .collect();
    assert_eq!(symbols, ["alpha_query_target", "beta_helper", "delta_caller"]);
    let mut sorted = symbols.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, ["alpha_query_target", "beta_helper", "delta_caller"]);
}

/// All-features cell: shared flow (identical values) plus guarded use. Both
/// gates clear, so nothing rejects: the combined `--no-embed` search
/// deep-equals the plain embed-off search (composition of the T3 neural-ignore
/// and rerank-degrade relations), `semantic --rerank` carries the same order
/// marker as the rerank-only cell, and the neural dry-run plan is unchanged.
/// No neural-on invocation touches a real root here.
#[test]
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
fn all_features_cell_full_flow_and_guarded_use() {
    let fx = assert_default_flow();

    let (code, plain, stderr) = fx.search(&["--no-embed"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={plain}");
    assert_eq!(plain["ok"], true);
    let (code, combined, _) = fx.search(&["--rerank", "--neural-embed", "--no-embed"], &[]);
    assert_eq!(code, 0, "value={combined}");
    assert_eq!(combined, plain, "combined no-embed diverged from plain no-embed");

    let (code, ranked, _) = fx.channel("semantic", &["--rerank"], &[]);
    assert_eq!(code, 0, "value={ranked}");
    assert_eq!(ranked["ok"], true);
    let symbols: Vec<&str> = ranked["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol"].as_str().expect("symbol"))
        .collect();
    assert_eq!(symbols, ["alpha_query_target", "beta_helper", "delta_caller"]);

    let (index, root) = fx.index_strs();
    let (code, plain, _) = run_json(
        &["--index-path", &index, "--json", "index", "--dry-run", &root],
        &[],
    );
    assert_eq!(code, 0, "value={plain}");
    let (code, neural, _) = run_json(
        &["--index-path", &index, "--json", "index", "--neural-embed", "--dry-run", &root],
        &[],
    );
    assert_eq!(code, 0, "value={neural}");
    assert_eq!(neural, plain, "neural flag changed the dry-run plan");
}

// ---------------------------------------------------------------------------
// Tests 5-6: help marker surface, split across the neural dimension with
// identical bodies. T3 pins `search --help` counts only; the per-subcommand
// counts, exact top-level counts, and zero-marker commands are new here.
// ---------------------------------------------------------------------------

/// Exact feature-marker census over every help surface: the four tuning-flag
/// subcommands carry identical marker counts, the top level matches them, and
/// the three flag-free subcommands carry zero markers. Substring counts are
/// clap-wrap independent.
fn assert_help_marker_surface() {
    for sub in ["index", "keyword", "semantic"] {
        let output = run(&[sub, "--help"], &[]);
        assert!(output.status.success());
        let help = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(!help.is_empty());
        assert_eq!(help.matches("neural-embed").count(), 2, "{sub} help={help}");
        assert_eq!(help.matches("--rerank").count(), 3, "{sub} help={help}");
        assert_eq!(help.matches("--rerank-top-k").count(), 1, "{sub} help={help}");
        assert_eq!(
            help.matches("(needs neural-embed feature)").count(),
            1,
            "{sub} help={help}"
        );
    }

    let output = run(&["--help"], &[]);
    assert!(output.status.success());
    let top = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(top.matches("neural-embed").count(), 2, "top help={top}");
    assert_eq!(top.matches("--rerank").count(), 3, "top help={top}");
    assert_eq!(top.matches("--rerank-top-k").count(), 1, "top help={top}");
    assert_eq!(top.matches("(needs neural-embed feature)").count(), 1, "top help={top}");

    // No tuning surface, no markers: these helps must not mention the flags.
    for sub in ["outline", "status", "doctor"] {
        let output = run(&[sub, "--help"], &[]);
        assert!(output.status.success());
        let help = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(!help.is_empty());
        assert_eq!(help.matches("neural-embed").count(), 0, "{sub} help={help}");
        assert_eq!(help.matches("--rerank").count(), 0, "{sub} help={help}");
    }

    // Clap `--version` restates the JSON version exactly (cross-entry check).
    let (code, value, stderr) = run_json(&["version", "--json"], &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    let output = run(&["--version"], &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout.lines().count(), 1);
    assert_eq!(stdout.trim(), format!("asgrep {}", value["version"].as_str().unwrap()));
}

#[test]
#[cfg(not(feature = "neural-embed"))]
fn non_neural_cells_help_marker_surface() {
    assert_help_marker_surface();
}

#[test]
#[cfg(feature = "neural-embed")]
fn neural_cells_help_marker_surface() {
    assert_help_marker_surface();
}

// ---------------------------------------------------------------------------
// Tests 7-8: machine contract surface, split across the rerank dimension with
// identical bodies. T1/T3 pin the tuning-flag and environment lists only; the
// envelope key set, exit-code deque, command roster, global flags, and format
// list are new here.
// ---------------------------------------------------------------------------

/// Full `capabilities` envelope census: exact key set, exit-code deque values,
/// command roster, global flags, and search formats -- plus the version and
/// schema relations to the `version` command.
fn assert_machine_surface() {
    let (code, value, stderr) = run_json(&["capabilities", "--json"], &[]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);

    let keys: BTreeSet<&str> = value
        .as_object()
        .expect("capabilities envelope is an object")
        .keys()
        .map(String::as_str)
        .collect();
    let expected: BTreeSet<&str> = [
        "agent_contract",
        "aliases",
        "canonical_tasks",
        "command",
        "commands",
        "description",
        "environment",
        "environment_bool_values",
        "exit_code",
        "exit_codes",
        "global_flags",
        "indexed_source",
        "integrations",
        "machine_schema",
        "notes",
        "ok",
        "output_limits",
        "query_prefixes",
        "root_specification",
        "schema_version",
        "search_formats",
        "search_tuning_flags",
        "sibling_binaries",
        "tool",
        "version",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected, "value={value}");

    // Exit-code deque: exactly the documented 0/1/2 ladder (codes only).
    let codes: Vec<u64> = value["exit_codes"]
        .as_array()
        .expect("exit codes")
        .iter()
        .map(|e| e["code"].as_u64().expect("code"))
        .collect();
    assert_eq!(codes, [0, 1, 2]);

    // Command roster: exact sorted names (identifiers, not message text).
    let mut names: Vec<&str> = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "bench",
            "call-path",
            "capabilities",
            "chain",
            "codemod",
            "codemode-batch",
            "codemode-serve",
            "doctor",
            "eval",
            "index",
            "install",
            "keyword",
            "outline",
            "reindex",
            "robot-docs",
            "search",
            "semantic",
            "status",
            "version",
            "watch",
        ]
    );

    let global: Vec<&str> = value["global_flags"]
        .as_array()
        .expect("global flags")
        .iter()
        .map(|f| f.as_str().expect("flag"))
        .collect();
    assert_eq!(
        global,
        [
            "--auto-index",
            "--durability",
            "--index-path",
            "--json",
            "--lang",
            "--limit",
            "--no-auto-index",
            "--robot-help",
            "--root",
            "--yes",
            "-j",
        ]
    );

    let formats: Vec<&str> = value["search_formats"]
        .as_array()
        .expect("search formats")
        .iter()
        .map(|f| f.as_str().expect("format"))
        .collect();
    assert_eq!(formats, ["native", "agent", "agent-capsule", "compact", "github", "gitlab"]);

    // Cross-command relations: capabilities version tracks the crate version,
    // and its machine schema tracks its own envelope schema.
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["machine_schema"]["schema_version"], value["schema_version"]);
}

#[test]
#[cfg(not(feature = "rerank"))]
fn non_rerank_cells_machine_surface() {
    assert_machine_surface();
}

#[test]
#[cfg(feature = "rerank")]
fn rerank_cells_machine_surface() {
    assert_machine_surface();
}
