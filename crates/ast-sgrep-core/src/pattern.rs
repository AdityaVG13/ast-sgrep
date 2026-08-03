use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::rank::SCORE_PATTERN;
use crate::search::{HitKind, SearchHit, SpanHitInput};
use crate::Result;
use ast_sgrep_lang::{
    cached_pattern_signatures, detect_language, match_pattern, needs_ast_grep_fallback,
};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
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
    // Three-layer structural stack (iva9.7):
    // 1) Index-backed pattern_nodes (O(signatures), no re-parse).
    // 2) In-process tree-sitter native matcher for classifiable shapes.
    // 3) Optional external ast-grep for exotic `$` shapes only.
    //
    // Fail-closed policy: exotic patterns that need layer 3 never silently
    // return empty when the external binary is absent. Classifiable native
    // shapes treat empty native hits as authoritative match-none (no subprocess
    // reintroduction for native gaps).
    if store.pattern_node_count()? > 0 {
        if let Some(signatures) = cached_pattern_signatures(pattern) {
            let indexed = search_pattern_cached(pattern, &signatures, store, lang_filter)?;
            if !indexed.is_empty() {
                return Ok(indexed);
            }
        }
    }
    let native = search_pattern_native(pattern, root, lang_filter)?;
    if !native.is_empty() {
        return Ok(native);
    }
    if needs_ast_grep_fallback(pattern) {
        // Fail-closed: exotic shapes never return silent empty when layer-3 is
        // unavailable (missing binary) or explicitly disabled (no subprocess).
        if !external_ast_grep_allowed() || find_ast_grep_binary().is_none() {
            return Err(crate::StoreError::Other(format!(
                "pattern requires structural fallback but ast-grep is unavailable (fail-closed): {pattern}"
            )));
        }
        return search_pattern_ast_grep(pattern, root, lang_filter);
    }
    Ok(native)
}

/// When set, skip external ast-grep entirely (iva9.7 fail-closed / no-subprocess mode).
fn external_ast_grep_allowed() -> bool {
    !matches!(
        std::env::var("ASGREP_DISABLE_AST_GREP").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
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
/// Shared timed `try_wait` loop for external ast-grep probe / search children.
fn wait_child_deadline(
    child: &mut Child,
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
                return Err(crate::StoreError::Other(format!(
                    "{label} wait failed: {e}"
                )));
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
            language: value
                .get("language")
                .and_then(|v| v.as_str())
                .map(str::to_string),
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
pub fn ast_grep_pattern_for_query(query: &str) -> Option<String> {
    let q = query.trim();
    let q = q
        .strip_prefix("defs:")
        .or_else(|| q.strip_prefix("callers:"))
        .unwrap_or(q)
        .trim();
    (!q.is_empty() && !q.contains(' ')).then(|| q.to_string())
}
