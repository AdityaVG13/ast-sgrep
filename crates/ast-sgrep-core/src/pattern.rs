use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::search::{HitKind, SearchHit};
use crate::rank::SCORE_PATTERN;
use crate::Result;

const PATTERN_TIMEOUT_SECS: u64 = 30;

/// Run ast-grep/sg for structural pattern search when available.
pub fn search_pattern(pattern: &str, root: &Path, lang_filter: Option<&str>) -> Result<Vec<SearchHit>> {
    let binary = find_ast_grep_binary();
    let Some(ast_grep) = binary else {
        return Err(crate::StoreError::Other(
            "ast-grep not found: install from https://github.com/ast-grep/ast-grep".to_string(),
        ));
    };

    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut cmd = Command::new(&ast_grep);
    cmd.arg("run")
        .arg("--pattern")
        .arg(pattern)
        .arg("--json")
        .arg(&canonical_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(lang) = lang_filter {
        cmd.arg("--lang").arg(lang);
    }

    let mut child = cmd.spawn().map_err(|e| {
        crate::StoreError::Other(format!("failed to run {ast_grep}: {e}"))
    })?;

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
    parse_ast_grep_json(&stdout, pattern, &canonical_root)
}

fn find_ast_grep_binary() -> Option<String> {
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

fn normalize_hit_path(file: &str, root: &Path) -> String {
    let p = Path::new(file);
    if let Ok(rel) = p.strip_prefix(root) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    if let Ok(canon) = p.canonicalize() {
        let croot = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if let Ok(rel) = canon.strip_prefix(&croot) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    file.replace('\\', "/")
}

fn parse_ast_grep_json(stdout: &[u8], pattern: &str, root: &Path) -> Result<Vec<SearchHit>> {
    let text = String::from_utf8_lossy(stdout);
    let mut hits = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let raw_file = value
            .get("file")
            .or_else(|| value.pointer("/range/filename"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if raw_file.is_empty() {
            continue;
        }
        let file = normalize_hit_path(raw_file, root);
        let start_line = value
            .pointer("/range/start/line")
            .or_else(|| value.get("start_line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let end_line = value
            .pointer("/range/end/line")
            .or_else(|| value.get("end_line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(start_line as u64) as u32;
        let excerpt = value
            .get("text")
            .or_else(|| value.get("lines"))
            .and_then(|v| v.as_str())
            .unwrap_or(pattern)
            .to_string();

        hits.push(SearchHit {
            kind: HitKind::Pattern,
            file,
            line_start: start_line,
            line_end: end_line,
            symbol: Some(pattern.to_string()),
            caller: None,
            callee: None,
            language: value
                .get("language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            score: SCORE_PATTERN,
            excerpt,
        });
    }

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_ndjson_line() {
        let json = r#"{"file":"src/main.rs","range":{"start":{"line":1},"end":{"line":1}},"text":"fn main() {}"}"#;
        let root = PathBuf::from("/repo");
        let hits = parse_ast_grep_json(json.as_bytes(), "fn main", &root).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, "src/main.rs");
    }

    #[test]
    fn normalizes_absolute_paths() {
        let root = std::fs::canonicalize("/tmp").unwrap_or_else(|_| PathBuf::from("/tmp"));
        let abs = root.join("src/lib.rs");
        let normalized = normalize_hit_path(&abs.to_string_lossy(), &root);
        assert_eq!(normalized, "src/lib.rs");
    }
}
