//! Pass 1 (oracle-foundry, Mission 1): independent L1 oracles for core
//! limit clamps, query-length validation, FTS escaping, schema-mismatch
//! parsing, and the mmap read-only boundary.
//!
//! Every expectation below is hand-computed from the documented contract,
//! not copied from production output. Errors are asserted by discriminant
//! (`is_err` / `None` / `matches!`), never by Display text.

use ast_sgrep_core::{
    clamp_agent_limit, clamp_output_limit, fts, validate_query_len, StoreError,
    DEFAULT_AGENT_LIMIT, MAX_OUTPUT_RESULTS, MAX_QUERY_CHARS,
};
use ast_sgrep_mmap::map_readonly;
use std::io::Write;

#[test]
fn output_limit_clamp_matches_hand_table() {
    // (requested, default, expected)
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
    assert_eq!(MAX_OUTPUT_RESULTS, 1000);
}

#[test]
fn agent_limit_clamp_uses_stricter_ceiling() {
    assert_eq!(DEFAULT_AGENT_LIMIT, 100);
    assert_eq!(clamp_agent_limit(None, 25), 25);
    assert_eq!(clamp_agent_limit(Some(0), 0), 1);
    assert_eq!(clamp_agent_limit(Some(100), 25), 100);
    assert_eq!(clamp_agent_limit(Some(101), 25), 100);
    assert_eq!(clamp_agent_limit(Some(1000), 25), 100);
    assert_eq!(clamp_agent_limit(Some(1), 25), 1);
}

#[test]
fn query_length_counts_chars_not_bytes() {
    assert_eq!(MAX_QUERY_CHARS, 4096);
    assert!(validate_query_len("").is_ok());
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS)).is_ok());
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
    // 'é' is 2 bytes but 1 char: 4096 of them are 8192 bytes yet valid.
    assert!(validate_query_len(&"é".repeat(MAX_QUERY_CHARS)).is_ok());
    assert!(validate_query_len(&"é".repeat(MAX_QUERY_CHARS + 1)).is_err());
}

#[test]
fn fts_term_escaping_matches_hand_table() {
    assert_eq!(fts::escape_fts_term("abc"), "\"abc\"");
    assert_eq!(fts::escape_fts_term("a\"b"), "\"a\"\"b\"");
    assert_eq!(fts::escape_fts_term("\""), "\"\"\"\"");
    assert_eq!(fts::escape_fts_term(""), "\"\"");
}

#[test]
fn fts_query_joins_terms_with_or() {
    let terms = vec!["foo".to_string(), "a\"b".to_string()];
    assert_eq!(fts::escape_fts_query(&terms), "\"foo\" OR \"a\"\"b\"");
    let empty: Vec<String> = vec![];
    assert_eq!(fts::escape_fts_query(&empty), "");
    let single = vec!["x".to_string()];
    assert_eq!(fts::escape_fts_query(&single), "\"x\"");
}

#[test]
fn schema_mismatch_roundtrips_and_rejects_garbage() {
    let err = StoreError::schema_newer_than_binary(7, 5);
    let message = match &err {
        StoreError::Other(m) => m.clone(),
        other => panic!("expected Other, got {other:?}"),
    };
    assert_eq!(StoreError::parse_schema_mismatch(&message), Some((7, 5)));
    // Absence, not failure-shaped success: unparseable input yields None.
    assert_eq!(StoreError::parse_schema_mismatch("garbage"), None);
    assert_eq!(StoreError::parse_schema_mismatch(""), None);
    assert_eq!(
        StoreError::parse_schema_mismatch(
            "index schema version x is newer than supported version y"
        ),
        None
    );
}

#[test]
fn mmap_readonly_returns_exact_bytes() {
    let bytes: &[u8] = b"hello\x00mmap-\xc3\xa9\n";
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(bytes).expect("write");
    file.flush().expect("flush");
    let mapped = map_readonly(file.as_file()).expect("map");
    assert_eq!(mapped.len(), bytes.len());
    assert_eq!(&mapped[..], bytes);
    // A second mapping of the same inode agrees (stable read-only view).
    let remapped = map_readonly(file.as_file()).expect("remap");
    assert_eq!(&remapped[..], &mapped[..]);
}
