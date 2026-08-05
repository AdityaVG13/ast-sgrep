use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::rank::SCORE_PATTERN;
use crate::search::{HitKind, SearchHit, SpanHitInput};
use crate::Result;
use ast_sgrep_lang::{
    cached_pattern_signatures, detect_language, match_pattern, required_pattern_literal,
};
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use walkdir::WalkDir;
/// Convert a simple query or `defs:` / `callers:` prefix into an ast-grep pattern.
pub fn ast_grep_pattern_for_query(query: &str) -> Option<String> {
    let q = query.trim();
    let q = q
        .strip_prefix("defs:")
        .or_else(|| q.strip_prefix("callers:"))
        .unwrap_or(q)
        .trim();
    (!q.is_empty() && !q.contains(' ')).then(|| q.to_string())
}

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
    // Union index signatures with native tree-sitter matches (92nj).
    // Production does not spawn external ast-grep by default; native-only is the
    // honest completeness path when the index is partial.
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if store.pattern_node_count()? > 0 {
        if let Some(signatures) = cached_pattern_signatures(pattern) {
            for hit in search_pattern_cached(pattern, &signatures, store, lang_filter)? {
                if seen.insert((hit.file.clone(), hit.line_start, hit.line_end)) {
                    hits.push(hit);
                }
            }
        }
    }
    match search_pattern_native(pattern, root, lang_filter) {
        Ok(native) => {
            for hit in native {
                if seen.insert((hit.file.clone(), hit.line_start, hit.line_end)) {
                    hits.push(hit);
                }
            }
        }
        Err(e) if hits.is_empty() => return Err(e),
        Err(_) => {}
    }
    Ok(hits)
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

/// Timed `try_wait` loop shared by the optional ast-grep version probe and bench runner.
/// Returns `Some(())` when the child exits (and succeeds if `require_success`), else kills and returns `None`.
fn wait_child_deadline(
    child: &mut Child,
    deadline: Instant,
    require_success: bool,
) -> Option<()> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if require_success && !status.success() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                return Some(());
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}
/// Optional external `ast-grep` for **bench comparison only**.
/// Disabled by default: never searches PATH or executes untrusted binaries
/// (`ast-sgrep-j0x4` / `agent-security-rl1p.5`). Requires both
/// `ASGREP_ALLOW_AST_GREP=1` and an absolute `ASGREP_AST_GREP` file path.
fn find_ast_grep_binary() -> Option<String> {
    if !crate::env_flag::env_flag("ASGREP_ALLOW_AST_GREP") {
        return None;
    }
    let path = std::env::var("ASGREP_AST_GREP").ok()?;
    let path = Path::new(&path);
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
    let ast_grep = find_ast_grep_binary()?;
    let root = root
        .canonicalize()
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

#[cfg(test)]
mod tests {
    use ast_sgrep_lang::cached_pattern_signatures;

    #[test]
    fn fixed_bakeoff_suite_is_index_or_native_resolvable() {
        const PATTERNS: &[&str] = &[
            "fn gitignore_matched",
            "fn parse_low",
            "struct WalkBuilder",
            "fn search_slice",
            "struct RegexMatcherBuilder",
            "struct StandardBuilder",
            "struct JSONBuilder",
            "struct GlobBuilder",
            "DecompressionMatcherBuilder",
            "struct TypesBuilder",
            "fn run",
            "struct OverrideBuilder",
            "fn open_mmap",
            "fn multi_line_with_matcher",
            "def full_dispatch_request",
            "class Blueprint",
            "class SecureCookieSessionInterface",
            "class DispatchingJinjaLoader",
            "class FlaskGroup",
            "def from_pyfile",
            "class AppContext",
            "class DefaultJSONProvider",
            "request_started",
            "class MethodView",
            "def get_flashed_messages",
            "class Request",
            "class App",
            "def setupmethod",
            "class TaggedJSONSerializer",
        ];
        assert_eq!(PATTERNS.len(), 29);
        for pattern in PATTERNS {
            assert!(
                cached_pattern_signatures(pattern).is_some(),
                "no indexed signature for {pattern}"
            );
            assert!(
                !ast_sgrep_lang::needs_ast_grep_fallback(pattern),
                "fixed suite unexpectedly requires a subprocess: {pattern}"
            );
        }
    }

    #[test]
    fn cached_metavariables_cover_kind_predicates() {
        assert!(cached_pattern_signatures("function $NAME($$$)")
            .unwrap()
            .contains(&"kind:method_declaration".to_string()));
        assert_eq!(
            cached_pattern_signatures("kind:function_item").unwrap(),
            vec!["kind:function_item"]
        );
    }

    #[test]
    fn external_ast_grep_is_disabled_without_explicit_allow() {
        // Even if PATH has ast-grep, production/bench helpers stay inert.
        std::env::remove_var("ASGREP_ALLOW_AST_GREP");
        std::env::remove_var("ASGREP_AST_GREP");
        assert!(super::find_ast_grep_binary().is_none());
        assert!(super::bench_ast_grep("fn foo", std::path::Path::new("."), 1).is_none());
    }
}
