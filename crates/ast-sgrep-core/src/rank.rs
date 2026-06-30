/// Ranking scores per PRD.
pub const SCORE_GRAPH: f64 = 5.0;
pub const SCORE_ANCHOR: f64 = 6.0;
pub const SCORE_DEF_BASE: f64 = 3.0;
pub const SCORE_CALLER_BASE: f64 = 1.5;
pub const SCORE_EXACT_SYMBOL: f64 = 5.0;
pub const SCORE_SUBSTRING_SYMBOL: f64 = 2.0;
pub const LEXICAL_RANK_DIVISOR: f64 = 60.0;

/// BM25-like lexical score from rank position.
pub fn score_lexical(rank: usize) -> f64 {
    1.0 / (LEXICAL_RANK_DIVISOR + rank as f64 + 1.0)
}

/// Symbol term match score.
pub fn score_symbol(term: &str, symbol: &str) -> f64 {
    let term_lower = term.to_lowercase();
    let sym_lower = symbol.to_lowercase();
    if sym_lower == term_lower {
        SCORE_EXACT_SYMBOL
    } else if sym_lower.contains(&term_lower) || term_lower.contains(&sym_lower) {
        SCORE_SUBSTRING_SYMBOL
    } else {
        0.0
    }
}

/// Best symbol score across all query terms.
pub fn best_symbol_score(terms: &[String], symbol: &str) -> f64 {
    terms
        .iter()
        .map(|t| score_symbol(t, symbol))
        .fold(0.0_f64, f64::max)
}

/// Definition hit score.
pub fn score_def(terms: &[String], symbol: &str) -> f64 {
    best_symbol_score(terms, symbol) * 2.0 + SCORE_DEF_BASE
}

/// Caller hit score.
pub fn score_caller(terms: &[String], callee: &str) -> f64 {
    best_symbol_score(terms, callee) * 2.0 + SCORE_CALLER_BASE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_score_decreases_with_rank() {
        assert!(score_lexical(0) > score_lexical(10));
    }

    #[test]
    fn exact_symbol_scores_higher() {
        assert!(score_symbol("foo", "foo") > score_symbol("foo", "foobar"));
    }
}
