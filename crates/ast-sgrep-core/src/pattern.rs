use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::rank::SCORE_PATTERN;
use crate::search::{HitKind, SearchHit, SpanHitInput};
use crate::Result;
use ast_sgrep_lang::{detect_language, match_pattern, needs_ast_grep_fallback};
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use walkdir::WalkDir;
const PATTERN_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize)]
pub struct PatternSearchProfile {
    pub files_considered: usize,
    pub files_prefiltered: usize,
    pub files_parsed: usize,
    pub bytes_scanned: u64,
    pub hits: usize,
    pub workers: usize,
    pub walk_ns: u128,
    pub prefilter_work_ns: u128,
    pub parse_match_work_ns: u128,
    pub parallel_span_ns: u128,
    pub rank_ns: u128,
    pub t1_ns: u128,
    pub prefilter_disabled_t1_ns: u128,
    pub t_inf_ns: u128,
    pub brent_upper_bound_ns: u128,
    pub serial_fraction: f64,
    pub observed_speedup: f64,
    pub prefilter_speedup: f64,
}

#[derive(Default)]
struct NativeFileResult {
    hits: Vec<SearchHit>,
    bytes_scanned: u64,
    prefiltered: bool,
    parsed: bool,
    prefilter_ns: u128,
    parse_match_ns: u128,
}

struct NativeSearchOutput {
    hits: Vec<SearchHit>,
    profile: PatternSearchProfile,
    total_elapsed_ns: u128,
    max_file_work_ns: u128,
}

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
fn cached_pattern_signatures(pattern: &str) -> Option<Vec<String>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Some(vec![]);
    }
    if !pattern.contains('$') {
        return Some(vec![pattern.to_string()]);
    }
    for (prefix, kind) in [("fn ", "function_item"), ("def ", "function_definition")] {
        if let Some(rest) = pattern.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| ch == '(' || ch.is_whitespace())
                .next()
                .unwrap_or_default();
            if name.starts_with('$') {
                return Some(vec![format!("kind:{kind}")]);
            }
            if is_pattern_identifier(name) {
                return Some(vec![format!("decl:{}:{name}", prefix.trim())]);
            }
            return None;
        }
    }
    let open = pattern.find('(')?;
    let close = pattern.rfind(')')?;
    if close + 1 != pattern.len() || !pattern[open + 1..close].contains("$$$") {
        return None;
    }
    let callee = pattern[..open].trim();
    if callee.starts_with('$') && !callee.contains('.') {
        return Some(vec!["kind:call_expression".into(), "kind:call".into()]);
    }
    if let Some(name) = callee.rsplit('.').next() {
        if callee.contains('$') && is_pattern_identifier(name) {
            return Some(vec![format!("call-name:{name}")]);
        }
    }
    is_pattern_path(callee).then(|| vec![format!("call:{callee}")])
}
fn is_pattern_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}
fn is_pattern_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('$')
        && value
            .split(['.', ':'])
            .filter(|p| !p.is_empty())
            .all(is_pattern_identifier)
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
    Ok(search_pattern_native_profiled(pattern, root, lang_filter, true)?.hits)
}

pub fn profile_pattern_search(
    pattern: &str,
    root: &Path,
    lang_filter: Option<&str>,
) -> Result<PatternSearchProfile> {
    let single_worker = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .map_err(|error| {
            crate::StoreError::Other(format!("failed to build pattern profiling pool: {error}"))
        })?;
    let baseline = single_worker
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, false))?;
    let serial = single_worker
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, true))?;
    let parallel = search_pattern_native_profiled(pattern, root, lang_filter, true)?;
    let identity = |hits: &[SearchHit]| {
        hits.iter()
            .map(|hit| (hit.file.clone(), hit.line_start, hit.line_end))
            .collect::<Vec<_>>()
    };
    if identity(&baseline.hits) != identity(&serial.hits)
        || identity(&serial.hits) != identity(&parallel.hits)
    {
        return Err(crate::StoreError::Other(
            "pattern prefilter or parallel execution changed the native hit set".into(),
        ));
    }

    let mut profile = serial.profile;
    profile.workers = parallel.profile.workers;
    profile.parallel_span_ns = parallel.total_elapsed_ns;
    profile.t1_ns = serial.total_elapsed_ns;
    profile.prefilter_disabled_t1_ns = baseline.total_elapsed_ns;
    let serial_ns = profile.walk_ns + profile.rank_ns;
    profile.t_inf_ns = serial_ns + serial.max_file_work_ns;
    profile.brent_upper_bound_ns =
        profile.t1_ns.div_ceil(profile.workers as u128) + profile.t_inf_ns;
    profile.serial_fraction = ratio(serial_ns, profile.t1_ns);
    profile.observed_speedup = ratio(profile.t1_ns, profile.parallel_span_ns);
    profile.prefilter_speedup = ratio(profile.prefilter_disabled_t1_ns, profile.t1_ns);
    Ok(profile)
}

fn ratio(numerator: u128, denominator: u128) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn search_pattern_native_profiled(
    pattern: &str,
    root: &Path,
    lang_filter: Option<&str>,
    use_prefilter: bool,
) -> Result<NativeSearchOutput> {
    let total_started = Instant::now();
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let ignore = crate::gitignore::IgnoreMatcher::new(&root);
    let walk_started = Instant::now();
    let paths = WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !should_skip_dir(entry.path()))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.into_path();
            if should_skip_file(&path) {
                return None;
            }
            let rel = path.strip_prefix(&root).ok()?;
            (!ignore.is_ignored(rel)).then_some(path)
        })
        .collect::<Vec<PathBuf>>();
    let walk_ns = walk_started.elapsed().as_nanos();
    let required_literal = use_prefilter
        .then(|| required_pattern_literal(pattern))
        .flatten();
    let parallel_started = Instant::now();
    let results = paths
        .par_iter()
        .map(|path| {
            let prefilter_started = Instant::now();
            let Ok(bytes) = fs::read(path) else {
                return NativeFileResult::default();
            };
            let bytes_scanned = bytes.len() as u64;
            if required_literal
                .as_ref()
                .is_some_and(|literal| memchr::memmem::find(&bytes, literal.as_bytes()).is_none())
            {
                return NativeFileResult {
                    bytes_scanned,
                    prefiltered: true,
                    prefilter_ns: prefilter_started.elapsed().as_nanos(),
                    ..NativeFileResult::default()
                };
            }
            let Ok(content) = std::str::from_utf8(&bytes) else {
                return NativeFileResult {
                    bytes_scanned,
                    prefilter_ns: prefilter_started.elapsed().as_nanos(),
                    ..NativeFileResult::default()
                };
            };
            let Some(lang) = detect_language(path, Some(content)) else {
                return NativeFileResult {
                    bytes_scanned,
                    prefilter_ns: prefilter_started.elapsed().as_nanos(),
                    ..NativeFileResult::default()
                };
            };
            if lang_filter.is_some_and(|filter| lang.as_str() != filter) {
                return NativeFileResult {
                    bytes_scanned,
                    prefilter_ns: prefilter_started.elapsed().as_nanos(),
                    ..NativeFileResult::default()
                };
            }
            let prefilter_ns = prefilter_started.elapsed().as_nanos();
            let parse_match_started = Instant::now();
            let rel = path
                .strip_prefix(&root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
            let hits = match_pattern(lang, content, pattern)
                .unwrap_or_default()
                .into_iter()
                .map(|matched| {
                    SearchHit::span(SpanHitInput {
                        kind: HitKind::Pattern,
                        file: rel.clone(),
                        line_start: matched.line_start,
                        line_end: matched.line_end,
                        score: SCORE_PATTERN,
                        excerpt: matched.excerpt,
                        symbol: Some(pattern.to_string()),
                        language: Some(lang.as_str().to_string()),
                    })
                })
                .collect();
            NativeFileResult {
                hits,
                bytes_scanned,
                parsed: true,
                prefilter_ns,
                parse_match_ns: parse_match_started.elapsed().as_nanos(),
                ..NativeFileResult::default()
            }
        })
        .collect::<Vec<_>>();
    let parallel_span_ns = parallel_started.elapsed().as_nanos();
    let rank_started = Instant::now();
    let mut hits = results
        .iter()
        .flat_map(|result| result.hits.iter().cloned())
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then(left.line_start.cmp(&right.line_start))
            .then(left.line_end.cmp(&right.line_end))
    });
    let rank_ns = rank_started.elapsed().as_nanos();
    let prefilter_work_ns = results
        .iter()
        .map(|result| result.prefilter_ns)
        .sum::<u128>();
    let parse_match_work_ns = results
        .iter()
        .map(|result| result.parse_match_ns)
        .sum::<u128>();
    let max_file_work_ns = results
        .iter()
        .map(|result| result.prefilter_ns + result.parse_match_ns)
        .max()
        .unwrap_or_default();
    let t1_ns = walk_ns + prefilter_work_ns + parse_match_work_ns + rank_ns;
    let serial_ns = walk_ns + rank_ns;
    let t_inf_ns = serial_ns + max_file_work_ns;
    let workers = rayon::current_num_threads().max(1);
    let workers_u128 = workers as u128;
    let brent_upper_bound_ns = t1_ns.div_ceil(workers_u128) + t_inf_ns;
    let serial_fraction = if t1_ns == 0 {
        0.0
    } else {
        serial_ns as f64 / t1_ns as f64
    };
    let profile = PatternSearchProfile {
        files_considered: paths.len(),
        files_prefiltered: results.iter().filter(|result| result.prefiltered).count(),
        files_parsed: results.iter().filter(|result| result.parsed).count(),
        bytes_scanned: results.iter().map(|result| result.bytes_scanned).sum(),
        hits: hits.len(),
        workers,
        walk_ns,
        prefilter_work_ns,
        parse_match_work_ns,
        parallel_span_ns,
        rank_ns,
        t1_ns,
        prefilter_disabled_t1_ns: 0,
        t_inf_ns,
        brent_upper_bound_ns,
        serial_fraction,
        observed_speedup: 0.0,
        prefilter_speedup: 0.0,
    };
    Ok(NativeSearchOutput {
        hits,
        profile,
        total_elapsed_ns: total_started.elapsed().as_nanos(),
        max_file_work_ns,
    })
}

fn required_pattern_literal(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    if !pattern.contains('$') {
        return Some(pattern.to_string());
    }
    for prefix in [
        "fn ",
        "def ",
        "function ",
        "func ",
        "class ",
        "struct ",
        "interface ",
        "type ",
    ] {
        if let Some(rest) = pattern.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| ch == '(' || ch == '{' || ch == '<' || ch.is_whitespace())
                .next()
                .unwrap_or_default();
            return (!name.is_empty() && !name.starts_with('$')).then(|| name.to_string());
        }
    }
    let callee = pattern.split_once('(')?.0.trim();
    callee
        .split(['.', ':'])
        .filter(|segment| !segment.is_empty() && !segment.starts_with('$'))
        .max_by_key(|segment| segment.len())
        .map(str::to_string)
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
    let deadline = Instant::now() + Duration::from_secs(PATTERN_TIMEOUT_SECS);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                child.kill().ok();
                return Err(crate::StoreError::Other(format!(
                    "ast-grep timed out after {PATTERN_TIMEOUT_SECS}s"
                )));
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                child.kill().ok();
                return Err(crate::StoreError::Other(format!(
                    "ast-grep wait failed: {e}"
                )));
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
        return Err(crate::StoreError::Other(format!(
            "ast-grep failed: {}",
            String::from_utf8_lossy(&stderr)
        )));
    }
    parse_ast_grep_json(&stdout, pattern, &root)
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
