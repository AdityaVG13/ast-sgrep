//! Topology identity pins for `asgrep`: everything that must stay EXACTLY
//! identical across all five feature sets.
//!
//! All five tests are deliberately UNGATED: green runs under `default`,
//! `rerank`, `neural-embed`, `all-features`, and `--no-default-features`
//! prove the cross-set delta set is EMPTY for the default flows, machine
//! surfaces, help surfaces, equivalences, and parse/diagnose surfaces.
//!
//! * `machine_surfaces_identical_across_sets` — version + capabilities exact.
//! * `help_markers_identical_across_sets` — help marker census + help exit 0.
//! * `default_flows_exact_and_topk_inert` — index/status/search exact counts.
//! * `embed_off_inert_and_channel_equivalence` — embed-off 3-way deep-equal +
//!   semantic channel deep-equal.
//! * `parse_diagnose_surfaces_identical` — exit taxonomy, dry-run, outline,
//!   doctor.
//!
//! Download-safety: the only neural-flag invocations are `--no-embed`
//! combinations (embed-off skips validation and never constructs an
//! embedder). All runs inherit testkit's hermetic env (`HF_HUB_OFFLINE=1`,
//! ambient `ASGREP_*` scrubbed).
//!
//! Assertions pin exit codes, JSON shapes, key sets, counts, and bytes only —
//! never message text.

use ast_sgrep_testkit::{asgrep_bin, run_env, run_json_full};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// Why file-local: one-use key-set comparator for the usage-envelope
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
        let mut args: Vec<&str> = vec![
            "--index-path",
            self.index.to_str().unwrap(),
            "--json",
            "search",
        ];
        args.extend_from_slice(extra);
        args.push("alpha_query_target");
        args.push(self.root.to_str().unwrap());
        let bin = asgrep_bin();
        run_json_full(&bin, &args, envs)
    }
}

/// INTENT: version + capabilities machine surfaces are byte/shape-identical in
/// every build (no `cfg(feature)` in the CLI surface; any per-feature drift
/// fails).
/// KILLS: BEHAVIOR-ONLY (drift pins).
/// Absorbs: T1 version x2 + capabilities subset, T3 version exact +
/// capabilities exact, T4 machine census pair (merged ungated), T2 version
/// key-set (DELETE).
#[test]
fn machine_surfaces_identical_across_sets() {
    // version --json: exact key set plus exact scalar values.
    let bin = asgrep_bin();
    let (code, value, stderr) = run_json_full(&bin, &["version", "--json"], &[]);
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
    assert_eq!(value["machine_schema_version"], value["schema_version"]);
    assert!(value["machine_schema_version"].as_str().is_some_and(|v| !v.is_empty()));

    // JSON<->text relation: the 3 text lines restate the JSON scalars exactly.
    let output = run_env(&bin, &["version"], &[]);
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

    // Clap `--version` restates the JSON version exactly (cross-entry check).
    let output = run_env(&bin, &["--version"], &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout.lines().count(), 1);
    assert_eq!(stdout.trim(), format!("asgrep {}", value["version"].as_str().unwrap()));

    // capabilities: exact tuning-flag and environment lists, element-for-element.
    let (code, value, stderr) = run_json_full(&bin, &["capabilities", "--json"], &[]);
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

    // Full envelope census: exact key set, exit-code deque, command roster,
    // global flags, search formats, cross-command relations.
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
    let codes: Vec<u64> = value["exit_codes"]
        .as_array()
        .expect("exit codes")
        .iter()
        .map(|e| e["code"].as_u64().expect("code"))
        .collect();
    assert_eq!(codes, [0, 1, 2]);
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
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["machine_schema"]["schema_version"], value["schema_version"]);
}

/// INTENT: help carries exactly the documented feature markers in every build
/// (exact occurrence counts; substring counts are clap-wrap independent) and
/// help exits 0.
/// KILLS: BEHAVIOR-ONLY (drift pins).
/// Absorbs: T3 search-help counts, T4 help census pair (merged ungated; the
/// cfg split proved nothing the ungated run does not), T1 help-exit-0 atom.
#[test]
fn help_markers_identical_across_sets() {
    let bin = asgrep_bin();
    let output = run_env(&bin, &["search", "--help"], &[]);
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
    let output = run_env(&bin, &["--help"], &[]);
    assert!(output.status.success());

    let output = run_env(&bin, &["search", "--help"], &[]);
    let help = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(!help.is_empty());
    assert_eq!(help.matches("neural-embed").count(), 2, "help={help}");
    assert_eq!(help.matches("--rerank").count(), 3, "help={help}");
    assert_eq!(help.matches("--rerank-top-k").count(), 1, "help={help}");
    assert_eq!(help.matches("(needs neural-embed feature)").count(), 1, "help={help}");

    for sub in ["index", "keyword", "semantic"] {
        let output = run_env(&bin, &[sub, "--help"], &[]);
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

    let output = run_env(&bin, &["--help"], &[]);
    let top = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(top.matches("neural-embed").count(), 2, "top help={top}");
    assert_eq!(top.matches("--rerank").count(), 3, "top help={top}");
    assert_eq!(top.matches("--rerank-top-k").count(), 1, "top help={top}");
    assert_eq!(top.matches("(needs neural-embed feature)").count(), 1, "top help={top}");

    for sub in ["outline", "status", "doctor"] {
        let output = run_env(&bin, &[sub, "--help"], &[]);
        assert!(output.status.success());
        let help = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(!help.is_empty());
        assert_eq!(help.matches("neural-embed").count(), 0, "{sub} help={help}");
        assert_eq!(help.matches("--rerank").count(), 0, "{sub} help={help}");
    }
}

/// INTENT: default index/status/search flows are exactly identical in every
/// build (exact extraction counts, exact status counters, exact hit count +
/// channel mix), and `--rerank-top-k` alone is a proven-inert no-op
/// (full-envelope deep-equal, stronger than a shape pin).
/// KILLS: gate-inversion, knob-leak.
/// Absorbs: T1 default search + top-k noop (looser pins subsumed), T3 exact
/// flows (kept verbatim).
#[test]
fn default_flows_exact_and_topk_inert() {
    let temp = TempDir::new().expect("tempdir");
    let bin = asgrep_bin();
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("root dir");
    fs::write(
        root.join("a.rs"),
        "fn alpha_query_target() {}\nfn beta_helper() { alpha_query_target(); }\n",
    )
    .expect("source");
    let index = temp.path().join("index.db");
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
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert_eq!(value["files_indexed"], 1);
    assert_eq!(value["symbols_extracted"], 2);
    assert_eq!(value["callers_extracted"], 1);
    assert_eq!(value["files_failed"], 0);
    assert_eq!(value["walk_errors"], false);

    let (code, status, stderr) = run_json_full(
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

/// INTENT: with embeddings off the neural gate is fully skipped (flag and env
/// forms deep-equal the plain embed-off envelope), and the `semantic` channel
/// returns deep-equal hits to `search --semantic-only` — in every build.
/// KILLS: gate-ordering, channel-divergence.
/// Absorbs: T1 no-embed success + semantic shape (looser pins subsumed), T3
/// neural-ignore 3-way + semantic channel equivalence (kept verbatim).
#[test]
fn embed_off_inert_and_channel_equivalence() {
    let fx = Fixture::one_file();
    let bin = asgrep_bin();
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

    let (code, via_search, stderr) = fx.search(&["--semantic-only"], &[]);
    assert_eq!(code, 0, "stderr={stderr} value={via_search}");
    assert!(stderr.is_empty());
    assert_eq!(via_search["ok"], true);
    let (code, via_semantic, stderr) = run_json_full(
        &bin,
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

/// INTENT: the parse layer is feature-independent (every flag parses in every
/// build; invalid values are usage errors everywhere) and the diagnose
/// surfaces (dry-run, outline, doctor, exit taxonomy) are exactly identical in
/// every build.
/// KILLS: exit-code-swap, parse-gate-confusion, BEHAVIOR-ONLY.
/// Absorbs: T1 unknown-flag discriminator, T2 exit taxonomy + dry-run +
/// doctor (cell-gated pins widened to ungated identity), T3 dry-run/outline
/// exact + exit taxonomy (kept verbatim).
#[test]
fn parse_diagnose_surfaces_identical() {
    let bin = asgrep_bin();
    let temp = TempDir::new().expect("tempdir");
    let index = temp.path().join("index.db");

    // Unknown flags are usage errors (exit 1): the discriminator proving a
    // non-1 exit elsewhere means the flag exists in clap surface.
    let output = run_env(&bin, &["search", "--definitely-not-a-flag", "q", "."], &[]);
    assert_eq!(output.status.code(), Some(1));

    // `--rerank-top-k` is parsed by clap in every build: an invalid value is a
    // usage error (exit 1), never an unknown-flag or gate error.
    let (code, bad_value, stderr) = run_json_full(
        &bin,
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

    let (code, missing_query, _) = run_json_full(
        &bin,
        &["--index-path", index.to_str().unwrap(), "--json", "search"],
        &[],
    );
    assert_eq!(code, 1, "value={missing_query}");
    assert_eq!(missing_query["error"]["kind"], "usage");
    assert_eq!(object_keys(&missing_query), object_keys(&bad_value));
    assert_eq!(object_keys(&missing_query["error"]), object_keys(&bad_value["error"]));

    let missing = temp.path().join("missing");
    let (code, missing_root, stderr) = run_json_full(
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
    assert_eq!(code, 2, "stderr={stderr} value={missing_root}");
    assert!(stderr.is_empty());
    assert_eq!(missing_root["ok"], false);
    assert_eq!(missing_root["error"]["kind"], "operational");
    assert_eq!(missing_root["exit_code"], 2);

    // Dry-run envelope: exact plan, never writes.
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("root dir");
    fs::write(
        root.join("a.rs"),
        "fn alpha_query_target() {}\nfn beta_helper() { alpha_query_target(); }\n",
    )
    .expect("source");
    let dry_index = temp.path().join("dry.db");
    let (code, dry, stderr) = run_json_full(
        &bin,
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

    // Outline: exact roster plus byte-identical output across two independent
    // indexes of the same root (determinism relation).
    let mut outlines = Vec::new();
    for name in ["first.db", "second.db"] {
        let index = temp.path().join(name);
        let (code, value, _) = run_json_full(
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
        assert_eq!(code, 0, "value={value}");
        let output = run_env(
            &bin,
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

    // Doctor: healthy envelope on a freshly indexed root.
    let (code, value, stderr) = run_json_full(
        &bin,
        &[
            "--index-path",
            temp.path().join("first.db").to_str().unwrap(),
            "--json",
            "doctor",
            root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "doctor");
    assert_eq!(value["healthy"], true);
    assert_eq!(value["issues"].as_array().map(Vec::len), Some(0));
    assert!(value["suggested_commands"].as_array().is_some_and(|c| !c.is_empty()));
}
