use super::*;

#[test]
fn clamps_to_hard_ceiling() {
    assert_eq!(clamp_output_limit(Some(0), 16), 16);
    assert_eq!(clamp_output_limit(None, 16), 16);
    assert_eq!(clamp_output_limit(Some(50), 16), 50);
    assert_eq!(clamp_output_limit(Some(10_000), 16), MAX_OUTPUT_RESULTS);
    assert_eq!(clamp_agent_limit(Some(500), 16), DEFAULT_AGENT_LIMIT);
}

#[test]
fn query_len_boundary() {
    assert!(validate_query_len("").is_ok());
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS)).is_ok());
    assert!(validate_query_len(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
}
