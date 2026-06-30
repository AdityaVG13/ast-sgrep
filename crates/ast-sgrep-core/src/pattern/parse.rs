use std::path::Path;

use crate::Result;

use super::pattern_hit;

pub fn normalize_hit_path(file: &str, root: &Path) -> String {
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

pub fn parse_ast_grep_json(stdout: &[u8], pattern: &str, root: &Path) -> Result<Vec<crate::search::SearchHit>> {
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
        let language = value
            .get("language")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        hits.push(pattern_hit(
            file,
            start_line,
            end_line,
            pattern,
            language,
            excerpt,
        ));
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
