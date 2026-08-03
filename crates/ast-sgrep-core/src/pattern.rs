use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::rank::SCORE_PATTERN;
use crate::search::{HitKind, SearchHit, SpanHitInput};
use crate::Result;
use ast_sgrep_lang::{
    cached_pattern_signatures, detect_language, match_pattern, needs_ast_grep_fallback, Language,
};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use walkdir::WalkDir;
const PATTERN_TIMEOUT_SECS: u64 = 30;
pub fn search_pattern(
    pattern: &str,
    store: &crate::store::IndexStore,
    root: &Path,
    lang_filter: Option<&str>,
) -> Result<Vec<SearchHit>> {
    // 1) Index-backed structural signatures (O(signatures), no re-parse).
    if store.pattern_node_count()? > 0 {
        if let Some(signatures) = cached_pattern_signatures(pattern) {
            let indexed = search_pattern_cached(pattern, &signatures, store, lang_filter)?;
            if !indexed.is_empty() {
                return Ok(indexed);
            }
        }
    }
    // 2) In-process tree-sitter match (literal + native metavariable shapes).
    let native = search_pattern_native(pattern, root, lang_filter)?;
    if !native.is_empty() || !needs_ast_grep_fallback(pattern) {
        // Prefer native results; only spawn external ast-grep when we truly cannot
        // represent the pattern and have zero native hits.
        if !native.is_empty() || find_ast_grep_binary().is_none() {
            return Ok(native);
        }
    }
    // 3) Full ast-grep for exotic structural rules (if installed).
    if needs_ast_grep_fallback(pattern) {
        return search_pattern_ast_grep(pattern, root, lang_filter);
    }
    Ok(native)
}
fn search_pattern_cached(
    pattern: &str,
    signatures: &[String],
    store: &crate::store::IndexStore,
    lang_filter: Option<&str>,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for signature in signatures {
        for row in store.pattern_nodes_matching(signature, lang_filter)? {
            if !seen.insert((row.path.clone(), row.line_start, row.line_end)) {
                continue;
            }
            hits.push(SearchHit::span(SpanHitInput {
                kind: HitKind::Pattern,
                file: row.path,
                line_start: row.line_start,
                line_end: row.line_end,
                score: SCORE_PATTERN,
                excerpt: row.excerpt,
                symbol: Some(pattern.to_string()),
                language: row.language,
            }));
        }
    }
    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line_start.cmp(&b.line_start)));
    Ok(hits)
}
fn search_pattern_native(
    pattern: &str,
    root: &Path,
    lang_filter: Option<&str>,
) -> Result<Vec<SearchHit>> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let ignore = crate::gitignore::IgnoreMatcher::new(&root);
    let mut hits = Vec::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !should_skip_dir(e.path()))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if should_skip_file(path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(&root) else {
            continue;
        };
        if ignore.is_ignored(rel) {
            continue;
        }
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let Some(lang) = detect_language(path, Some(&content)) else {
            continue;
        };
        if lang_filter.is_some_and(|f| lang.as_str() != f) {
            continue;
        }
        if let Ok(matches) = match_pattern(lang, &content, pattern) {
            hits.extend(matches.into_iter().map(|m| {
                SearchHit::span(SpanHitInput {
                    kind: HitKind::Pattern,
                    file: rel_str.clone(),
                    line_start: m.line_start,
                    line_end: m.line_end,
                    score: SCORE_PATTERN,
                    excerpt: m.excerpt,
                    symbol: Some(pattern.to_string()),
                    language: Some(lang.as_str().to_string()),
                })
            }));
        }
    }
    Ok(hits)
}
fn search_pattern_ast_grep(
    pattern: &str,
    root: &Path,
    lang_filter: Option<&str>,
) -> Result<Vec<SearchHit>> {
    let Some(ast_grep) = find_ast_grep_binary() else {
        return Err(crate::StoreError::Other(
            "ast-grep not found: install from https://github.com/ast-grep/ast-grep".into(),
        ));
    };
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut cmd = Command::new(&ast_grep);
    cmd.arg("run")
        .arg("--pattern")
        .arg(pattern)
        .arg("--json")
        .arg(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(lang) = lang_filter {
        cmd.arg("--lang").arg(lang);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| crate::StoreError::Other(format!("failed to run {ast_grep}: {e}")))?;
    let status = wait_child_deadline(
        &mut child,
        Duration::from_secs(PATTERN_TIMEOUT_SECS),
        "ast-grep",
    )?;
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
        return Err(crate::StoreError::Other(format!(
            "ast-grep failed: {}",
            String::from_utf8_lossy(&stderr)
        )));
    }
    parse_ast_grep_json(&stdout, pattern, &root)
}

fn wait_child_deadline(
    child: &mut std::process::Child,
    timeout: Duration,
    label: &str,
) -> Result<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                child.kill().ok();
                return Err(crate::StoreError::Other(format!(
                    "{label} timed out after {}s",
                    timeout.as_secs()
                )));
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                child.kill().ok();
                return Err(crate::StoreError::Other(format!("{label} wait failed: {e}")));
            }
        }
    }
}
fn find_ast_grep_binary() -> Option<String> {
    for name in ["ast-grep", "sg"] {
        let Ok(output) = Command::new(name).arg("--version").output() else {
            continue;
        };
        if output.status.success() && String::from_utf8_lossy(&output.stdout).contains("ast-grep") {
            return Some(name.into());
        }
    }
    if let Ok(path) = std::env::var("ASGREP_AST_GREP") {
        if Path::new(&path).is_file() {
            return Some(path);
        }
    }
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools/ast-grep");
    bundled
        .is_file()
        .then(|| bundled.to_string_lossy().into_owned())
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
    let mut hits = Vec::new();
    for line in String::from_utf8_lossy(stdout).lines() {
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
        hits.push(SearchHit::span(SpanHitInput {
            kind: HitKind::Pattern,
            file: normalize_hit_path(raw_file, root),
            line_start: start_line,
            line_end: end_line,
            score: SCORE_PATTERN,
            excerpt: value
                .get("text")
                .or_else(|| value.get("lines"))
                .and_then(|v| v.as_str())
                .unwrap_or(pattern)
                .to_string(),
            symbol: Some(pattern.to_string()),
            // ast-grep historically emits Title Case ("Rust"); native hits use
            // Language::as_str ("rust"). Normalize so matches_lang / --lang filters
            // and cross-engine dedup stay equivalent (amm8).
            language: value
                .get("language")
                .and_then(|v| v.as_str())
                .map(Language::normalize_id),
        }));
    }
    Ok(hits)
}
pub fn bench_ast_grep(pattern: &str, root: &Path, iterations: u32) -> Option<f64> {
    let ast_grep = find_ast_grep_binary()?;
    let root = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let mut total = 0.0f64;
    for _ in 0..iterations {
        let start = Instant::now();
        let _ = Command::new(&ast_grep)
            .args(["run", "--pattern", pattern, &root])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        total += start.elapsed().as_secs_f64() * 1000.0;
    }
    Some(total / f64::from(iterations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn ast_grep_language_field_normalizes_to_as_str() {
        let root = PathBuf::from("/tmp/proj");
        let stdout = concat!(
            r#"{"file":"/tmp/proj/src/lib.rs","language":"Rust","range":{"start":{"line":1},"end":{"line":1}},"text":"fn foo() {}"}"#,
            "\n",
            r#"{"file":"/tmp/proj/Main.cs","language":"C#","range":{"start":{"line":2},"end":{"line":2}},"text":"class A {}"}"#,
            "\n",
            r#"{"file":"/tmp/proj/a.cpp","language":"C++","range":{"start":{"line":3},"end":{"line":3}},"text":"int x;"}"#,
            "\n",
        );
        let hits = parse_ast_grep_json(stdout.as_bytes(), "fn $NAME", &root).unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].language.as_deref(), Some("rust"));
        assert_eq!(hits[1].language.as_deref(), Some("csharp"));
        assert_eq!(hits[2].language.as_deref(), Some("cpp"));
        assert!(crate::search::matches_lang(hits[0].language.as_deref(), Some("rust")));
        assert!(crate::search::matches_lang(Some("Rust"), Some("rust")));
        assert!(crate::search::matches_lang(Some("C#"), Some("csharp")));
    }

    #[test]
    fn native_and_normalized_ast_grep_share_as_str_casing() {
        for lang in Language::all() {
            let title = {
                let s = lang.as_str();
                let mut c = s.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            };
            assert_eq!(Language::normalize_id(&title), lang.as_str());
            assert_eq!(Language::normalize_id(lang.as_str()), lang.as_str());
        }
        assert_eq!(Language::normalize_id("C#"), "csharp");
        assert_eq!(Language::normalize_id("C++"), "cpp");
    }

    #[test]
    fn cached_signatures_delegate_to_lang_byte_identically() {
        assert_eq!(
            cached_pattern_signatures("fn parse_low($$$)").unwrap(),
            vec!["decl:fn:parse_low".to_string()]
        );
        assert_eq!(
            cached_pattern_signatures("fn $NAME($$$)").unwrap(),
            vec!["kind:function_item".to_string()]
        );
        assert_eq!(
            cached_pattern_signatures("$F($$$)").unwrap(),
            vec![
                "kind:call_expression".to_string(),
                "kind:call".to_string(),
            ]
        );
    }
}

