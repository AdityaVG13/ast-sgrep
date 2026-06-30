use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::Result;

const PATTERN_TIMEOUT_SECS: u64 = 30;

pub fn find_ast_grep_binary() -> Option<String> {
    for name in ["ast-grep", "sg"] {
        if Command::new(name)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(name.to_string());
        }
    }
    None
}

pub fn run_ast_grep(
    binary: &str,
    pattern: &str,
    root: &Path,
    lang_filter: Option<&str>,
) -> Result<Vec<u8>> {
    let mut cmd = Command::new(binary);
    cmd.arg("run")
        .arg("--pattern")
        .arg(pattern)
        .arg("--json")
        .arg(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(lang) = lang_filter {
        cmd.arg("--lang").arg(lang);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| crate::StoreError::Other(format!("failed to run {binary}: {e}")))?;

    let deadline = Instant::now() + Duration::from_secs(PATTERN_TIMEOUT_SECS);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    child.kill().ok();
                    return Err(crate::StoreError::Other(format!(
                        "ast-grep timed out after {PATTERN_TIMEOUT_SECS}s"
                    )));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                child.kill().ok();
                return Err(crate::StoreError::Other(format!("ast-grep wait failed: {e}")));
            }
        }
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut stdout).map_err(|e| {
            crate::StoreError::Other(format!("failed to read ast-grep stdout: {e}"))
        })?;
    }
    if let Some(mut err) = child.stderr.take() {
        err.read_to_end(&mut stderr).ok();
    }
    if !status.success() && stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(crate::StoreError::Other(format!("ast-grep failed: {stderr}")));
    }
    Ok(stdout)
}
