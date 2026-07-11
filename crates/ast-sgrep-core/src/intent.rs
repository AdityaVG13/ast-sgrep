use crate::query::{ParsedQuery, QueryMode};
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
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') { return QueryIntent::Literal; }
    if looks_structural(t) { return QueryIntent::Structural; }
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
    if token.contains("::") || token.contains('_') || token.ends_with("()") { return true; }
    let mut prev_lower = false;
    for c in token.chars() {
        if prev_lower && c.is_uppercase() { return true; }
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
            ..ChannelWeights::default()
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
pub fn route_hits(parsed: &ParsedQuery, hits: &mut [SearchHit]) {
    let w = weights_for(classify(parsed));
    for hit in hits {
        hit.score *= match hit.kind {
            HitKind::Asgrep => w.lexical,
            HitKind::Def => w.def,
            HitKind::Caller => w.caller,
            HitKind::Graph => w.graph,
            HitKind::Anchor => w.anchor,
            HitKind::Embed => w.embed,
            HitKind::Pattern => w.pattern,
            HitKind::Import => 1.0,
        };
    }
}
