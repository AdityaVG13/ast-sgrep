//! Exact-lane and walk-guard helpers, split verbatim from `pattern.rs`.

use super::{pattern_may_carry_expando_meta, search_pattern_cached, search_pattern_native_profiled};
use crate::pattern_bench::find_ast_grep_binary;
use crate::search::SearchHit;
use crate::Result;
use ast_sgrep_lang::{cached_pattern_signatures, index_can_serve_pattern};
use std::path::Path;

/// The structured fail-closed error for patterns the native classifier
/// rejects. The message names the permanent non-delegation decision:
/// external ast-grep is bench-only, so no gate state ever answers
/// beyond-native patterns.
pub(crate) fn structural_fallback_error(
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
pub(crate) fn serve_cached_exact_lane(
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
pub(crate) fn native_walk_candidate_narrowed(
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
pub(crate) fn depth_truncation_disagreement_check(
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
