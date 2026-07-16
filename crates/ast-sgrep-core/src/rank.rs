use std::borrow::Cow;

pub const RRF_K: f64 = 60.0;
pub const SCORE_GRAPH: f64 = 5.0;
pub const SCORE_ANCHOR: f64 = 6.0;
pub const SCORE_DEF_BASE: f64 = 3.0;
pub const SCORE_CALLER_BASE: f64 = 1.5;
pub const SCORE_EXACT_SYMBOL: f64 = 5.0;
pub const SCORE_SUBSTRING_SYMBOL: f64 = 2.0;
const MIN_SUBSTRING_SYMBOL_CHARS: usize = 2;
pub const SCORE_PATTERN: f64 = 7.0;
pub const SCORE_EMBED: f64 = 4.0;
pub const LEXICAL_RRF_SCALE: f64 = 200.0;
pub fn rrf_score(rank: usize, k: f64) -> f64 {
    1.0 / (k + rank as f64 + 1.0)
}
pub fn fuse_rrf(ranks: &[usize], k: f64) -> f64 {
    ranks.iter().map(|r| rrf_score(*r, k)).sum()
}
pub fn score_lexical_rrf(per_term_ranks: &[usize]) -> f64 {
    fuse_rrf(per_term_ranks, RRF_K) * LEXICAL_RRF_SCALE
}
fn normalized_symbol(symbol: &str) -> Cow<'_, str> {
    if symbol
        .bytes()
        .all(|b| b.is_ascii() && !b.is_ascii_uppercase())
    {
        Cow::Borrowed(symbol)
    } else {
        Cow::Owned(symbol.to_lowercase())
    }
}
fn score_normalized_symbol(term: &str, symbol: &str) -> f64 {
    if symbol == term {
        SCORE_EXACT_SYMBOL
    } else if term.chars().take(MIN_SUBSTRING_SYMBOL_CHARS).count()
        == MIN_SUBSTRING_SYMBOL_CHARS
        && symbol.chars().take(MIN_SUBSTRING_SYMBOL_CHARS).count()
            == MIN_SUBSTRING_SYMBOL_CHARS
        && (symbol.contains(term) || term.contains(symbol))
    {
        SCORE_SUBSTRING_SYMBOL
    } else {
        0.0
    }
}
pub fn score_symbol(term: &str, symbol: &str) -> f64 {
    score_normalized_symbol(term, normalized_symbol(symbol).as_ref())
}
pub fn best_symbol_score(terms: &[String], symbol: &str) -> f64 {
    let symbol = normalized_symbol(symbol);
    terms
        .iter()
        .map(|term| score_normalized_symbol(term, symbol.as_ref()))
        .fold(0.0_f64, f64::max)
}
pub fn coverage_symbol_score(terms: &[String], symbol: &str) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let symbol = normalized_symbol(symbol);
    let mut sum = 0.0;
    let mut matched = 0usize;
    for term in terms {
        let score = score_normalized_symbol(term, symbol.as_ref());
        if score > 0.0 {
            matched += 1;
            sum += score;
        }
    }
    sum * (matched as f64 / terms.len() as f64)
}
pub fn score_def(terms: &[String], symbol: &str) -> f64 {
    coverage_symbol_score(terms, symbol) * 2.0 + SCORE_DEF_BASE
}
pub fn score_caller(terms: &[String], callee: &str) -> f64 {
    coverage_symbol_score(terms, callee) * 2.0 + SCORE_CALLER_BASE
}

#[cfg(test)]
mod tests {
    use super::{score_symbol, SCORE_EXACT_SYMBOL, SCORE_SUBSTRING_SYMBOL};

    #[test]
    fn single_character_only_scores_an_exact_symbol() {
        assert_eq!(score_symbol("i", "i"), SCORE_EXACT_SYMBOL);
        assert_eq!(score_symbol("i", "init"), 0.0);
        assert_eq!(score_symbol("init", "i"), 0.0);
        assert_eq!(score_symbol("λ", "λambda"), 0.0);
    }

    #[test]
    fn multi_character_substrings_keep_their_rank_signal() {
        assert_eq!(score_symbol("in", "init"), SCORE_SUBSTRING_SYMBOL);
        assert_eq!(score_symbol("init", "in"), SCORE_SUBSTRING_SYMBOL);
    }
}
