use super::*;

#[test]
fn cached_signatures_stay_byte_identical_for_legacy_shapes() {
    // No metavariables → exact pattern text is the index key.
    assert_eq!(
        cached_pattern_signatures("fn parse_low").unwrap(),
        vec!["fn parse_low".to_string()]
    );
    // Historical core classifier: fn/def metavariable → single kind key.
    assert_eq!(
        cached_pattern_signatures("fn $NAME($$$)").unwrap(),
        vec!["kind:function_item".to_string()]
    );
    assert_eq!(
        cached_pattern_signatures("def $NAME").unwrap(),
        vec!["kind:function_definition".to_string()]
    );
    assert_eq!(
        cached_pattern_signatures("fn parse_low($$$)").unwrap(),
        vec!["decl:fn:parse_low".to_string()]
    );
    assert_eq!(
        cached_pattern_signatures("$OBJ.method($$$)").unwrap(),
        vec!["call-name:method".to_string()]
    );
    assert_eq!(
        cached_pattern_signatures("foo.bar($$$)").unwrap(),
        vec!["call:foo.bar".to_string()]
    );
    assert_eq!(
        cached_pattern_signatures("kind:function_item").unwrap(),
        vec!["kind:function_item".to_string()]
    );
}

#[test]
fn structural_term_signatures_match_legacy_formats() {
    assert_eq!(
        structural_term_signatures("renew"),
        [
            "call-name:renew".to_string(),
            "call:renew".to_string(),
            "decl:fn:renew".to_string(),
            "decl:def:renew".to_string(),
            "decl:function:renew".to_string(),
            "renew".to_string(),
        ]
    );
}

#[test]
fn required_literal_skips_decl_keywords() {
    assert_eq!(
        required_pattern_literal("Needle($$$ARGS)").as_deref(),
        Some("Needle")
    );
    assert_eq!(required_pattern_literal("$FUNC($$$ARGS)"), None);
    assert_eq!(required_pattern_literal("fn $NAME($$$ARGS)"), None);
    assert_eq!(
        required_pattern_literal("fn parse_low").as_deref(),
        Some("fn parse_low")
    );
    assert_eq!(
        required_pattern_literal("fn parse_low($$$)").as_deref(),
        Some("parse_low")
    );
}

#[test]
fn wildcard_call_signatures_stay_byte_identical() {
    assert_eq!(
        cached_pattern_signatures("$F($$$)").unwrap(),
        vec!["kind:call_expression".to_string(), "kind:call".to_string(),]
    );
}
