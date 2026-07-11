pub const RRF_K: f64 = 60.0;
pub const SCORE_GRAPH: f64 = 5.0;
pub const SCORE_ANCHOR: f64 = 6.0;
pub const SCORE_DEF_BASE: f64 = 3.0;
pub const SCORE_CALLER_BASE: f64 = 1.5;
pub const SCORE_EXACT_SYMBOL: f64 = 5.0;
pub const SCORE_SUBSTRING_SYMBOL: f64 = 2.0;
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
pub fn score_symbol(term: &str, symbol: &str) -> f64 {
    let sym = symbol.to_lowercase();
    if sym == term {
        SCORE_EXACT_SYMBOL
    } else if sym.contains(term) || term.contains(&sym) {
        SCORE_SUBSTRING_SYMBOL
    } else {
        0.0
    }
}
pub fn best_symbol_score(terms: &[String], symbol: &str) -> f64 {
    terms
        .iter()
        .map(|t| score_symbol(t, symbol))
        .fold(0.0_f64, f64::max)
}
pub fn coverage_symbol_score(terms: &[String], symbol: &str) -> f64 {
    if terms.is_empty() { return 0.0; }
    let mut sum = 0.0;
    let mut matched = 0usize;
    for term in terms {
        let s = score_symbol(term, symbol);
        if s > 0.0 {
            matched += 1;
            sum += s;
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
