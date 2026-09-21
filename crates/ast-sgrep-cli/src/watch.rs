//! `watch` re-exec shim.
//!
//! The file watcher lives in the `asgrep-watch` helper binary, which owns
//! the `notify` link. `notify` pulls CoreFoundation/CoreServices on macOS,
//! ~1.5ms of dyld on EVERY one-shot invocation even though only `watch`
//! uses it — so the main binary never links it and `watch` re-execs the
//! helper with the identical argv (the helper re-parses via the same parser,
//! `crate::parse_watch_launch`, so options cannot drift).

#[cfg(not(unix))]
use anyhow::Context;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) const WATCH_HELPER_BIN: &str = "asgrep-watch";
pub(crate) const WATCH_HELPER_ENV: &str = "ASGREP_WATCH_BIN";

/// Helper path next to the running binary (`.exe` on Windows).
pub(crate) fn sibling_watch_helper(current_exe: &Path) -> PathBuf {
    let file = if cfg!(windows) {
        "asgrep-watch.exe"
    } else {
        WATCH_HELPER_BIN
    };
    current_exe
        .parent()
        .map(|dir| dir.join(file))
        .unwrap_or_else(|| PathBuf::from(file))
}

fn resolve_watch_helper() -> anyhow::Result<PathBuf> {
    // Explicit override wins (mirrors ASGREP_MCP_BIN), but must name a file:
    // a typo here must fail loudly, not silently fall through elsewhere.
    if let Ok(path) = std::env::var(WATCH_HELPER_ENV) {
        let trimmed = path.trim();
        anyhow::ensure!(!trimmed.is_empty(), "{WATCH_HELPER_ENV} is empty");
        let candidate = PathBuf::from(trimmed);
        anyhow::ensure!(
            candidate.is_file(),
            "{WATCH_HELPER_ENV} points at {} which is not a file",
            candidate.display()
        );
        return Ok(candidate);
    }
    if let Ok(exe) = std::env::current_exe() {
        let sibling = sibling_watch_helper(&exe);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    if let Some(found) = crate::install::which(WATCH_HELPER_BIN) {
        return Ok(found);
    }
    anyhow::bail!(
        "asgrep-watch helper not found next to asgrep or on PATH; install it alongside asgrep \
         (cargo install ast-sgrep-watch) or set {WATCH_HELPER_ENV} to its absolute path"
    )
}

/// Hand `watch` to the helper. The argv is forwarded verbatim (it already
/// parsed successfully here). Unix `exec` keeps pid/stdios/signals identical
/// to an in-process watch; other platforms spawn and forward the exit code.
pub(crate) fn run_watch_via_helper() -> anyhow::Result<()> {
    let helper = resolve_watch_helper()?;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only returns on failure; on success this process IS the helper.
        let error = std::process::Command::new(&helper).args(&args).exec();
        Err(anyhow::Error::from(error).context(format!("failed to exec {}", helper.display())))
    }
    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(&helper)
            .args(&args)
            .status()
            .with_context(|| format!("failed to spawn {}", helper.display()))?;
        std::process::exit(status.code().unwrap_or(1))
    }
}
