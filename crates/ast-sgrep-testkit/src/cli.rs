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
