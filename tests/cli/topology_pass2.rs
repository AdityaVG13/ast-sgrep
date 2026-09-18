//! T2 feature-combination matrix for `asgrep`.
//!
//! Where T1 pins the DEFAULT-build contract, T2 pins the 2x2
//! (`neural-embed` x `rerank`) combination matrix. Each cell is cfg-gated to
//! exactly one feature set, and every assertion discriminates its cell: it
//! FAILS if run under the wrong feature set (e.g. a default-cell test
//! asserting `Err`/exit-2 fails where the feature cell asserts `Ok`/exit-0).
//!
//! Matrix (`--no-default-features` == default because `default = []`):
//!
//! | neural-embed | rerank | cell         | runs offline? |
//! |--------------|--------|--------------|----------------|
//! | off          | off    | default      | yes            |
//! | off          | off    | no-default*  | yes            |
//! | on           | off    | neural-only  | yes (see below)|
//! | off          | on     | rerank-only  | yes            |
//! | on           | on     | all-features | yes (see below)|
//!
//! `*` The equivalence cell compiles identically under `default` and
//! `--no-default-features` and pins the shared envelope so the two builds
//! cannot drift.
//!
//! Download-safety: instantiating the neural embedder pulls a model through
//! fastembed/hf-hub, so NO test in this file runs `search`/`index`
//! `--neural-embed` against a real root under `neural-embed`/`all-features`.
//! Those cells pin the CLI-visible surface instead: lib-level
//! `validate_search_feature_flags` (no model load), `--neural-embed` flag
//! parsing via exit-code discriminators (unknown flag = exit 1, parsed flag
//! on a missing root = exit 2), and `index --dry-run` (walks directories
//! only; dispatch returns before any `Indexer`/embedder exists). Feature
//! cells additionally set `HF_HUB_OFFLINE=1` so an accidental model load
//! fails fast without network rather than downloading.
//!
//! Assertions pin exit codes, JSON shapes, and counts only -- never message text.

use ast_sgrep_core::search::{validate_search_feature_flags, SearchOptions};
use serde_json::Value;
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
use std::collections::BTreeSet;
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

    // Used by the feature cells; the default cell indexes/drives subcommands directly.
    #[allow(dead_code)]
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

/// Hermetic `SearchOptions` for validation probes: the struct's `Default`
/// reads ambient `ASGREP_*` env, so the gating fields are set explicitly.
fn validation_options(neural: bool, rerank: bool) -> SearchOptions {
    let mut options = SearchOptions::default();
    options.use_embed = true;
    options.use_neural_embed = neural;
    options.use_rerank = rerank;
    options
}

// ---------------------------------------------------------------------------
// Default cell: neither optional feature. Fails under any feature build.
// ---------------------------------------------------------------------------

/// Default build exit taxonomy on inputs T1/machine_contracts do not pin:
/// a missing search root is operational (exit 2), a missing QUERY positional
/// is a usage error (exit 1). Both envelopes carry `ok:false`.
#[test]
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
fn default_exit_taxonomy_missing_root_and_missing_query() {
    let temp = TempDir::new().expect("tempdir");
    let missing = temp.path().join("missing");
    let index = temp.path().join("index.db");
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
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["kind"], "operational");

    let (code, value, stderr) = run_json(
        &["--index-path", index.to_str().unwrap(), "--json", "search"],
        &[],
    );
    assert_eq!(code, 1, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["kind"], "usage");
}

/// `index --dry-run` reports a plan and never writes: exit 0, `dry_run` set,
/// `mutates_index` false, at least one candidate file, and no index file left
/// behind. (Dry-run semantics are intentionally binary-level here; unit
/// coverage of the walker lives elsewhere.)
#[test]
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
fn default_index_dry_run_reports_plan_without_writing() {
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
            "--dry-run",
            root.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["mutates_index"], false);
    assert_eq!(value["walk_errors"], false);
    assert!(value["files_would_index"].as_u64().unwrap_or(0) >= 1);
    assert!(!index.exists(), "dry-run must not create the index file");
}

/// `doctor --json` on a freshly indexed root is healthy: exit 0, `ok:true`,
/// `healthy:true`, an empty issues array, and at least one suggested command.
/// (The unhealthy/missing-root face is pinned in machine_contracts.)
#[test]
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
fn default_doctor_healthy_envelope() {
    let fx = Fixture::indexed();
    let (code, value, stderr) = run_json(
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "doctor",
            fx.root.to_str().unwrap(),
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

/// Lib-level negative guard for the default cell: requesting either optional
/// path through `validate_search_feature_flags` errors, while the plain
/// local-embedding configuration validates. Binary-level fail-closed is T1's;
/// this pins the shared gate both binaries and lib callers use.
#[test]
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
fn default_lib_validation_rejects_optional_paths() {
    assert!(validate_search_feature_flags(&validation_options(false, false)).is_ok());
    assert!(validate_search_feature_flags(&validation_options(true, false)).is_err());
    assert!(validate_search_feature_flags(&validation_options(false, true)).is_err());
    assert!(validate_search_feature_flags(&validation_options(true, true)).is_err());
}

// ---------------------------------------------------------------------------
// --no-default-features equivalence cell.
// `default = []`, so this compiles identically under `default` and
// `--no-default-features`; it pins the shared machine envelope so the two
// builds cannot drift apart.
// ---------------------------------------------------------------------------

/// The `version --json` envelope key set is exactly the three payload keys
/// plus the five machine-envelope keys. T1 pins individual fields; this pins
/// the full set, so a default build and a `--no-default-features` build emit
/// byte-comparable shapes (values aside).
#[test]
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
fn nodefault_version_envelope_key_set_matches_default() {
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
}

// ---------------------------------------------------------------------------
// neural-embed-only cell. Compile-asserts + offline surface only: nothing here
// may instantiate the neural embedder (model download). Never run
// `search`/`index --neural-embed` against a real root under this feature.
// ---------------------------------------------------------------------------

/// Lib-level matrix pin: the neural path validates, the rerank path still
/// fails (negative guard -- this cell has no `rerank`), and requesting both
/// fails because of the missing half.
#[test]
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
fn neural_only_lib_validation_accepts_neural_rejects_rerank() {
    assert!(validate_search_feature_flags(&validation_options(false, false)).is_ok());
    assert!(validate_search_feature_flags(&validation_options(true, false)).is_ok());
    assert!(validate_search_feature_flags(&validation_options(false, true)).is_err());
    assert!(validate_search_feature_flags(&validation_options(true, true)).is_err());
}

/// CLI-visible neural surface without a model load:
/// * `index --neural-embed --dry-run` on a real root succeeds (dry-run walks
///   directories only; dispatch returns before any embedder exists), proving
///   the flag is parsed and accepted on the index path.
/// * the same on a missing root is operational (exit 2), not a clap usage
///   error (exit 1) -- a second parse discriminator.
/// * `search --rerank` still fails closed (exit 2, operational): rerank is
///   off in this cell.
/// * plain default search stays offline on local embeddings (exit 0, hits).
#[test]
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
fn neural_only_flag_parsed_rerank_closed_default_search_offline() {
    const OFFLINE: &[(&str, &str)] = &[("HF_HUB_OFFLINE", "1"), ("ASGREP_NEURAL_EMBED", "0")];
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
            "--dry-run",
            root.to_str().unwrap(),
        ],
        OFFLINE,
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    assert_eq!(value["dry_run"], true);

    let missing = temp.path().join("missing");
    let (code, value, _) = run_json(
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "index",
            "--neural-embed",
            "--dry-run",
            missing.to_str().unwrap(),
        ],
        OFFLINE,
    );
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");

    let fx = Fixture::indexed();
    let (code, value, stderr) = fx.search(&["--rerank"], OFFLINE);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert_eq!(value["error"]["kind"], "operational");

    let (code, value, stderr) = fx.search(&[], OFFLINE);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    assert!(value["hits"].as_array().is_some_and(|h| !h.is_empty()));
}

// ---------------------------------------------------------------------------
// rerank-only cell. A missing local reranker degrades to unranked hits
// (exit 0), so the accept-side runs offline; note the degrade path writes a
// stderr diagnostic, so accept-side assertions must NOT require empty stderr.
// ---------------------------------------------------------------------------

/// Accept-side entry points T1 does not cover: `keyword --rerank`, bare
/// `--rerank`, and `ASGREP_RERANK=1` all succeed offline with `ok:true` and a
/// hits array bounded by the default limit.
#[test]
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
fn rerank_only_accept_entry_points_offline() {
    const OFFLINE: &[(&str, &str)] = &[("HF_HUB_OFFLINE", "1")];
    let fx = Fixture::indexed();

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
        OFFLINE,
    );
    assert_eq!(code, 0, "value={value}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits array");
    assert!(hits.len() <= 16, "hits exceed default limit: {}", hits.len());

    let output = run(
        &[
            "--index-path",
            fx.index.to_str().unwrap(),
            "--json",
            "--rerank",
            "alpha_query_target",
            fx.root.to_str().unwrap(),
        ],
        OFFLINE,
    );
    assert_eq!(output.status.code(), Some(0));
    let bare: Value = serde_json::from_slice(&output.stdout).expect("bare JSON");
    assert_eq!(bare["ok"], true);
    assert!(bare["hits"].is_array());

    let (code, value, _) = fx.search(&[], &[("HF_HUB_OFFLINE", "1"), ("ASGREP_RERANK", "1")]);
    assert_eq!(code, 0, "value={value}");
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());
}

/// Negative guard for the rerank-only cell: the neural path is still off, so
/// `search --neural-embed` fails closed (exit 2, operational) at the binary
/// level and at the lib gate, while the rerank flag validates.
#[test]
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
fn rerank_only_neural_still_fails_closed() {
    const OFFLINE: &[(&str, &str)] = &[("HF_HUB_OFFLINE", "1")];
    let fx = Fixture::indexed();
    let (code, value, stderr) = fx.search(&["--neural-embed"], OFFLINE);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["kind"], "operational");

    assert!(validate_search_feature_flags(&validation_options(false, true)).is_ok());
    assert!(validate_search_feature_flags(&validation_options(true, false)).is_err());
}

// ---------------------------------------------------------------------------
// all-features cell. Same download-safety rule as neural-only: validation +
// surface probes only, never `--neural-embed` search/index on a real root.
// ---------------------------------------------------------------------------

/// Lib-level pin: both optional paths validate, alone and together.
#[test]
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
fn all_features_lib_validation_accepts_both() {
    assert!(validate_search_feature_flags(&validation_options(false, false)).is_ok());
    assert!(validate_search_feature_flags(&validation_options(true, false)).is_ok());
    assert!(validate_search_feature_flags(&validation_options(false, true)).is_ok());
    assert!(validate_search_feature_flags(&validation_options(true, true)).is_ok());
}

/// Offline surface under both features: plain search runs on local embeddings
/// (exit 0, non-empty hits, silent stderr), `keyword --rerank` is accepted
/// (exit 0; degrade path may write stderr, so no silence assertion), and
/// `--neural-embed` parses (dry-run on a real root succeeds; on a missing
/// root it is operational exit 2, never clap exit 1).
#[test]
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
fn all_features_offline_surface_without_model_load() {
    const OFFLINE: &[(&str, &str)] = &[("HF_HUB_OFFLINE", "1"), ("ASGREP_NEURAL_EMBED", "0")];
    let fx = Fixture::indexed();

    let (code, value, stderr) = fx.search(&[], OFFLINE);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert_eq!(value["ok"], true);
    assert!(value["hits"].as_array().is_some_and(|h| !h.is_empty()));

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
        OFFLINE,
    );
    assert_eq!(code, 0, "value={value}");
    assert_eq!(value["ok"], true);
    assert!(value["hits"].is_array());

    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("root dir");
    fs::write(root.join("a.rs"), "fn alpha_query_target() {}\n").expect("source");
    let index = temp.path().join("index.db");
    let (code, value, _) = run_json(
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "index",
            "--neural-embed",
            "--dry-run",
            root.to_str().unwrap(),
        ],
        OFFLINE,
    );
    assert_eq!(code, 0, "value={value}");
    assert_eq!(value["dry_run"], true);

    let missing = temp.path().join("missing");
    let (code, value, _) = run_json(
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "index",
            "--neural-embed",
            "--dry-run",
            missing.to_str().unwrap(),
        ],
        OFFLINE,
    );
    assert_eq!(code, 2, "value={value}");
    assert_eq!(value["error"]["kind"], "operational");
}
