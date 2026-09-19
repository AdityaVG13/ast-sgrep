//! Core limit-clamp and query-length oracles (consolidated).
//!
//! Consolidates the CAT=limit-clamp facets and the query-length facets of
//! `oracle_foundry_pass{1,2}` into 2 intent-grouped tests. Every expectation
//! is hand-computed from the documented contract; failures assert
//! discriminants, never message text.

use ast_sgrep_core::{
    clamp_agent_limit, clamp_output_limit, validate_query_len, DEFAULT_AGENT_LIMIT,
    MAX_EXCERPT_LINES, MAX_FILE_FILTER_CHARS, MAX_OUTPUT_RESULTS, MAX_QUERY_CHARS,
    MAX_REGEX_PATTERN_CHARS, MAX_SEARCH_HIT_EXCERPT_BYTES, MAX_STDIN_LINE_BYTES,
};

/// INTENT: output/agent limit clamps honor None/0→default, default floor 1,
/// and their distinct ceilings (1000 vs 100); size constants stay pinned.
/// KILLS: ceiling-drop/floor-drop, ceiling-swap (100↔1000), `>0`-survivor,
/// and const-arithmetic/ceiling-drift mutants.
/// ABSORBS: output_limit_clamp_matches_hand_table,
/// agent_limit_clamp_uses_stricter_ceiling, clamp_ceilings_are_not_interchangeable,
/// limit_constants_pin_hand_values.
#[test]
fn limit_clamp_matrix_and_constants() {
    // Facet 1: output clamp hand table (None/0→default, floor 1, ceiling 1000).
    let cases: &[(Option<usize>, usize, usize)] = &[
        (None, 25, 25),
        (Some(0), 25, 25),
        (None, 0, 1),
        (Some(0), 0, 1),
        (Some(1), 25, 1),
        (Some(999), 25, 999),
        (Some(MAX_OUTPUT_RESULTS), 25, MAX_OUTPUT_RESULTS),
        (Some(MAX_OUTPUT_RESULTS + 1), 25, MAX_OUTPUT_RESULTS),
        (Some(usize::MAX), 25, MAX_OUTPUT_RESULTS),
        (Some(5), 9_999, 5),
    ];
    for (requested, default, expected) in cases {
        assert_eq!(
            clamp_output_limit(*requested, *default),
            *expected,
            "requested={requested:?} default={default}"
        );
    }
    // Facet 2: agent clamp uses the stricter ceiling 100 with the same rules.
    assert_eq!(clamp_agent_limit(None, 25), 25);
    assert_eq!(clamp_agent_limit(Some(0), 0), 1);
    assert_eq!(clamp_agent_limit(Some(100), 25), 100);
    assert_eq!(clamp_agent_limit(Some(101), 25), 100);
    assert_eq!(clamp_agent_limit(Some(1000), 25), 100);
    assert_eq!(clamp_agent_limit(Some(1), 25), 1);
    // Facet 3: the two ceilings are not interchangeable (101/1000 split cases).
    assert_eq!(clamp_output_limit(Some(101), 25), 101);
    assert_eq!(clamp_output_limit(Some(1000), 25), 1000);
    assert_eq!(clamp_agent_limit(None, 0), 1);
    assert_eq!(clamp_output_limit(Some(0), 5), 5);
    // Survivor witness (cargo-mutants 27.1.0, limits.rs:29:40): `*n > 0`
    // flipped to `>=` keeps Some(0) and clamps to 1; nonzero defaults expose it.
    assert_eq!(clamp_agent_limit(Some(0), 25), 25);
    // Facet 4: hand-value pins for every clamp ceiling and size constant.
    assert_eq!(MAX_OUTPUT_RESULTS, 1000);
    assert_eq!(DEFAULT_AGENT_LIMIT, 100);
    assert_eq!(MAX_SEARCH_HIT_EXCERPT_BYTES, 65_536);
    assert_eq!(MAX_EXCERPT_LINES, 100);
    assert_eq!(MAX_FILE_FILTER_CHARS, 1024);
    assert_eq!(MAX_REGEX_PATTERN_CHARS, 4096);
    assert_eq!(MAX_STDIN_LINE_BYTES, 1_048_576);
}

/// INTENT: query length is validated in chars (not bytes) with an exclusive
/// MAX boundary: exactly-MAX ok, MAX+1 rejected, multibyte counted as chars.
/// KILLS: bytes-for-chars and `>`→`>=` mutants.
/// ABSORBS: query_length_counts_chars_not_bytes,
/// query_length_boundary_is_exclusive_over_chars.
#[test]
fn query_length_chars_and_exclusive_boundary() {
    assert_eq!(MAX_QUERY_CHARS, 4096);
    // Facet 1: char (not byte) counting, incl a pure-multibyte MAX-length query.
    assert!(validate_query_len("").is_ok());
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS)).is_ok());
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
    // 'é' is 2 bytes but 1 char: 4096 of them are 8192 bytes yet valid.
    assert!(validate_query_len(&"é".repeat(MAX_QUERY_CHARS)).is_ok());
    assert!(validate_query_len(&"é".repeat(MAX_QUERY_CHARS + 1)).is_err());
    // Facet 2: the boundary is exclusive over mixed-script char counts.
    assert!(validate_query_len(&"aé".repeat(MAX_QUERY_CHARS / 2)).is_ok());
    assert!(validate_query_len(&("aé".repeat(MAX_QUERY_CHARS / 2) + "a")).is_err());
}
