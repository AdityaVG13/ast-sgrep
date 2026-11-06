//! Bundle B native-walk search engine, split verbatim from `pattern.rs`.
//! Owns the profiled walk and its per-file helpers; lanes, normalization,
//! and the `Native*` result types stay in `pattern.rs` (reached via `super`).

use super::{
    expand_dir, pattern_may_carry_expando_meta, read_pattern_bytes_capped, NativeFileResult,
    NativeSearchOutput, PatternSearchProfile,
};
use crate::pattern_comment_syntax::{
    comment_placement_template_route_refused, never_matches_comment_placement_sg_accepted,
    php_leading_attribute_function_face, scan_pattern_comment_syntax,
    scan_pattern_comment_syntax_for,
};
use crate::rank::SCORE_PATTERN;
use crate::search::{HitKind, SearchHit, SpanHitInput};
use crate::Result;
use ast_sgrep_lang::{detect_language, match_pattern, required_pattern_literal};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub(crate) fn search_pattern_native_profiled(
    pattern: &str,
    root: &Path,
    lang_filter: Option<&str>,
    use_prefilter: bool,
    candidate_paths: Option<std::collections::HashSet<String>>,
    config: &crate::config::CoreConfig,
) -> Result<NativeSearchOutput> {
    let canonical = ast_sgrep_lang::Language::canonical_filter(lang_filter);
    let lang_filter = canonical.as_deref();
    let total_started = Instant::now();
    let root = crate::paths::canonicalize_path(root, true).unwrap_or_else(|_| root.to_path_buf());
    let (paths, walk_ns) = bfs_corpus_paths(&root, config)?;
    // NeverMatches-classified patterns cannot phantom-hit —
    // match_pattern routes them to the structural arm that answers empty, so
    // the only arms that can answer are the registered reference-exact
    // php-hook and dollar-literal lanes. The per-file answerable skip and
    // this census are keyed on `native_pattern_answerable`, which cannot see
    // those lanes (for a NeverMatches classify its only false arm is the
    // pattern-side comment refusal), so comment-carrying hook faces were
    // skipped in every file, the census counted every corpus language
    // unanswerable, and the backstop composed the skips into the loud
    // structural-fallback error where the reference answers. For exactly
    // this class the matcher's own per-file answer decides: hits = hits, an
    // empty is an honest empty (the census must not count an
    // unanswerability the walk does not have). Every other class keeps its
    // registered contract byte-for-byte: classifier-accepted decl/call
    // templates keep the skip + census, ingress-rejected garbage keeps the
    // loud refusal, and `$`-less patterns never classify NeverMatches.
    // That blanket exemption was reference-exact only for the comment
    // placements the reference accepts — a comment-free NeverMatches face
    // (walk-decided) or an INLINE `/* */` block face in php (the hook
    // family). The probed reference refuses every other comment placement
    // — glued `#`/`//` line comments (py/rb/php `$x # note`, php/rust
    // `$x // note`) and trailing `/* */` after a statement in every probed
    // language — and the blanket exemption had silenced those
    // reference-agreed-loud faces into ok:true []. Those faces fall back to
    // the census path below byte-for-byte, restoring the loudness;
    // `matcher_decides` becomes per-language because the accepted block
    // placement is language-scoped (php).
    // The accepted block placement narrows further — a LEADING `/* */`
    // (`block_leading`) is refused in every probed language (it spawns a
    // second top-level node) and leaves the exemption in ALL languages,
    // closing the silenced php leading-block faces.
    let never_matches = matches!(
        ast_sgrep_lang::classify_native(pattern),
        Some(ast_sgrep_lang::NativeKind::NeverMatches)
    );
    // `search_pattern` computes the same scan for its early-return gate;
    // this native-walk entry keeps its own (it has other callers, and the
    // scan is one linear pass over the pattern text).
    let comment_scan = scan_pattern_comment_syntax(pattern);
    let comment_free = !(comment_scan.line || comment_scan.block || comment_scan.hash);
    // The NeverMatches exemption must ALSO defer to the lang-side
    // answerability on the COMMENT-FREE class. A two-statement
    // brace-compound spelling (`1_000 ($A) { $_0x1F }` — `)` followed by
    // `{`) is reference-refused ("Multiple AST nodes are detected") in
    // seven languages, but the comment-free NeverMatches placement was
    // exempt and the walk answered a silent empty. That consult is kept
    // byte-identical for the comment-free class.
    // The COMMENT-CARRYING half of the disjunct was a blanket `true`
    // (`!comment_free`) — it exempted every inline-block NeverMatches face
    // from the per-file skip and the census even when the walk arm cannot
    // decide the face. The php member-call slot grammar refuses comment
    // trivia in an argument slot (`$obj->m(/* m */ $A)`), so the structural
    // NeverMatches arm answered empty for every file and the face composed
    // into a SILENT ok:true [] where the reference answers the row. The
    // blanket is replaced by the exact set of walk-decidable
    // comment-carrying faces — the php comment-transparent lanes: the
    // assignment-hook and operand-template lanes decide comments by AST
    // structure, so they keep answering; every other comment-carrying
    // NeverMatches face falls back to the census path, which the backstop
    // composes into the loud fail-closed class (fail-open -> fail-closed).
    let matcher_decides = |lang: ast_sgrep_lang::Language| -> bool {
        never_matches
            && never_matches_comment_placement_sg_accepted(lang, comment_scan)
            && (comment_free && ast_sgrep_lang::native_pattern_answerable(lang, pattern)
                || (!comment_free && ast_sgrep_lang::php_comment_transparent_operand_lane(pattern)))
    };
    // The template-exists route needs the same comment gate. A `$`-less
    // pattern short-circuits `native_pattern_answerable`'s own comment
    // refusal (the `!contains('$')` arm), so a comment placement the
    // reference refuses reached the walk through the general template and
    // answered a silent empty. The gate is scoped to NON-NeverMatches
    // patterns: NeverMatches faces keep their registered routing
    // byte-for-byte (matcher_decides above; the lang-side refusal inside
    // native_pattern_answerable).
    // The gate consumes the LANGUAGE-AWARE scan — ruby regex-literal `#`s
    // and php leading-attribute `#[` faces are not comments
    // (scan_pattern_comment_syntax_for) — plus the ts/js `#[` and php
    // assignment-tail comment refusals inside the gate itself.
    let template_comment_refused = |lang: ast_sgrep_lang::Language| -> bool {
        !never_matches
            && comment_placement_template_route_refused(
                lang,
                pattern,
                scan_pattern_comment_syntax_for(lang, pattern),
            )
    };
    // Single per-language answerability decision shared by the census and the
    // per-file skip so the two sites cannot drift: a language is unanswerable
    // when the NeverMatches exemption does not cover it AND (the comment gate
    // refuses the placement OR the lang-side gate finds no template).
    // The lang-side verdict gets ONE scoped override.
    // `native_pattern_answerable` refuses every `$`-carrying
    // comment-carrying pattern through its lane gate BEFORE any
    // classification — the lane is placement-unaware (every inline-block
    // placement the reference ANSWERS in the ten non-ruby/non-py
    // block-comment languages) and content-blind (php leading `#[`
    // attributes the reference answers). When the comment gate above did
    // NOT refuse the placement/content (the core adjudication accepted the
    // face) AND the pattern actually carries comment bytes (so the lane
    // refusal is the comment arm's verdict), the walk decides instead.
    // Everything else keeps its registered loud class: faces with no
    // comment bytes refuse for REAL non-comment reasons (classifier
    // rejects, the php meta-target veto `$A = $B`, NeverMatches
    // multi-root), and NeverMatches faces keep their routing byte-for-byte
    // (the `$x = /ab#{y}cd/` ruby spelling stays census-loud).
    let language_unanswerable = |lang: ast_sgrep_lang::Language| -> bool {
        if matcher_decides(lang) {
            return false;
        }
        if template_comment_refused(lang) {
            return true;
        }
        if !ast_sgrep_lang::native_pattern_answerable(lang, pattern) {
            // The lane refusal stands (language unanswerable) UNLESS the
            // override fires: non-NeverMatches AND the pattern actually
            // carries comment bytes — i.e. the refusal is the lane's
            // placement-unaware/content-blind comment verdict on a face the
            // core adjudication accepted above. Carve-out: a `$`-carrying
            // php attribute face keeps the refusal — the walk cannot bind
            // metas inside attribute argument lists (`#[Route($X)]` answers
            // walk-empty where the reference binds), so un-refusing would
            // trade the registered fail-closed loud for a silent empty; the
            // carve-out repairs exactly the `$`-less spellings through the
            // template route above.
            let php_attribute_carved = matches!(lang, ast_sgrep_lang::Language::Php)
                && comment_scan.hash_bracket
                && php_leading_attribute_function_face(pattern);
            let lane_refusal_overridden = !never_matches
                && !php_attribute_carved
                && (comment_scan.line || comment_scan.block || comment_scan.hash);
            !lane_refusal_overridden
        } else {
            false
        }
    };
    // Census unanswerable corpus languages (see
    // census_unanswerable_languages).
    let unanswerable_corpus_languages =
        census_unanswerable_languages(paths.iter(), lang_filter, &language_unanswerable);
    // An expando-spelled metavariable normalizes to `$` per file language
    // inside the walk, so a raw-byte required literal (`µA`) would
    // prefilter away µ-free files the meta reading answers.
    let required_literal = if use_prefilter && !pattern_may_carry_expando_meta(pattern) {
        required_pattern_literal(pattern)
    } else {
        None
    };
    let parallel_started = Instant::now();
    let file_ctx = FileMatchCtx {
        pattern,
        root: &root,
        lang_filter,
        candidate_paths: candidate_paths.as_ref(),
        required_literal: required_literal.as_ref(),
    };
    let results = paths
        .par_iter()
        .map(|path| native_match_file(&file_ctx, &language_unanswerable, path))
        .collect::<Vec<_>>();
    let parallel_span_ns = parallel_started.elapsed().as_nanos();
    let rank_started = Instant::now();
    let hits = merge_and_sort_hits(&results);
    let rank_ns = rank_started.elapsed().as_nanos();
    let (profile, max_file_work_ns) =
        assemble_profile(&results, paths.len(), walk_ns, parallel_span_ns, rank_ns);
    Ok(NativeSearchOutput {
        hits,
        profile,
        total_elapsed_ns: total_started.elapsed().as_nanos(),
        max_file_work_ns,
        unanswerable_corpus_languages,
    })
}

/// Per-file inputs shared by every `native_match_file` call in one walk.
struct FileMatchCtx<'a> {
    pattern: &'a str,
    root: &'a Path,
    lang_filter: Option<&'a str>,
    candidate_paths: Option<&'a std::collections::HashSet<String>>,
    required_literal: Option<&'a String>,
}

/// BFS the corpus root on the capped walk pool, claiming each file exactly
/// once. Returns the discovered file paths and the walk's wall nanoseconds.
///
/// Breadth-first traversal, one parallel level at a time. Each frontier
/// dir is expanded on a walk-pool worker with its own
/// IgnoreMatcher; files are claimed exactly once (each file has exactly
/// one parent dir, and each dir appears in exactly one frontier); child
/// dirs form the next level. No mixed-depth subroot sets, so no overlap
/// or gap hazards. Skipped/ignored dirs prune their whole subtree.
///
/// CPU budget (user requirement: never >3-4% sustained): BFS levels are
/// short bursts; walker parallelism is capped (default 4 workers, ~40ms
/// per distinct structural pattern on an M5 Max repo corpus). Sustained
/// duty remains <1% of machine capacity under continuous load. Operators
/// on constrained hosts can lower ASGREP_WALK_THREADS (1-2); power users
/// can raise it for faster cold walks.
fn bfs_corpus_paths(root: &Path, config: &crate::config::CoreConfig) -> Result<(Vec<PathBuf>, u128)> {
    let walk_started = Instant::now();
    let walk_workers = config.walk_threads;
    let walk_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(walk_workers)
        .build()
        .map_err(|error| crate::StoreError::Other(format!("failed to build walk pool: {error}")))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut frontier: Vec<std::sync::Arc<Path>> = vec![std::sync::Arc::from(root.to_path_buf())];
    let ignore = crate::gitignore::IgnoreMatcher::new(root);
    while !frontier.is_empty() {
        let collected: Vec<(Vec<PathBuf>, Vec<std::sync::Arc<Path>>)> = walk_pool.install(|| {
            frontier
                .par_iter()
                .map(|dir| expand_dir(&ignore, root, dir))
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
    Ok((paths, walk_ns))
}

/// Census the corpus languages (post lang-filter) with NO native template
/// for the pattern. Computed from the path set BEFORE the byte prefilter
/// runs so prefiltered-away files cannot hide unanswerability behind a
/// silent empty result. One unanswerable corpus language means a query over
/// it can never be answered natively (the reference rejects the same
/// shape); the caller turns this into the loud fail-closed error when the
/// query answers empty. NeverMatches-classified patterns are exempt where
/// the reference accepts the comment placement (matcher_decides): the walk
/// answers those files directly, so no language is unanswerable for them —
/// every other comment face keeps this census and the loud backstop.
/// "Every other comment face" includes the non-NeverMatches (`$`-less)
/// comment faces (template_comment_refused) — they previously dodged this
/// census because the `$`-less arm of native_pattern_answerable answers
/// before any comment refusal.
fn census_unanswerable_languages<'a>(
    paths: impl Iterator<Item = &'a PathBuf>,
    lang_filter: Option<&str>,
    language_unanswerable: &impl Fn(ast_sgrep_lang::Language) -> bool,
) -> usize {
    let mut unanswerable: Vec<ast_sgrep_lang::Language> = Vec::new();
    for path in paths {
        // Extension-only detection misses content-only languages — an
        // extension-less file carrying a shebang (`pybox`) was invisible
        // to this census, so the unanswerable-language gate stayed silent
        // exactly where the per-file skip below guaranteed a silent empty.
        // When the path alone cannot classify, detect from a capped
        // content read.
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
        if !unanswerable.contains(&lang) && language_unanswerable(lang) {
            unanswerable.push(lang);
        }
    }
    unanswerable.len()
}

/// One file's native match: candidate narrowing, byte prefilter, language
/// gates, the unanswerability skip, then `match_pattern` and hit
/// conversion. Each early return keeps its registered `NativeFileResult`
/// shape (which profile counters get set).
fn native_match_file(
    ctx: &FileMatchCtx<'_>,
    language_unanswerable: &impl Fn(ast_sgrep_lang::Language) -> bool,
    path: &Path,
) -> NativeFileResult {
    let prefilter_started = Instant::now();
    if let Some(allowed) = ctx.candidate_paths {
        let rel_ok = path
            .strip_prefix(ctx.root)
            .ok()
            .and_then(|rel| crate::index::indexed_rel_path(rel).ok())
            .map(|rel| allowed.contains(&rel))
            .unwrap_or(false);
        if !rel_ok {
            return NativeFileResult::default();
        }
    }
    let Some(bytes) = read_pattern_bytes_capped(path) else {
        return NativeFileResult::default();
    };
    let bytes_scanned = bytes.len() as u64;
    if ctx
        .required_literal
        .is_some_and(|literal| memchr::memmem::find(&bytes, literal.as_bytes()).is_none())
    {
        return NativeFileResult {
            bytes_scanned,
            prefiltered: true,
            prefilter_ns: prefilter_started.elapsed().as_nanos(),
            ..NativeFileResult::default()
        };
    }
    let bail = || NativeFileResult {
        bytes_scanned,
        prefilter_ns: prefilter_started.elapsed().as_nanos(),
        ..NativeFileResult::default()
    };
    let Ok(content) = std::str::from_utf8(&bytes) else {
        return bail();
    };
    let Some(lang) = detect_language(path, Some(content)) else {
        return bail();
    };
    if ctx
        .lang_filter
        .is_some_and(|filter| lang.as_str() != filter)
    {
        return bail();
    }
    // Language-aware native walk. A file whose language has no native
    // template for this pattern is SKIPPED — its non-matches are
    // unanswerability, not evidence of absence — exactly the per-file
    // semantics the reference applies when a pattern fails its
    // per-language pattern gate (no-lang `throw $A` skips the unparseable
    // languages and answers 6 hits). The census above records the skip so
    // the caller can fail closed when the WHOLE query answers empty (the
    // reference exits 8 on the same --lang-pinned inputs); without the
    // skip, classifier-accepted shapes whose grammar cannot parse them
    // still over-matched here (`function $A($B) { $$$C }` on python
    // answered 2 phantom hits where the reference exits 8).
    // NeverMatches-classified patterns are exempt (matcher_decides):
    // their structural arm answers empty, so the phantom-hit hazard does
    // not exist and match_pattern's own per-file answer is the honest one
    // (the php hook / dollar-literal lanes answer their registered
    // reference-exact faces here).
    // The exemption is per-language and covers only the reference-accepted
    // comment placements (comment-free, or inline `/* */` in php) — glued
    // line-comment and trailing-block faces fall back to this skip + the
    // census, loud.
    // Non-NeverMatches comment faces join the same discipline
    // (language_unanswerable): a `$`-less pattern short-circuits
    // native_pattern_answerable's own refusal, so the placements the
    // reference refuses must skip the file here instead of walking into a
    // silent empty.
    // The universal-root faces keep their REGISTERED line-collapsed rows —
    // per-file detection (the file language decides the expando
    // normalization), spans suppressed at the conversion site below.
    let universal_root = ast_sgrep_lang::is_universal_root_pattern(lang, ctx.pattern);
    if language_unanswerable(lang) {
        return bail();
    }
    let prefilter_ns = prefilter_started.elapsed().as_nanos();
    let parse_match_started = Instant::now();
    let Some(rel) = path
        .strip_prefix(ctx.root)
        .ok()
        .and_then(|path| crate::index::indexed_rel_path(path).ok())
    else {
        return bail();
    };
    let hits = match_pattern(lang, content, ctx.pattern)
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
                symbol: Some(ctx.pattern.to_string()),
                language: Some(lang.as_str().to_string()),
                // The native walk knows the matched-node byte span; it
                // keys the same-line dedup at the union site and the
                // finish-layer DedupKey. EXCEPTION: the universal-root
                // lane (`$$A`) keeps its REGISTERED line-collapsed rows
                // (per-node multiplicity there would starve the finish
                // keep-truncate), so its spans stay suppressed.
                byte_span: if universal_root {
                    None
                } else {
                    Some((matched.byte_start, matched.byte_end))
                },
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
    }
}

/// Union per-file hits in file/line order (the native lane's registered
/// deterministic order).
fn merge_and_sort_hits(results: &[NativeFileResult]) -> Vec<SearchHit> {
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
    hits
}

/// Roll the per-file results into the Brent-decomposed parallel profile.
/// Returns the profile and `max_file_work_ns` (the t_inf critical-path
/// term the caller reports alongside it).
fn assemble_profile(
    results: &[NativeFileResult],
    files_considered: usize,
    walk_ns: u128,
    parallel_span_ns: u128,
    rank_ns: u128,
) -> (PatternSearchProfile, u128) {
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
        files_considered,
        files_prefiltered: results.iter().filter(|result| result.prefiltered).count(),
        files_parsed: results.iter().filter(|result| result.parsed).count(),
        bytes_scanned: results.iter().map(|result| result.bytes_scanned).sum(),
        hits: results.iter().map(|result| result.hits.len()).sum(),
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
    (profile, max_file_work_ns)
}
