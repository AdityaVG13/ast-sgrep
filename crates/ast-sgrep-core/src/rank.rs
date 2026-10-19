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
pub(crate) fn normalized_symbol(symbol: &str) -> Cow<'_, str> {
    if symbol
        .bytes()
        .all(|b| b.is_ascii() && !b.is_ascii_uppercase())
    {
        Cow::Borrowed(symbol)
    } else {
        Cow::Owned(symbol.to_lowercase())
    }
}
fn has_minimum_substring_chars(value: &str) -> bool {
    value.len() >= MIN_SUBSTRING_SYMBOL_CHARS
        && (value.is_ascii() || value.chars().nth(MIN_SUBSTRING_SYMBOL_CHARS - 1).is_some())
}
fn score_pre_normalized_symbol(term: &str, symbol: &str, symbol_can_substring: bool) -> f64 {
    if symbol == term {
        SCORE_EXACT_SYMBOL
    } else if symbol_can_substring
        && has_minimum_substring_chars(term)
        && (symbol.contains(term) || term.contains(symbol))
    {
        SCORE_SUBSTRING_SYMBOL
    } else {
        0.0
    }
}
fn score_normalized_symbol(term: &str, symbol: &str, symbol_can_substring: bool) -> f64 {
    // Normalize BOTH sides: prefixed modes (callers:/defs:/imports:) pass the raw,
    // possibly mixed-case target as the term, while `symbol` is already normalized.
    let term = normalized_symbol(term);
    score_pre_normalized_symbol(term.as_ref(), symbol, symbol_can_substring)
}
pub fn score_symbol(term: &str, symbol: &str) -> f64 {
    let symbol = normalized_symbol(symbol);
    score_normalized_symbol(term, symbol.as_ref(), has_minimum_substring_chars(&symbol))
}
/// Normalize query terms once per query before scoring many rows.
pub fn normalize_query_terms(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .map(|t| normalized_symbol(t).into_owned())
        .collect()
}
pub fn best_symbol_score(terms: &[String], symbol: &str) -> f64 {
    best_symbol_score_normalized(&normalize_query_terms(terms), symbol)
}
/// Like [`best_symbol_score`] but assumes `terms` are already lowercased.
pub fn best_symbol_score_normalized(normalized_terms: &[String], symbol: &str) -> f64 {
    let symbol = normalized_symbol(symbol);
    let symbol_can_substring = has_minimum_substring_chars(&symbol);
    normalized_terms
        .iter()
        .map(|term| score_pre_normalized_symbol(term, symbol.as_ref(), symbol_can_substring))
        .fold(0.0_f64, f64::max)
}
pub fn coverage_symbol_score(terms: &[String], symbol: &str) -> f64 {
    coverage_symbol_score_normalized(&normalize_query_terms(terms), symbol)
}
/// Like [`coverage_symbol_score`] but assumes `terms` are already lowercased.
pub fn coverage_symbol_score_normalized(normalized_terms: &[String], symbol: &str) -> f64 {
    if normalized_terms.is_empty() {
        return 0.0;
    }
    let symbol = normalized_symbol(symbol);
    let symbol_can_substring = has_minimum_substring_chars(&symbol);
    let mut sum = 0.0;
    for term in normalized_terms {
        let score = score_pre_normalized_symbol(term, symbol.as_ref(), symbol_can_substring);
        if score > 0.0 {
            sum += score;
        }
    }
    // Each additional matching term contributes independently; unmatched query context must not
    // dilute evidence from terms that already match the symbol.
    sum
}
/// Scale a coverage score onto a base: zero coverage stays zero (do not award
/// base score to non-matches — prevents rank pollution from weak LIKE hits).
fn score_scaled_coverage(coverage: f64, base: f64) -> f64 {
    if coverage == 0.0 {
        0.0
    } else {
        coverage * 2.0 + base
    }
}
pub fn score_def(terms: &[String], symbol: &str) -> f64 {
    score_def_normalized(&normalize_query_terms(terms), symbol)
}
pub fn score_def_normalized(normalized_terms: &[String], symbol: &str) -> f64 {
    let coverage = coverage_symbol_score_normalized(normalized_terms, symbol);
    score_scaled_coverage(coverage, SCORE_DEF_BASE)
}
/// BM25-style inverse document frequency for one query term over a name
/// table: how much evidence a match on this term carries. Always positive —
/// a term matching every name still counts, just barely. `df` = names
/// matching the term under the same substring predicate as scoring;
/// `total` = names in the table.
pub fn term_idf(df: u64, total: u64) -> f64 {
    let df = df as f64;
    let total = total.max(1) as f64;
    ((total - df + 0.5) / (df + 0.5) + 1.0).ln()
}

/// Scale raw IDF weights so the rarest query term weighs exactly 1.0.
///
/// The router ceiling (`intent::channel_ceiling`) assumes an exact match
/// can reach full per-term coverage, so absolute IDF would deflate the
/// whole Def/Caller channel against constant-scored channels whenever
/// every query term is corpus-common. Only *relative* rarity may
/// discriminate between hits of one query — a single-term query therefore
/// scores bit-identically to unweighted coverage.
pub fn max_normalize_idf(weights: &mut [f64]) {
    let max = weights.iter().copied().fold(0.0f64, f64::max);
    if max > 0.0 {
        for w in weights.iter_mut() {
            *w /= max;
        }
    }
}

/// IDF-weighted coverage: each matching term contributes its symbol score
/// scaled by the term's rarity weight (aligned by index with
/// `normalized_terms`). A def matching a rare query token is stronger
/// evidence than one matching a token hundreds of symbols carry; unweighted
/// coverage cannot tell them apart. Weights must be positive (see
/// [`term_idf`]); a short weights slice treats missing terms as weight 1.
pub fn coverage_symbol_score_weighted(
    normalized_terms: &[String],
    weights: &[f64],
    symbol: &str,
) -> f64 {
    if normalized_terms.is_empty() {
        return 0.0;
    }
    let symbol = normalized_symbol(symbol);
    let symbol_can_substring = has_minimum_substring_chars(&symbol);
    let mut sum = 0.0;
    for (i, term) in normalized_terms.iter().enumerate() {
        let score = score_pre_normalized_symbol(term, symbol.as_ref(), symbol_can_substring);
        if score > 0.0 {
            sum += score * weights.get(i).copied().unwrap_or(1.0);
        }
    }
    sum
}

pub fn score_def_weighted(normalized_terms: &[String], weights: &[f64], symbol: &str) -> f64 {
    let coverage = coverage_symbol_score_weighted(normalized_terms, weights, symbol);
    score_scaled_coverage(coverage, SCORE_DEF_BASE)
}

pub fn score_caller_weighted(normalized_terms: &[String], weights: &[f64], callee: &str) -> f64 {
    let coverage = coverage_symbol_score_weighted(normalized_terms, weights, callee);
    score_scaled_coverage(coverage, SCORE_CALLER_BASE)
}

pub fn score_caller(terms: &[String], callee: &str) -> f64 {
    score_caller_normalized(&normalize_query_terms(terms), callee)
}
pub fn score_caller_normalized(normalized_terms: &[String], callee: &str) -> f64 {
    let coverage = coverage_symbol_score_normalized(normalized_terms, callee);
    score_scaled_coverage(coverage, SCORE_CALLER_BASE)
}
