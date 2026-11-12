use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::io_bounds::MAX_INDEX_FILE_BYTES;
use crate::pattern_comment_syntax::{
    comment_placement_template_route_refused, literal_trailing_lane_verdict, PatternCommentScans,
};
use crate::rank::SCORE_PATTERN;
use crate::search::{HitKind, SearchHit, SpanHitInput};
use crate::Result;
use ast_sgrep_lang::{
    detect_language, match_pattern, needs_ast_grep_fallback, required_pattern_literal,
};
use rayon::prelude::*;
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub use crate::pattern_bench::bench_ast_grep;

#[path = "native_walk.rs"]
mod native_walk;
use self::native_walk::search_pattern_native_profiled;

#[path = "rename_scan.rs"]
mod rename_scan;
use self::rename_scan::rename_colliding_callee_rest;

#[path = "lane_guards.rs"]
mod lane_guards;
use self::lane_guards::{
    depth_truncation_disagreement_check, native_walk_candidate_narrowed, serve_cached_exact_lane,
    structural_fallback_error, ExactLaneEligibility,
};

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
    /// Number of DISTINCT corpus languages (post lang-filter) with NO
    /// native template for this pattern. Censused from the path set BEFORE
    /// the byte prefilter runs, so prefiltered-away files cannot hide
    /// unanswerability behind silent empty results.
    unanswerable_corpus_languages: usize,
}

/// The reference rewrites the per-language expando char (`µ` U+00B5 for the
/// csharp/go/kotlin/php/python/ruby/rust/swift family, U+10000 for c/cpp) to
/// `$` BEFORE parsing, so an expando-spelled run is a metavariable, not
/// identifier bytes. The indexed lane and the byte prefilter below key on RAW
/// pattern bytes: without this guard the index serves literal `µA`/`µµA`
/// identifier rows while the meta reading answers every identifier, and the
/// prefilter keeps only files containing the literal `µA` bytes, dropping
/// µ-free files the meta reading answers. Neither lane decides hits — the
/// native walk normalizes per file language inside `match_pattern` — so a
/// coarse containment test is the sound guard: over-broad matches only
/// disable the fast lanes (a perf cost on the rare literal-µ java/js/ts
/// identifier, which have no expando), never results. Crate-visible so the
/// codemod lane's raw-byte `required_pattern_literal` prefilter carries the
/// same guard as the two search lanes below.
pub(crate) fn pattern_may_carry_expando_meta(pattern: &str) -> bool {
    pattern.contains('µ') || pattern.contains('\u{10000}')
}

/// Strip a leading BOM and surrounding whitespace — the single ingress
/// normalization shared by the search lane and the codemod lane (O4).
/// Returns a borrowed slice; empty means "refuse loudly at the caller".
pub(crate) fn normalize_pattern_text(pattern: &str) -> &str {
    pattern.trim().trim_start_matches('\u{feff}').trim()
}

/// Byte-prefilter literal for `pattern`, or `None` when no literal may
/// narrow the walk (O4). An expando-spelled metavariable normalizes to `$`
/// per file language inside the walk, so a raw-byte required literal (`µA`)
/// would prefilter away µ-free files the meta reading answers. Over-broad
/// matches only disable the fast prefilter, never results.
pub(crate) fn required_literal_for(pattern: &str) -> Option<String> {
    if pattern_may_carry_expando_meta(pattern) {
        None
    } else {
        required_pattern_literal(pattern)
    }
}

pub fn search_pattern(
    pattern: &str,
    store: &crate::store::IndexStore,
    root: &Path,
    lang_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    search_pattern_with_config(
        pattern,
        store,
        root,
        lang_filter,
        limit,
        &crate::config::CoreConfig::from_env(),
    )
}

/// [`search_pattern`] with an explicitly resolved
/// [`crate::config::CoreConfig`]. [`Searcher`](crate::search::Searcher)
/// passes its construction-resolved config; the plain entry point resolves
/// `CoreConfig::from_env()` here so behavior is identical either way.
pub fn search_pattern_with_config(
    pattern: &str,
    store: &crate::store::IndexStore,
    root: &Path,
    lang_filter: Option<&str>,
    limit: usize,
    config: &crate::config::CoreConfig,
) -> Result<Vec<SearchHit>> {
    // Union index signatures with native tree-sitter matches.
    // Production does not spawn external ast-grep by default; native-only is the
    // honest completeness path when the index is partial.
    // Strip a leading BOM BEFORE any lane sees the pattern, exactly as
    // match_pattern's R3 strip does. The raw U+FEFF is not whitespace for
    // `str::trim`, so without this strip it rides into
    // `required_pattern_literal` (`\ufeffgreet`) and the byte prefilter drops
    // every file that does not literally start with a BOM-prefixed callee —
    // the index-served lane answered silent ok:true-0 where the reference
    // answers through the BOM (BOM-led `greet($A)` python).
    let pattern = normalize_pattern_text(pattern);
    // The BOM strip runs BEFORE this guard, so a BOM-only pattern cannot
    // ride past it as a non-empty string and then degrade to a silent
    // ok:true-empty answer for the vacuous "" pattern. The codemod lane
    // refuses the same input loudly at its ingress; search now refuses it in
    // the same operational class as the structural-ingress rejection below
    // (the reference exits nonzero on an empty pattern too).
    if pattern.is_empty() {
        return Err(crate::StoreError::Other("pattern must not be empty".into()));
    }
    let canonical = ast_sgrep_lang::Language::canonical_filter(lang_filter);
    let lang_filter = canonical.as_deref();
    // Loud pattern-ingress. A `$`-pattern the native classifier rejects
    // can never be answered natively; reject it BEFORE scanning instead of
    // letting match_pattern's per-file empty results compose into a silent
    // `ok:true` empty envelope. Classifier-accepted patterns never take
    // this arm, so their zero-hit results stay ok:true. ONE class of
    // reference-ANSWERABLE faces ingress-rejects before any per-language
    // gate runs: the classifier gives up on comment trivia inside
    // argument/metadata slots (`classify_native` returns None for
    // `calc($A, /* n */ $B)` — its argument validator refuses the `/*` doc
    // — and the general lane refuses it too), so `needs_ast_grep_fallback`
    // rejects faces the reference answers. When the pattern carries comment
    // syntax AND at least one language's placement/content gate accepts the
    // face, route it to the native walk instead. Fail-closed safety is
    // preserved by construction: the post-walk backstop below keys on
    // `needs_ast_grep_fallback` — still true for every face exempted here —
    // so an exempted face either answers (reference-agreeing) or rejects;
    // it can never compose into a silent empty. Comment-free refusals and
    // every reference-refused placement keep the ingress contract
    // byte-identical.
    // Blind + per-language comment scans, computed ONCE per entry and
    // threaded through the ingress gate, the exact lane, and the native
    // walk below (O3). Values are byte-identical to the old per-gate
    // rescans — only the repetition is gone.
    let comment_scans = PatternCommentScans::compute(pattern);
    if needs_ast_grep_fallback(pattern)
        && !comment_carrying_face_gate_accepted(pattern, &comment_scans)
    {
        return Err(structural_fallback_error(pattern, config));
    }
    // De-collide rest/single capture names (see
    // rename_colliding_callee_rest). Runs AFTER the ingress gate so a
    // classifier-rejected spelling keeps its loud fail-closed class
    // untouched, and BEFORE every lane below so the index-signature route,
    // the candidate kind narrowing, the unanswerable-language census, and
    // the native matcher all see one consistent spelling. The original
    // spelling stays on the fail-closed backstop check. Hit `symbol` fields
    // carry the de-collided spelling on the affected faces — the faces
    // answered silent-empty before, so no previously-emitted symbol changes.
    let match_spelling = rename_colliding_callee_rest(pattern);
    // Per-language walk-answerability, computed once at ingress for the
    // exact spelling the walk gates on (d-NEEDLESS hoist 1): the walk
    // consults this table instead of re-deriving classification verdicts
    // per file. Values match the old per-file closures byte-for-byte.
    let comment_scans = comment_scans.with_answerability(&match_spelling);
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut unanswerable_corpus_languages = 0usize;
    // The exact-signature early-return must run ONLY for comment-free
    // patterns: an exact index signature is a TEXT row and cannot express
    // the comment-placement gates the walk adjudicates — and it cannot
    // express the paren-token contract either (the bare `q($$$A)`
    // signature rows served the ruby/swift paren-less over-answer where
    // the reference answers []). `$$$`-rest spellings are likewise
    // walk-served: the rest-slot semantics and the paren gate are matcher
    // decisions, never index rows.
    let comment_free = comment_scans.comment_free();
    // Depth-budget-truncated files hold incomplete pattern_nodes rows, so
    // the "index is complete" premise below is false whenever one is in
    // scope. Skip the exact-serve early-return AND the candidate narrowing:
    // both would silently drop the truncated files' hits; the native walk
    // answers them instead.
    //
    // No index at all (missing db file, root still present): the pattern
    // lane falls through to the native walk, which needs no index — native
    // structural search works on an unindexed tree by construction (P1).
    // Every other lane (hybrid, lexical, semantic) keeps failing closed
    // with EmptyIndex, as do Searcher construction, the CLI pre-gate
    // (NL-002), and corrupt (non-missing) stores, which surface Database.
    let has_pattern_rows = match store.pattern_node_count() {
        Ok(n) => n > 0,
        Err(e) if e.is_empty_index() && root.is_dir() => false,
        Err(e) => return Err(e.into()),
    };
    let depth_truncated_in_scope =
        has_pattern_rows && store.has_depth_truncated_files(lang_filter)?;
    // Exact-lane preconditions, snapshotted once per entry (S2): the lane
    // re-checks nothing the caller already knows.
    let eligibility = ExactLaneEligibility::for_spellings(
        &match_spelling,
        pattern,
        comment_free,
        has_pattern_rows,
        depth_truncated_in_scope,
    );
    if let Some(early) =
        serve_cached_exact_lane(store, &match_spelling, lang_filter, limit, &eligibility)?
    {
        return Ok(early);
    }
    let mut native_accepted = false;
    // PASS 131 (130A-F4): the same-line dedup key carries the hit's
    // matched-node byte span. Two INDEPENDENT same-line nodes (`q(1);
    // q(2);` — disjoint spans) are two sg rows and must both survive at
    // this layer (f131, sg receipt); the SAME node found by overlapping
    // query arms (equal spans) still collapses to one. Containment
    // narrowing (outermost-wins, §41.3/F66a-7) and one-row-per-line
    // presentation live downstream (RRF line-fusion on the hybrid path),
    // not in this union: line-keying here broke f131.
    if let Some((native_hits, unanswerable)) = native_walk_candidate_narrowed(
        store,
        &match_spelling,
        root,
        lang_filter,
        has_pattern_rows,
        depth_truncated_in_scope,
        config,
        &comment_scans,
    )? {
        for hit in native_hits {
            if seen.insert((
                hit.file.clone(),
                hit.line_start,
                hit.line_end,
                hit.byte_span,
            )) {
                hits.push(hit);
            }
        }
        unanswerable_corpus_languages = unanswerable;
        native_accepted = true;
    }
    if native_accepted
        && hits.is_empty()
        && (unanswerable_corpus_languages > 0 || needs_ast_grep_fallback(pattern))
    {
        // Fail-closed: a beyond-native shape NEVER returns a silent empty
        // result. The delegation result path was REMOVED, not pending — the
        // external engine is bench-only (`bench_ast_grep`), so the gate-on
        // state fails closed loudly too, permanently. The same rejection now
        // happens at ingress above; this stays as a defense-in-depth
        // backstop on the post-walk path. The backstop ALSO fires when the
        // corpus holds languages with no native template for the pattern —
        // per-file `Ok(empty)` there is unanswerability, not source
        // robustness, and must stay loud.
        return Err(structural_fallback_error(pattern, config));
    }
    // Post-walk honesty check (see depth_truncation_disagreement_check).
    depth_truncation_disagreement_check(
        store,
        &match_spelling,
        lang_filter,
        limit,
        depth_truncated_in_scope,
        hits.is_empty(),
    )?;
    Ok(hits)
}

/// Whether the pattern carries comment syntax AND at least one language's
/// placement/content gate accepts the face — the ingress-exemption class
/// documented at the `needs_ast_grep_fallback` call in [`search_pattern`].
/// Comment-free patterns are never exempt; placements the reference refuses
/// (leading/trailing/line blocks, ruby inline, php assignment-tail
/// comments) are accepted by NO language and keep the ingress rejection.
/// Scans arrive precomputed from the entry (O3); values are byte-identical
/// to the old per-gate rescans.
fn comment_carrying_face_gate_accepted(pattern: &str, scans: &PatternCommentScans) -> bool {
    if scans.comment_free() {
        return false;
    }
    ast_sgrep_lang::Language::all().iter().any(|lang| {
        let scan = scans.for_lang(*lang);
        !comment_placement_template_route_refused(
            *lang,
            pattern,
            scan,
            literal_trailing_lane_verdict(*lang, pattern, scan),
        )
    })
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
                // Index-lane rows carry no byte span (pattern_nodes has no
                // byte columns) — the line-keyed dedup above stays.
                byte_span: None,
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
    let config = crate::config::CoreConfig::from_env();
    let single_worker = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .map_err(|error| {
            crate::StoreError::Other(format!("failed to build pattern profiling pool: {error}"))
        })?;
    // One scan set for all three walks (O3): same pattern, same values.
    // Answerability is filled for the same walk spelling the walk gates
    // on (d-NEEDLESS hoist 1).
    let scans = PatternCommentScans::compute(pattern).with_answerability(pattern);
    let baseline = single_worker
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, false, None, &config, &scans))?;
    let serial = single_worker
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, true, None, &config, &scans))?;
    let parallel = search_pattern_native_profiled(pattern, root, lang_filter, true, None, &config, &scans)?;
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
/// rayon walk cannot `fs::read` an unbounded blob into RAM.
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

/// File admission for the native pattern walk (BFS `expand_dir`).
///
/// [`crate::gitignore::should_skip_file`] answers `unwrap_or(true)` for an
/// extension-less path — the indexer's admission contract. The pattern walk
/// must NOT inherit that arm: both the unanswerable-language census and the
/// per-file route classify by CONTENT (`detect_language(path,
/// Some(content))` shebang arm), so an extension-less shebang carrier
/// (`pybox`) dropped here never reaches either — the census's capped content
/// read was dead code for exactly the files it names, and a rust-spelled
/// template over a python-by-content corpus composed a silent `ok:true`-empty
/// instead of the registered census-loud refusal.
/// Extension-less files therefore ride to the walk: one capped read of cost,
/// and files whose content names no language still answer nothing per-file.
/// Dotfiles and non-indexable EXTENSIONS keep the registered skip.
fn pattern_walk_admits(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
    {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(_) => !should_skip_file(path),
        None => true,
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
            if !pattern_walk_admits(&path) {
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
