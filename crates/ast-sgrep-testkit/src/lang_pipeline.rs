//! Lang error-pipeline kit: detect -> parse/extract -> match -> score -> rank.
//!
//! # Contract
//!
//! - One canonical copy of the full-pipeline runner formerly local to the
//!   lang error-API drills: every downstream error drill runs hostile inputs
//!   through [`run_pipeline`], never a fork.
//! - Score/rank live downstream of `ast-sgrep-lang`, so the kit runs a small
//!   deterministic in-test score+rank lane ([`score_hit`] / [`rank_hits`])
//!   over match hits. Deterministic: fixed weights, total tie-break, no
//!   wall-clock, no randomness.
//! - [`run_pipeline`] returns `None` exactly when detection yields `None`
//!   (short-circuit: no parse, no match, no ranked output — the documented
//!   unsupported outcome). Parse/match failures also short-circuit to `None`.
//! - Assert helpers panic (never `Result`) on unsound output, matching suite
//!   convention: invented/dropped hits or out-of-range spans are test
//!   failures, not fallible ops.

use ast_sgrep_lang::{detect_language, match_pattern, ParserRegistry, PatternMatch};
use std::path::Path;

/// INTENT: deterministic in-test hit score (downstream stand-in): excerpt
/// weight + capture count + span length. Pure projection.
pub fn score_hit(hit: &PatternMatch) -> u64 {
    hit.excerpt.len() as u64 * 2
        + hit.captures.len() as u64
        + hit.byte_end.saturating_sub(hit.byte_start) as u64
}

/// INTENT: deterministic rank — score descending, then byte-start, byte-end,
/// excerpt ascending (total tie-break). Pure projection.
pub fn rank_hits(mut hits: Vec<PatternMatch>) -> Vec<PatternMatch> {
    hits.sort_by(|a, b| {
        score_hit(b)
            .cmp(&score_hit(a))
            .then(a.byte_start.cmp(&b.byte_start))
            .then(a.byte_end.cmp(&b.byte_end))
            .then(a.excerpt.cmp(&b.excerpt))
    });
    hits
}

/// INTENT: ranked output must be exactly the hits, ordered: same length,
/// same multiset of hit keys, scores non-increasing, deterministic
/// tie-break. Panics on invented/dropped/reordered output.
pub fn assert_rank_sound(ranked: &[PatternMatch], hits: &[PatternMatch]) {
    assert_eq!(ranked.len(), hits.len());
    let mut ranked_keys: Vec<(usize, usize, &str)> = ranked
        .iter()
        .map(|h| (h.byte_start, h.byte_end, h.excerpt.as_str()))
        .collect();
    let mut hit_keys: Vec<(usize, usize, &str)> = hits
        .iter()
        .map(|h| (h.byte_start, h.byte_end, h.excerpt.as_str()))
        .collect();
    ranked_keys.sort();
    hit_keys.sort();
    assert_eq!(ranked_keys, hit_keys);
    for pair in ranked.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let order = score_hit(b)
            .cmp(&score_hit(a))
            .then(a.byte_start.cmp(&b.byte_start))
            .then(a.byte_end.cmp(&b.byte_end))
            .then(a.excerpt.cmp(&b.excerpt));
        assert!(order != std::cmp::Ordering::Greater);
    }
}

/// INTENT: every ranked span must point inside the source it was matched
/// against (`byte_start <= byte_end <= len`, addressable). Panics on
/// out-of-range spans.
pub fn assert_spans_in_source(ranked: &[PatternMatch], source: &str) {
    for hit in ranked {
        assert!(hit.byte_start <= hit.byte_end);
        assert!(hit.byte_end <= source.len());
        assert!(source.get(hit.byte_start..hit.byte_end).is_some());
    }
}

/// INTENT: full-pipeline outcome — extraction flags plus the ranked hits.
pub struct PipelineOutcome {
    pub depth_truncated: bool,
    pub rows_empty: bool,
    pub ranked: Vec<PatternMatch>,
}

/// INTENT: full-pipeline runner: detect -> parse -> match -> score -> rank.
/// Returns `None` exactly when detection yields `None` (short-circuit: no
/// parse, no match, no ranked output — the documented unsupported outcome).
pub fn run_pipeline(
    registry: &ParserRegistry,
    path: &str,
    content: &str,
    pattern: &str,
) -> Option<PipelineOutcome> {
    let lang = detect_language(Path::new(path), Some(content))?;
    let extraction = registry.parse(lang, content).ok()?;
    let hits = match_pattern(lang, content, pattern).ok()?;
    let ranked = rank_hits(hits);
    Some(PipelineOutcome {
        depth_truncated: extraction.depth_truncated,
        rows_empty: extraction.symbols.is_empty()
            && extraction.calls.is_empty()
            && extraction.imports.is_empty(),
        ranked,
    })
}
