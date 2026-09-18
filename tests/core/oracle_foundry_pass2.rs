//! Pass 2 (oracle-foundry, Mission 1): L2 mutation-discriminating oracles for
//! core rank/fusion/query/limit contracts.
//!
//! Each test names the mutant class it kills: adjacent-rank confusion,
//! sum-vs-max fusion swaps, `>`-vs-`>=` boundary flips, ceiling swaps,
//! silent-drop (fail-open) mutants, and tuple-field swaps. Expectations are
//! hand-computed; failures assert discriminants, never message text.

use ast_sgrep_core::fusion::{weighted_rrf_score, ChannelRanks, FusionChannel};
use ast_sgrep_core::intent::ChannelWeights;
use ast_sgrep_core::rank::{
    best_symbol_score, coverage_symbol_score, fuse_rrf, rrf_score, score_lexical_rrf,
    score_symbol,
};
use ast_sgrep_core::{
    clamp_agent_limit, clamp_output_limit, path_scope_glob, validate_query_len, ParsedQuery,
    QueryMode, StoreError, DEFAULT_AGENT_LIMIT, MAX_EXCERPT_LINES, MAX_FILE_FILTER_CHARS,
    MAX_OUTPUT_RESULTS, MAX_QUERY_CHARS, MAX_REGEX_PATTERN_CHARS,
    MAX_SEARCH_HIT_EXCERPT_BYTES, MAX_STDIN_LINE_BYTES,
};

fn unit_weights() -> ChannelWeights {
    ChannelWeights {
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

#[test]
fn rrf_score_distinguishes_adjacent_ranks() {
    // Kills: `+ 1.0` dropped (1/60 vs 1/61), rank ignored, k ignored.
    assert_eq!(rrf_score(0, 60.0), 1.0 / 61.0);
    assert_eq!(rrf_score(1, 60.0), 1.0 / 62.0);
    assert_eq!(rrf_score(0, 0.0), 1.0);
    assert!(rrf_score(0, 0.0) > rrf_score(0, 60.0));
    let ranks: Vec<f64> = (0..5).map(|r| rrf_score(r, 60.0)).collect();
    for pair in ranks.windows(2) {
        assert!(pair[0] > pair[1], "RRF must strictly decrease: {ranks:?}");
    }
}

#[test]
fn fuse_rrf_sums_terms_and_zeroes_on_empty() {
    // Kills: sum replaced by max/first-only, empty defaulting to nonzero.
    assert_eq!(fuse_rrf(&[], 60.0), 0.0);
    assert_eq!(fuse_rrf(&[0], 60.0), 1.0 / 61.0);
    assert_eq!(fuse_rrf(&[0, 1], 60.0), 1.0 / 61.0 + 1.0 / 62.0);
}

#[test]
fn score_lexical_rrf_applies_scale() {
    // Kills: LEXICAL_RRF_SCALE dropped (would return 1/61, not 200/61).
    let got = score_lexical_rrf(&[0]);
    assert!((got - 200.0 / 61.0).abs() < 1e-12, "got {got}");
    assert_eq!(score_lexical_rrf(&[]), 0.0);
}

#[test]
fn score_symbol_exact_substring_absent_ladder() {
    // Kills: constant score, exact/substring branch swap, one-sided case fold,
    // and the 2-char substring floor flipped to 1.
    assert_eq!(score_symbol("foo", "foo"), 5.0);
    assert_eq!(score_symbol("Foo", "foo"), 5.0);
    assert_eq!(score_symbol("ab", "abc"), 2.0);
    assert_eq!(score_symbol("abc", "ab"), 2.0);
    assert_eq!(score_symbol("xyz", "abc"), 0.0);
    assert_eq!(score_symbol("a", "abc"), 0.0);
    assert_eq!(score_symbol("ab", "a"), 0.0);
}

#[test]
fn best_and_coverage_scores_split_max_vs_sum() {
    // Kills: best implemented as sum (would be 4.0) and coverage as max (2.0).
    let terms = vec!["foo".to_string(), "bar".to_string()];
    assert_eq!(best_symbol_score(&terms, "foo bar"), 2.0);
    assert_eq!(coverage_symbol_score(&terms, "foo bar"), 4.0);
    assert_eq!(best_symbol_score(&[], "foo"), 0.0);
    assert_eq!(coverage_symbol_score(&[], "foo"), 0.0);
    assert_eq!(best_symbol_score(&["foo".to_string()], "foo"), 5.0);
}

#[test]
fn weighted_rrf_ignores_absent_channels_and_clamps_weights() {
    // Kills: None treated as rank 0 (would score 8/61), first-channel-only
    // sum, weight-clamp removal, and NaN-weight passthrough (NaN sum).
    assert_eq!(FusionChannel::ALL.len(), 8);
    let weights = unit_weights();
    assert_eq!(ChannelRanks::default().get(FusionChannel::Lexical), None);
    assert_eq!(weighted_rrf_score(&ChannelRanks::default(), &weights), 0.0);
    let one = ChannelRanks {
        lexical: Some(0),
        ..ChannelRanks::default()
    };
    assert_eq!(weighted_rrf_score(&one, &weights), 1.0 / 61.0);
    let two = ChannelRanks {
        lexical: Some(0),
        definition: Some(0),
        ..ChannelRanks::default()
    };
    assert_eq!(weighted_rrf_score(&two, &weights), 2.0 / 61.0);
    let huge = ChannelWeights {
        lexical: 100.0,
        ..unit_weights()
    };
    assert_eq!(weighted_rrf_score(&one, &huge), 2.0 / 61.0);
    let nan = ChannelWeights {
        lexical: f64::NAN,
        ..unit_weights()
    };
    assert_eq!(weighted_rrf_score(&one, &nan), 1.0 / 61.0);
}

#[test]
fn independent_rrf_rational_oracle_agrees() {
    // Deliberately different check (O-C08): cross-multiplication relation
    // instead of restating the formula. A `+2`, rank-ignored, or
    // constant-returning mutant breaks the identity score * (k+rank+1) == 1.
    for rank in 0..5usize {
        for k in [0.0, 60.0] {
            let score = rrf_score(rank, k);
            let identity = score * (k + rank as f64 + 1.0);
            assert!((identity - 1.0).abs() < 1e-12, "rank={rank} k={k}");
        }
    }
}

#[test]
fn parsed_query_mode_table_kills_prefix_mutants() {
    // Kills: prefix misrouting, Word/Literal case-rule swap, raw-prefix trim.
    let defs = ParsedQuery::parse("defs:foo");
    assert_eq!(defs.mode, QueryMode::Defs);
    assert_eq!(defs.target.as_deref(), Some("foo"));
    let callers = ParsedQuery::parse("callers: Bar");
    assert_eq!(callers.mode, QueryMode::Callers);
    assert_eq!(callers.target.as_deref(), Some("Bar"));
    assert_eq!(callers.terms, vec!["bar".to_string()]);
    assert_eq!(ParsedQuery::parse("imports:os").mode, QueryMode::Imports);
    let pattern = ParsedQuery::parse("pattern:$A");
    assert_eq!(pattern.mode, QueryMode::Pattern);
    assert_eq!(pattern.terms, vec!["$A".to_string()]);
    let literal = ParsedQuery::parse("literal:Foo");
    assert_eq!(literal.mode, QueryMode::Literal);
    assert_eq!(literal.terms, vec!["Foo".to_string()]);
    assert_eq!(literal.raw, "literal:Foo");
    let regex = ParsedQuery::parse("regex:A+");
    assert_eq!(regex.mode, QueryMode::Regex);
    assert_eq!(regex.terms, vec!["A+".to_string()]);
    let word = ParsedQuery::parse("word:Foo");
    assert_eq!(word.mode, QueryMode::Word);
    assert_eq!(word.terms, vec!["foo".to_string()]);
    let hybrid = ParsedQuery::parse("hello world");
    assert_eq!(hybrid.mode, QueryMode::Hybrid);
    assert_eq!(hybrid.target, None);
    assert_eq!(hybrid.terms, vec!["hello".to_string(), "world".to_string()]);
    let empty = ParsedQuery::parse("");
    assert_eq!(empty.mode, QueryMode::Hybrid);
    assert_eq!(empty.target, None);
    assert!(empty.terms.is_empty());
}

#[test]
fn path_scope_splits_and_refuses_loudly() {
    // Kills: scope silent-drop (fail-open), bare/duplicate/escape acceptance,
    // quote-blind `in:` detection, and glob-passthrough removal.
    let scoped = ParsedQuery::parse("foo in:src");
    assert_eq!(scoped.path_scope.as_deref(), Some("src"));
    assert_eq!(scoped.path_scope_error, None);
    assert_eq!(scoped.terms, vec!["foo".to_string()]);
    assert_eq!(ParsedQuery::parse("foo in:").path_scope_error.is_some(), true);
    assert_eq!(ParsedQuery::parse("foo in:").path_scope, None);
    assert!(ParsedQuery::parse("a in:x in:y").path_scope_error.is_some());
    assert!(ParsedQuery::parse("a in:../up").path_scope_error.is_some());
    let abs = tempfile::tempdir().expect("tempdir");
    let abs_query = format!("a in:{}", abs.path().display());
    assert!(ParsedQuery::parse(&abs_query).path_scope_error.is_some());
    let quoted = ParsedQuery::parse("\"a in:src\" b");
    assert_eq!(quoted.path_scope, None);
    assert_eq!(quoted.path_scope_error, None);
    assert_eq!(path_scope_glob("src"), "src/**");
    assert_eq!(path_scope_glob("src/"), "src/**");
    assert_eq!(path_scope_glob("a*b"), "a*b");
    assert_eq!(path_scope_glob("a?"), "a?");
}

#[test]
fn limit_constants_pin_hand_values() {
    // Kills: arithmetic mutants on the const expressions (`64 * 1024`
    // flipped to `+` or `/`) and silent ceiling drift.
    assert_eq!(MAX_SEARCH_HIT_EXCERPT_BYTES, 65_536);
    assert_eq!(MAX_EXCERPT_LINES, 100);
    assert_eq!(MAX_FILE_FILTER_CHARS, 1024);
    assert_eq!(MAX_REGEX_PATTERN_CHARS, 4096);
    assert_eq!(MAX_STDIN_LINE_BYTES, 1_048_576);
}

#[test]
fn clamp_ceilings_are_not_interchangeable() {
    // Kills: agent/output ceiling swap, zero-means-default removal, and the
    // default floor (`default.max(1)`) dropped to bare `default`.
    assert_eq!(DEFAULT_AGENT_LIMIT, 100);
    assert_eq!(MAX_OUTPUT_RESULTS, 1000);
    assert_eq!(clamp_agent_limit(Some(101), 25), 100);
    assert_eq!(clamp_output_limit(Some(101), 25), 101);
    assert_eq!(clamp_agent_limit(Some(1000), 25), 100);
    assert_eq!(clamp_output_limit(Some(1000), 25), 1000);
    assert_eq!(clamp_output_limit(None, 0), 1);
    assert_eq!(clamp_agent_limit(None, 0), 1);
    assert_eq!(clamp_output_limit(Some(0), 5), 5);
    // Survivor witness (cargo-mutants 27.1.0, limits.rs:29:40): `*n > 0`
    // flipped to `>=` keeps Some(0) and clamps to 1; nonzero defaults expose it.
    assert_eq!(clamp_agent_limit(Some(0), 25), 25);
}

#[test]
fn query_length_boundary_is_exclusive_over_chars() {
    // Kills: `>` flipped to `>=` (would reject exactly-MAX), bytes-for-chars.
    assert_eq!(MAX_QUERY_CHARS, 4096);
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS)).is_ok());
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
    assert!(validate_query_len(&"aé".repeat(MAX_QUERY_CHARS / 2)).is_ok());
    assert!(validate_query_len(&("aé".repeat(MAX_QUERY_CHARS / 2) + "a")).is_err());
}

#[test]
fn schema_mismatch_field_order_kills_swap() {
    // Kills: (disk, supported) tuple swap and magnitude-ordered return.
    let err = StoreError::schema_newer_than_binary(12, 3);
    let message = match &err {
        StoreError::Other(m) => m.clone(),
        other => panic!("expected Other, got {other:?}"),
    };
    assert_eq!(StoreError::parse_schema_mismatch(&message), Some((12, 3)));
    assert_eq!(
        StoreError::parse_schema_mismatch(
            "index schema version 5 is newer than supported version 7"
        ),
        Some((5, 7))
    );
}
