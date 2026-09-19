use crate::fixture::sample_root;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
pub struct CliSession {
    pub _temp: TempDir,
    pub root: PathBuf,
    pub index_path: PathBuf,
    pub bin: PathBuf,
}
impl CliSession {
    pub fn sample(bin: PathBuf) -> Self {
        let temp = TempDir::new().expect("tempdir");
        let session = Self {
            root: sample_root(),
            index_path: temp.path().join("index.db"),
            bin,
            _temp: temp,
        };
        session.index().expect("index sample fixture");
        session
    }
    pub fn search_json(&self, query: &str, extra: &[&str]) -> Value {
        let mut args = vec!["--index-path", self.index_path.to_str().unwrap(), "--json"];
        args.extend(extra);
        if !query.is_empty() {
            args.push(query);
        }
        args.push(self.root.to_str().unwrap());
        serde_json::from_slice(&self.run_success(&args).stdout).expect("search json")
    }
    pub fn run_success(&self, args: &[&str]) -> Output {
        let out = self.run(args).expect("run command");
        assert!(
            out.status.success(),
            "expected success (args={args:?} cwd={:?}), stderr: {}, stdout: {}",
            std::env::current_dir().ok(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
                .chars()
                .take(300)
                .collect::<String>()
        );
        out
    }
    pub fn run_failure(&self, args: &[&str]) -> Output {
        let out = self.run(args).expect("run command");
        assert!(
            !out.status.success(),
            "expected failure, stdout: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        out
    }
    pub fn run(&self, args: &[&str]) -> Result<Output, String> {
        Command::new(&self.bin)
            .args(args)
            .output()
            .map_err(|e| e.to_string())
    }
    fn index(&self) -> Result<Output, String> {
        self.run(&[
            "--index-path",
            self.index_path.to_str().unwrap(),
            "index",
            self.root.to_str().unwrap(),
        ])
    }
}

/// Locate the `asgrep` binary: `CARGO_BIN_EXE_asgrep` when cargo sets it for
/// the test target, else `$CARGO_TARGET_DIR/<profile>/`, else the workspace
/// `target/<profile>/` relative to this crate's manifest.
pub fn asgrep_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_asgrep") {
        return PathBuf::from(p);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let exe = format!("asgrep{}", std::env::consts::EXE_SUFFIX);
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(dir).join(profile).join(&exe);
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join(exe)
}

/// Run the CLI with a hermetic env: `NO_COLOR=1`, `HF_HUB_OFFLINE=1` (an
/// accidental model load fails fast instead of downloading), and every
/// inherited `ASGREP_*` var scrubbed so the parent environment cannot shift
/// flag defaults. Panics only when the child cannot spawn; exit status is the
/// caller's verdict (see [`assert_success`] / [`assert_failure_envelope`]).
pub fn run(bin: &Path, args: &[&str]) -> Output {
    run_env(bin, args, &[])
}

/// [`run`] with extra child env applied after the scrub, so extras win on
/// collision (e.g. a test deliberately planting `ASGREP_INDEX_PATH`).
pub fn run_env(bin: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(bin);
    cmd.args(args).env("NO_COLOR", "1").env("HF_HUB_OFFLINE", "1");
    let scrub: Vec<String> = std::env::vars()
        .map(|(key, _)| key)
        .filter(|key| key.starts_with("ASGREP_"))
        .collect();
    for var in &scrub {
        cmd.env_remove(var);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("run asgrep")
}

/// Parse stdout as one standalone JSON value. Panics with a stdout+stderr dump
/// when stdout is not JSON.
pub fn parse_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// [`run`] + [`parse_stdout`]: the dominant CLI idiom in one call.
pub fn run_json(bin: &Path, args: &[&str]) -> Value {
    let output = run(bin, args);
    parse_stdout(&output)
}

/// [`run_env`] + [`parse_stdout`] keeping the exit code and stderr alongside
/// the parsed body: the machine-envelope idiom for suites that pin rejection
/// codes, stderr silence, and JSON shapes in one assertion block.
pub fn run_json_full(bin: &Path, args: &[&str], envs: &[(&str, &str)]) -> (i32, Value, String) {
    let output = run_env(bin, args, envs);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let value = parse_stdout(&output);
    (output.status.code().expect("exit code"), value, stderr)
}

/// Assert the strictest machine success envelope: exit 0, `schema_version`
/// `1.0.0`, `tool` `asgrep`, `command` echo, `ok: true`, `exit_code: 0`.
/// Returns the parsed body.
pub fn assert_success(output: &Output, command: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["schema_version"], "1.0.0");
    assert_eq!(value["tool"], "asgrep");
    assert_eq!(value["command"], command);
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    value
}

/// INTENT: the human failure face — pinned exit code, a stderr explanation,
/// and no machine success shape on stdout (see [`assert_no_success_shape`]).
/// The machine-envelope counterpart is [`assert_failure_envelope`].
pub fn assert_human_error(output: &Output, exit_code: i32) {
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !output.stderr.is_empty(),
        "human error must explain itself on stderr"
    );
    assert_no_success_shape(output);
}

/// INTENT: the human success face — exit 0 with non-empty stdout. The
/// machine-envelope counterpart is [`assert_success`].
pub fn assert_human_success(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.stdout.is_empty(),
        "human success must print to stdout"
    );
}

/// INTENT: failure paths must never print a success shape — neither a parsed
/// `ok:true` / `exit_code: 0` envelope nor (for non-JSON human output) the
/// `ok:true` substring. Pure assertion over stdout bytes.
pub fn assert_no_success_shape(output: &Output) {
    if output.stdout.is_empty() {
        return;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) {
        assert_ne!(
            value["ok"], true,
            "failure path must not print ok:true: {value}"
        );
        assert_ne!(
            value["exit_code"], 0,
            "failure path must not print exit_code 0: {value}"
        );
    } else {
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !text.contains("\"ok\":true") && !text.contains("\"ok\": true"),
            "human failure must not print a success shape: {text}"
        );
    }
}

/// INTENT: canonical one-function `greet` fixture (`a.rs`) for CLI error
/// beats: the smallest tree an index/search round-trip can pin. The caller
/// keeps the [`TempDir`] alive. Cf. [`CliSession::sample`]
/// (sample-root-fixed) and [`crate::OracleCorpus`] (hand-corpus). Panics on
/// IO failure.
pub fn fixture_root() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("a.rs"),
        "fn greet() -> &'static str {\n    \"hello\"\n}\n",
    )
    .expect("write fixture");
    dir
}

/// INTENT: default-state-path index beat — `index <root>` with NO
/// `--index-path`, asserting exit 0: plants the default `.asgrep/index.db`
/// layout the fault drills corrupt. Delta vs
/// [`crate::run_index`]/[`crate::run_reindex`]: those are
/// `--index-path`-fixed (timeout, explicit cwd), so they cannot pin the
/// default layout; this inherits cwd and uses [`run`]'s hermetic env.
/// `bin` is explicit per crate convention (`env!("CARGO_BIN_EXE_asgrep")`
/// expands only in the test target). Panics on nonzero exit.
pub fn run_index_default(bin: &Path, root: &Path) {
    let root_arg = root.to_string_lossy().into_owned();
    let output = run(bin, &["index", root_arg.as_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "fixture index must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// INTENT: message-redacted envelope projection for determinism relations:
/// `(schema_version, tool, command, ok, exit_code, error.kind, key count,
/// error key count)`. Message text is excluded by design — equal shapes mean
/// the same machine envelope reached the caller. Pure projection.
pub fn envelope_shape(value: &Value) -> (String, String, String, bool, i64, String, usize, usize) {
    (
        value["schema_version"].as_str().unwrap_or("").to_owned(),
        value["tool"].as_str().unwrap_or("").to_owned(),
        value["command"].as_str().unwrap_or("").to_owned(),
        value["ok"].as_bool().unwrap_or(true),
        value["exit_code"].as_i64().unwrap_or(-1),
        value["error"]["kind"].as_str().unwrap_or("").to_owned(),
        value.as_object().map(|o| o.len()).unwrap_or(0),
        value["error"].as_object().map(|o| o.len()).unwrap_or(0),
    )
}

/// INTENT: non-empty-hits pin for recovered searches — the success envelope
/// must carry a `hits` array and it must be non-empty (parity against an
/// empty hit list would prove nothing). Pure assertion.
pub fn assert_fixture_hits(value: &Value) {
    let hits = value["hits"]
        .as_array()
        .expect("search success must carry a hits array");
    assert!(
        !hits.is_empty(),
        "recovered search must hit the fixture: {value}"
    );
}

/// Assert the strictest machine failure envelope: pinned exit code, fixed keys,
/// `ok: false`, matching `exit_code`, `error.kind`, and a string
/// `error.message` (presence only — content is never asserted). Usage errors
/// pass `(1, "usage")`, operational failures `(2, "operational")`. Returns the
/// parsed body.
pub fn assert_failure_envelope(
    output: &Output,
    command: &str,
    exit_code: i32,
    kind: &str,
) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["schema_version"], "1.0.0");
    assert_eq!(value["tool"], "asgrep");
    assert_eq!(value["command"], command);
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], exit_code);
    assert_eq!(value["error"]["kind"], kind);
    assert!(
        value["error"]["message"].is_string(),
        "error.message must be a string: {value}"
    );
    value
}
