use crate::query::{ParsedQuery, QueryMode};
use crate::rank::{
    rrf_score, score_symbol, LEXICAL_RRF_SCALE, RRF_K, SCORE_ANCHOR, SCORE_CALLER_BASE,
    SCORE_DEF_BASE, SCORE_EMBED, SCORE_EXACT_SYMBOL, SCORE_GRAPH, SCORE_PATTERN,
};
use crate::search::{HitKind, SearchHit};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryIntent {
    Literal,
    Symbol,
    Structural,
    Conceptual,
}
impl QueryIntent {
    pub fn as_str(self) -> &'static str {
        match self {
            QueryIntent::Literal => "literal",
            QueryIntent::Symbol => "symbol",
            QueryIntent::Structural => "structural",
            QueryIntent::Conceptual => "conceptual",
        }
    }
}
pub fn classify(parsed: &ParsedQuery) -> QueryIntent {
    match parsed.mode {
        QueryMode::Defs | QueryMode::Callers | QueryMode::Imports => QueryIntent::Symbol,
        QueryMode::Pattern => QueryIntent::Structural,
        QueryMode::Literal | QueryMode::Word | QueryMode::Regex => QueryIntent::Literal,
        QueryMode::Hybrid => classify_hybrid(&parsed.raw),
    }
}
fn classify_hybrid(raw: &str) -> QueryIntent {
    let t = raw.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return QueryIntent::Literal;
    }
    if looks_structural(t) {
        return QueryIntent::Structural;
    }
    let tokens: Vec<&str> = t.split_whitespace().collect();
    let idents = tokens
        .iter()
        .filter(|x| ident_like(x) || title_case(x))
        .count();
    if !tokens.is_empty() && tokens.len() <= 2 && idents > 0 {
        QueryIntent::Symbol
    } else {
        QueryIntent::Conceptual
    }
}
fn title_case(token: &str) -> bool {
    let mut chars = token.chars();
    chars.next().is_some_and(|c| c.is_uppercase())
        && token.chars().skip(1).any(|c| c.is_lowercase())
        && token.chars().all(|c| c.is_alphanumeric())
}
fn looks_structural(raw: &str) -> bool {
    raw.contains('{')
        || raw.contains(';')
        || raw.contains("=>")
        || raw.contains("->")
        || raw.contains("($")
        || raw.contains("$_")
        || raw.contains("$$")
}
fn ident_like(token: &str) -> bool {
    if token.contains("::") || token.contains('_') || token.ends_with("()") {
        return true;
    }
    let mut prev_lower = false;
    for c in token.chars() {
        if prev_lower && c.is_uppercase() {
            return true;
        }
        prev_lower = c.is_lowercase();
    }
    false
}
#[derive(Debug, Clone, Copy)]
pub struct ChannelWeights {
    pub lexical: f64,
    pub def: f64,
    pub caller: f64,
    pub graph: f64,
    pub anchor: f64,
    pub embed: f64,
    pub pattern: f64,
}
impl Default for ChannelWeights {
    fn default() -> Self {
        Self {
            lexical: 1.0,
            def: 1.0,
            caller: 1.0,
            graph: 1.0,
            anchor: 1.0,
            embed: 1.0,
            pattern: 1.0,
        }
    }
}
pub fn default_weights(intent: QueryIntent) -> ChannelWeights {
    match intent {
        QueryIntent::Conceptual => ChannelWeights {
            lexical: 1.1,
            def: 0.9,
            caller: 0.8,
            graph: 0.7,
            anchor: 0.8,
            embed: 1.1,
            pattern: 0.1,
        },
        _ => ChannelWeights::default(),
    }
}
pub fn weights_for(intent: QueryIntent) -> ChannelWeights {
    let mut w = default_weights(intent);
    if let Ok(spec) = std::env::var("ASGREP_INTENT_WEIGHTS") {
        apply_spec(&mut w, intent, &spec);
    }
    w
}
fn clamp_channel_weight(v: f64) -> f64 {
    v.clamp(0.25, 2.0)
}
fn apply_spec(weights: &mut ChannelWeights, intent: QueryIntent, spec: &str) {
    for class_spec in spec.split(';') {
        let Some((class, pairs)) = class_spec.split_once(':') else {
            continue;
        };
        if class.trim() != intent.as_str() {
            continue;
        }
        for pair in pairs.split(',') {
            let Some((ch, value)) = pair.split_once('=') else {
                continue;
            };
            let Ok(v) = value.trim().parse::<f64>() else {
                continue;
            };
            if !v.is_finite() {
                continue;
            }
            let v = clamp_channel_weight(v);
            match ch.trim() {
                "lexical" => weights.lexical = v,
                "def" => weights.def = v,
                "caller" => weights.caller = v,
                "graph" => weights.graph = v,
                "anchor" => weights.anchor = v,
                "embed" => weights.embed = v,
                "pattern" => weights.pattern = v,
                _ => {}
            }
        }
    }
}
fn matched_term_count(parsed: &ParsedQuery, target: Option<&str>) -> usize {
    target
        .map(|target| {
            parsed
                .terms
                .iter()
                .filter(|term| score_symbol(term, target) > 0.0)
                .count()
        })
        .unwrap_or(0)
}

fn channel_ceiling(parsed: &ParsedQuery, hit: &SearchHit) -> f64 {
    match hit.kind {
        // e2hc.14(a): lexical issues ONE OR-query so fuse_rrf never fuses —
        // each hit has a single rank. The max raw score is rrf_score(0, RRF_K)
        // * LEXICAL_RRF_SCALE, NOT terms × that. The old `terms *` multiplier
        // capped every lexical hit at 1/terms, letting noise-floor Embed and
        // binary Graph hits structurally outrank perfect lexical matches on
        // multi-term queries.
        HitKind::Asgrep => rrf_score(0, RRF_K) * LEXICAL_RRF_SCALE,
        // Def/Caller producers sum only terms that match this hit's target.
        // Normalize against the same matched-term set so unrelated query context
        // cannot dilute evidence, while exact matches still outrank substrings.
        HitKind::Def => {
            let matched = matched_term_count(parsed, hit.symbol.as_deref()).max(1) as f64;
            2.0 * SCORE_EXACT_SYMBOL * matched + SCORE_DEF_BASE
        }
        HitKind::Caller => {
            let matched = matched_term_count(parsed, hit.callee.as_deref()).max(1) as f64;
            2.0 * SCORE_EXACT_SYMBOL * matched + SCORE_CALLER_BASE
        }
        HitKind::Graph => SCORE_GRAPH,
        HitKind::Anchor => SCORE_ANCHOR,
        HitKind::Embed => SCORE_EMBED,
        HitKind::Pattern => SCORE_PATTERN,
        HitKind::Import => 2.0,
    }
}
pub fn route_hits(parsed: &ParsedQuery, hits: &mut [SearchHit]) {
    let w = weights_for(classify(parsed));
    // Count nonempty terms (single-char queries like hybrid "i" must remain live).
    let nonempty_terms = parsed.terms.iter().filter(|term| !term.is_empty()).count();
    for hit in hits {
        let text_channel = matches!(
            hit.kind,
            HitKind::Asgrep | HitKind::Def | HitKind::Caller | HitKind::Graph | HitKind::Anchor
        );
        if nonempty_terms == 0 && text_channel {
            hit.score = 0.0;
            continue;
        }
        let weight = match hit.kind {
            HitKind::Asgrep => w.lexical,
            HitKind::Def => w.def,
            HitKind::Caller => w.caller,
            HitKind::Graph => w.graph,
            HitKind::Anchor => w.anchor,
            HitKind::Embed => w.embed,
            HitKind::Pattern => w.pattern,
            HitKind::Import => 1.0,
        };
        hit.score = (hit.score / channel_ceiling(parsed, hit)).clamp(0.0, 1.0) * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{ParsedQuery, QueryMode};
    use crate::rank::{score_def, LEXICAL_RRF_SCALE, RRF_K};
    use crate::search::{HitKind, SearchHit};

    /// e2hc.14(a): The Asgrep (lexical) channel ceiling must NOT scale with
    /// term_count. lexical issues one OR-query so fuse_rrf never fuses — each
    /// hit has a single rank, and the max raw score is rrf_score(0, RRF_K) *
    /// LEXICAL_RRF_SCALE. The old `terms *` multiplier capped every lexical hit
    /// at 1/terms, letting noise-floor Embed and binary Graph hits outrank
    /// perfect lexical matches on multi-term queries.
    #[test]
    fn asgrep_ceiling_does_not_scale_with_term_count() {
        let parsed_1 = ParsedQuery {
            raw: "foo".into(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: vec!["foo".into()],
        };
        let parsed_5 = ParsedQuery {
            raw: "foo bar baz qux quux".into(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: vec![
                "foo".into(),
                "bar".into(),
                "baz".into(),
                "qux".into(),
                "quux".into(),
            ],
        };
        let lexical_hit = SearchHit {
            kind: HitKind::Asgrep,
            file: "test.rs".into(),
            line_start: 1,
            line_end: 1,
            symbol: None,
            caller: None,
            callee: None,
            language: None,
            score: 1.0,
            excerpt: "foo".into(),
        };
        let ceiling_1 = channel_ceiling(&parsed_1, &lexical_hit);
        let ceiling_5 = channel_ceiling(&parsed_5, &lexical_hit);
        assert_eq!(
            ceiling_1, ceiling_5,
            "Asgrep ceiling must be term-count-independent"
        );
        assert_eq!(
            ceiling_1,
            rrf_score(0, RRF_K) * LEXICAL_RRF_SCALE,
            "Asgrep ceiling must equal the max single-rank raw score"
        );
    }

    /// e2hc.14(c): Hybrid single-char queries (e.g. `"i"`) must not zero text
    /// channels at routing. The old `chars().count() > 1` filter set the term
    /// count to 0 for single-char tokens and dropped every text-channel hit.
    #[test]
    fn route_hits_preserves_single_char_text_channel_hits() {
        let parsed = ParsedQuery {
            raw: "i".into(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: vec!["i".into()],
        };
        let raw_score = score_def(&["i".to_string()], "i");
        assert!(
            raw_score > 0.0,
            "score_def for exact single-char must be > 0"
        );
        let mut hits = vec![SearchHit {
            kind: HitKind::Def,
            file: "test.rs".into(),
            line_start: 1,
            line_end: 1,
            symbol: Some("i".into()),
            caller: None,
            callee: None,
            language: None,
            score: raw_score,
            excerpt: "fn i() {}".into(),
        }];
        route_hits(&parsed, &mut hits);
        let weight = weights_for(classify(&parsed)).def;
        let expected = (raw_score / channel_ceiling(&parsed, &hits[0])).clamp(0.0, 1.0) * weight;
        assert!(
            (hits[0].score - expected).abs() < 1e-12,
            "single-char Def hit score {} must equal normalized*weight {}",
            hits[0].score,
            expected
        );
    }

    /// Adding unrelated query context must not reduce exact symbol evidence.
    #[test]
    fn def_and_caller_scores_ignore_unmatched_query_context() {
        let focused = ParsedQuery {
            raw: "foo".into(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: vec!["foo".into()],
        };
        let expanded = ParsedQuery {
            raw: "foo noise junk".into(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: vec!["foo".into(), "noise".into(), "junk".into()],
        };
        for kind in [HitKind::Def, HitKind::Caller] {
            let raw_score = match kind {
                HitKind::Def => crate::rank::score_def(&focused.terms, "foo"),
                HitKind::Caller => crate::rank::score_caller(&focused.terms, "foo"),
                _ => unreachable!(),
            };
            let make_hit = || SearchHit {
                kind,
                file: "test.rs".into(),
                line_start: 1,
                line_end: 1,
                symbol: (kind == HitKind::Def).then(|| "foo".into()),
                caller: (kind == HitKind::Caller).then(|| "caller".into()),
                callee: (kind == HitKind::Caller).then(|| "foo".into()),
                language: None,
                score: raw_score,
                excerpt: "foo".into(),
            };
            let mut focused_hits = vec![make_hit()];
            let mut expanded_hits = vec![make_hit()];
            route_hits(&focused, &mut focused_hits);
            route_hits(&expanded, &mut expanded_hits);
            assert_eq!(
                focused_hits[0].score, expanded_hits[0].score,
                "{kind:?} score must not fall when only unmatched terms are added"
            );
        }
    }

    #[test]
    fn exact_def_match_still_outranks_substring_match() {
        let parsed = ParsedQuery {
            raw: "foo".into(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: vec!["foo".into()],
        };
        let mut hits = [
            SearchHit {
                kind: HitKind::Def,
                file: "exact.rs".into(),
                line_start: 1,
                line_end: 1,
                symbol: Some("foo".into()),
                caller: None,
                callee: None,
                language: None,
                score: crate::rank::score_def(&parsed.terms, "foo"),
                excerpt: "foo".into(),
            },
            SearchHit {
                kind: HitKind::Def,
                file: "substring.rs".into(),
                line_start: 1,
                line_end: 1,
                symbol: Some("foobar".into()),
                caller: None,
                callee: None,
                language: None,
                score: crate::rank::score_def(&parsed.terms, "foobar"),
                excerpt: "foobar".into(),
            },
        ];
        route_hits(&parsed, &mut hits);
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn multi_term_lexical_hit_not_capped_at_inverse_terms() {
        let parsed = ParsedQuery {
            raw: "foo bar baz".into(),
            mode: QueryMode::Hybrid,
            target: None,
            terms: vec!["foo".into(), "bar".into(), "baz".into()],
        };
        let raw_score = rrf_score(0, RRF_K) * LEXICAL_RRF_SCALE;
        let mut hits = vec![SearchHit {
            kind: HitKind::Asgrep,
            file: "test.rs".into(),
            line_start: 1,
            line_end: 1,
            symbol: None,
            caller: None,
            callee: None,
            language: None,
            score: raw_score,
            excerpt: "foo bar baz".into(),
        }];
        route_hits(&parsed, &mut hits);
        // With the fix, normalized = raw / ceiling = 1.0, then * weight.
        // Pre-fix, normalized = raw / (3 * raw) = 1/3, then * weight.
        let expected = weights_for(classify(&parsed)).lexical;
        assert!(
            (hits[0].score - expected).abs() < 1e-12,
            "rank-0 lexical hit on 3-term query should equal 1.0 * weight ({expected}); got {}",
            hits[0].score
        );
    }
}
