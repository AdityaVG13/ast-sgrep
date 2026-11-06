//! Single hermetic spawn core for every CLI invocation the suites drive.
//!
//! # Contract
//!
//! - One preamble: `NO_COLOR=1` + `HF_HUB_OFFLINE=1` pinned, every inherited
//!   `ASGREP_*` var scrubbed, explicit `envs` applied last so they win on
//!   collision ([`hermetic_command`]).
//! - One bounds table: [`SPAWN_RUN_TIMEOUT`] (60s per invocation),
//!   [`SPAWN_RECV_TIMEOUT`] / [`SPAWN_WAIT_TIMEOUT`] (15s MCP read/wait
//!   bounds), [`SPAWN_POLL_INTERVAL`] (25ms wait-loop poll).
//! - One wait core ([`spawn_hermetic`]) with two shapes: [`SpawnMode::Collect`]
//!   (try-wait loop, then collect) and [`SpawnMode::Drain`] (reader threads
//!   pump the pipes while waiting, so outputs larger than the 64KB pipe
//!   buffer cannot deadlock the child — the >300s vs 0.016s huge-line facet).
//!   The three legacy entry points (`CliSession::run`, `cli_recovery::run_timeout`,
//!   `cli_oracle::run_drain_timeout`) are thin timeout/cwd/mode selections
//!   over this core; their panic texts are preserved verbatim.
//! - Panics (never `Result`) on spawn/timeout failure; exit status is the
//!   caller's verdict.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Hard per-invocation bound for CLI spawns: a wedged binary panics the test
/// instead of hanging the suite. Re-exported as `RUN_TIMEOUT` by
/// `cli_recovery` (same item, one source).
pub const SPAWN_RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// Per-read bound for `LiveSession::recv` / `recv_raw`: a regressed MCP
/// server fails the test instead of hanging CI.
pub const SPAWN_RECV_TIMEOUT: Duration = Duration::from_secs(15);

/// Bound for `LiveSession::wait_clean` / `crash_kill` before the child is
/// killed and the test panics.
pub const SPAWN_WAIT_TIMEOUT: Duration = Duration::from_secs(15);

/// Poll interval for every spawn-wait loop (CLI collect/drain waits and the
/// MCP exit waits alike).
pub const SPAWN_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Wait shape for [`spawn_hermetic`]: `Collect` polls for exit then collects
/// (small outputs), `Drain` pumps both pipes on reader threads while polling
/// (outputs past the 64KB pipe buffer would deadlock `Collect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    Collect,
    Drain,
}

/// Build the hermetic child command: pins `NO_COLOR=1` + `HF_HUB_OFFLINE=1`
/// (an accidental model load fails fast instead of downloading), scrubs every
/// inherited `ASGREP_*` var so the parent environment cannot shift flag
/// defaults, then applies `envs` so explicit entries win on collision.
pub fn hermetic_command(bin: &Path, args: &[&str], envs: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("NO_COLOR", "1")
        .env("HF_HUB_OFFLINE", "1");
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
    cmd
}

/// One hermetic spawn: [`hermetic_command`] plus `cwd`, then the
/// [`SpawnMode`] wait. `timeout: None` blocks indefinitely (`Collect` only);
/// `Some` kills the child and panics past the deadline (panic text matches
/// the legacy per-shape message). Panics when the child cannot spawn.
pub fn spawn_hermetic(
    bin: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    cwd: Option<&Path>,
    mode: SpawnMode,
    timeout: Option<Duration>,
) -> Output {
    let mut cmd = hermetic_command(bin, args, envs);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    match mode {
        SpawnMode::Collect => {
            let Some(bound) = timeout else {
                return cmd.output().expect("run asgrep");
            };
            let mut child = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn asgrep");
            let deadline = Instant::now() + bound;
            loop {
                match child.try_wait().expect("try_wait") {
                    Some(_) => return child.wait_with_output().expect("collect output"),
                    None => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let output =
                                child.wait_with_output().expect("collect output after kill");
                            panic!(
                                "asgrep {args:?} exceeded {bound:?}; stderr: {}",
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
                        std::thread::sleep(SPAWN_POLL_INTERVAL);
                    }
                }
            }
        }
        SpawnMode::Drain => {
            let mut child = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn asgrep");
            let stdout_handle = std::thread::spawn({
                let stdout = child.stdout.take().expect("asgrep stdout");
                move || {
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut std::io::BufReader::new(stdout), &mut buf)
                        .expect("drain asgrep stdout");
                    buf
                }
            });
            let stderr_handle = std::thread::spawn({
                let stderr = child.stderr.take().expect("asgrep stderr");
                move || {
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut std::io::BufReader::new(stderr), &mut buf)
                        .expect("drain asgrep stderr");
                    buf
                }
            });
            let Some(bound) = timeout else {
                let status = child.wait().expect("wait for asgrep");
                return Output {
                    status,
                    stdout: stdout_handle.join().expect("stdout drain"),
                    stderr: stderr_handle.join().expect("stderr drain"),
                };
            };
            let deadline = Instant::now() + bound;
            loop {
                match child.try_wait().expect("try_wait") {
                    Some(status) => {
                        return Output {
                            status,
                            stdout: stdout_handle.join().expect("stdout drain"),
                            stderr: stderr_handle.join().expect("stderr drain"),
                        };
                    }
                    None => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let status = child.wait().expect("wait after kill");
                            let stderr = stderr_handle.join().unwrap_or_default();
                            panic!(
                                "asgrep {args:?} exceeded {bound:?} (status {status}); stderr: {}",
                                String::from_utf8_lossy(&stderr)
                            );
                        }
                        std::thread::sleep(SPAWN_POLL_INTERVAL);
                    }
                }
            }
        }
    }
}
