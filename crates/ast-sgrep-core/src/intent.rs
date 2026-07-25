use crate::query::{ParsedQuery, QueryMode};
use crate::rank::{
    rrf_score, LEXICAL_RRF_SCALE, RRF_K, SCORE_ANCHOR, SCORE_CALLER_BASE, SCORE_DEF_BASE,
    SCORE_EMBED, SCORE_EXACT_SYMBOL, SCORE_GRAPH, SCORE_PATTERN,
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
            let v = v.clamp(0.25, 2.0);
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
fn channel_ceiling(kind: HitKind, term_count: usize) -> f64 {
    let terms = term_count.max(1) as f64;
    match kind {
        // e2hc.14(a): lexical issues ONE OR-query so fuse_rrf never fuses —
        // each hit has a single rank. The max raw score is rrf_score(0, RRF_K)
        // * LEXICAL_RRF_SCALE, NOT terms × that. The old `terms *` multiplier
        // capped every lexical hit at 1/terms, letting noise-floor Embed and
        // binary Graph hits structurally outrank perfect lexical matches on
        // multi-term queries.
        HitKind::Asgrep => rrf_score(0, RRF_K) * LEXICAL_RRF_SCALE,
        HitKind::Def => 2.0 * SCORE_EXACT_SYMBOL * terms + SCORE_DEF_BASE,
        HitKind::Caller => 2.0 * SCORE_EXACT_SYMBOL * terms + SCORE_CALLER_BASE,
        HitKind::Graph => SCORE_GRAPH,
        HitKind::Anchor => SCORE_ANCHOR,
        HitKind::Embed => SCORE_EMBED,
        HitKind::Pattern => SCORE_PATTERN,
        HitKind::Import => 2.0,
    }
}
pub fn route_hits(parsed: &ParsedQuery, hits: &mut [SearchHit]) {
    let w = weights_for(classify(parsed));
    let substantive_terms = parsed
        .terms
        .iter()
        .filter(|term| !term.is_empty())
        .count();
    for hit in hits {
        let text_channel = matches!(
            hit.kind,
            HitKind::Asgrep | HitKind::Def | HitKind::Caller | HitKind::Graph | HitKind::Anchor
        );
        if substantive_terms == 0 && text_channel {
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
        hit.score =
            (hit.score / channel_ceiling(hit.kind, substantive_terms)).clamp(0.0, 1.0) * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{ParsedQuery, QueryMode};
    use crate::rank::{score_def, LEXICAL_RRF_SCALE, RRF_K, SCORE_DEF_BASE, SCORE_EXACT_SYMBOL};
    use crate::search::{HitKind, SearchHit};

    /// e2hc.14(a): The Asgrep (lexical) channel ceiling must NOT scale with
    /// term_count. lexical issues one OR-query so fuse_rrf never fuses — each
    /// hit has a single rank, and the max raw score is rrf_score(0, RRF_K) *
    /// LEXICAL_RRF_SCALE. The old `terms *` multiplier capped every lexical hit
    /// at 1/terms, letting noise-floor Embed and binary Graph hits outrank
    /// perfect lexical matches on multi-term queries.
    #[test]
    fn asgrep_ceiling_does_not_scale_with_term_count() {
        let ceiling_1 = channel_ceiling(HitKind::Asgrep, 1);
        let ceiling_5 = channel_ceiling(HitKind::Asgrep, 5);
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

    /// e2hc.14(c): Single-char symbol queries (e.g. `defs:i`) must not be
    /// zeroed at routing. The old `> 1` filter set substantive_terms=0 for
    /// single-char terms, hitting the `score = 0.0; continue;` path for all
    /// text-channel hits.
    #[test]
    fn route_hits_preserves_single_char_text_channel_hits() {
        let parsed = ParsedQuery {
            raw: "defs:i".into(),
            mode: QueryMode::Defs,
            target: Some("i".into()),
            terms: vec!["i".into()],
        };
        let raw_score = score_def(&["i".to_string()], "i");
        assert!(raw_score > 0.0, "score_def for exact single-char must be > 0");
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
        assert!(
            hits[0].score > 0.0,
            "single-char Def hit must not be zeroed at routing; got score={}",
            hits[0].score
        );
        // The normalized score should be ~1.0 (exact match, 1 term) * weight.
        let ceiling = 2.0 * SCORE_EXACT_SYMBOL + SCORE_DEF_BASE;
        let expected_normalized = (raw_score / ceiling).clamp(0.0, 1.0);
        assert!(
            hits[0].score >= expected_normalized * 0.5,
            "single-char Def hit score {} should be close to normalized {} * weight",
            hits[0].score,
            expected_normalized
        );
    }

    /// e2hc.14(a) regression: multi-term lexical hits must not be capped at
    /// 1/terms. With the fix, a rank-0 lexical hit on a 3-term query should
    /// normalize to ~1.0, not ~1/3.
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
        // With the fix, normalized = raw / ceiling = raw / raw = 1.0, then * weight.
        // Pre-fix, normalized = raw / (3 * raw) = 1/3, then * weight.
        assert!(
            hits[0].score > 0.5,
            "rank-0 lexical hit on 3-term query should normalize near 1.0 * weight, not 1/3; got {}",
            hits[0].score
        );
    }
}
