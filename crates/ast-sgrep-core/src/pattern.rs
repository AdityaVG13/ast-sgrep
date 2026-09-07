use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::io_bounds::MAX_INDEX_FILE_BYTES;
use crate::rank::SCORE_PATTERN;
use crate::search::{HitKind, SearchHit, SpanHitInput};
use crate::Result;
use ast_sgrep_lang::{
    cached_pattern_signatures, detect_language, index_can_serve_pattern, match_pattern,
    needs_ast_grep_fallback, required_pattern_literal,
};
use rayon::prelude::*;
use serde::Serialize;
use std::borrow::Cow;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
    /// PASS 60 (H-CONF-029): number of DISTINCT corpus languages (post
    /// lang-filter) with NO native template for this pattern. Censused from
    /// the path set BEFORE the byte prefilter runs, so prefiltered-away
    /// files cannot hide unanswerability behind silent empty results.
    unanswerable_corpus_languages: usize,
}

pub fn search_pattern(
    pattern: &str,
    store: &crate::store::IndexStore,
    root: &Path,
    lang_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    // Union index signatures with native tree-sitter matches (92nj).
    // Production does not spawn external ast-grep by default; native-only is the
    // honest completeness path when the index is partial.
    // H-CONF-032 (pass 63): strip a leading BOM BEFORE any lane sees the
    // pattern, exactly as match_pattern's R3 strip does. The raw U+FEFF is
    // not whitespace for `str::trim`, so without this strip it rides into
    // `required_pattern_literal` (`\ufeffgreet`) and the byte prefilter drops
    // every file that does not literally start with a BOM-prefixed callee —
    // the index-served lane answered silent ok:true-0 where sg answers
    // through the BOM (pass-62b face, BOM-led `greet($A)` python).
    let pattern = pattern.trim().trim_start_matches('\u{feff}').trim();
    // PASS 65 (LOW a): the H-CONF-032 strip runs BEFORE this guard, so a
    // BOM-only pattern cannot ride past it as a non-empty string and then
    // degrade to a silent ok:true-empty answer for the vacuous "" pattern.
    // The codemod lane refuses the same input loudly at its ingress; search
    // now refuses it in the same operational class as the structural-ingress
    // rejection below (sg exits nonzero on an empty pattern too).
    if pattern.is_empty() {
        return Err(crate::StoreError::Other(
            "pattern must not be empty".into(),
        ));
    }
    let canonical = ast_sgrep_lang::Language::canonical_filter(lang_filter);
    let lang_filter = canonical.as_deref();
    // H-CONF-023 (pass 30): loud pattern-ingress. A `$`-pattern the native
    // classifier rejects can never be answered natively; reject it BEFORE
    // scanning instead of letting match_pattern's per-file empty results
    // compose into a silent `ok:true` empty envelope. Classifier-accepted
    // patterns never take this arm, so their zero-hit results stay ok:true.
    if needs_ast_grep_fallback(pattern) {
        return Err(structural_fallback_error(pattern));
    }
    // PASS 67c (H-CONF-036 / F26-0561): de-collide rest/single capture names
    // (see rename_colliding_callee_rest). Runs AFTER the H-CONF-023 gate so a
    // classifier-rejected spelling keeps its loud fail-closed class untouched,
    // and BEFORE every lane below so the index-signature route, the candidate
    // kind narrowing, the unanswerable-language census, and the native matcher
    // all see one consistent spelling. The original spelling stays on the
    // fail-closed backstop check. Hit `symbol` fields carry the de-collided
    // spelling on the affected faces — the faces answered silent-empty before,
    // so no previously-emitted symbol changes.
    let match_spelling = rename_colliding_callee_rest(pattern);
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut unanswerable_corpus_languages = 0usize;
    if store.pattern_node_count()? > 0 {
        if let Some(signatures) = cached_pattern_signatures(&match_spelling) {
            let indexed =
                search_pattern_cached(&match_spelling, &signatures, store, lang_filter, limit)?;
            // Exact ident / decl / call signatures are complete in pattern_nodes.
            // Re-walking the tree cannot add a hit the index missed.
            if index_can_serve_pattern(&match_spelling, &signatures) {
                return Ok(indexed);
            }
            // EXP-005 (H-CONF-010, pass 14): kind-only signatures are inexact
            // (every `function_definition` — any arity, any return type).
            // Unioning those rows into the result set let a zero-param
            // `def main():` match a one-param template. The native walk below
            // decides every hit, and its candidate narrowing
            // (`candidate_kind_signatures`) covers exactly the files that can
            // hold a match, so inexact rows are dropped, never merged.
        }
    }
    // br-perf-candidates: narrow the native walk to files holding a node of
    // the pattern's kind when the exact shape is not indexable. Sound: files
    // without such a node cannot contain a match; the native matcher still
    // decides every hit on surviving files.
    let candidate_paths = match ast_sgrep_lang::candidate_kind_signatures(&match_spelling) {
        Some(kinds) if store.pattern_node_count()? > 0 => {
            Some(store.pattern_node_candidate_paths(&kinds, lang_filter)?)
        }
        _ => None,
    };
    let native_accepted = match search_pattern_native_profiled(
        &match_spelling,
        root,
        lang_filter,
        true,
        candidate_paths,
    ) {
        Ok(native) => {
            for hit in native.hits {
                if seen.insert((hit.file.clone(), hit.line_start, hit.line_end)) {
                    hits.push(hit);
                }
            }
            unanswerable_corpus_languages = native.unanswerable_corpus_languages;
            true
        }
        Err(_) => false,
    };
    if native_accepted
        && hits.is_empty()
        && (unanswerable_corpus_languages > 0 || needs_ast_grep_fallback(pattern))
    {
        // Fail-closed (iva9.7; H-CONF-006 fixed in pass 14): a beyond-native
        // shape NEVER returns a silent empty result. GA-24 (H-SURF-005, pass
        // 19): the delegation result path was REMOVED, not pending — the
        // external engine is bench-only (`bench_ast_grep`), so the gate-on
        // state fails closed loudly too, permanently. H-CONF-023 (pass 30):
        // the same rejection now happens at ingress above; this stays as a
        // defense-in-depth backstop on the post-walk path. H-CONF-029 (pass
        // 60): the backstop ALSO fires when the corpus holds languages with
        // no native template for the pattern — per-file `Ok(empty)` there is
        // unanswerability, not source robustness, and must stay loud (the
        // 23 fuzz fail-open faces).
        return Err(structural_fallback_error(pattern));
    }
    Ok(hits)
}

/// The structured fail-closed error for patterns the native classifier
/// rejects (H-CONF-006 pass 14 / H-CONF-023 pass 30). The message names the
/// permanent non-delegation decision (GA-24): external ast-grep is
/// bench-only, so no gate state ever answers beyond-native patterns.
fn structural_fallback_error(pattern: &str) -> crate::StoreError {
    let state = if external_ast_grep_allowed() && find_ast_grep_binary().is_some() {
        "external ast-grep is configured but is bench-only; search never delegates"
    } else {
        "structural fallback is disabled or unavailable"
    };
    crate::StoreError::Other(format!(
        "pattern requires structural fallback ({state}; fail-closed): {pattern}"
    ))
}

/// PASS 67c (H-CONF-036 / F26-0561): sg keeps `$$$Rest` (multi-capture) and
/// `$Rest` (single capture) in DISTINCT capture slots even when they share a
/// base name; the native chain matcher binds both through one map key, so a
/// classifier-accepted pattern like `$O.out.$$$A($A)` cannot bind the
/// property-name rest — every file answers match-none and the walk composes
/// into a silent `ok:true` empty where sg 0.45.2 answers hits
/// (`System.out.println(total)`). The chain lane otherwise implements sg's
/// rest-binds-property semantics exactly (`a.b.$$$C($A)` / `$O.$$$M($A)` /
/// `$O.$$$M.println($A)` probes agree hit-for-hit), so the fix renames ONLY
/// the colliding callee-path rest occurrences to a fresh deterministic name.
/// This is a deliberate behavior change, not a hit-set-preserving no-op:
/// pre-fix the colliding rest bound through the SAME map key as the single
/// capture, so the property text had to unify with the pattern's `$A`
/// argument and colliding patterns answered match-none; post-fix the fresh
/// name binds the rest sg's way and those faces answer hits. The preserved
/// invariant is name-distinct ingress: non-colliding patterns are untouched,
/// and every changed face moved from wrong-empty (or wrong unification) to
/// the sg-agreed hit set. Scoped to whole dot-segment rests in the callee
/// path (name/property slots); argument-slot rests keep their registered
/// semantics (a `$$$A` reuse in a tail argument slot never unifies with the
/// head rest in the subject, where sg unifies same-name rests — the
/// H-CONF-020 trailing-name residual family), and `::`-path name slots are
/// out of the probed scope and untouched.
/// Determinism: the fresh-name loop is a pure function of the pattern string.
fn rename_colliding_callee_rest(pattern: &str) -> Cow<'_, str> {
    let Some(head_end) = pattern.find('(') else {
        return Cow::Borrowed(pattern);
    };
    let (head, tail) = pattern.split_at(head_end);
    let mut singles = std::collections::BTreeSet::new();
    let mut rests = std::collections::BTreeSet::new();
    scan_meta_names(pattern, &mut singles, &mut rests);
    let mut renames: Vec<(&str, String)> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for segment in head.split('.') {
        let Some(base) = segment.strip_prefix("$$$") else {
            continue;
        };
        if base.is_empty() || !base.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if seen.contains(&base) {
            continue;
        }
        seen.push(base);
        if !singles.contains(base) {
            continue;
        }
        let mut ordinal = 1usize;
        // PASS 69E (r19 reconciliation): the fresh name MUST stay inside the
        // canonical metavariable alphabet `[A-Z_][A-Z0-9_]*` (F26-0182,
        // pass 69a). The original `_r` suffix carried a lowercase tail byte,
        // so the de-collided spelling itself classified MixedCase and routed
        // to NeverMatches — the collision faces went silent-empty again
        // (caught by `hconf036_rest_name_collision_with_single_meta_answers_sg_hits`
        // in the pass-69 battery). Uppercase suffix + ordinal digits keep the
        // rename canonical under both the old and corrected grammars.
        let mut fresh = format!("{base}_R");
        while singles.contains(fresh.as_str()) || rests.contains(fresh.as_str()) {
            ordinal += 1;
            fresh = format!("{base}_R{ordinal}");
        }
        rests.insert(fresh.clone());
        renames.push((base, fresh));
    }
    if renames.is_empty() {
        return Cow::Borrowed(pattern);
    }
    let renamed_head = head
        .split('.')
        .map(|segment| match segment.strip_prefix("$$$") {
            Some(base) => match renames.iter().find(|(name, _)| *name == base) {
                Some((_, fresh)) => format!("$$${fresh}"),
                None => segment.to_string(),
            },
            None => segment.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".");
    let mut renamed = String::with_capacity(pattern.len() + 8);
    renamed.push_str(&renamed_head);
    renamed.push_str(tail);
    Cow::Owned(renamed)
}

/// Every metavariable NAME in `pattern`, split by capture class: `$Name`
/// (single) vs `$$$Name`+ (rest). `$$A` universal tokens are neither.
fn scan_meta_names(
    pattern: &str,
    singles: &mut std::collections::BTreeSet<String>,
    rests: &mut std::collections::BTreeSet<String>,
) {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            i += 1;
            continue;
        }
        let mut run = 0usize;
        while i + run < chars.len() && chars[i + run] == '$' {
            run += 1;
        }
        let name_start = i + run;
        let mut name_end = name_start;
        while name_end < chars.len()
            && (chars[name_end].is_ascii_alphanumeric() || chars[name_end] == '_')
        {
            name_end += 1;
        }
        if name_end > name_start {
            let name: String = chars[name_start..name_end].iter().collect();
            match run {
                1 => {
                    singles.insert(name);
                }
                2 => {}
                _ => {
                    rests.insert(name);
                }
            }
            i = name_end;
        } else {
            i = name_start.max(i + 1);
        }
    }
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
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let cap = limit.max(1);
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // Fetch a little extra per signature so a later signature that sorts
    // earlier by path can still enter the keep-set, then truncate.
    let per_sig = cap.saturating_mul(signatures.len().max(1));
    for signature in signatures {
        for row in store.pattern_nodes_matching_limited(signature, lang_filter, per_sig)? {
            if !seen.insert((row.path.clone(), row.line_start, row.line_end)) {
                continue;
            }
            let excerpt = store.fill_pattern_excerpt(&row)?;
            hits.push(SearchHit::span(SpanHitInput {
                kind: HitKind::Pattern,
                file: row.path,
                line_start: row.line_start,
                line_end: row.line_end,
                score: SCORE_PATTERN,
                excerpt,
                symbol: Some(pattern.to_string()),
                language: row.language,
            }));
        }
    }
    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line_start.cmp(&b.line_start)));
    hits.truncate(cap);
    Ok(hits)
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
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, false, None))?;
    let serial = single_worker
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, true, None))?;
    let parallel = search_pattern_native_profiled(pattern, root, lang_filter, true, None)?;
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

/// Load a pattern-search file only when it fits the index size cap.
/// Oversized files are skipped (same 64 MiB bound as `index_file`) so a
/// rayon walk cannot `fs::read` an unbounded blob into RAM (R-PATTERN-UNBOUNDED-READ).
fn read_pattern_bytes_capped(path: &Path) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_INDEX_FILE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    match file
        .take(MAX_INDEX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
    {
        Ok(_) if (bytes.len() as u64) <= MAX_INDEX_FILE_BYTES => Some(bytes),
        _ => None,
    }
}

/// Expand one directory for the BFS walker: returns its directly-held files
/// (gitignore-filtered) and pruned child directories. `dir` is the dir being
/// expanded; `root` anchors gitignore rel-path computation.
fn expand_dir(
    ignore: &crate::gitignore::IgnoreMatcher,
    root: &Path,
    dir: &std::sync::Arc<Path>,
) -> (Vec<PathBuf>, Vec<std::sync::Arc<Path>>) {
    let mut files = Vec::new();
    let mut child_dirs = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(_) => return (files, child_dirs),
    };
    for entry in read.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_symlink() || ft.is_file() {
            if should_skip_file(&path) {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            if !ft.is_symlink() && !ignore.is_ignored(rel) {
                files.push(path);
            }
            continue;
        }
        if ft.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            if ignore.is_dir_ignored(rel) {
                continue;
            }
            child_dirs.push(std::sync::Arc::from(path.into_boxed_path()));
        }
    }
    (files, child_dirs)
}

fn search_pattern_native_profiled(
    pattern: &str,
    root: &Path,
    lang_filter: Option<&str>,
    use_prefilter: bool,
    candidate_paths: Option<std::collections::HashSet<String>>,
) -> Result<NativeSearchOutput> {
    let canonical = ast_sgrep_lang::Language::canonical_filter(lang_filter);
    let lang_filter = canonical.as_deref();
    let total_started = Instant::now();
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let walk_started = Instant::now();
    // br-perf-parwalk-bfs: breadth-first traversal, one parallel level at a
    // time. Each frontier dir is expanded on a walk-pool worker with its own
    // IgnoreMatcher; files are claimed exactly once (each file has exactly
    // one parent dir, and each dir appears in exactly one frontier); child
    // dirs form the next level. No mixed-depth subroot sets, so no overlap
    // or gap hazards. Skipped/ignored dirs prune their whole subtree.
    //
    // CPU budget (user requirement: never >3-4% sustained): BFS levels are
    // short bursts; walker parallelism is capped (default 4 workers, ~40ms
    // per distinct structural pattern on an M5 Max repo corpus). Sustained
    // duty remains <1% of machine capacity under continuous load. Operators
    // on constrained hosts can lower ASGREP_WALK_THREADS (1-2); power users
    // can raise it for faster cold walks.
    let walk_workers = std::env::var("ASGREP_WALK_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(4);
    let walk_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(walk_workers)
        .build()
        .map_err(|error| crate::StoreError::Other(format!("failed to build walk pool: {error}")))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut frontier: Vec<std::sync::Arc<Path>> =
        vec![std::sync::Arc::from(root.clone().into_boxed_path())];
    while !frontier.is_empty() {
        let collected: Vec<(Vec<PathBuf>, Vec<std::sync::Arc<Path>>)> = walk_pool.install(|| {
            frontier
                .par_iter()
                .map(|dir| {
                    let thread_ignore = crate::gitignore::IgnoreMatcher::new(&root);
                    expand_dir(&thread_ignore, &root, dir)
                })
                .collect::<Vec<_>>()
        });
        let mut next: Vec<std::sync::Arc<Path>> = Vec::new();
        for (mut files, children) in collected {
            paths.append(&mut files);
            next.extend(children);
        }
        frontier = next;
    }
    let walk_ns = walk_started.elapsed().as_nanos();
    // PASS 60 (H-CONF-029): census the corpus languages (post lang-filter)
    // with NO native template for this pattern. Computed from the path set
    // BEFORE the byte prefilter runs so prefiltered-away files cannot hide
    // unanswerability behind a silent empty result. One unanswerable corpus
    // language means a query over it can never be answered natively (sg
    // rejects the same shape); the caller turns this into the loud
    // fail-closed error when the query answers empty.
    let unanswerable_corpus_languages = {
        let mut unanswerable: Vec<ast_sgrep_lang::Language> = Vec::new();
        for path in paths.iter() {
            // PASS 65 (LOW d): extension-only detection misses content-only
            // languages — an extension-less file carrying a shebang (`pybox`)
            // was invisible to this census, so the unanswerable-language gate
            // stayed silent exactly where the per-file skip below guaranteed
            // a silent empty (H-CONF-029 class fail-open). When the path
            // alone cannot classify, detect from a capped content read.
            let mut lang = ast_sgrep_lang::detect_language(path, None);
            if lang.is_none() {
                if let Some(bytes) = read_pattern_bytes_capped(path) {
                    if let Ok(content) = std::str::from_utf8(&bytes) {
                        lang = ast_sgrep_lang::detect_language(path, Some(content));
                    }
                }
            }
            let Some(lang) = lang else {
                continue;
            };
            if lang_filter.is_some_and(|filter| lang.as_str() != filter) {
                continue;
            }
            if !unanswerable.contains(&lang)
                && !ast_sgrep_lang::native_pattern_answerable(lang, pattern)
            {
                unanswerable.push(lang);
            }
        }
        unanswerable.len()
    };
    let required_literal = use_prefilter
        .then(|| required_pattern_literal(pattern))
        .flatten();
    let parallel_started = Instant::now();
    let results = paths
        .par_iter()
        .map(|path| {
            let prefilter_started = Instant::now();
            if let Some(allowed) = &candidate_paths {
                let rel_ok = path
                    .strip_prefix(&root)
                    .map(|rel| allowed.contains(&rel.to_string_lossy().replace('\\', "/")))
                    .unwrap_or(false);
                if !rel_ok {
                    return NativeFileResult::default();
                }
            }
            let Some(bytes) = read_pattern_bytes_capped(path) else {
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
            // H-CONF-031 (pass 63): language-aware native walk. A file whose
            // language has no native template for this pattern is SKIPPED —
            // its non-matches are unanswerability, not evidence of absence —
            // exactly the per-file semantics sg applies when a pattern fails
            // its per-language pattern gate (no-lang `throw $A` skips the
            // unparseable languages and answers 6 hits). The census above
            // records the skip so the caller can fail closed when the WHOLE
            // query answers empty (sg exits 8 on the same --lang-pinned
            // inputs); without the skip, classifier-accepted shapes whose
            // grammar cannot parse them still over-matched here
            // (`function $A($B) { $$$C }` on python answered 2 phantom hits
            // where sg exits 8).
            if !ast_sgrep_lang::native_pattern_answerable(lang, pattern) {
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
                prefiltered: false,
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
        unanswerable_corpus_languages,
    })
}

/// Timed `try_wait` loop shared by the optional ast-grep version probe and bench runner.
/// Returns `Some(())` when the child exits (and succeeds if `require_success`), else kills and returns `None`.
fn wait_child_deadline(child: &mut Child, deadline: Instant, require_success: bool) -> Option<()> {
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
///
/// GA-24 (H-SURF-005, pass 19): the former `run_external_ast_grep` delegation
/// result path was removed — zero callers, and delegating search hits to the
/// reference binary would make differential parity circular and import
/// unpinned version behavior into the trust boundary. Search never delegates;
/// this gate serves the bench comparison lane only (`bench_ast_grep`).
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
