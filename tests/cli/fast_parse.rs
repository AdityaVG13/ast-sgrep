//! Differential + pin tests for the bare-search fast-parse pre-parser.
//!
//! The fast path (`cli_args::try_fast_parse`) must produce a byte-identical
//! `Cli` to clap on every shape it accepts. These tests prove that through
//! the real binary: each argv runs twice — fast path (default) and clap
//! control (`ASGREP_NO_FAST_PARSE=1`) — asserting identical stdout, stderr,
//! and exit code. `ASGREP_FAST_PARSE_DEBUG=1` exposes which path parsed, so
//! every case also pins fired-vs-fallback: a test that only asserts equality
//! would pass vacuously if the fast path never fired.
//!
//! Fallback cases need no behavior proof beyond the marker (fallback runs
//! pristine `Cli::try_parse_from` on identical input), but the differential
//! runs anyway as belt-and-braces against hook bugs.

use ast_sgrep_testkit::CliSession;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

const MARKER_FIRED: &str = "asgrep: fast-parse fired";
const MARKER_FALLBACK: &str = "asgrep: fast-parse fallback";
/// Per-probe wall budget. Probes are chosen to exit fast; the budget exists
/// so a future long-lived subcommand fails visibly instead of hanging the
/// suite (the parse marker is already captured on kill).
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Spawn asgrep with hermetic env (testkit scrub semantics: all `ASGREP_*`
/// removed, then extras applied), `cwd` as working dir and `HOME`, and a
/// kill-after-timeout guard. Returns whatever the child produced.
fn run_probe(args: &[&str], cwd: &Path, extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(asgrep_bin());
    cmd.args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .env("HF_HUB_OFFLINE", "1")
        .env("HOME", cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars() {
        if key.starts_with("ASGREP_") {
            cmd.env_remove(&key);
        }
    }
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn asgrep");
    let start = Instant::now();
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        if start.elapsed() > PROBE_TIMEOUT {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().expect("wait")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn strip_marker(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| !line.contains("asgrep: fast-parse"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `args` on the fast path and the clap control, assert identical
/// stdout/stderr/exit, and return whether the fast path fired. `extra_env`
/// applies to both runs (differential); the control additionally forces
/// clap via the opt-out.
fn assert_differential(args: &[&str], cwd: &Path, extra_env: &[(&str, &str)]) -> bool {
    let mut fast_env = vec![("ASGREP_FAST_PARSE_DEBUG", "1")];
    fast_env.extend_from_slice(extra_env);
    let mut control_env = vec![
        ("ASGREP_FAST_PARSE_DEBUG", "1"),
        ("ASGREP_NO_FAST_PARSE", "1"),
    ];
    control_env.extend_from_slice(extra_env);
    let fast = run_probe(args, cwd, &fast_env);
    let control = run_probe(args, cwd, &control_env);
    let fast_stderr = stderr_of(&fast);
    let control_stderr = stderr_of(&control);
    assert!(
        control_stderr.contains(MARKER_FALLBACK),
        "control must parse via clap for {args:?}:\n{control_stderr}"
    );
    assert_eq!(
        fast.status.code(),
        control.status.code(),
        "exit differs for {args:?}\nfast stderr: {fast_stderr}\ncontrol stderr: {control_stderr}"
    );
    assert_eq!(
        fast.stdout, control.stdout,
        "stdout differs for {args:?}\nfast stderr: {fast_stderr}"
    );
    assert_eq!(
        strip_marker(&fast_stderr),
        strip_marker(&control_stderr),
        "stderr differs for {args:?}"
    );
    assert_ne!(
        fast_stderr.contains(MARKER_FIRED),
        fast_stderr.contains(MARKER_FALLBACK),
        "exactly one marker expected for {args:?}:\n{fast_stderr}"
    );
    fast_stderr.contains(MARKER_FIRED)
}

fn assert_fires(args: &[&str], cwd: &Path, extra_env: &[(&str, &str)]) {
    assert!(
        assert_differential(args, cwd, extra_env),
        "fast path must fire for {args:?}"
    );
}

fn assert_falls_back(args: &[&str], cwd: &Path, extra_env: &[(&str, &str)]) {
    assert!(
        !assert_differential(args, cwd, extra_env),
        "fast path must fall back for {args:?}"
    );
}

/// Full stdout/stderr/exit identity between fast and clap parses across the
/// covered bare-search surface (each asserts fired) and the bail surface
/// (each asserts fallback). The S1–S4 golden shapes run against a real
/// index; the rest dispatch-error identically in an empty dir (no index).
#[test]
fn fast_parse_differential_matrix() {
    let session = CliSession::sample(asgrep_bin());
    let root = session.root.to_str().unwrap().to_owned();
    let index = session.index_path.to_str().unwrap().to_owned();
    let empty = TempDir::new().expect("tempdir");
    let no_index = empty.path().to_str().unwrap().to_owned();

    // Golden shapes with a real index: full JSON/human stdout identity.
    for query in [
        "literal:SearchHit",
        "compact output path interning",
        "callers:process_request",
        "pattern:Searcher",
    ] {
        let args = vec![
            "--index-path".to_string(),
            index.clone(),
            "--json".to_string(),
            query.to_string(),
            root.clone(),
        ];
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_fires(&argv, empty.path(), &[]);
    }

    // Covered shapes (must fire). Dispatch errors identically without an
    // index; parsing is what is under test.
    let fired: Vec<Vec<String>> = vec![
        vec!["q".into()],
        vec!["q".into(), no_index.clone()],
        vec!["--json".into()],
        vec!["--json".into(), "q".into()],
        vec!["-j".into(), "q".into(), no_index.clone()],
        vec!["--limit".into(), "5".into(), "q".into(), no_index.clone()],
        vec!["--limit=5".into(), "q".into()],
        vec!["--limit".into(), "0".into(), "q".into()],
        vec!["--limit".into(), "1000".into(), "q".into()],
        vec!["--root".into(), no_index.clone(), "q".into()],
        vec![format!("--root={no_index}"), "q".into()],
        vec!["--lang".into(), "rs".into(), "q".into()],
        vec!["--lang=rs".into(), "q".into()],
        vec!["--index-path".into(), "/nonexistent.db".into(), "q".into()],
        vec!["--durability".into(), "strict".into(), "q".into()],
        vec!["--durability=fast-unsafe".into(), "q".into()],
        vec!["--no-auto-index".into(), "q".into()],
        vec!["--yes".into(), "q".into()],
        vec!["--force".into(), "q".into()],
        vec!["--robot-help".into()],
        vec!["--no-embed".into(), "q".into()],
        vec!["--neural-embed".into(), "q".into()],
        vec!["--semantic-only".into(), "q".into()],
        vec!["--tantivy".into(), "q".into()],
        vec!["--rerank".into(), "q".into()],
        vec!["--files-with-matches".into(), "q".into()],
        vec!["--ann-threshold".into(), "7".into(), "q".into()],
        vec!["--ann-probes=0".into(), "q".into()],
        vec!["--rerank-top-k".into(), "3".into(), "q".into()],
        vec!["--format".into(), "compact".into(), "q".into()],
        vec!["--format=agent".into(), "q".into()],
        vec!["--excerpt-lines".into(), "2".into(), "q".into()],
        vec!["--snippet-tokens".into(), "8".into(), "q".into()],
        vec!["--response-snippet-tokens".into(), "10".into(), "q".into()],
        vec!["--file-filter".into(), "*.rs".into(), "q".into()],
        vec!["--budget-tokens".into(), "100".into(), "q".into()],
        vec!["--preview".into(), "full".into(), "q".into()],
        vec!["--preview=none".into(), "q".into()],
        // Flags after positionals, and the full kitchen sink.
        vec![
            "q".into(),
            no_index.clone(),
            "--json".into(),
            "--limit".into(),
            "5".into(),
        ],
        vec![
            "--index-path".into(),
            "/nonexistent.db".into(),
            "--json".into(),
            "--limit".into(),
            "9".into(),
            "--lang".into(),
            "py".into(),
            "--no-embed".into(),
            "--format".into(),
            "compact".into(),
            "--file-filter".into(),
            "*.py".into(),
            "--files-with-matches".into(),
            "some query".into(),
            no_index.clone(),
        ],
        // `--` separator: flag-looking tokens become positionals.
        vec!["--".into(), "--json".into()],
        vec!["--".into(), "q".into(), no_index.clone()],
        // Lone `-` is a positional value by clap convention.
        vec!["-".into()],
        // The typo rewriter recovers `---json` as `--json` pre-parse, so
        // both parsers see the fixed argv and the fast path fires.
        vec!["---json".into(), "q".into()],
        // Empty argv and empty query: clap accepts, dispatch teaches.
        vec![],
        vec!["".into()],
        // Unicode and odd-but-valid queries.
        vec!["héllo wörld".into()],
        vec!["--".into(), "-x".into()],
        vec!["literal:SearchHit".into(), no_index.clone(), "--".into()],
    ];
    for case in &fired {
        let argv: Vec<&str> = case.iter().map(String::as_str).collect();
        assert_fires(&argv, empty.path(), &[]);
    }

    // Bail shapes (must fall back; differential still asserted).
    let fallback: Vec<Vec<String>> = vec![
        vec!["--version".into()],
        vec!["-V".into()],
        vec!["--help".into()],
        vec!["-h".into()],
        vec!["help".into()],
        vec!["help".into(), "index".into()],
        vec!["search".into(), "q".into()],
        vec!["find".into(), "q".into()],
        vec!["query".into(), "q".into(), no_index.clone()],
        vec!["index".into(), "--dry-run".into(), no_index.clone()],
        // Post-`--` and second positionals still bail on subcommand tokens
        // (deliberate over-bail: rare slow path, zero misroute risk).
        vec!["--".into(), "search".into()],
        vec!["q".into(), "search".into()],
        // Scalar repeats are clap errors.
        vec!["--json".into(), "--json".into(), "q".into()],
        vec!["-j".into(), "--json".into(), "q".into()],
        vec!["--yes".into(), "--force".into(), "q".into()],
        vec![
            "--limit".into(),
            "1".into(),
            "--limit".into(),
            "2".into(),
            "q".into(),
        ],
        vec![
            "--format".into(),
            "compact".into(),
            "--format=agent".into(),
            "q".into(),
        ],
        // Unknown flags and combined shorts stay clap-owned.
        vec!["--bogus".into(), "q".into()],
        vec!["-x".into(), "q".into()],
        vec!["-jj".into(), "q".into()],
        vec!["--json=true".into(), "q".into()],
        vec!["--no-embed=false".into(), "q".into()],
        // Positional overflow.
        vec!["a".into(), "b".into(), "c".into()],
        vec!["--".into(), "a".into(), "b".into(), "c".into()],
        // Missing or flag-like values.
        vec!["--limit".into()],
        vec!["--limit".into(), "--json".into(), "q".into()],
        vec!["--limit".into(), "-5".into(), "q".into()],
        vec!["--root".into()],
        // Invalid values (bail lets clap render the identical error).
        vec!["--limit".into(), "abc".into(), "q".into()],
        vec!["--limit=".into(), "q".into()],
        vec![
            "--limit".into(),
            "99999999999999999999999".into(),
            "q".into(),
        ],
        vec!["--limit".into(), "1001".into(), "q".into()],
        vec!["--format".into(), "bogus".into(), "q".into()],
        vec!["--preview".into(), "huge".into(), "q".into()],
        vec!["--durability".into(), "nope".into(), "q".into()],
        vec!["--ann-threshold".into(), "1.5".into(), "q".into()],
        vec!["--rerank-top-k".into(), "-1".into(), "q".into()],
        vec!["--budget-tokens".into(), "xyz".into(), "q".into()],
        // Empty plain-String/PathBuf values bail (no guess about clap).
        vec!["--lang".into(), "".into(), "q".into()],
        vec!["--file-filter=".into(), "q".into()],
        vec!["--root=".into(), "q".into()],
    ];
    for case in &fallback {
        let argv: Vec<&str> = case.iter().map(String::as_str).collect();
        assert_falls_back(&argv, empty.path(), &[]);
    }

    // `--auto-index` mutates, so runs are marker-only (no differential):
    // parsing fires, dispatch indexes the empty dir (contained, fast).
    for args in [
        vec![
            "--auto-index".to_string(),
            "q".to_string(),
            no_index.clone(),
        ],
        vec!["--auto-index".to_string(), "q".to_string()],
    ] {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = run_probe(&argv, empty.path(), &[("ASGREP_FAST_PARSE_DEBUG", "1")]);
        assert!(
            stderr_of(&out).contains(MARKER_FIRED),
            "auto-index shape must fire for {argv:?}:\n{}",
            stderr_of(&out)
        );
    }
}

/// Env bail is selective: clap-consumed `ASGREP_*` vars force fallback,
/// while parse-irrelevant ones keep the fast path. Both runs carry the
/// same env so the differential holds.
#[test]
fn fast_parse_env_bail_is_selective() {
    let empty = TempDir::new().expect("tempdir");
    let root = empty.path().to_str().unwrap().to_owned();
    let args = vec!["--json".to_string(), "q".to_string(), root];
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    for var in [
        "ASGREP_LIMIT",
        "ASGREP_INDEX_PATH",
        "ASGREP_DURABILITY",
        "ASGREP_NO_AUTO_INDEX",
        "ASGREP_AUTO_INDEX",
        "ASGREP_NO_EMBED",
        "ASGREP_NEURAL_EMBED",
        "ASGREP_SEMANTIC_ONLY",
        "ASGREP_TANTIVY",
        "ASGREP_ANN_THRESHOLD",
        "ASGREP_ANN_PROBES",
        "ASGREP_RERANK",
        "ASGREP_RERANK_TOP_K",
    ] {
        assert_falls_back(&argv, empty.path(), &[(var, "1")]);
    }
    // Parse-irrelevant vars must NOT bail.
    for var in ["ASGREP_SQLITE_DEFAULTS", "ASGREP_BENCH_STRICT"] {
        assert_fires(&argv, empty.path(), &[(var, "1")]);
    }
    // The kill-switch forces fallback even on the fastest shape.
    assert_falls_back(&argv, empty.path(), &[("ASGREP_NO_FAST_PARSE", "1")]);
}

/// Non-UTF8 argv: bail pre-separator (may be flag-like bytes), accept as
/// the ROOT positional post-`--` (clap takes a `PathBuf` there).
#[cfg(unix)]
#[test]
fn fast_parse_non_utf8_argv() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let empty = TempDir::new().expect("tempdir");
    let run_raw = |args: &[OsString], extra_env: &[(&str, &str)]| -> Output {
        let mut cmd = Command::new(asgrep_bin());
        cmd.args(args)
            .current_dir(empty.path())
            .env("NO_COLOR", "1")
            .env("HF_HUB_OFFLINE", "1")
            .env("HOME", empty.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, _) in std::env::vars() {
            if key.starts_with("ASGREP_") {
                cmd.env_remove(&key);
            }
        }
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        cmd.output().expect("run asgrep")
    };
    let non_utf8 = OsString::from_vec(vec![0xff, 0xfe]);

    // Pre-separator non-UTF8 second positional: bail, identical behavior.
    let args = vec![OsString::from("q"), non_utf8.clone()];
    let fast = run_raw(&args, &[("ASGREP_FAST_PARSE_DEBUG", "1")]);
    let control = run_raw(
        &args,
        &[
            ("ASGREP_FAST_PARSE_DEBUG", "1"),
            ("ASGREP_NO_FAST_PARSE", "1"),
        ],
    );
    assert!(stderr_of(&fast).contains(MARKER_FALLBACK));
    assert_eq!(fast.status.code(), control.status.code());
    assert_eq!(fast.stdout, control.stdout);
    assert_eq!(
        strip_marker(&stderr_of(&fast)),
        strip_marker(&stderr_of(&control))
    );

    // Post-`--` non-UTF8 ROOT: fires with identical behavior.
    let args = vec![OsString::from("--"), OsString::from("q"), non_utf8.clone()];
    let fast = run_raw(&args, &[("ASGREP_FAST_PARSE_DEBUG", "1")]);
    let control = run_raw(
        &args,
        &[
            ("ASGREP_FAST_PARSE_DEBUG", "1"),
            ("ASGREP_NO_FAST_PARSE", "1"),
        ],
    );
    assert!(
        stderr_of(&fast).contains(MARKER_FIRED),
        "post--- non-UTF8 ROOT must fire:\n{}",
        stderr_of(&fast)
    );
    assert_eq!(fast.status.code(), control.status.code());
    assert_eq!(fast.stdout, control.stdout);
    assert_eq!(
        strip_marker(&stderr_of(&fast)),
        strip_marker(&stderr_of(&control))
    );

    // Non-UTF8 `--root` value: fires (PathBuf), identical behavior.
    let args = vec![
        OsString::from("--root"),
        non_utf8.clone(),
        OsString::from("q"),
    ];
    let fast = run_raw(&args, &[("ASGREP_FAST_PARSE_DEBUG", "1")]);
    assert!(
        stderr_of(&fast).contains(MARKER_FIRED),
        "non-UTF8 --root must fire:\n{}",
        stderr_of(&fast)
    );
    let control = run_raw(
        &args,
        &[
            ("ASGREP_FAST_PARSE_DEBUG", "1"),
            ("ASGREP_NO_FAST_PARSE", "1"),
        ],
    );
    assert_eq!(fast.status.code(), control.status.code());
    assert_eq!(fast.stdout, control.stdout);
}

/// The subcommand bail set must equal clap's routable tokens: names and
/// aliases derived at runtime from `capabilities --json` (which is built
/// from the live clap `Command`), plus the implicit `help` subcommand
/// (verified behaviorally: `asgrep help` exits 0). For each token the
/// universal probe `<token> /nonexistent-fast-parse-probe-root` would be
/// fast-acceptable if the token were missing from the set (one or two
/// positionals, no other flags), so a fired marker fails the pin. Every
/// probe errors before any write (missing required args, root checks, or
/// read-only dispatch in a fresh tempdir); the timeout kills any future
/// long-lived subcommand after the marker is captured.
#[test]
fn fast_parse_bails_on_every_subcommand() {
    let probe_root = "/nonexistent-fast-parse-probe-root";
    let caps_out = run_probe(&["capabilities", "--json"], Path::new("."), &[]);
    assert!(caps_out.status.success());
    let caps: serde_json::Value =
        serde_json::from_slice(&caps_out.stdout).expect("capabilities JSON");
    let mut tokens: Vec<String> = Vec::new();
    for cmd in caps["commands"].as_array().expect("commands array") {
        tokens.push(cmd["name"].as_str().expect("name").to_owned());
        for alias in cmd
            .get("aliases")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            tokens.push(alias.as_str().expect("alias").to_owned());
        }
    }
    assert!(
        tokens.len() >= 20,
        "capabilities must enumerate the subcommands, got {tokens:?}"
    );
    // Clap's implicit `help` subcommand is in neither the derive enum nor
    // capabilities; verify it exists before pinning it.
    let help = run_probe(&["help"], Path::new("."), &[]);
    assert!(
        help.status.success() && String::from_utf8_lossy(&help.stdout).contains("Usage"),
        "implicit `help` subcommand must exist"
    );
    tokens.push("help".to_owned());

    for token in &tokens {
        let dir = TempDir::new().expect("tempdir");
        let out = run_probe(
            &[token.as_str(), probe_root],
            dir.path(),
            &[("ASGREP_FAST_PARSE_DEBUG", "1")],
        );
        assert!(
            stderr_of(&out).contains(MARKER_FALLBACK),
            "token `{token}` must fall back (exit {:?}):\nstdout: {}\nstderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            stderr_of(&out)
        );
    }
}
