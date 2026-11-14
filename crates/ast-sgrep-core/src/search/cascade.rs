use super::critic;
use super::passes;
use super::passes::bmh::asgrep_line_hit;
use super::passes::literal::literal_file_pass;
use super::passes::symbol::{search_callers, search_defs};
use super::types::{self, HitKind, SearchHit, SearchOptions, SpanHitInput};
use crate::query::ParsedQuery;
use crate::store::IndexStore;
use crate::Result;
use std::collections::HashSet;

pub(crate) const CASCADE_PREFILTER_FILE_LIMIT: usize = 100;
/// One cascade term must not spend the whole 100-file budget. `index` otherwise
/// fills CLI helpers before `durability` can add `store/mod.rs`.
pub(crate) const CASCADE_PER_TERM_FILE_LIMIT: usize = 32;
/// Concept extras (`embed` after user `embeddings`) keep slots after common
/// user tokens have already matched files.
pub(crate) const CASCADE_EXTRA_FILE_RESERVE: usize = 32;
/// Cap on reported query expansions (ufk7).
pub(crate) const MAX_QUERY_EXPANSIONS: usize = 5;
pub(crate) const NL_FANOUT_SYMBOL_LIMIT: usize = 4;
pub(crate) const NL_FANOUT_HITS_PER_CHANNEL: usize = 16;
/// Caller-leg fan-out hits rank below defs in fusion (Sep-7 recorded shape).
pub(crate) const FANOUT_CALLER_SCALE: f64 = 0.35;

/// Per-query token sets shared by the conceptual cascade lanes (d-NEEDLESS
/// hoist 3). `search_hybrid` computes one instance per conceptual query and
/// threads it through def-terms, fanout, and discovery-extras — one
/// model-backed `expand_concepts` per query, tokens read instead of
/// re-expanded. (`tokenize` sorts, so sorted iteration below reproduces the
/// old per-call token order exactly.)
#[derive(Clone)]
pub(crate) struct ConceptualQueryTokens {
    pub user: HashSet<String>,
    pub expanded: HashSet<String>,
}

impl ConceptualQueryTokens {
    pub(crate) fn for_query(query: &str) -> Self {
        Self {
            user: ast_sgrep_embed::tokenize(query).into_iter().collect(),
            expanded: ast_sgrep_embed::tokenize(&ast_sgrep_embed::expand_concepts(query))
                .into_iter()
                .collect(),
        }
    }
}

/// `conceptual_def_terms` over precomputed per-query tokens: one expansion
/// per query, tokens passed down. Output matches the old expand-per-call
/// form exactly (the surviving terms are sorted + truncated).
pub(crate) fn conceptual_def_terms_with_tokens(tokens: &ConceptualQueryTokens) -> Vec<String> {
    // Snake_case from expansion can name a def (`intern_paths`). Other tokens
    // must appear in the user query: dumping a whole CONCEPT_GROUPS list into
    // symbol LIKE was unique-hybrid p100 (auth NL) and a rarity cap on that
    // dump ranked auth_refresh for "durable session write".
    let mut terms: Vec<String> = tokens
        .expanded
        .iter()
        .filter(|tok| {
            let tok = *tok;
            tok.chars().count() >= 4
                && !critic::is_generic_concept_token(tok)
                && (tok.contains('_')
                    || tokens.user.contains(tok)
                    || tokens.user.iter().any(|u| u.contains(tok.as_str())))
        })
        .cloned()
        .collect();
    terms.sort();
    terms.dedup();
    terms.truncate(8);
    terms
}

/// `conceptual_fanout_pass` over precomputed per-query tokens: one
/// expansion per query, tokens passed down.
pub(crate) fn conceptual_fanout_pass_with_tokens(
    store: &IndexStore,
    options: &SearchOptions,
    tokens: &ConceptualQueryTokens,
    semantic: &[SearchHit],
) -> Result<Vec<SearchHit>> {
    let query_tokens: HashSet<String> = tokens.expanded.clone();
    let mut seen_symbols = HashSet::new();
    let mut ranked = Vec::new();
    for hit in semantic {
        let Some(symbol) = hit.symbol.as_deref() else {
            continue;
        };
        if symbol.is_empty() || critic::is_generic_entrypoint(symbol) {
            continue;
        }
        if !seen_symbols.insert(symbol.to_lowercase()) {
            continue;
        }
        let affinity = critic::identifier_tokens(symbol)
            .into_iter()
            .filter(|token| query_tokens.contains(token))
            .count();
        ranked.push((affinity, symbol));
    }
    if !ranked.iter().any(|(affinity, _)| *affinity > 0) {
        // Zero-affinity defs (auth_refresh for "combine two search channels"
        // on a tiny fixture) were unique-hybrid p100: two extra search_defs
        // SQL round-trips that cannot help ranking.
        return Ok(Vec::new());
    }
    ranked.retain(|(affinity, _)| *affinity > 0);
    ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    let symbols = ranked
        .into_iter()
        .map(|(_, symbol)| symbol)
        .take(NL_FANOUT_SYMBOL_LIMIT);
    let mut hits = Vec::new();
    let mut fanout_options = options.clone();
    fanout_options.limit = NL_FANOUT_HITS_PER_CHANNEL;
    for symbol in symbols {
        let def_query = ParsedQuery::parse(&format!("defs:{symbol}"));
        hits.extend(search_defs(store, &fanout_options, &def_query)?);
        // Caller/graph leg: the pinned conceptual contract (cli_smoke
        // conceptual_query_fans_out) requires call-site contributors — a
        // callee name is unreachable by term match from prose, so this
        // affinity-driven leg is the only source of that evidence. The
        // callers query materializes the direct Caller row and its Graph
        // edge side by side; both must survive so fusion can list the full
        // contributor set on the merged same-line hit.
        let caller_query = ParsedQuery::parse(&format!("callers:{symbol}"));
        let mut caller_count = 0usize;
        let mut graph_count = 0usize;
        for mut hit in search_callers(store, &fanout_options, &caller_query)? {
            let count = match hit.kind {
                HitKind::Caller => &mut caller_count,
                HitKind::Graph => &mut graph_count,
                _ => continue,
            };
            if *count >= NL_FANOUT_HITS_PER_CHANNEL {
                continue;
            }
            if critic::is_generic_entrypoint(hit.caller.as_deref().unwrap_or("")) {
                hit.score *= 0.25;
            }
            hit.score *= FANOUT_CALLER_SCALE;
            hits.push(hit);
            *count += 1;
        }
        hits.extend(pattern_hits_for_symbol(
            store,
            options.lang_filter.as_deref(),
            symbol,
        )?);
    }
    Ok(hits)
}

/// Indexed pattern nodes matching the fan-out symbol's structural signatures —
/// the `pattern` contributor leg of the conceptual fan-out (bounded per
/// channel; deduped by location).
pub(crate) fn pattern_hits_for_symbol(
    store: &IndexStore,
    lang_filter: Option<&str>,
    symbol: &str,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for signature in ast_sgrep_lang::structural_term_signatures(symbol) {
        let remaining = NL_FANOUT_HITS_PER_CHANNEL.saturating_sub(hits.len());
        if remaining == 0 {
            break;
        }
        for row in store.pattern_nodes_matching_limited(&signature, lang_filter, remaining)? {
            if !seen.insert((row.path.clone(), row.line_start, row.line_end)) {
                continue;
            }
            let excerpt = store.fill_pattern_excerpt(&row)?;
            hits.push(SearchHit::span(SpanHitInput {
                kind: HitKind::Pattern,
                file: row.path,
                line_start: row.line_start,
                line_end: row.line_end,
                score: crate::rank::SCORE_PATTERN * 0.85,
                excerpt,
                symbol: Some(symbol.to_owned()),
                language: row.language,
                byte_span: None,
            }));
        }
    }
    Ok(hits)
}

pub(crate) fn cascade_stopword(term: &str) -> bool {
    // English function words that match too many files and stall discovery
    // on the first 3+ character token ("how does hybrid search work").
    matches!(
        term.to_ascii_lowercase().as_str(),
        "how"
            | "does"
            | "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "what"
            | "where"
            | "when"
            | "why"
            | "who"
            | "are"
            | "was"
            | "were"
            | "into"
            | "about"
            | "than"
            | "then"
            | "them"
            | "they"
            | "have"
            | "has"
            | "had"
            | "but"
            | "can"
            | "could"
            | "would"
            | "should"
            | "will"
            | "also"
    )
}

pub(crate) fn literal_prefilter_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    drop_common: bool,
    user_first: Option<&HashSet<String>>,
) -> Result<(Vec<SearchHit>, HashSet<String>)> {
    // Trigram MATCH needs 3 chars. Shorter needles use literal_sql LIKE/GLOB
    // with ORDER BY over the whole `lines` table — ~22 ms on a 54k-file
    // corpus for a digit like "0". Cascade file discovery does not need them.
    // Generic user terms survive when fewer than two specific user terms
    // carry the query (same relative rule as the structural stage terms):
    // "derive the next command to run" must still discover files via
    // `command`. Generic extras (borrowed/concept tokens) stay dropped;
    // the corpus-df filter below still bounds truly common terms.
    let keep_user_generics = drop_common
        && user_first.is_some_and(|user| {
            user.iter()
                .filter(|term| {
                    term.chars().count() >= 3
                        && !cascade_stopword(term)
                        && !critic::is_generic_concept_token(term)
                })
                .count()
                < 2
        });
    let mut terms = parsed
        .terms
        .iter()
        .filter(|term| {
            term.chars().count() >= 3
                && !cascade_stopword(term)
                && !(drop_common
                    && critic::is_generic_concept_token(term)
                    && !(keep_user_generics && user_first.is_some_and(|user| user.contains(*term))))
        })
        .collect::<Vec<_>>();
    let _df_span =
        crate::perf_profile::Span::start("lexical_df_gate_sort", "search", "trigram_df.min_df");
    // Bundle `min_df` is a no-op on `ASGREP_READPATH=sql` (no mmap). Applying
    // the baked-df common-term drop / rare-first sort only on the bundle arm
    // would fill the 100-file cascade with a different set. Dedicated
    // `literal:` still uses `scan_shortcut`. Hybrid discovery locksteps the
    // SQL control arm.
    if terms.is_empty() {
        return Ok((Vec::new(), HashSet::new()));
    }
    // Rarest-trigram-df first, then stop at the file cap. Scanning every
    // leftover English term and *then* merging is equivalent for the 100
    // files kept, and it was the unique-hybrid p90: common terms still paid
    // a full literal_pass after the shortlist was already full.
    // Precompute sort keys once: the old comparator ran df lookups per
    // comparison (O(n log n) probes) and dragged the df chain into the sort
    // monomorphization (~23KiB of .text). Keys are deterministic within a
    // query and the order is total, so decorate-sort-undecorate is identical.
    let mut keyed: Vec<(bool, i64, &String)> = terms
        .iter()
        .map(|term| {
            let extra = user_first.is_some_and(|user| !user.contains(*term));
            (extra, discovery_df(store, term), *term)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
    terms = keyed.into_iter().map(|(_, _, term)| term).collect();
    // Bundle-backed when the read bundle exists (mmap, ~1MB of file table),
    // cached-only otherwise: the prefilter already handles `None` via the
    // per-term `literal_pass` fallback below.
    let corpus = store.line_corpus_for_query()?;
    // Residency changes the candidate source, never the global quota policy.
    let file_cap = CASCADE_PREFILTER_FILE_LIMIT;
    let mut files = HashSet::new();
    // Files the query's own terms found. Learned borrows may add lexical
    // candidates, but the structural passes constrain to this set when it is
    // non-empty: a borrowed term must not spend the cascade budget that the
    // user's evidence needs (measured: embed-on gold MRR 0.902 -> 0.873 and one
    // structurally exact target fell out of the def pool when borrows widened
    // the constraint).
    let mut user_files = HashSet::new();
    let mut out = Vec::new();
    let extra_reserve = if user_first.is_some() && file_cap > CASCADE_PER_TERM_FILE_LIMIT {
        CASCADE_EXTRA_FILE_RESERVE
            .min(file_cap / 4)
            .min(file_cap.saturating_sub(CASCADE_PER_TERM_FILE_LIMIT))
    } else {
        0
    };
    let user_budget = file_cap.saturating_sub(extra_reserve);
    let mut prefilter_options = options.clone();
    prefilter_options.case_insensitive = true;
    drop(_df_span);
    for term in terms {
        let extra = user_first.is_some_and(|user| !user.contains(term.as_str()));
        let budget = if extra { file_cap } else { user_budget };
        let remaining = budget.saturating_sub(files.len());
        if remaining == 0 {
            continue;
        }
        let term_cap = remaining.min(CASCADE_PER_TERM_FILE_LIMIT);
        let hits = {
            let _span = crate::perf_profile::Span::start(
                "lexical_term_fetch",
                "search",
                "literal_file_pass",
            );
            match corpus.as_ref() {
                Some(corpus) => {
                    let lower = term.to_lowercase();
                    corpus
                        .scan_distinct_files(
                            term,
                            term_cap,
                            &files,
                            options.lang_filter.as_deref(),
                            types::matches_lang,
                            |content| {
                                passes::literal::content_matches_literal(
                                    content,
                                    term,
                                    Some(&lower),
                                    false,
                                )
                            },
                        )
                        .into_iter()
                        .map(|row| {
                            asgrep_line_hit(
                                row.path.to_owned(),
                                row.language.map(str::to_owned),
                                row.line_no,
                                row.content.to_owned(),
                                1.0,
                            )
                        })
                        .collect()
                }
                None => {
                    prefilter_options.limit = term_cap;
                    literal_file_pass(store, &prefilter_options, term, &files)?
                }
            }
        };
        for hit in hits {
            if files.insert(hit.file.clone()) {
                if !extra {
                    user_files.insert(hit.file.clone());
                }
                out.push(hit);
            }
        }
    }
    Ok((out, user_files))
}

pub(crate) fn discovery_df(_store: &IndexStore, _term: &str) -> i64 {
    // SQL has no bundle df (`min_df` = None → MAX). Using baked df here
    // reorders cascade terms on the bundle arm only.
    i64::MAX
}

/// Boost hybrid recall with pre-indexed pattern_nodes (decls/calls extracted at index time).
pub(crate) fn structural_index_pass(
    store: &IndexStore,
    options: &SearchOptions,
    parsed: &ParsedQuery,
    allowed_files: &HashSet<String>,
) -> Result<Vec<SearchHit>> {
    use crate::rank::SCORE_PATTERN;
    use crate::search::types::{HitKind, SpanHitInput};
    let lang = options.lang_filter.as_deref();
    let mut sig_to_term: Vec<(String, String)> = Vec::new();
    for term in &parsed.terms {
        if term.len() < 3 || !term.chars().all(|c| c == '_' || c.is_alphanumeric()) {
            continue;
        }
        for sig in ast_sgrep_lang::structural_term_signatures(term) {
            match sig_to_term.binary_search_by(|(key, _)| key.as_str().cmp(&sig)) {
                Ok(_) => {}
                Err(i) => sig_to_term.insert(i, (sig, term.clone())),
            }
        }
    }
    if sig_to_term.is_empty() {
        return Ok(Vec::new());
    }
    let signatures: Vec<String> = sig_to_term.iter().map(|(sig, _)| sig.clone()).collect();
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    let rows = {
        let _span = crate::perf_profile::Span::start(
            "hybrid_structural_sql",
            "search",
            "pattern_nodes_matching_for_files",
        );
        // Same row budget as the symbol/caller/anchor passes: enough to score
        // inside the 100-file cascade, never the whole node population.
        let pattern_row_budget = options.retained_limit().clamp(32, 500);
        store.pattern_nodes_matching_for_files(
            &signatures,
            lang,
            allowed_files,
            pattern_row_budget,
        )?
    };
    for (row, signature) in rows {
        if !seen.insert((row.path.clone(), row.line_start, row.line_end)) {
            continue;
        }
        let term = sig_to_term
            .binary_search_by(|(key, _)| key.as_str().cmp(&signature))
            .ok()
            .map(|i| sig_to_term[i].1.clone())
            .unwrap_or(signature);
        // Excerpts attach lazily in finish (attach_indexed_excerpts_if_empty) for
        // the survivors only. Fetching one indexed excerpt per pattern node here
        // ran a SQL query for rows fusion then discarded — measured p50 244us /
        // p90 691us inside the search. Pattern nodes store an empty excerpt, so a
        // survivor gets the identical string from the same call, once.
        let excerpt = row.excerpt.clone();
        hits.push(SearchHit::span(SpanHitInput {
            kind: HitKind::Pattern,
            file: row.path,
            line_start: row.line_start,
            line_end: row.line_end,
            score: SCORE_PATTERN * 0.85,
            excerpt,
            symbol: Some(term),
            language: row.language,
            byte_span: None,
        }));
    }
    Ok(hits)
}
