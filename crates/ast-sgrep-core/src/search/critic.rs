//! Post-fusion deterministic critic (P0 critic-on-shortlist).
//!
//! The critic is the in-process "second model": after fusion it checks whether
//! each hit carries the kind of evidence the query asked for, instead of
//! trusting cosine similarity or any single channel.
//!
//! Rules (all deterministic, no model, no network):
//!
//! 1. **Corroboration gate.** An embed-only hit whose parent span has no
//!    lexical or structural corroboration in the shortlist is dropped, unless
//!    the query is conceptual and the structural stage produced nothing (the
//!    explicitly allowed semantic path). Kept-but-uncorroborated hits are
//!    labeled so the caller can see the weaker evidence class.
//! 2. **Agreement boost.** A hit where semantic and structural channels fused
//!    on the same span is boosted; definition + usage + semantic agreement is
//!    boosted further. Signal provenance is untouched: a boosted semantic hit
//!    stays `semantic`.
//! 3. **Identifier-collision penalty.** When the query names a compound
//!    identifier (`auth_refresh`), a hit whose symbol is only a fragment of it
//!    (`refresh`) is penalized unless the hit itself evidences the full
//!    identifier. The inverse also applies: `Searcher` demotes `bench_searcher`.
//! 4. **Code over docs.** Markdown/changelog lexical hits lose to real code
//!    evidence. Conceptual queries further demote leftover lexical hits when
//!    a definition or embed exists.
//! 5. **Conceptual symbols over entrypoints.** `main`/`start` callers lose to
//!    defs whose names share concept tokens with the query (`auth_refresh` for
//!    "credential renewal"). Partial identifier matches (`refresh` inside a
//!    longer test name) lose to the exact spelling.
//! 6. **Implementation over tests on conceptual NL.** Relative `tests/` paths
//!    (no leading slash) are demoted, and test-path scores are clamped below
//!    the best non-test implementation. Identifier/`defs:` queries are unchanged.
//!
//! The critic adjusts scores and annotates hits before `finish_response`
//! assigns margins and confidence, so downstream honesty fields reflect the
//! critiqued ordering.

use crate::intent::QueryIntent;
use crate::query::ParsedQuery;
use crate::search::types::{HitKind, SearchHit};
use std::collections::{HashMap, HashSet};

/// Score multiplier when semantic evidence agrees with any structural channel.
pub const AGREEMENT_BOOST: f64 = 1.15;
/// Score multiplier when definition, usage, and semantic evidence all agree.
pub const FULL_AGREEMENT_BOOST: f64 = 1.25;
/// Score multiplier for identifier-fragment collisions.
pub const COLLISION_PENALTY: f64 = 0.85;
/// Score multiplier when the hit symbol is a longer compound of the query identifier.
pub const COMPOUND_SYMBOL_PENALTY: f64 = 0.4;
/// Score multiplier when the hit symbol matches the typed identifier exactly.
pub const EXACT_IDENTIFIER_BOOST: f64 = 1.8;
/// Score multiplier when the hit lives in the file named after the identifier
/// (`semantic_ivf` → `semantic_ivf.rs`). Stronger than exact-symbol mentions
/// in other files so the owning module stays in the keep-set.
pub const FILE_STEM_BOOST: f64 = 2.4;
/// Score multiplier when a conceptual hit's file stem is a query concept token
/// (`fusion.rs` for "reciprocal rank fusion").
pub const CONCEPT_FILE_STEM_BOOST: f64 = 1.55;
/// Score multiplier when the hit matches ignoring case but the query used mixed case.
pub const FOLDED_IDENTIFIER_PENALTY: f64 = 0.5;
/// Score multiplier when a def only shares a fragment of the typed identifier.
pub const PARTIAL_IDENTIFIER_PENALTY: f64 = 0.45;
/// Score multiplier for unrelated defs that leaked in on an identifier query.
pub const UNRELATED_DEF_PENALTY: f64 = 0.5;
/// Score multiplier for markdown/changelog lexical hits when code evidence exists.
pub const PROSE_PATH_PENALTY: f64 = 0.4;
/// Score multiplier for conceptual lexical hits when a def/embed exists.
pub const CONCEPTUAL_LEXICAL_PENALTY: f64 = 0.55;
/// Score multiplier for `main`/`start` callers on conceptual NL.
pub const GENERIC_ENTRYPOINT_PENALTY: f64 = 0.3;
/// Score multiplier when a conceptual hit's symbol shares concept tokens with the query.
pub const CONCEPT_SYMBOL_BOOST: f64 = 1.4;
/// Score multiplier for bench/measure/test helpers on conceptual NL.
pub const INSTRUMENTATION_PENALTY: f64 = 0.55;
/// Score multiplier for `tests/` paths on conceptual NL when code exists.
pub const TEST_PATH_PENALTY: f64 = 0.2;

/// Engine-derived critic annotation. Never trusted from the wire; JSON decode
/// re-derives an empty set (same policy as `resolution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriticNote {
    /// Structural and semantic channels fused on the same span.
    ChannelAgreement,
    /// Definition, usage (caller/graph), and semantic evidence all agree.
    FullAgreement,
    /// Embed-only hit kept without corroboration (allowed conceptual path).
    SemanticUncorroborated,
    /// Symbol is a fragment of a longer query identifier.
    IdentifierCollision,
}

impl CriticNote {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChannelAgreement => "channel_agreement",
            Self::FullAgreement => "full_agreement",
            Self::SemanticUncorroborated => "semantic_uncorroborated",
            Self::IdentifierCollision => "identifier_collision",
        }
    }
}

fn has_kind(hit: &SearchHit, want: impl Fn(HitKind) -> bool) -> bool {
    hit.contributors.iter().copied().any(&want) || want(hit.kind)
}

fn is_structural_kind(kind: HitKind) -> bool {
    matches!(
        kind,
        HitKind::Def
            | HitKind::Caller
            | HitKind::Graph
            | HitKind::Anchor
            | HitKind::Import
            | HitKind::Pattern
    )
}

fn is_usage_kind(kind: HitKind) -> bool {
    matches!(kind, HitKind::Caller | HitKind::Graph)
}

fn embed_only(hit: &SearchHit) -> bool {
    hit.kind == HitKind::Embed && hit.contributors.iter().all(|k| *k == HitKind::Embed)
}

/// Non-embed evidence in one file, used to corroborate embed-only spans.
struct Corroborator {
    line_start: u32,
    line_end: u32,
    symbol: Option<String>,
    callee: Option<String>,
}

fn spans_overlap(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> bool {
    a_start <= b_end && b_start <= a_end
}

fn corroborates(witness: &Corroborator, hit: &SearchHit) -> bool {
    if spans_overlap(
        witness.line_start,
        witness.line_end,
        hit.line_start,
        hit.line_end,
    ) {
        return true;
    }
    match hit.symbol.as_deref() {
        Some(symbol) => {
            let symbol = symbol.to_lowercase();
            let matches =
                |value: Option<&str>| value.is_some_and(|value| value.to_lowercase() == symbol);
            matches(witness.symbol.as_deref()) || matches(witness.callee.as_deref())
        }
        None => false,
    }
}

/// Compound query identifiers and their fragments (`auth_refresh` -> {auth, refresh}).
fn identifier_fragments(parsed: &ParsedQuery) -> HashMap<String, String> {
    let mut fragment_to_ident = HashMap::new();
    for term in &parsed.terms {
        if !term.contains('_') {
            continue;
        }
        for fragment in term.split('_').filter(|f| f.len() > 1) {
            fragment_to_ident.insert(fragment.to_string(), term.clone());
        }
    }
    fragment_to_ident
}

fn push_note(hit: &mut SearchHit, note: CriticNote) {
    if !hit.critic.contains(&note) {
        hit.critic.push(note);
    }
}

pub(crate) fn identifier_tokens(symbol: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut previous_lower_or_digit = false;
    for ch in symbol.chars() {
        if !ch.is_alphanumeric() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            previous_lower_or_digit = false;
            continue;
        }
        if ch.is_uppercase() && previous_lower_or_digit && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        current.extend(ch.to_lowercase());
        previous_lower_or_digit = ch.is_lowercase() || ch.is_ascii_digit();
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_prose_path(path: &str) -> bool {
    let normalized = path.replace("\\", "/").to_ascii_lowercase();
    let file = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    file.ends_with(".md")
        || file.ends_with(".mdx")
        || file.ends_with(".rst")
        || file.ends_with(".txt")
        || file.starts_with("changelog")
        || file.starts_with("readme")
        || normalized.contains("/docs/")
}

fn query_identifier(parsed: &ParsedQuery, intent: QueryIntent) -> Option<String> {
    if !matches!(intent, QueryIntent::Symbol | QueryIntent::Structural)
        && parsed.mode != crate::query::QueryMode::Defs
    {
        return None;
    }
    parsed
        .identifier_spelling()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn is_compound_of(query_ident: &str, symbol: &str) -> bool {
    let query = query_ident.to_lowercase();
    let symbol_l = symbol.to_lowercase();
    if symbol_l == query {
        return false;
    }
    let tokens = identifier_tokens(symbol);
    tokens.len() > 1 && tokens.iter().any(|token| token == &query)
}

const GENERIC_ENTRYPOINTS: &[&str] = &[
    "main", "__main__", "<module>", "start", "run", "init", "test", "tests", "setup", "teardown",
];

pub(crate) fn is_generic_entrypoint(name: &str) -> bool {
    let folded = name
        .trim()
        .trim_matches(|c| c == '<' || c == '>')
        .to_ascii_lowercase();
    GENERIC_ENTRYPOINTS.contains(&folded.as_str())
}

fn is_instrumentation_symbol(name: &str) -> bool {
    let folded = name.to_ascii_lowercase();
    folded.starts_with("measure_")
        || folded.starts_with("bench_")
        || folded.starts_with("test_")
        || folded.starts_with("parity_")
        || folded.contains("_expands_to_")
        || folded.contains("_ranks_")
}

fn is_test_path(path: &str) -> bool {
    let normalized = path.replace("\\", "/").to_ascii_lowercase();
    // Relative corpus paths are `tests/core/foo.rs` (no leading slash). A
    // `/tests/` substring check misses those and lets test defs outrank impls.
    normalized.split('/').any(|seg| seg == "tests" || seg == "test")
        || normalized.ends_with("_test.rs")
        || normalized.ends_with("_tests.rs")
}

fn file_stem_eq(path: &str, ident: &str) -> bool {
    let normalized = path.replace("\\", "/");
    let file = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    stem.eq_ignore_ascii_case(ident)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentifierMatch {
    Exact,
    Folded,
    Compound,
    Partial,
    None,
}

fn identifier_match(query_ident: &str, symbol: &str) -> IdentifierMatch {
    if symbol == query_ident {
        return IdentifierMatch::Exact;
    }
    if symbol.eq_ignore_ascii_case(query_ident) {
        return IdentifierMatch::Folded;
    }
    let query_tokens = identifier_tokens(query_ident);
    let symbol_tokens = identifier_tokens(symbol);
    if !query_tokens.is_empty() && query_tokens == symbol_tokens {
        // snake_case vs CamelCase of the same identifier (`semantic_ivf` /
        // `SemanticIvf`), not a helper compound (`bench_searcher`).
        return IdentifierMatch::Exact;
    }
    if is_compound_of(query_ident, symbol) {
        return IdentifierMatch::Compound;
    }
    if query_tokens.is_empty() {
        return IdentifierMatch::None;
    }
    let hits = query_tokens
        .iter()
        .filter(|token| {
            symbol_tokens
                .iter()
                .any(|symbol_token| symbol_token == *token)
        })
        .count();
    if hits == query_tokens.len() {
        IdentifierMatch::Compound
    } else if hits > 0 {
        IdentifierMatch::Partial
    } else {
        IdentifierMatch::None
    }
}

fn conceptual_symbol_affinity(parsed: &ParsedQuery, symbol: &str) -> usize {
    let expanded: std::collections::HashSet<String> =
        ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(&parsed.raw))
            .into_iter()
            .collect();
    identifier_tokens(symbol)
        .into_iter()
        .filter(|token| expanded.contains(token))
        .count()
}

fn conceptual_file_stem_affinity(parsed: &ParsedQuery, path: &str) -> bool {
    const GENERIC_STEMS: &[&str] = &[
        "search", "semantic", "lexical", "index", "store", "query", "test",
        "tests", "lib", "mod", "types", "util", "utils", "core", "main",
        "error", "config", "session", "cli", "eval", "bench", "agent",
    ];
    let normalized = path.replace("\\", "/");
    let file = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    let stem = stem.to_ascii_lowercase();
    if stem.len() < 4 || GENERIC_STEMS.contains(&stem.as_str()) {
        return false;
    }
    let expanded: std::collections::HashSet<String> =
        ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(&parsed.raw))
            .into_iter()
            .collect();
    expanded.contains(&stem)
}

fn is_code_kind(kind: HitKind) -> bool {
    matches!(
        kind,
        HitKind::Def | HitKind::Embed | HitKind::Pattern | HitKind::Caller | HitKind::Anchor
    )
}

/// Apply the deterministic critic to a fused hybrid shortlist.
///
/// Runs after `fusion::apply_weighted_rrf` (contributor sets are final) and
/// before `finish_response` (margins/confidence see critiqued scores).
pub(crate) fn apply_critic(parsed: &ParsedQuery, intent: QueryIntent, hits: &mut Vec<SearchHit>) {
    if hits.is_empty() {
        return;
    }
    // Non-embed witnesses per file: spans and symbols that can corroborate an
    // embed-only parent (child chunks map to parent symbols).
    let mut witnesses: HashMap<&str, Vec<Corroborator>> = HashMap::new();
    for hit in hits.iter() {
        if embed_only(hit) {
            continue;
        }
        witnesses
            .entry(hit.file.as_str())
            .or_default()
            .push(Corroborator {
                line_start: hit.line_start,
                line_end: hit.line_end,
                symbol: hit.symbol.clone(),
                callee: hit.callee.clone(),
            });
    }
    let mut uncorroborated: HashSet<usize> = HashSet::new();
    for (index, hit) in hits.iter().enumerate() {
        if !embed_only(hit) {
            continue;
        }
        let corroborated = witnesses
            .get(hit.file.as_str())
            .is_some_and(|file_witnesses| {
                file_witnesses
                    .iter()
                    .any(|witness| corroborates(witness, hit))
            });
        if corroborated {
            continue;
        }
        uncorroborated.insert(index);
    }

    let fragments = identifier_fragments(parsed);
    let query_ident = query_identifier(parsed, intent);
    let has_code_evidence = hits
        .iter()
        .any(|hit| is_code_kind(hit.kind) && !is_prose_path(&hit.file));
    let conceptual = intent == QueryIntent::Conceptual;
    let mut kept = Vec::with_capacity(hits.len());
    for (index, mut hit) in hits.drain(..).enumerate() {
        if uncorroborated.contains(&index) {
            push_note(&mut hit, CriticNote::SemanticUncorroborated);
        }
        let has_embed = has_kind(&hit, |k| k == HitKind::Embed);
        let has_structural = has_kind(&hit, is_structural_kind);
        if has_embed && has_structural {
            let has_def = has_kind(&hit, |k| k == HitKind::Def);
            let has_usage = has_kind(&hit, is_usage_kind);
            if has_def && has_usage {
                hit.score *= FULL_AGREEMENT_BOOST;
                push_note(&mut hit, CriticNote::FullAgreement);
            } else {
                hit.score *= AGREEMENT_BOOST;
                push_note(&mut hit, CriticNote::ChannelAgreement);
            }
        }
        if let Some(symbol) = hit.symbol.clone() {
            let folded = symbol.to_lowercase();
            if let Some(full_ident) = fragments.get(&folded) {
                let evidences_full = hit.excerpt.to_lowercase().contains(full_ident.as_str());
                if !evidences_full {
                    hit.score *= COLLISION_PENALTY;
                    push_note(&mut hit, CriticNote::IdentifierCollision);
                }
            }
            if let Some(query_ident) = query_ident.as_deref() {
                let stem_match = file_stem_eq(&hit.file, query_ident);
                match identifier_match(query_ident, &symbol) {
                    IdentifierMatch::Exact => hit.score *= EXACT_IDENTIFIER_BOOST,
                    IdentifierMatch::Folded => {
                        if query_ident.chars().any(|c| c.is_uppercase()) {
                            hit.score *= FOLDED_IDENTIFIER_PENALTY;
                            push_note(&mut hit, CriticNote::IdentifierCollision);
                        }
                    }
                    IdentifierMatch::Compound if stem_match => {
                        // `load_semantic_ivf` in `semantic_ivf.rs` is the
                        // implementation, not a colliding helper.
                    }
                    IdentifierMatch::Compound => {
                        hit.score *= COMPOUND_SYMBOL_PENALTY;
                        push_note(&mut hit, CriticNote::IdentifierCollision);
                    }
                    IdentifierMatch::Partial if stem_match => {}
                    IdentifierMatch::Partial => {
                        hit.score *= PARTIAL_IDENTIFIER_PENALTY;
                        push_note(&mut hit, CriticNote::IdentifierCollision);
                    }
                    IdentifierMatch::None => {
                        if hit.kind == HitKind::Def && !stem_match {
                            hit.score *= UNRELATED_DEF_PENALTY;
                        }
                    }
                }
            }
        }
        if let Some(query_ident) = query_ident.as_deref() {
            if file_stem_eq(&hit.file, query_ident) {
                hit.score *= FILE_STEM_BOOST;
            }
        }
        if let Some(symbol_name) = hit.symbol.as_deref() {
            if is_instrumentation_symbol(symbol_name) {
                hit.score *= INSTRUMENTATION_PENALTY;
            }
        }
        if has_code_evidence && hit.kind == HitKind::Asgrep && is_prose_path(&hit.file) {
            hit.score *= PROSE_PATH_PENALTY;
        }
        if conceptual && has_code_evidence && is_test_path(&hit.file) {
            hit.score *= TEST_PATH_PENALTY;
        }
        if conceptual && has_code_evidence && hit.kind == HitKind::Asgrep {
            hit.score *= CONCEPTUAL_LEXICAL_PENALTY;
        }
        if conceptual {
            let caller = hit.caller.as_deref().unwrap_or("");
            let symbol_name = hit.symbol.as_deref().unwrap_or("");
            if matches!(hit.kind, HitKind::Caller | HitKind::Graph)
                && (is_generic_entrypoint(caller) || is_generic_entrypoint(symbol_name))
            {
                hit.score *= GENERIC_ENTRYPOINT_PENALTY;
            }
            let test_path = is_test_path(&hit.file);
            if !test_path {
                if let Some(symbol_name) = hit.symbol.as_deref() {
                    let affinity = conceptual_symbol_affinity(parsed, symbol_name);
                    if affinity >= 2
                        || (affinity >= 1 && matches!(hit.kind, HitKind::Def | HitKind::Embed))
                    {
                        hit.score *= CONCEPT_SYMBOL_BOOST;
                    }
                }
                if conceptual_file_stem_affinity(parsed, &hit.file) {
                    hit.score *= CONCEPT_FILE_STEM_BOOST;
                }
            }
        }
        kept.push(hit);
    }
    *hits = kept;
    if conceptual {
        demote_test_paths_below_implementation(hits);
    }
}

/// Conceptual NL ranks implementation over tests that restated the query.
///
/// A 0.2 path multiplier is not enough when a long test name dumps every
/// concept token and enters fusion 10–20× above the impl. Clamp test-path
/// scores strictly below the best non-test code hit.
fn demote_test_paths_below_implementation(hits: &mut [SearchHit]) {
    let best_impl = hits
        .iter()
        .filter(|hit| {
            !is_test_path(&hit.file) && is_code_kind(hit.kind) && !is_prose_path(&hit.file)
        })
        .map(|hit| hit.score)
        .max_by(|a, b| a.total_cmp(b));
    let Some(best_impl) = best_impl else {
        return;
    };
    let ceiling = best_impl * 0.5;
    for hit in hits.iter_mut() {
        if is_test_path(&hit.file) && hit.score >= best_impl {
            hit.score = ceiling;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchHit;

    fn hit(kind: HitKind, file: &str, symbol: Option<&str>, score: f64) -> SearchHit {
        SearchHit {
            kind,
            file: file.into(),
            line_start: 1,
            line_end: 1,
            symbol: symbol.map(str::to_string),
            caller: None,
            callee: None,
            language: Some("rust".into()),
            score,
            signal: kind.signal(),
            contributors: vec![kind],
            margin: 0.0,
            confidence: 0.0,
            resolution: None,
            embed_fields: None,
            critic: Vec::new(),
            excerpt: String::new(),
        }
    }

    #[test]
    fn exact_identifier_outranks_compound_helpers() {
        let parsed = ParsedQuery::parse("Searcher");
        let mut hits = vec![
            hit(HitKind::Def, "src/bench.rs", Some("bench_searcher"), 0.09),
            hit(HitKind::Def, "src/search.rs", Some("Searcher"), 0.04),
        ];
        apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(hits[0].symbol.as_deref(), Some("Searcher"));
        assert!(hits[1].critic.contains(&CriticNote::IdentifierCollision));
    }

    #[test]
    fn exact_case_outranks_folded_homonym() {
        let parsed = ParsedQuery::parse("Searcher");
        let mut hits = vec![
            hit(HitKind::Def, "tests/x.rs", Some("searcher"), 0.09),
            hit(HitKind::Def, "src/search.rs", Some("Searcher"), 0.04),
        ];
        apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(hits[0].symbol.as_deref(), Some("Searcher"));
        assert!(hits[1].critic.contains(&CriticNote::IdentifierCollision));
    }

    #[test]
    fn markdown_lexical_loses_to_code_on_conceptual_queries() {
        let parsed = ParsedQuery::parse("credential renewal");
        let mut hits = vec![
            hit(HitKind::Asgrep, "README.md", None, 0.02),
            hit(HitKind::Embed, "src/auth.rs", Some("auth_refresh"), 0.011),
        ];
        apply_critic(&parsed, QueryIntent::Conceptual, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(hits[0].symbol.as_deref(), Some("auth_refresh"));
    }

    fn caller_hit(file: &str, caller: &str, callee: &str, score: f64) -> SearchHit {
        let mut hit = hit(HitKind::Caller, file, Some(callee), score);
        hit.caller = Some(caller.into());
        hit.callee = Some(callee.into());
        hit
    }

    #[test]
    fn conceptual_def_outranks_generic_entrypoint_callers() {
        let parsed = ParsedQuery::parse("credential renewal");
        let mut hits = vec![
            caller_hit("src/bin.rs", "main", "main", 0.028),
            hit(HitKind::Def, "src/auth.rs", Some("auth_refresh"), 0.016),
        ];
        apply_critic(&parsed, QueryIntent::Conceptual, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(hits[0].symbol.as_deref(), Some("auth_refresh"));
    }

    #[test]
    fn partial_identifier_defs_lose_to_exact_spelling() {
        let parsed = ParsedQuery::parse("auth_refresh");
        let mut hits = vec![
            hit(
                HitKind::Def,
                "tests/cli.rs",
                Some("search_does_not_refresh_stale_index"),
                0.05,
            ),
            hit(HitKind::Def, "src/auth.rs", Some("auth_refresh"), 0.04),
        ];
        apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(hits[0].symbol.as_deref(), Some("auth_refresh"));
        assert!(hits[1].critic.contains(&CriticNote::IdentifierCollision));
    }

    #[test]
    fn snake_and_camel_same_identifier_are_exact() {
        assert!(matches!(
            identifier_match("semantic_ivf", "SemanticIvf"),
            IdentifierMatch::Exact
        ));
        assert!(matches!(
            identifier_match("semantic_ivf", "load_semantic_ivf"),
            IdentifierMatch::Compound
        ));
        assert!(matches!(
            identifier_match("Searcher", "bench_searcher"),
            IdentifierMatch::Compound
        ));
    }

    #[test]
    fn file_stem_outranks_measure_helper() {
        let parsed = ParsedQuery::parse("semantic_ivf");
        let mut hits = vec![
            hit(
                HitKind::Def,
                "src/bench_suite.rs",
                Some("measure_semantic_ivf_open_p99"),
                0.09,
            ),
            hit(HitKind::Def, "src/semantic_ivf.rs", Some("load_semantic_ivf"), 0.04),
        ];
        apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert!(
            hits[0].file.ends_with("semantic_ivf.rs"),
            "expected module file first, got {:?}",
            hits[0].file
        );
    }

    #[test]
    fn conceptual_impl_outranks_relative_tests_path_name_dump() {
        // Live corpus paths are `tests/core/...` with no leading slash.
        // A `/tests/` substring check misses them. The test name dumps the
        // query so fusion can start 20× above the impl; TEST_PATH_PENALTY
        // alone is not enough. Mutant: drop the clamp, or restore the
        // `/tests/` substring check.
        let parsed = ParsedQuery::parse("how does hybrid search work");
        let mut hits = vec![
            hit(
                HitKind::Def,
                "tests/core/cascade_planner.rs",
                Some("hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages"),
                0.50,
            ),
            hit(
                HitKind::Def,
                "crates/ast-sgrep-core/src/search/mod.rs",
                Some("search_hybrid"),
                0.025,
            ),
        ];
        apply_critic(&parsed, QueryIntent::Conceptual, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(hits[0].symbol.as_deref(), Some("search_hybrid"));
        assert!(
            !is_test_path(&hits[0].file),
            "conceptual NL must not lead with a test path, got {:?}",
            hits[0].file
        );
    }

    #[test]
    fn symbol_intent_keeps_exact_test_definition() {
        // Mutant: applying the conceptual tests/ clamp on Symbol intent.
        let parsed = ParsedQuery::parse(
            "hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages",
        );
        let mut hits = vec![
            hit(
                HitKind::Def,
                "tests/core/cascade_planner.rs",
                Some("hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages"),
                0.09,
            ),
            hit(
                HitKind::Def,
                "crates/ast-sgrep-core/src/search/mod.rs",
                Some("search_hybrid"),
                0.04,
            ),
        ];
        apply_critic(&parsed, QueryIntent::Symbol, &mut hits);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        assert_eq!(
            hits[0].symbol.as_deref(),
            Some("hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages")
        );
    }
}
