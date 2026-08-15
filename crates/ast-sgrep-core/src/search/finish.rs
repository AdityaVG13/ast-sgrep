use super::fusion::dedup_hits;
use super::types::{
    assign_hit_confidence, assign_signal_margins, HitKind, SearchHit, SearchOptions,
    SearchResponse, SnapshotStamp,
};
use crate::query::{ParsedQuery, QueryMode};
use crate::Result;

const MAX_HITS_PER_FILE: usize = 3;

fn identifier_tokens(symbol: &str) -> Vec<String> {
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
    if symbol.to_lowercase() == snake_spelling {
        3
    } else {
        2
    }
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
    mut hits: Vec<SearchHit>,
    dedup: bool,
) -> Result<SearchResponse> {
    if dedup {
        hits = dedup_hits(hits);
    }
    if let Some(ref filter) = options.file_filter {
        // iva9.2: invalid globs error — never silently skip the filter.
        let re = super::compile_glob(filter).map_err(|e| {
            crate::StoreError::Other(format!("invalid file_filter glob '{filter}': {e}"))
        })?;
        hits.retain(|h| re.is_match(&h.file));
    }
    assign_signal_margins(&mut hits);
    // Confidence is independent of ranking order but must run after margins
    // (which rewrite display `signal` from `kind`) and on every path -- including
    // `dedup=false` (`search_semantic`) where `dedup_hits` never runs (pass5).
    assign_hit_confidence(&mut hits);
    if options.count_only {
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for hit in &hits {
            *counts.entry(hit.file.clone()).or_default() += 1;
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| a.0.cmp(&b.0));
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
            query_expansions: Vec::new(),
        };
        super::record_ledger_from_env(&response);
        return Ok(response);
    }
    let gate_limit = rerank_candidate_limit(options);
    let hybrid = parsed.mode == QueryMode::Hybrid;
    let best_definition = if hybrid {
        hits.iter()
            .filter(|hit| hit.kind == HitKind::Def)
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        definition_query_affinity(parsed, a)
                            .cmp(&definition_query_affinity(parsed, b))
                    })
                    .then_with(|| b.file.cmp(&a.file))
            })
            .cloned()
    } else {
        None
    };
    let keep = if hybrid {
        gate_limit.saturating_mul(MAX_HITS_PER_FILE).max(gate_limit)
    } else {
        gate_limit
    };
    let prune_keep = keep.saturating_mul(4).max(keep.saturating_add(32));
    let multi_term = parsed.terms.len() > 1;
    if hits.len() > prune_keep {
        // Keep coverage in the pre-truncate sort key so high-coverage lower-score
        // hits survive the keep*4 prune (8mb8).
        hits.select_nth_unstable_by(prune_keep, |a, b| {
            cmp_ranked_hits(
                a,
                excerpt_term_coverage(&parsed.terms, a),
                b,
                excerpt_term_coverage(&parsed.terms, b),
                multi_term,
            )
        });
        hits.truncate(prune_keep);
    }
    let mut keyed: Vec<(u32, SearchHit)> = hits
        .into_iter()
        .map(|h| (excerpt_term_coverage(&parsed.terms, &h), h))
        .collect();
    let mut compare = |(ca, a): &(u32, SearchHit), (cb, b): &(u32, SearchHit)| {
        cmp_ranked_hits(a, *ca, b, *cb, multi_term)
    };
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
    if options.use_rerank {
        hits = maybe_rerank(&parsed.raw, hits, options.rerank_top_k);
        hits = enforce_result_gates(hits, parsed.mode, options.limit);
    }
    let (read_bytes_estimate, returned_excerpt_bytes, prevented_read_bytes) =
        super::estimate_prevented_reads(&options.root, &hits);
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
        query_expansions: Vec::new(),
    };
    super::record_ledger_from_env(&response);
    Ok(response)
}
pub(super) fn rerank_candidate_limit(options: &SearchOptions) -> usize {
    if options.use_rerank {
        options.limit.max(options.rerank_top_k)
    } else {
        options.limit
    }
}
fn maybe_rerank(query: &str, hits: Vec<SearchHit>, top_k: usize) -> Vec<SearchHit> {
    if hits.is_empty() {
        return hits;
    }
    let k = top_k.max(1).min(hits.len());
    let docs: Vec<String> = hits
        .iter()
        .take(k)
        .map(|h| {
            format!(
                "{}:{} {}",
                h.file,
                h.line_start,
                h.excerpt.lines().next().unwrap_or("")
            )
        })
        .collect();
    #[cfg(feature = "rerank")]
    {
        match ast_sgrep_embed::rerank(query, &docs) {
            Ok(scores) => {
                return apply_rerank_order(hits, k, scores.into_iter().map(|s| (s.index, s.score)))
            }
            Err(e) => eprintln!("[asgrep] rerank skipped: {e}"),
        }
    }
    #[cfg(not(feature = "rerank"))]
    {
        let _ = (query, &docs);
    }
    hits
}
#[cfg(any(feature = "rerank", test))]
pub(super) fn apply_rerank_order(
    mut hits: Vec<SearchHit>,
    top_k: usize,
    scores: impl IntoIterator<Item = (usize, f32)>,
) -> Vec<SearchHit> {
    let k = top_k.min(hits.len());
    let mut prefix: Vec<Option<SearchHit>> = hits.drain(..k).map(Some).collect();
    let mut seen = vec![false; k];
    let mut ranked: Vec<(f32, usize)> = scores
        .into_iter()
        .filter(|(index, score)| {
            let valid =
                *index < k && score.is_finite() && !seen.get(*index).copied().unwrap_or(true);
            if valid {
                seen[*index] = true;
            }
            valid
        })
        .map(|(index, score)| (score, index))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = Vec::with_capacity(prefix.len() + hits.len());
    out.extend(
        ranked
            .into_iter()
            .filter_map(|(_, index)| prefix[index].take()),
    );
    out.extend(prefix.into_iter().flatten());
    out.append(&mut hits);
    out
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
                hits.insert(head - 1, definition);
            }
        }
    }
    hits.truncate(limit);
    hits
}
fn cap_per_file(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut counts = std::collections::HashMap::new();
    let mut kept = Vec::with_capacity(hits.len());
    let mut overflow = Vec::new();
    for hit in hits {
        let c = counts.entry(hit.file.clone()).or_insert(0);
        if *c < MAX_HITS_PER_FILE {
            *c += 1;
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
    terms
        .iter()
        .filter(|term| {
            // Match term casing: mixed/upper terms stay case-sensitive (hhca).
            if term.chars().any(|c| c.is_uppercase()) {
                contains_term_token(&hit.excerpt, term)
            } else {
                contains_term_token(&hit.excerpt.to_lowercase(), &term.to_lowercase())
            }
        })
        .count() as u32
}
