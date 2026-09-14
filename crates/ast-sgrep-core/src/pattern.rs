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

/// PASS 96 (F-95A-1): sg 0.45.2 rewrites the per-language expando char
/// (`µ` U+00B5 for the csharp/go/kotlin/php/python/ruby/rust/swift family,
/// U+10000 for c/cpp) to `$` BEFORE parsing, so an expando-spelled run is a
/// metavariable, not identifier bytes. The indexed lane and the byte
/// prefilter below key on RAW pattern bytes: without this guard the index
/// serves literal `µA`/`µµA` identifier rows while the meta reading answers
/// every identifier, and the prefilter keeps only files containing the
/// literal `µA` bytes, dropping µ-free files the meta reading answers.
/// Neither lane decides hits — the native walk normalizes per file language
/// inside `match_pattern` — so a coarse containment test is the sound guard:
/// over-broad matches only disable the fast lanes (a perf cost on the rare
/// literal-µ java/js/ts identifier, which have no expando), never results.
/// PASS 98 (FB-97B-2): crate-visible so the codemod lane's raw-byte
/// `required_pattern_literal` prefilter carries the same guard as the two
/// search lanes below — the codemod prefilter was the third raw-byte consumer
/// the r46 remediation left unguarded.
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
    // PASS 94a (FB-93A-3): ONE class of sg-ANSWERABLE faces ingress-rejects
    // before any per-language gate runs: the classifier gives up on comment
    // trivia inside argument/metadata slots (`classify_native` returns None
    // for `calc($A, /* n */ $B)` — its argument validator refuses the `/*`
    // doc — and the general lane refuses it too), so
    // `needs_ast_grep_fallback` rc2s faces sg 0.45.2 answers (probes run1
    // cells 1-77). When the pattern carries comment syntax AND at least one
    // language's placement/content gate accepts the face, route it to the
    // native walk instead. Fail-closed safety is preserved by construction:
    // the post-walk backstop below keys on `needs_ast_grep_fallback` —
    // still true for every face exempted here — so an exempted face either
    // answers (sg-agreeing) or rc2s; it can never compose into a silent
    // empty. Comment-free refusals and every placement sg rc8s keep the
    // ingress contract byte-identical.
    if needs_ast_grep_fallback(pattern) && !comment_carrying_face_gate_accepted(pattern) {
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
    // PASS 105 (FB-104B-1/FB-104A-4): the comment scan is computed ONCE here
    // and reused by the matcher_decides chain below (net-zero below this
    // point; the early-return hot path pays one linear scan over the tiny
    // pattern text). The exact-signature early-return must run ONLY for
    // comment-free patterns: an exact index signature is a TEXT row and
    // cannot express the comment-placement gates the walk adjudicates
    // (FB-104B-1) — and it cannot express the paren-token contract either
    // (FB-104A-4: the bare `q($$$A)` signature rows served the ruby/swift
    // paren-less over-answer where sg answers []). `$$$`-rest spellings are
    // likewise walk-served: the rest-slot semantics (FB-104A-2) and the
    // paren gate are matcher decisions, never index rows.
    let comment_scan = scan_pattern_comment_syntax(pattern);
    let comment_free = !(comment_scan.line || comment_scan.block || comment_scan.hash);
    if store.pattern_node_count()? > 0
        && comment_free
        && !match_spelling.contains("$$$")
        && !pattern_may_carry_expando_meta(pattern)
        // PASS 122 (F3, f122c): keyword-literal roots (`null`/`true`/`false`,
        // py `None`/`True`/`False`, `this`/`super`) have NO pattern_nodes
        // identifier rows, so the "ident-exact" index serve below composed a
        // silent empty where sg answers the keyword leaf node (oracle grid
        // /tmp/phase122/f3). Route the class to the native walk, which
        // answers sg-exactly; every other ident signature keeps the
        // index-served lane.
        && !ast_sgrep_lang::pattern_is_keyword_literal_root(&match_spelling)
    {
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
            // PASS 131 (130A-F4): the same-line dedup key carries the hit's
            // matched-node byte span. Its registered rationale is collapsing
            // the SAME node found by overlapping query arms — two hits with
            // equal (file, lines, span). Two INDEPENDENT same-line hits
            // (`q(1); q(2);` — different spans) are distinct sg rows and
            // must both survive; the old line-only key collapsed them.
            // Index-lane rows carry no span (`None`), keeping the
            // line-keyed behavior there (registered residual, CNR §45).
            for hit in native.hits {
                if seen.insert((
                    hit.file.clone(),
                    hit.line_start,
                    hit.line_end,
                    hit.byte_span,
                )) {
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

/// PASS 94a (FB-93A-3): whether the pattern carries comment syntax AND at
/// least one language's placement/content gate accepts the face — the
/// ingress-exemption class documented at the `needs_ast_grep_fallback`
/// call in [`search_pattern`]. Comment-free patterns are never exempt;
/// placements sg 0.45.2 rc8s (leading/trailing/line blocks, ruby inline,
/// php assignment-tail comments) are accepted by NO language and keep the
/// ingress rejection.
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
                // Index-lane rows carry no byte span (pattern_nodes has no
                // byte columns) — the line-keyed dedup above stays
                // (registered residual, CNR §45).
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

/// PASS 89a (88c-H1): quote-external comment-syntax scan of a pattern —
/// escape-aware string discipline, the same shape as the lang-side comment
/// refusal the census fallback consults for every non-exempt face
/// (`contains_hash_outside_strings` / `contains_comment_syntax_outside_strings`).
/// `line` flags `//`, `block` flags `/*`, `hash` flags `#`; `block_trailing`
/// records that the final `*/` ends the pattern (only whitespace follows, or
/// the `/*` never closes) — the placement sg 0.45.2 rc8s in every probed
/// language, unlike the inline placements the r37 hook family answers.
/// PASS 91a (F-r40-1): `block_leading` records that a block comment STARTS
/// the pattern (only ASCII whitespace before its `/*`) — the leading
/// placement spawns a second top-level node, so sg 0.45.2 rc8s it in every
/// probed language (probes matrix A, pass91a artifacts) exactly like the
/// trailing placement, and it is excluded from the php inline exemption.
/// PASS 94a (F-93B-4): `hash_bracket` records that a quote-external `#` is
/// immediately followed by `[`. That pair is attribute-like syntax, not a
/// comment: in php a leading `#[...]` is an 8-attribute (sg answers/accepts,
/// probes run2 cells 18-23/27/31), in rust it is the real `#[derive($A)]`
/// syntax, and in ts/js it is INVALID syntax sg rc8s (run2 cells 37-38 —
/// the subject answered a silent fail-open there pre-94a). The blind flags
/// (this scan) stay content-blind so the NeverMatches route and the
/// lane-refusal override precondition keep their registered inputs;
/// [`scan_pattern_comment_syntax_for`] refines them per language for the
/// template route.
#[derive(Clone, Copy)]
struct PatternCommentScan {
    line: bool,
    block: bool,
    hash: bool,
    block_trailing: bool,
    block_leading: bool,
    hash_bracket: bool,
}

fn scan_pattern_comment_syntax(pattern: &str) -> PatternCommentScan {
    let bytes = pattern.as_bytes();
    let mut scan = PatternCommentScan {
        line: false,
        block: false,
        hash: false,
        block_trailing: false,
        block_leading: false,
        hash_bracket: false,
    };
    scan.block_leading = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|first| bytes[first..].starts_with(b"/*"));
    let mut quote: Option<u8> = None;
    let mut in_block = false;
    let mut last_block_end = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if in_block {
            if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_block = false;
                last_block_end = i + 2;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                scan.block = true;
                in_block = true;
                i += 2;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => scan.line = true,
            b'#' => {
                scan.hash = true;
                if bytes.get(i + 1) == Some(&b'[') {
                    scan.hash_bracket = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    scan.block_trailing = scan.block
        && (in_block
            || bytes[last_block_end.min(bytes.len())..]
                .iter()
                .all(|byte| byte.is_ascii_whitespace()));
    scan
}

/// PASS 94a (F-93B-3 / F-93B-4): the language-aware adjudication of the same
/// byte scan. The BLIND scan keeps feeding the NeverMatches route
/// (`never_matches_comment_placement_sg_accepted`) and the lane-refusal
/// override precondition byte-identically; the TEMPLATE route consumes this
/// refined scan. Two sg 0.45.2 grammar facts the language-free byte scan
/// cannot see:
///   - ruby: `/`-delimited regex literals are not comment territory — every
///     quote-external `#` inside `/.../ ` bodies (plain `#` or `#{...}`
///     interpolation) clears the hash flag (probes run2 cells 1-5: sg
///     answers `re = /ab#{x}cd/`, `h2 = /a#b/`), while `#` after division
///     stays a comment (cells 8-9, 12-14: sg rc8s `a / b # c`).
///   - php: a LEADING `#[...]` attribute list whose remainder is empty or a
///     `function` declaration is attribute syntax, not a comment (run2
///     cells 18-23, 27, 31: sg answers/accepts); attribute-after-code and
///     attribute+property tails keep the comment flag (cells 24-25, 29, 34:
///     sg rc8).
fn scan_pattern_comment_syntax_for(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
) -> PatternCommentScan {
    let mut scan = scan_pattern_comment_syntax(pattern);
    if scan.hash {
        match lang {
            ast_sgrep_lang::Language::Ruby if ruby_hash_all_inside_regex_literals(pattern) => {
                scan.hash = false;
            }
            ast_sgrep_lang::Language::Php if php_leading_attribute_function_face(pattern) => {
                scan.hash = false;
                scan.hash_bracket = false;
            }
            _ => {}
        }
    }
    scan
}

/// PASS 94a (F-93B-3): true when EVERY quote-external `#` in `pattern` lies
/// inside a `/.../ ` regex literal — the interpolation `#{...}` and plain
/// `#` bodies alike. A `/` opens a regex literal when division is
/// impossible: pattern start, after an operator/open-bracket byte, or after
/// a ruby control keyword; after an identifier, number, or closing bracket
/// it is division and following `#` bytes are real comments (probe matrix:
/// `re = /ab#{x}cd/` regex vs `a / b # c` / `m2 = (p1 + p2) / 2 # tail`
/// division — probes_run2.jsonl). Escapes, `[...]` char classes, and
/// `#{...}` interpolation nesting are tracked; strings are quote-excluded
/// by the caller's blind scan and re-checked here the same way.
fn ruby_hash_all_inside_regex_literals(pattern: &str) -> bool {
    const KEYWORDS: [&str; 16] = [
        "if", "elsif", "unless", "while", "until", "and", "or", "not", "then", "when", "case",
        "begin", "do", "else", "return", "yield",
    ];
    let bytes = pattern.as_bytes();
    let mut saw_hash = false;
    let mut hash_outside_regex = false;
    let mut quote: Option<u8> = None;
    let mut in_regex = false;
    let mut in_class = false;
    let mut interp_depth = 0usize;
    // last significant (non-whitespace) byte outside quotes/regex; 0 = none yet
    let mut last_sig: u8 = 0;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if in_regex {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'#' {
                saw_hash = true;
                if bytes.get(i + 1) == Some(&b'{') && interp_depth == 0 {
                    interp_depth = 1;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if interp_depth > 0 {
                if b == b'{' {
                    interp_depth += 1;
                } else if b == b'}' {
                    interp_depth -= 1;
                }
                i += 1;
                continue;
            }
            if b == b'[' {
                in_class = true;
            } else if b == b']' {
                in_class = false;
            } else if b == b'/' && !in_class {
                in_regex = false;
                last_sig = b'/';
            }
            i += 1;
            continue;
        }
        if b == b'#' {
            saw_hash = true;
            hash_outside_regex = true;
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' || b == b'`' {
            quote = Some(b);
            i += 1;
            continue;
        }
        if b == b'/'
            && bytes.get(i + 1) != Some(&b'=')
            && ruby_regex_plausible(bytes, i, last_sig, &KEYWORDS)
        {
            in_regex = true;
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() {
            last_sig = b;
        }
        i += 1;
    }
    saw_hash && !hash_outside_regex
}

/// Whether a `/` at `at` can open a ruby regex literal: yes at pattern
/// start, after an operator/open-bracket byte, or directly after a control
/// keyword; no after an identifier/number/closing bracket (division).
fn ruby_regex_plausible(bytes: &[u8], at: usize, last_sig: u8, keywords: &[&str]) -> bool {
    if last_sig == 0 {
        return true;
    }
    if matches!(
        last_sig,
        b'(' | b'[' | b'{' | b',' | b';' | b'=' | b'~' | b'!' | b'?' | b':' | b'&' | b'|' | b'+'
            | b'-' | b'*' | b'%' | b'<' | b'>'
    ) {
        return true;
    }
    let mut end = at;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_alphabetic() {
        start -= 1;
    }
    std::str::from_utf8(&bytes[start..end])
        .map(|word| keywords.contains(&word))
        .unwrap_or(false)
}

/// PASS 94a (F-93B-4): true when `pattern` is a php attribute face sg
/// 0.45.2 accepts — the first non-whitespace bytes are `#[`, brackets
/// balance (quote- and escape-aware), and the remainder after the matching
/// `]` is empty or a `function` declaration. Attribute-after-code and
/// attribute+property tails are NOT this face (sg rc8s them: run2 cells
/// 24-25, 29, 34) and keep the comment flag.
fn php_leading_attribute_function_face(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let Some(start) = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .filter(|first| bytes[*first..].starts_with(b"#["))
    else {
        return false;
    };
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let rest = pattern[i + 1..].trim_start();
                    return rest.is_empty() || rest.starts_with("function");
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// PASS 94a (FB-93A-3): true when a quote-external, depth-0 `=` (an
/// assignment, not `==` `=>` `<=` `>=` `!=`) occurs BEFORE the first
/// quote-external `/*` comment start. sg 0.45.2's php pattern gate refuses
/// assignment-rooted metavariable patterns whose comment trivia sits in the
/// assignment tail (`$A = /* w */ $B;`, `$A = f(/* w */ $B);` — probes run1
/// cell 39, run2 cells 40, 42-44, 46) while call/operand/before-`=` comment
/// faces answer (cells 36-38, 101). Textual proxy for that per-language
/// gate; php-only like the php lanes it mirrors.
fn php_comment_follows_top_level_assignment(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut quote: Option<u8> = None;
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'/' if bytes.get(i + 1) == Some(&b'*') => return false,
            b'=' if depth == 0 => {
                let prev = if i == 0 { b' ' } else { bytes[i - 1] };
                let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                if !matches!(next, b'=' | b'>') && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// PASS 91a (F-r40-1): the block-placement acceptance shared by both routing
/// routes. Comment-free (`!block`) is accepted; a LEADING block comment is
/// refused in EVERY language (sg 0.45.2 rc8s the placement on the probed
/// matrix — pass91a artifacts, probes_run1 matrix A/B); the php inline
/// non-leading non-trailing placement keeps the r37 hook-family exemption
/// (f87a-pinned {10}/{4}/{8}); every other block placement (trailing,
/// unclosed, non-php) is refused.
fn block_placement_sg_accepted(
    lang: ast_sgrep_lang::Language,
    scan: PatternCommentScan,
) -> bool {
    if !scan.block {
        return true;
    }
    if scan.block_leading {
        return false;
    }
    matches!(lang, ast_sgrep_lang::Language::Php) && !scan.block_trailing
}

/// PASS 89a (88c-H1): whether sg accepts a NeverMatches pattern's comment
/// placement in `lang`, per the probed 0.45.2 matrix (f89a_ suite docs):
/// comment-free faces (the walk decides, byte-equal to pre-r37 where
/// `native_pattern_answerable` returned true) and INLINE `/* */` faces in php
/// (the r37 hook family, f87a-pinned {10}/{4}/{8}) are exempt from the
/// H-CONF-031 skip + H-CONF-029 census. Every glued line comment and every
/// trailing/unclosed block comment keeps the pre-r37 census-loud contract —
/// sg rc8s the registered spellings (py/rb/php `$x # note`, php/rust
/// `$x // note`, `;`-statement-trailing `//`/`/*` in every probed language).
/// PASS 91a (F-r40-1): a LEADING block comment is no longer exempt in any
/// language (`block_placement_sg_accepted`) — sg rc8s it everywhere probed,
/// and the php walk answered it silent where sg refused the pattern.
/// For the `#`-as-syntax languages (rust/c/cpp/go/js/ts) the census fallback
/// finds the face answerable and the walk decides — byte-equal to the
/// pre-r37 behavior there too.
fn never_matches_comment_placement_sg_accepted(
    lang: ast_sgrep_lang::Language,
    scan: PatternCommentScan,
) -> bool {
    !scan.line && !scan.hash && block_placement_sg_accepted(lang, scan)
}

/// PASS 91a (F-r40-4): the same comment-placement acceptance must govern the
/// template-exists route. A `$`-less pattern returns answerable from
/// `native_pattern_answerable` before that function's comment refusal is
/// ever consulted (the `!contains('$')` arm, lang F62-1), so a comment face
/// that dodges NeverMatches classification built a template over the comment
/// trivia, walked, and answered a silent empty where sg 0.45.2 rc8s the
/// placement (probes_run1/run2 matrix B: py/rb hash faces, js/rust/ts/php
/// leading blocks — every cell rc8).
///
/// PASS 92 (F-r41-1): PLACEMENT-AWARE narrowing per the pass92 matrix
/// (artifacts/conformance/pass92/probes_run1.jsonl + probes_run2.jsonl).
/// 91a's blanket arms over-refused placements sg ANSWERS — the two
/// registered rust container-slot block faces (P60-HCONF-030-II
/// `{lit_rs.rs:13}`, P63-F62-4-RS `{slots.rs:5}`) flipped census-loud rc2
/// (2 oracle P1 unflips) — and py floor-div faces (`a // b`: `//` is NOT
/// comment syntax in python, sg answers hits). The narrowed gate:
///   - hash `#`: refused in python/ruby/php at EVERY placement — leading
///     and trailing are sg-rc8 (f91a_-pinned loud), and the EMBEDDED
///     placement (sg rc1-lenient: `f(1, # c\n 2)`) is UNREACHABLE through
///     the query layer, which whitespace-collapses the newline that would
///     terminate the comment (the registered user-WIP ingress class), so
///     no embedded-hash spelling can ever reach this gate. `#`-as-syntax
///     languages (rust/c/cpp/go/js/ts) keep answering (matrix C:
///     `#include <stdio.h>`, `#[derive($A)]`, `this.#x = 1`).
///   - line `//`: refused in every language EXCEPT python (floor-div, sg
///     answers `a // b` {8}); leading/trailing spellings are sg-rc8 or
///     registered-census-loud (js/ts trailing is sg rc1-lenient but stays
///     loud: the H-CONF-029 genus, zero movement), and the embedded-line
///     acceptance sg shows (`a + // c\n b` answers hits in rust) is
///     unreachable for the same newline-collapse reason.
///   - block: LEADING and TRAILING refused in every language (f91a_-pinned;
///     the registered census-loud genus where sg is lenient); ruby inline
///     refused (no `/* */` syntax — sg rc8s); INLINE non-leading
///     non-trailing blocks ACCEPTED in every other language — the unflip:
///     sg rc1-lenient on `a + /* c */ b` everywhere probed and ANSWERS the
///     rust container-slot faces via the pass-60/63 comment-slot machinery
///     (php's r37 hook-family exemption is the already-accepted subset).
/// PASS 94a (F-93B-4): `#[` gets its own arms. In ts/js the pair is INVALID
/// syntax — sg 0.45.2 rc8s the face (run2 cells 37-38; the subject answered
/// a silent fail-open ok:true [] there pre-94a) — so a quote-external
/// `hash_bracket` refuses. In php the language-aware scan has ALREADY
/// cleared the flag for the leading-attribute faces sg accepts; everything
/// else `#`-carrying keeps the py/rb/php refusal.
/// PASS 94a (FB-93A-3): php assignment-rooted patterns whose block-comment
/// trivia sits AFTER a top-level `=` are sg-rc8 (run1 cell 39, run2 cells
/// 40, 42-44, 46) — sg's php pattern gate refuses the metavariable
/// assignment doc — so they stay refused even at the inline placement the
/// other ten languages answer. The `pattern` parameter feeds that textual
/// proxy; both consumers (template gate + lane override) call this one
/// function so the class cannot drift apart.
/// NeverMatches-classified faces never take this gate — their loudness
/// comes from the lang-side refusal inside `native_pattern_answerable` and
/// their walk-decided registered silent cells (rust/js `$x # note`) must
/// not move.
fn comment_placement_template_route_refused(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    scan: PatternCommentScan,
) -> bool {
    use ast_sgrep_lang::Language;
    if scan.hash_bracket && matches!(lang, Language::TypeScript | Language::JavaScript) {
        return true;
    }
    // `$`-carrying only: the `$`-LESS assignment-rooted php faces are the
    // r37 hook family the walk answers (f87a-pinned {10}/{4}/{8};
    // f91a-pinned accepted placement `alpha = 1 + /* c */ 2;`) — refusing
    // them here flipped f91a loud. The sg-rc8 matrix (run1 cell 39, run2
    // cells 40, 42-44, 46) is entirely metavariable-carrying.
    if matches!(lang, Language::Php)
        && pattern.contains('$')
        && scan.block
        && php_comment_follows_top_level_assignment(pattern)
    {
        return true;
    }
    if scan.hash && matches!(lang, Language::Python | Language::Ruby | Language::Php) {
        return true;
    }
    if scan.line && !matches!(lang, Language::Python) {
        // PASS 102 (F-101A-4): the meta-free whole-token literal route takes
        // the literal trailing-comment lane's verdict — sg 0.45.2 parses the
        // trailing line comment as a real child and ANSWERS the
        // comment-carrying js/ts rows (m4 matrix, `q(µAble) // c`), while
        // the subject rc2'd the whole class. The lane admits exactly the
        // sg-accepted, cleanly-parsing, trailing-line-comment faces (the
        // `$`-carrying registered matrix and the go/rust/java/csharp rc8
        // spellings stay refused — the lane refuses them by construction),
        // so the walk decides every admitted face and the loud fold keeps
        // everything else exactly where the r44/r45 matrices registered it.
        if !pattern.contains('$')
            && ast_sgrep_lang::literal_trailing_comment_lane(lang, pattern)
        {
            return false;
        }
        return true;
    }
    if scan.block && (scan.block_leading || scan.block_trailing || matches!(lang, Language::Ruby))
    {
        return true;
    }
    false
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
    // PASS 87a (86a-H1): NeverMatches-classified patterns cannot phantom-hit —
    // match_pattern routes them to the structural arm that answers empty, so
    // the only arms that can answer are the registered sg-exact php-hook and
    // dollar-literal lanes. The per-file answerable skip and this census are
    // keyed on `native_pattern_answerable`, which cannot see those lanes (for
    // a NeverMatches classify its only false arm is the pattern-side comment
    // refusal), so comment-carrying hook faces (the r35 FB-84a-03 family,
    // suite-answered {10} at library) were skipped in every file, the census
    // counted every corpus language unanswerable, and the iva9.7 backstop
    // composed the skips into the loud structural-fallback error — rc2 where
    // sg answers. For exactly this class the matcher's own per-file answer
    // decides: hits = hits, an empty is an honest empty (the census must not
    // count an unanswerability the walk does not have). Every other class
    // keeps its registered contract byte-for-byte: classifier-accepted
    // decl/call templates keep the skip + census (H-CONF-029/031, including
    // the deliberate 64c-F6 pin), ingress-rejected garbage keeps the loud
    // H-CONF-023 refusal, and `$`-less patterns never classify NeverMatches.
    // PASS 89a (88c-H1): that blanket exemption was sg-exact only for the
    // comment placements sg accepts — a comment-free NeverMatches face
    // (walk-decided; pre-r37 `native_pattern_answerable` returned true there)
    // or an INLINE `/* */` block face in php (the r37 hook family,
    // f87a-pinned {10}/{4}/{8}). The probed 0.45.2 matrix rc8s every other
    // comment placement — glued `#`/`//` line comments (py/rb/php
    // `$x # note`, php/rust `$x // note`) and trailing `/* */` after a
    // statement in every probed language — and the blanket exemption had
    // silenced those sg-agreed-loud faces into ok:true []. Those faces fall
    // back to the census path below byte-for-byte, restoring the pre-r37
    // loudness; `matcher_decides` becomes per-language because the accepted
    // block placement is language-scoped (php).
    // PASS 91a (F-r40-1): the accepted block placement narrows further — a
    // LEADING `/* */` (`block_leading`) is rc8 in every probed language (it
    // spawns a second top-level node) and leaves the exemption in ALL
    // languages, closing the silenced php leading-block faces (90A-F1).
    let never_matches = matches!(
        ast_sgrep_lang::classify_native(pattern),
        Some(ast_sgrep_lang::NativeKind::NeverMatches)
    );
    // PASS 105 (FB-104B-1): `search_pattern` computes the same scan for its
    // early-return gate; this native-walk entry keeps its own (it has other
    // callers, and the scan is one linear pass over the pattern text).
    let comment_scan = scan_pattern_comment_syntax(pattern);
    let comment_free = !(comment_scan.line || comment_scan.block || comment_scan.hash);
    // PASS 92 (F-r41-2): the NeverMatches exemption must ALSO defer to the
    // lang-side answerability on the COMMENT-FREE class. A two-statement
    // brace-compound spelling (`1_000 ($A) { $_0x1F }` — `)` followed by
    // `{`) is sg-rc8 ("Multiple AST nodes are detected") in seven languages,
    // but the comment-free NeverMatches placement was exempt and the walk
    // answered a silent empty (fuzz face F26-0614). That consult is kept
    // byte-identical for the comment-free class.
    // PASS 105 (FB-104B-1): the COMMENT-CARRYING half of the disjunct was a
    // blanket `true` (`!comment_free`) — it exempted every inline-block
    // NeverMatches face from the per-file skip and the census even when the
    // walk arm cannot decide the face. The php member-call slot grammar
    // refuses comment trivia in an argument slot (`$obj->m(/* m */ $A)`,
    // 94a cell 94), so the structural NeverMatches arm answered empty for
    // every file and the face composed into a SILENT ok:true [] where sg
    // 0.45.2 answers the row (m1 oracle). The blanket is replaced by the
    // exact set of walk-decidable comment-carrying faces — the php
    // comment-transparent lanes (the r35 hook family f87a pins): the
    // assignment-hook and operand-template lanes decide comments by AST
    // structure, so they keep answering; every other comment-carrying
    // NeverMatches face falls back to the census path, which the backstop
    // composes into the loud fail-closed class (fail-open -> fail-closed).
    let matcher_decides = |lang: ast_sgrep_lang::Language| -> bool {
        never_matches
            && never_matches_comment_placement_sg_accepted(lang, comment_scan)
            && (comment_free && ast_sgrep_lang::native_pattern_answerable(lang, pattern)
                || (!comment_free
                    && ast_sgrep_lang::php_comment_transparent_operand_lane(pattern)))
    };
    // PASS 91a (F-r40-4): the template-exists route needs the same comment
    // gate. A `$`-less pattern short-circuits `native_pattern_answerable`'s
    // own comment refusal (the `!contains('$')` arm), so a comment placement
    // sg 0.45.2 rc8s reached the walk through the general template and
    // answered a silent empty. The gate is scoped to NON-NeverMatches
    // patterns: NeverMatches faces keep their registered routing
    // byte-for-byte (matcher_decides above; the lang-side refusal inside
    // native_pattern_answerable).
    // PASS 94a (F-93B-3/F-93B-4): the gate consumes the LANGUAGE-AWARE scan
    // — ruby regex-literal `#`s and php leading-attribute `#[` faces are not
    // comments (scan_pattern_comment_syntax_for) — plus the ts/js `#[` and
    // php assignment-tail comment refusals inside the gate itself.
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
    // PASS 94a (FB-93A-3 / F-93B-4): the lang-side verdict gets ONE scoped
    // override. `native_pattern_answerable` refuses every `$`-carrying
    // comment-carrying pattern through its lane gate BEFORE any
    // classification — the lane is placement-unaware (every inline-block
    // placement sg ANSWERS in the ten non-ruby/non-py block-comment
    // languages, probes run1 cells 1-77) and content-blind (php leading
    // `#[` attributes sg answers, run2 cells 18-23/31). When the comment
    // gate above did NOT refuse the placement/content (the core
    // adjudication accepted the face) AND the pattern actually carries
    // comment bytes (so the lane refusal is the comment arm's verdict), the
    // walk decides instead. Everything else keeps its registered loud
    // class: faces with no comment bytes refuse for REAL non-comment
    // reasons (classifier rejects, the php meta-target veto `$A = $B`,
    // NeverMatches multi-root), and NeverMatches faces keep their routing
    // byte-for-byte (the `$x = /ab#{y}cd/` ruby spelling stays
    // census-loud, sg rc1-lenient genus).
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
            // core adjudication accepted above. PASS 94a carve-out: a
            // `$`-carrying php attribute face keeps the refusal — the walk
            // cannot bind metas inside attribute argument lists
            // (`#[Route($X)]` answers walk-empty where sg binds {2,4},
            // probes run2 cell 19), so un-refusing would trade the
            // registered fail-closed loud for a silent empty; the carve-out
            // repairs exactly the `$`-less spellings through the template
            // route above.
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
    // PASS 60 (H-CONF-029): census the corpus languages (post lang-filter)
    // with NO native template for this pattern. Computed from the path set
    // BEFORE the byte prefilter runs so prefiltered-away files cannot hide
    // unanswerability behind a silent empty result. One unanswerable corpus
    // language means a query over it can never be answered natively (sg
    // rejects the same shape); the caller turns this into the loud
    // fail-closed error when the query answers empty. NeverMatches-classified
    // patterns are exempt where sg accepts the comment placement
    // (matcher_decides above; 88c-H1): the walk answers those files directly,
    // so no language is unanswerable for them — every other comment face
    // keeps this census and the loud backstop, exactly pre-r37.
    // PASS 91a (F-r40-4): "every other comment face" now includes the
    // non-NeverMatches (`$`-less) comment faces (template_comment_refused
    // above) — they previously dodged this census because the `$`-less arm
    // of native_pattern_answerable answers before any comment refusal.
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
            if !unanswerable.contains(&lang) && language_unanswerable(lang) {
                unanswerable.push(lang);
            }
        }
        unanswerable.len()
    };
    // PASS 96 (F-95A-1): an expando-spelled metavariable normalizes to `$`
    // per file language inside the walk, so a raw-byte required literal
    // (`µA`) would prefilter away µ-free files the meta reading answers
    // (the f96 aux-file pin exercises exactly that drop).
    let required_literal = if use_prefilter && !pattern_may_carry_expando_meta(pattern) {
        required_pattern_literal(pattern)
    } else {
        None
    };
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
            // PASS 87a (86a-H1): NeverMatches-classified patterns are exempt
            // (matcher_decides): their structural arm answers empty, so the
            // phantom-hit hazard does not exist and match_pattern's own
            // per-file answer is the honest one (the php hook / dollar-literal
            // lanes answer their registered sg-exact faces here).
            // PASS 89a (88c-H1): the exemption is per-language and covers only
            // the sg-accepted comment placements (comment-free, or inline
            // `/* */` in php) — glued line-comment and trailing-block faces
            // fall back to this skip + the census, pre-r37 loud.
            // PASS 91a (F-r40-4): non-NeverMatches comment faces join the
            // same discipline (language_unanswerable): a `$`-less pattern
            // short-circuits native_pattern_answerable's own refusal, so the
            // placements sg 0.45.2 rc8s must skip the file here instead of
            // walking into a silent empty.
            // PASS 131 (130A-F4): the universal-root faces keep their
            // REGISTERED line-collapsed rows — per-file detection (the file
            // language decides the expando normalization), spans suppressed
            // at the conversion site below.
            let universal_root = ast_sgrep_lang::is_universal_root_pattern(lang, pattern);
            if language_unanswerable(lang) {
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
                        // PASS 131 (130A-F4): the native walk knows the
                        // matched-node byte span; it keys the same-line dedup
                        // at the union site and the finish-layer DedupKey.
                        // EXCEPTION: the universal-root lane (`$$A`) keeps
                        // its REGISTERED line-collapsed rows (f96 pins the
                        // 12-line `uA` set; per-node multiplicity there
                        // would starve the finish keep-truncate), so its
                        // spans stay suppressed.
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
