use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::io_bounds::MAX_INDEX_FILE_BYTES;
use crate::pattern_comment_syntax::{
    comment_placement_template_route_refused, never_matches_comment_placement_sg_accepted,
    php_leading_attribute_function_face, scan_pattern_comment_syntax,
    scan_pattern_comment_syntax_for,
};
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
use std::time::Instant;

pub use crate::pattern_bench::bench_ast_grep;
use crate::pattern_bench::find_ast_grep_binary;

#[path = "native_walk.rs"]
mod native_walk;
use self::native_walk::search_pattern_native_profiled;

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
    let pattern = pattern.trim().trim_start_matches('\u{feff}').trim();
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
    if needs_ast_grep_fallback(pattern) && !comment_carrying_face_gate_accepted(pattern) {
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
    let mut hits = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut unanswerable_corpus_languages = 0usize;
    // The comment scan is computed ONCE here and reused by the
    // matcher_decides chain below (net-zero below this point; the
    // early-return hot path pays one linear scan over the tiny pattern
    // text). The exact-signature early-return must run ONLY for
    // comment-free patterns: an exact index signature is a TEXT row and
    // cannot express the comment-placement gates the walk adjudicates —
    // and it cannot express the paren-token contract either (the bare
    // `q($$$A)` signature rows served the ruby/swift paren-less
    // over-answer where the reference answers []). `$$$`-rest spellings
    // are likewise walk-served: the rest-slot semantics and the paren gate
    // are matcher decisions, never index rows.
    let comment_scan = scan_pattern_comment_syntax(pattern);
    let comment_free = !(comment_scan.line || comment_scan.block || comment_scan.hash);
    // Depth-budget-truncated files hold incomplete pattern_nodes rows, so
    // the "index is complete" premise below is false whenever one is in
    // scope. Skip the exact-serve early-return AND the candidate narrowing:
    // both would silently drop the truncated files' hits; the native walk
    // answers them instead.
    let has_pattern_rows = store.pattern_node_count()? > 0;
    let depth_truncated_in_scope =
        has_pattern_rows && store.has_depth_truncated_files(lang_filter)?;
    if let Some(early) = serve_cached_exact_lane(
        store,
        &match_spelling,
        pattern,
        lang_filter,
        limit,
        comment_free,
        has_pattern_rows,
        depth_truncated_in_scope,
    )? {
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

/// The structured fail-closed error for patterns the native classifier
/// rejects. The message names the permanent non-delegation decision:
/// external ast-grep is bench-only, so no gate state ever answers
/// beyond-native patterns.
fn structural_fallback_error(
    pattern: &str,
    config: &crate::config::CoreConfig,
) -> crate::StoreError {
    let state = if config.external_ast_grep_enabled && find_ast_grep_binary(config).is_some() {
        "external ast-grep is configured but is bench-only; search never delegates"
    } else {
        "structural fallback is disabled or unavailable"
    };
    crate::StoreError::Other(format!(
        "pattern requires structural fallback ({state}; fail-closed): {pattern} — working spellings: 'fn $NAME($$$)' (every fn-shaped item), a no-parens name ('fn plain_name' filters by name), or 'defs:NAME' for the definition site; rust bodyless concrete names answer trait signatures only"
    ))
}

/// The exact-signature index-serve early returns of [`search_pattern`].
/// Returns `Ok(Some(hits))` when a cached lane is authoritative for the
/// pattern and the search is answered there; `Ok(None)` means the native
/// walk (or a refusal) decides.
///
/// Decl signatures are budget-exempt at extraction (a beyond-budget node
/// still records its own decl row; only recursion stops), so a pattern whose
/// signatures are ALL decl: has COMPLETE rows even with depth-truncated
/// files in scope — serve the cached lane instead of refusing. This branch
/// is hoisted ABOVE the truncation skip: it exists precisely for the
/// truncated scope. call:/kind:/ident rows stay incomplete past the budget
/// and keep the walk/refusal paths below. Comment placement remains a
/// matcher decision, so comment-carrying spellings stay excluded.
// Stable lane shape: query context is threaded, not bundled; bundling would
// churn every pattern lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
fn serve_cached_exact_lane(
    store: &crate::store::IndexStore,
    match_spelling: &str,
    pattern: &str,
    lang_filter: Option<&str>,
    limit: usize,
    comment_free: bool,
    has_pattern_rows: bool,
    depth_truncated_in_scope: bool,
) -> Result<Option<Vec<SearchHit>>> {
    let Some(signatures) = cached_pattern_signatures(match_spelling) else {
        return Ok(None);
    };
    let decl_exact = !signatures.is_empty() && signatures.iter().all(|s| s.starts_with("decl:"));
    if depth_truncated_in_scope && comment_free && decl_exact {
        let indexed =
            search_pattern_cached(match_spelling, &signatures, store, lang_filter, limit)?;
        return Ok(Some(indexed));
    }
    if has_pattern_rows
        && !depth_truncated_in_scope
        && comment_free
        && !match_spelling.contains("$$$")
        && !pattern_may_carry_expando_meta(pattern)
        // Keyword-literal roots (`null`/`true`/`false`, py
        // `None`/`True`/`False`, `this`/`super`) have NO pattern_nodes
        // identifier rows, so the "ident-exact" index serve below composed
        // a silent empty where the reference answers the keyword leaf node.
        // Route the class to the native walk, which answers exactly; every
        // other ident signature keeps the index-served lane.
        && !ast_sgrep_lang::pattern_is_keyword_literal_root(match_spelling)
    {
        let indexed =
            search_pattern_cached(match_spelling, &signatures, store, lang_filter, limit)?;
        // Exact ident / decl / call signatures are complete in pattern_nodes.
        // Re-walking the tree cannot add a hit the index missed.
        if index_can_serve_pattern(match_spelling, &signatures) {
            return Ok(Some(indexed));
        }
        // Kind-only signatures are inexact (every `function_definition`
        // — any arity, any return type). Unioning those rows into the
        // result set let a zero-param `def main():` match a one-param
        // template. The native walk below decides every hit, and its
        // candidate narrowing (`candidate_kind_signatures`) covers exactly
        // the files that can hold a match, so inexact rows are dropped,
        // never merged.
    }
    Ok(None)
}

/// The native-walk leg of [`search_pattern`]: kind-candidate narrowing
/// (skipped while depth-truncated files are in scope) followed by the
/// profiled walk. A walk ERROR is not propagated — it composes into
/// `Ok(None)` so the caller's fail-closed backstop decides, exactly as the
/// historical `Err(_) => false` arm; candidate SQL errors DO propagate.
///
/// Narrow the native walk to files holding a node of the pattern's kind
/// when the exact shape is not indexable. Sound: files without such a node
/// cannot contain a match; the native matcher still decides every hit on
/// surviving files.
fn native_walk_candidate_narrowed(
    store: &crate::store::IndexStore,
    match_spelling: &str,
    root: &Path,
    lang_filter: Option<&str>,
    has_pattern_rows: bool,
    depth_truncated_in_scope: bool,
    config: &crate::config::CoreConfig,
) -> Result<Option<(Vec<SearchHit>, usize)>> {
    let candidate_paths = match ast_sgrep_lang::candidate_kind_signatures(match_spelling) {
        Some(kinds) if has_pattern_rows && !depth_truncated_in_scope => {
            Some(store.pattern_node_candidate_paths(&kinds, lang_filter)?)
        }
        _ => None,
    };
    Ok(
        match search_pattern_native_profiled(
            match_spelling,
            root,
            lang_filter,
            true,
            candidate_paths,
            config,
        ) {
            Ok(native) => Some((native.hits, native.unanswerable_corpus_languages)),
            Err(_) => None,
        },
    )
}

/// Post-walk honesty check: with a depth-truncated file in scope the
/// cached lane must not serve, so the walk is the only answerer. When THAT
/// answers empty for a shape whose signature rows demonstrably exist in
/// untruncated files, the lanes disagree — the walk did not decide a shape
/// the tree provably contains (partial templates parse but match nothing
/// natively) — refuse loudly instead of composing ok:true-empty. Rows
/// absent = possibly genuinely absent, so walk-empty stays an honest answer
/// there (residual: occurrences entirely inside truncated files are
/// unanswerable by any lane; registered in the conformance ledger).
fn depth_truncation_disagreement_check(
    store: &crate::store::IndexStore,
    match_spelling: &str,
    lang_filter: Option<&str>,
    limit: usize,
    depth_truncated_in_scope: bool,
    walk_answered_empty: bool,
) -> Result<()> {
    if !(depth_truncated_in_scope && walk_answered_empty) {
        return Ok(());
    }
    if let Some(signatures) = cached_pattern_signatures(match_spelling) {
        if index_can_serve_pattern(match_spelling, &signatures) {
            let indexed =
                search_pattern_cached(match_spelling, &signatures, store, lang_filter, limit)?;
            if !indexed.is_empty() {
                return Err(crate::StoreError::Other(format!(
                    "pattern {match_spelling:?} cannot be answered completely: \
                     depth-truncated file(s) in scope make the cached lane \
                     non-authoritative, and the native walk returned no hits \
                     for this cache-servable shape whose rows exist. Search \
                     the identifier form (pattern:<name>) or raise the \
                     extraction depth budget and re-index"
                )));
            }
        }
    }
    Ok(())
}

/// Whether the pattern carries comment syntax AND at least one language's
/// placement/content gate accepts the face — the ingress-exemption class
/// documented at the `needs_ast_grep_fallback` call in [`search_pattern`].
/// Comment-free patterns are never exempt; placements the reference refuses
/// (leading/trailing/line blocks, ruby inline, php assignment-tail
/// comments) are accepted by NO language and keep the ingress rejection.
fn comment_carrying_face_gate_accepted(pattern: &str) -> bool {
    let scan = scan_pattern_comment_syntax(pattern);
    if !(scan.line || scan.block || scan.hash) {
        return false;
    }
    ast_sgrep_lang::Language::all().iter().any(|lang| {
        !comment_placement_template_route_refused(
            *lang,
            pattern,
            scan_pattern_comment_syntax_for(*lang, pattern),
        )
    })
}

/// The reference keeps `$$$Rest` (multi-capture) and `$Rest` (single
/// capture) in DISTINCT capture slots even when they share a base name; the
/// native chain matcher binds both through one map key, so a
/// classifier-accepted pattern like `$O.out.$$$A($A)` cannot bind the
/// property-name rest — every file answers match-none and the walk composes
/// into a silent `ok:true` empty where the reference answers hits
/// (`System.out.println(total)`). The chain lane otherwise implements the
/// reference's rest-binds-property semantics exactly, so the fix renames
/// ONLY the colliding callee-path rest occurrences to a fresh deterministic
/// name. This is a deliberate behavior change, not a hit-set-preserving
/// no-op: pre-fix the colliding rest bound through the SAME map key as the
/// single capture, so the property text had to unify with the pattern's `$A`
/// argument and colliding patterns answered match-none; post-fix the fresh
/// name binds the rest the reference's way and those faces answer hits. The
/// preserved invariant is name-distinct ingress: non-colliding patterns are
/// untouched, and every changed face moved from wrong-empty (or wrong
/// unification) to the reference-agreed hit set. Scoped to whole dot-segment
/// rests in the callee path (name/property slots); argument-slot rests keep
/// their registered semantics, and `::`-path name slots are out of the
/// probed scope and untouched.
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
        // The fresh name MUST stay inside the canonical metavariable
        // alphabet `[A-Z_][A-Z0-9_]*`. The original `_r` suffix carried a
        // lowercase tail byte, so the de-collided spelling itself classified
        // MixedCase and routed to NeverMatches — the collision faces went
        // silent-empty again. Uppercase suffix + ordinal digits keep the
        // rename canonical.
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
    let baseline = single_worker
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, false, None, &config))?;
    let serial = single_worker
        .install(|| search_pattern_native_profiled(pattern, root, lang_filter, true, None, &config))?;
    let parallel = search_pattern_native_profiled(pattern, root, lang_filter, true, None, &config)?;
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
