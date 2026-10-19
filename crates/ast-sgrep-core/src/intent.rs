use crate::query::{ParsedQuery, QueryMode};
use crate::rank::{
    rrf_score, score_symbol, LEXICAL_RRF_SCALE, RRF_K, SCORE_ANCHOR, SCORE_CALLER_BASE,
    SCORE_DEF_BASE, SCORE_EMBED, SCORE_EXACT_SYMBOL, SCORE_GRAPH, SCORE_PATTERN,
};
use crate::search::{HitKind, SearchHit};
use serde::{Deserialize, Serialize};
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
    // `;` is not in this list. A paragraph uses it as a clause separator
    // ("raise an exception; if so"). Treating that as a pattern routed the
    // query around the conceptual exclusion in finish's coverage sort and
    // buried the cosine order. A semicolon counts only when it closes a
    // call or a bracket (`foo();`, `items[i];`).
    const MARKERS: &[&str] = &["{", "=>", "->", "($", "$_", "$$"];
    MARKERS.iter().any(|m| raw.contains(m)) || semicolon_closes_code(raw)
}

fn semicolon_closes_code(raw: &str) -> bool {
    let mut rest = raw;
    while let Some(index) = rest.find(';') {
        let at = raw.len() - rest.len() + index;
        if code_token_before_semicolon(raw, at) {
            return true;
        }
        rest = &rest[index + 1..];
    }
    false
}

fn code_token_before_semicolon(raw: &str, semi: usize) -> bool {
    let Some(last) = raw[..semi].trim_end().chars().next_back() else {
        return false;
    };
    match last {
        ']' | '}' => true,
        ')' => preceding_paren_is_call(raw[..semi].trim_end()),
        _ => false,
    }
}

/// `foo();` is a call. `error (if any);` is a parenthetical: the `(` is
/// not glued to an identifier.
fn preceding_paren_is_call(before_including_paren: &str) -> bool {
    let mut depth = 0i32;
    let mut open_at = None;
    for (index, ch) in before_including_paren.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open_at = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(open) = open_at else {
        return false;
    };
    before_including_paren[..open]
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelWeights {
    pub lexical: f64,
    pub def: f64,
    pub caller: f64,
    pub graph: f64,
    pub anchor: f64,
    pub embed: f64,
    pub pattern: f64,
    pub import: f64,
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
            import: 1.0,
        }
    }
}
pub fn default_weights(intent: QueryIntent) -> ChannelWeights {
    match intent {
        QueryIntent::Conceptual => ChannelWeights {
            lexical: 0.75,
            def: 1.35,
            caller: 0.45,
            graph: 0.25,
            anchor: 0.7,
            embed: 1.45,
            pattern: 0.25,
            import: 0.5,
        },
        QueryIntent::Symbol => ChannelWeights {
            lexical: 0.8,
            def: 2.0,
            caller: 1.0,
            graph: 0.7,
            anchor: 1.0,
            embed: 0.7,
            pattern: 0.25,
            import: 0.8,
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
                "import" => weights.import = v,
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
            let denom = if parsed.identifier_spelling().is_some() {
                (parsed.terms.len().max(1) as f64).max(matched)
            } else {
                matched
            };
            2.0 * SCORE_EXACT_SYMBOL * denom + SCORE_DEF_BASE
        }
        HitKind::Caller => {
            let matched = matched_term_count(parsed, hit.callee.as_deref()).max(1) as f64;
            let denom = if parsed.identifier_spelling().is_some() {
                (parsed.terms.len().max(1) as f64).max(matched)
            } else {
                matched
            };
            2.0 * SCORE_EXACT_SYMBOL * denom + SCORE_CALLER_BASE
        }
        HitKind::Graph => SCORE_GRAPH,
        HitKind::Anchor => SCORE_ANCHOR,
        HitKind::Embed => SCORE_EMBED,
        HitKind::Pattern => SCORE_PATTERN,
        HitKind::Import => 2.0,
    }
}
pub fn route_hits(parsed: &ParsedQuery, hits: &mut [SearchHit]) {
    // Normalize only. Intent channel weights are owned by
    // `fusion::apply_weighted_rrf` so hybrid search does not multiply them twice.
    // Count all non-empty terms, including single-char (a639).
    let substantive_terms = parsed.terms.iter().filter(|term| !term.is_empty()).count();
    for hit in hits {
        let text_channel = matches!(
            hit.kind,
            HitKind::Asgrep | HitKind::Def | HitKind::Caller | HitKind::Graph | HitKind::Anchor
        );
        if substantive_terms == 0 && text_channel {
            hit.score = 0.0;
            continue;
        }
        hit.score = (hit.score / channel_ceiling(parsed, hit)).clamp(0.0, 1.0);
    }
}
