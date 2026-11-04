use super::critic::identifier_tokens;
use super::fusion::{dedup_hits, NameIntern};
use super::types::{
    assign_hit_confidence, assign_signal_margins, HitKind, SearchHit, SearchOptions,
    SearchResponse, SnapshotStamp,
};
use crate::query::{ParsedQuery, QueryMode};
use crate::Result;
use std::collections::HashMap;

/// Hybrid/Regex pages emit each file's BEST hit before any file's second
/// hit (first-occurrence order, overflow backfilled). One hit per file:
/// duplicate rows above gold compress gold's rank, and a page of N rows
/// covers up to N distinct files (frozen-suite evidence 2026-10-29:
/// repoqa MRR@10 .7219 -> .7421, file-diversity@50 .469 -> 1.000 at cap 1
/// vs cap 3; cosqa bit-identical).
const MAX_HITS_PER_FILE: usize = 1;

/// Pre-gate headroom kept at 3x the gate limit independent of the page
/// cap: with cap 1 the gate must still see enough ranked rows to fill
/// `limit` distinct-first slots before truncation.
const GATE_HEADROOM: usize = 3;

pub(super) fn definition_query_affinity(parsed: &ParsedQuery, hit: &SearchHit) -> u8 {
    let Some(symbol) = hit.symbol.as_deref() else {
        return 0;
    };
    let symbol_tokens = identifier_tokens(symbol);
    if symbol_tokens.is_empty() || symbol_tokens.len() > parsed.terms.len() {
        return 0;
    }
    let matches_boundary = parsed.terms.windows(symbol_tokens.len()).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(symbol_tokens.iter().map(String::as_str))
    });
    if !matches_boundary {
        return 0;
    }
    let snake_spelling = symbol_tokens.join("_");
    let mut affinity: u8 = if symbol.to_lowercase() == snake_spelling {
        3
    } else {
        2
    };
    if parsed
        .identifier_spelling()
        .is_some_and(|spelling| spelling == symbol)
    {
        affinity = affinity.saturating_add(2);
    }
    affinity
}

/// Shared ranking key for pre-truncate prune and final sort in `finish_response`.
/// Multi-term queries prefer coverage so high-coverage lower-score evidence is retained (8mb8).
fn cmp_ranked_hits(
    a: &SearchHit,
    coverage_a: u32,
    b: &SearchHit,
    coverage_b: u32,
    multi_term: bool,
) -> std::cmp::Ordering {
    let score_ord = b
        .score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal);
    let coverage_ord = coverage_b.cmp(&coverage_a);
    let primary = if multi_term {
        coverage_ord.then(score_ord)
    } else {
        score_ord.then(coverage_ord)
    };
    primary
        .then_with(|| a.file.cmp(&b.file))
        .then_with(|| a.line_start.cmp(&b.line_start))
        // Total-order tail: sort_unstable_by is NOT stable and the input
        // order feeding it comes from a randomly seeded HashMap
        // (lexical_from_fts), so any residual Equal flips hit order between
        // processes and breaks the documented cross-process byte-stability
        // contract. These arms evaluate only on exact upstream ties.
        .then_with(|| a.line_end.cmp(&b.line_end))
        .then_with(|| a.symbol.cmp(&b.symbol))
        .then_with(|| a.caller.cmp(&b.caller))
        .then_with(|| a.callee.cmp(&b.callee))
        .then_with(|| a.excerpt.cmp(&b.excerpt))
}

fn same_definition_locus(hit: &SearchHit, definition: &SearchHit) -> bool {
    hit.kind == HitKind::Def
        && hit.file == definition.file
        && hit.line_start == definition.line_start
        && hit.symbol == definition.symbol
}

/// Preserve the pre-1.3 non-fallible response API. Invalid globs keep the
/// legacy behavior and are ignored; internal search paths use the checked API.
pub fn finish_response(
    parsed: &ParsedQuery,
    options: &SearchOptions,
    hits: Vec<SearchHit>,
    dedup: bool,
) -> SearchResponse {
    let mut compatibility_options = options.clone();
    if compatibility_options
        .file_filter
        .as_ref()
        .is_some_and(|filter| super::compile_glob(filter).is_err())
    {
        compatibility_options.file_filter = None;
    }
    finish_response_checked(parsed, &compatibility_options, hits, dedup)
        .expect("compatibility options were validated")
}

pub(crate) fn finish_response_checked(
    parsed: &ParsedQuery,
    options: &SearchOptions,
    hits: Vec<SearchHit>,
    dedup: bool,
) -> Result<SearchResponse> {
    finish_response_checked_lazy(parsed, options, hits, dedup, None, false)
}

/// Apply the same path contract before channel joins and final ranking.
/// An explicit file filter takes precedence over a resolved query scope.
pub(super) fn filter_hit_paths(
    parsed: &ParsedQuery,
    options: &SearchOptions,
    hits: &mut Vec<SearchHit>,
) -> Result<()> {
    if let Some(ref filter) = options.file_filter {
        // Invalid globs error -- never silently skip the filter.
        let re = super::compile_glob(filter).map_err(|e| {
            crate::StoreError::Other(format!("invalid file_filter glob '{filter}': {e}"))
        })?;
        hits.retain(|h| re.is_match(&h.file));
    } else if let Some(scope) = parsed.path_scope.as_deref() {
        let glob = if parsed.path_scope_exact {
            scope.to_string()
        } else {
            crate::query::path_scope_glob(scope)
        };
        let re = super::compile_glob(&glob).map_err(|e| {
            crate::StoreError::Other(format!("invalid in: path scope '{scope}': {e}"))
        })?;
        hits.retain(|h| re.is_match(&h.file));
    }
    Ok(())
}

/// Variant that defers per-hit excerpt SQL out of the channel passes.
/// `lazy_excerpt_store` is the index whose `attach_indexed_
/// excerpts` fills empty excerpts AFTER dedup/margins/confidence/best_def
/// (none of which read excerpts) and BEFORE the coverage prune (which does).
/// Channel passes marked lazy skip their own attachment; hits removed by
/// dedup/filter before attachment never cost an excerpt fetch.
pub(crate) fn finish_response_checked_lazy(
    parsed: &ParsedQuery,
    options: &SearchOptions,
    mut hits: Vec<SearchHit>,
    dedup: bool,
    lazy_excerpt_store: Option<&crate::store::IndexStore>,
    skip_fs_byte_estimates: bool,
) -> Result<SearchResponse> {
    if dedup {
        hits = dedup_hits(hits);
    }
    filter_hit_paths(parsed, options, &mut hits)?;
    assign_signal_margins(&mut hits);
    // Confidence is independent of ranking order but must run after margins
    // (which rewrite display `signal` from `kind`) and on every path -- including
    // `dedup=false` (`search_semantic`) where `dedup_hits` never runs (pass5).
    assign_hit_confidence(&mut hits);
    if options.count_only {
        let mut files: Vec<&str> = hits.iter().map(|hit| hit.file.as_str()).collect();
        files.sort_unstable();
        let mut counts: Vec<(String, u32)> = Vec::new();
        let mut i = 0;
        while i < files.len() {
            let name = files[i];
            let start = i;
            i += 1;
            while i < files.len() && files[i] == name {
                i += 1;
            }
            counts.push((name.to_string(), (i - start) as u32));
        }
        let response = SearchResponse {
            query: parsed.raw.clone(),
            limit: options.limit,
            hits: vec![],
            counts,
            read_bytes_estimate: 0,
            returned_excerpt_bytes: 0,
            prevented_read_bytes: 0,
            // Stamped by the Searcher, which owns the snapshot (d3l5).
            snapshot: SnapshotStamp::default(),
            // Count mode aggregates every hit pre-gates; nothing is cut.
            truncated: false,
            query_expansions: Vec::new(),
        };
        super::record_ledger_from_env(&response);
        return Ok(response);
    }
    let gate_limit = options.retained_limit();
    // br-tvv: hits are intact here (post dedup/filter, pre ranking gates).
    // Any len above the gate budget here means the envelope cannot show the
    // full answer set — the flag must say so.
    let pre_gate_count = hits.len();
    let hybrid = parsed.mode == QueryMode::Hybrid;
    let best_definition_index = if hybrid {
        hits.iter()
            .enumerate()
            .filter(|(_, hit)| hit.kind == HitKind::Def)
            .max_by(|(_, a), (_, b)| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        definition_query_affinity(parsed, a)
                            .cmp(&definition_query_affinity(parsed, b))
                    })
                    // Rescue must share the normal result-order tail. File
                    // alone leaves same-file definitions dependent on channel order.
                    .then_with(|| cmp_ranked_hits(b, 0, a, 0, false))
            })
            .map(|(index, _)| index)
    } else {
        None
    };
    // Fill deferred structural excerpts after the stages that ignore
    // them and before the first excerpt-dependent prune. Hydration changes
    // fields, not indices: clone the selected definition only after its text
    // is attached so rescue cannot publish stale empty evidence.
    if let Some(store) = lazy_excerpt_store {
        crate::search::passes::symbol::attach_indexed_excerpts_if_empty(store, &mut hits)?;
    }
    let best_definition = best_definition_index.map(|index| hits[index].clone());
    let keep = if hybrid {
        gate_limit.saturating_mul(GATE_HEADROOM).max(gate_limit)
    } else {
        gate_limit
    };
    let prune_keep = keep.saturating_mul(4).max(keep.saturating_add(32));
    // Conceptual NL and identifier queries: docs that repeat the split
    // tokens have perfect coverage and would otherwise bury the definition
    // (`auth_refresh` must not lose to a comment that says "auth refresh").
    let intent = crate::intent::classify(parsed);
    let multi_term = parsed.terms.len() > 1
        && !matches!(
            intent,
            crate::intent::QueryIntent::Conceptual | crate::intent::QueryIntent::Symbol
        );
    // Coverage is a pure function of (terms, excerpt). Compute once per hit so
    // select_nth / sort do not re-lowercase excerpts on every comparison.
    let mut keyed: Vec<(u32, SearchHit)> = hits
        .into_iter()
        .map(|h| (excerpt_term_coverage(&parsed.terms, &h), h))
        .collect();
    // Structural hybrid still coverage-sorts, so token-parrot decoys fill a
    // short page and drop the synonym embed. Rescue the min-coverage embed
    // file (the one that did not parrot query tokens). A "below max" pick
    // still prefers a slightly-less-parrot decoy by cosine.
    let structural_embed_rescue = if matches!(intent, crate::intent::QueryIntent::Structural)
        && options.use_embed
    {
        // Token-parrot files have high coverage on *some* row (usually the
        // Asgrep comment). The synonym file's rows stay near zero coverage;
        // that is the 2D pin. Per-row coverage on the Def/Embed excerpt is
        // often zero for both, so the file max is the discriminator.
        let mut file_cov: HashMap<&str, u32> = HashMap::with_capacity(keyed.len());
        for (cov, hit) in &keyed {
            file_cov
                .entry(hit.file.as_str())
                .and_modify(|seen| *seen = (*seen).max(*cov))
                .or_insert(*cov);
        }
        let max_file_cov = file_cov.values().copied().max().unwrap_or(0);
        let min_file_cov = file_cov.values().copied().min().unwrap_or(0);
        if min_file_cov >= max_file_cov {
            None
        } else {
            keyed
                .iter()
                .filter(|(_, hit)| {
                    (hit.kind == HitKind::Embed
                        || hit.contributors.iter().any(|k| *k == HitKind::Embed))
                        && file_cov
                            .get(hit.file.as_str())
                            .is_some_and(|c| *c == min_file_cov)
                })
                .max_by(|left, right| {
                    left.1
                        .score
                        .partial_cmp(&right.1.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| left.1.file.cmp(&right.1.file))
                        .then_with(|| left.1.line_start.cmp(&right.1.line_start))
                })
                .map(|(_, hit)| hit.clone())
        }
    } else {
        None
    };
    let mut compare = |(ca, a): &(u32, SearchHit), (cb, b): &(u32, SearchHit)| {
        cmp_ranked_hits(a, *ca, b, *cb, multi_term)
    };
    if keyed.len() > prune_keep {
        keyed.select_nth_unstable_by(prune_keep, &mut compare);
        keyed.truncate(prune_keep);
    }
    if keyed.len() > keep {
        keyed.select_nth_unstable_by(keep, &mut compare);
        keyed.truncate(keep);
    }
    keyed.sort_unstable_by(compare);
    let mut hits: Vec<_> = keyed.into_iter().map(|(_, h)| h).collect();
    if let Some(definition) = best_definition {
        let retained = hits
            .iter()
            .any(|hit| same_definition_locus(hit, &definition));
        if !retained {
            hits.push(definition);
        }
    }
    hits = enforce_result_gates(hits, parsed.mode, gate_limit);
    if let Some(embed) = structural_embed_rescue {
        // Cap is one row per file. Same-file, different line must not
        // re-enter as a second slot; attach embed provenance instead.
        // If that row sits past rank 2, swap it into the head so a later
        // tighter clamp (MCP/codemode open_limit → request) still sees it.
        let steal_head = |hits: &[SearchHit]| {
            (0..hits.len().min(2))
                .rev()
                .find(|&i| hits[i].kind != HitKind::Def)
                .or_else(|| hits.iter().position(|hit| hit.kind != HitKind::Def))
        };
        if let Some(pos) = hits.iter().position(|hit| hit.file == embed.file) {
            if !hits[pos].contributors.iter().any(|k| *k == HitKind::Embed) {
                hits[pos].contributors.push(HitKind::Embed);
            }
            if hits[pos].symbol.is_none() {
                hits[pos].symbol = embed.symbol.clone();
            }
            if pos >= 2 {
                if let Some(steal) = steal_head(&hits) {
                    hits.swap(steal, pos);
                }
            }
        } else if let Some(index) = steal_head(&hits).or_else(|| hits.len().checked_sub(1)) {
            hits[index] = embed;
        } else {
            hits.push(embed);
        }
    }
    let (read_bytes_estimate, returned_excerpt_bytes, prevented_read_bytes) =
        if skip_fs_byte_estimates {
            let returned = hits.iter().map(|h| h.excerpt.len() as u64).sum();
            (0, returned, 0)
        } else {
            super::estimate_prevented_reads(&options.root, &hits)
        };
    let response = SearchResponse {
        query: parsed.raw.clone(),
        limit: options.limit,
        hits,
        counts: vec![],
        read_bytes_estimate,
        returned_excerpt_bytes,
        prevented_read_bytes,
        // Stamped by the Searcher, which owns the snapshot (d3l5).
        snapshot: SnapshotStamp::default(),
        truncated: pre_gate_count > gate_limit,
        query_expansions: Vec::new(),
    };
    super::record_ledger_from_env(&response);
    Ok(response)
}
pub(super) fn enforce_result_gates(
    mut hits: Vec<SearchHit>,
    mode: QueryMode,
    limit: usize,
) -> Vec<SearchHit> {
    if matches!(mode, QueryMode::Hybrid | QueryMode::Regex) {
        hits = cap_per_file(hits);
    }
    if mode == QueryMode::Hybrid {
        let preferred_definition = hits.iter().find(|hit| hit.kind == HitKind::Def).cloned();
        let head = limit.min(hits.len());
        if head > 0 && !hits[..head].iter().any(|hit| hit.kind == HitKind::Def) {
            if let Some(definition) = preferred_definition {
                if let Some(index) = hits
                    .iter()
                    .position(|hit| same_definition_locus(hit, &definition))
                {
                    hits.remove(index);
                }
                // Rank 2 (or the sole slot): on the page, and still there
                // after a later tighter clamp (MCP open_limit → request).
                // `head - 1` is the last row of a wide page, which that cut drops.
                let insert_at = 1.min(head.saturating_sub(1));
                hits.insert(insert_at, definition);
            }
        }
    }
    hits.truncate(limit);
    hits
}
fn cap_per_file(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut intern = NameIntern::new(hits.len());
    let mut counts: Vec<usize> = Vec::new();
    let mut kept = Vec::with_capacity(hits.len());
    let mut overflow = Vec::new();
    for hit in hits {
        let id = intern.intern(&hit.file) as usize;
        if counts.len() <= id {
            counts.resize(id + 1, 0);
        }
        if counts[id] < MAX_HITS_PER_FILE {
            counts[id] += 1;
            kept.push(hit);
        } else {
            overflow.push(hit);
        }
    }
    kept.extend(overflow);
    kept
}
fn contains_term_token(text: &str, term: &str) -> bool {
    !term.is_empty()
        && text.match_indices(term).any(|(start, matched)| {
            let before = text[..start].chars().next_back();
            let after = text[start + matched.len()..].chars().next();
            before.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
                && after.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
        })
}
pub(super) fn excerpt_term_coverage(terms: &[String], hit: &SearchHit) -> u32 {
    if terms.is_empty() {
        return 0;
    }
    let mut excerpt_lower: Option<String> = None;
    terms
        .iter()
        .filter(|term| {
            // Match term casing: mixed/upper terms stay case-sensitive (hhca).
            if term.chars().any(|c| c.is_uppercase()) {
                contains_term_token(&hit.excerpt, term)
            } else {
                let lowered = excerpt_lower.get_or_insert_with(|| hit.excerpt.to_lowercase());
                contains_term_token(lowered, &term.to_lowercase())
            }
        })
        .count() as u32
}
