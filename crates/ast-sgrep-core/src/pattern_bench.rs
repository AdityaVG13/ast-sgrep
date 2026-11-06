//! Bench-only external `ast-grep` lane, split verbatim from `pattern.rs`
//! (search never delegates; this gate serves the bench comparison only).

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Timed `try_wait` loop shared by the optional ast-grep version probe and bench runner.
/// Returns `Some(())` when the child exits (and succeeds if `require_success`), else kills and returns `None`.
fn wait_child_deadline(child: &mut Child, deadline: Instant, require_success: bool) -> Option<()> {
    let kill = |child: &mut Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if require_success && !status.success() {
                    kill(child);
                    return None;
                }
                return Some(());
            }
            Ok(None) if Instant::now() >= deadline => {
                kill(child);
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                kill(child);
                return None;
            }
        }
    }
}
/// Optional external `ast-grep` for **bench comparison only**.
/// Disabled by default: never searches PATH or executes untrusted binaries.
/// Requires both `ASGREP_ALLOW_AST_GREP=1` and an absolute `ASGREP_AST_GREP`
/// file path.
///
/// The former `run_external_ast_grep` delegation result path was removed —
/// zero callers, and delegating search hits to the reference binary would
/// make differential parity circular and import unpinned version behavior
/// into the trust boundary. Search never delegates; this gate serves the
/// bench comparison lane only (`bench_ast_grep`).
pub(crate) fn find_ast_grep_binary(config: &crate::config::CoreConfig) -> Option<String> {
    if !config.allow_ast_grep {
        return None;
    }
    let path = config.ast_grep_path.as_deref()?;
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    // Timed version probe — reject hung/non-ast-grep binaries.
    let mut child = Command::new(path)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    wait_child_deadline(
        &mut child,
        Instant::now() + Duration::from_millis(1_500),
        true,
    )?;
    let output = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .contains("ast-grep")
        .then(|| path.to_string_lossy().into_owned())
}
pub fn bench_ast_grep(pattern: &str, root: &Path, iterations: u32) -> Option<f64> {
    let config = crate::config::CoreConfig::from_env();
    let ast_grep = find_ast_grep_binary(&config)?;
    let root = crate::paths::canonicalize_path(root, true)
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let mut total = 0.0f64;
    for _ in 0..iterations {
        let start = Instant::now();
        let mut child = Command::new(&ast_grep)
            .args(["run", "--pattern", pattern, &root])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        wait_child_deadline(&mut child, Instant::now() + Duration::from_secs(30), false)?;
        total += start.elapsed().as_secs_f64() * 1000.0;
    }
    Some(total / f64::from(iterations))
}
