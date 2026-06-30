use std::path::Path;
use std::process::Command;

use crate::search::{HitKind, SearchHit};
use crate::rank::SCORE_PATTERN;
use crate::Result;

/// Run ast-grep/sg for structural pattern search when available.
pub fn search_pattern(pattern: &str, root: &Path, lang_filter: Option<&str>) -> Result<Vec<SearchHit>> {
    let binary = find_ast_grep_binary();
    let Some(ast_grep) = binary else {
        return Ok(vec![SearchHit {
            kind: HitKind::Pattern,
            file: String::new(),
            line_start: 0,
            line_end: 0,
            symbol: None,
            caller: None,
            callee: None,
            language: None,
            score: 0.0,
            excerpt: "ast-grep not found: install from https://github.com/ast-grep/ast-grep".to_string(),
        }]);
    };

    let mut cmd = Command::new(&ast_grep);
    cmd.arg("run")
        .arg("--pattern")
        .arg(pattern)
        .arg("--json")
        .arg(root);

    if let Some(lang) = lang_filter {
        cmd.arg("--lang").arg(lang);
    }

    let output = cmd.output().map_err(|e| {
        crate::StoreError::Other(format!("failed to run {ast_grep}: {e}"))
    })?;

    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::StoreError::Other(format!("ast-grep failed: {stderr}")));
    }

    parse_ast_grep_json(&output.stdout, pattern)
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

fn parse_ast_grep_json(stdout: &[u8], pattern: &str) -> Result<Vec<SearchHit>> {
    let text = String::from_utf8_lossy(stdout);
    let mut hits = Vec::new();

    // ast-grep --json emits one JSON object per line (NDJSON)
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let file = value
            .get("file")
            .or_else(|| value.pointer("/range/filename"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if file.is_empty() {
            continue;
        }
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

    #[test]
    fn parses_ndjson_line() {
        let json = r#"{"file":"src/main.rs","range":{"start":{"line":1},"end":{"line":1}},"text":"fn main() {}"}"#;
        let hits = parse_ast_grep_json(json.as_bytes(), "fn main").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, "src/main.rs");
    }
}
