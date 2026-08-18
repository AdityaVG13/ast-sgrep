use super::*;
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

#[test]
fn score_def_and_caller_zero_when_no_coverage() {
    let terms = vec!["nomatch_xyz".into()];
    assert_eq!(score_def(&terms, "process_request"), 0.0);
    assert_eq!(score_caller(&terms, "process_request"), 0.0);
    let hit = vec!["process".into()];
    assert!(score_def(&hit, "process_request") > 0.0);
}

#[test]
fn symbol_scoring_is_case_insensitive_on_the_term_side() {
    // Regression for Issue #12 / F-01: prefixed callers:/defs: pass the raw
    // (possibly mixed-case) target as the term; scoring must normalize both sides.
    assert_eq!(
        score_symbol("RefreshToken", "refreshToken"),
        SCORE_EXACT_SYMBOL
    );
    assert_eq!(
        best_symbol_score(&["RefreshToken".to_string()], "refreshToken"),
        SCORE_EXACT_SYMBOL
    );
    assert!(coverage_symbol_score(&["RefreshToken".to_string()], "refreshToken") > 0.0);
    assert_eq!(
        score_symbol("Refresh", "refreshToken"),
        SCORE_SUBSTRING_SYMBOL
    );
}

#[test]
fn coverage_score_is_monotone_when_query_expands() {
    let focused = vec!["init".to_string(), "handler".to_string()];
    let expanded = vec![
        "init".to_string(),
        "handler".to_string(),
        "noise".to_string(),
        "zzz".to_string(),
    ];

    assert!(
        coverage_symbol_score(&expanded, "init_handler")
            >= coverage_symbol_score(&focused, "init_handler")
    );
}

/// am6l: pre-normalized terms must match the normalizing public path.
#[test]
fn normalized_term_apis_match_public_scorers() {
    let terms = vec!["RefreshToken".into(), "Auth".into()];
    let norm = normalize_query_terms(&terms);
    assert_eq!(
        best_symbol_score(&terms, "refreshToken"),
        best_symbol_score_normalized(&norm, "refreshToken")
    );
    assert_eq!(
        coverage_symbol_score(&terms, "refreshToken"),
        coverage_symbol_score_normalized(&norm, "refreshToken")
    );
    assert_eq!(
        score_caller(&terms, "refreshToken"),
        score_caller_normalized(&norm, "refreshToken")
    );
}
