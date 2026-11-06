//! F14: callee/arg-slot/classifier cluster (S2+S3+S4) — one classification path.
//!
//! The three arg-slot grammars (plain-call / member / RHS), the two slot
//! matchers (positional / backtracking), the twin chain classifiers
//! (`.` vs `->`), and the six callee-path extractors route through one
//! parameterized path: [`parse_arg_slots`], [`match_arg_slots`],
//! [`classify_chain_arg_list`], [`resolve_callee`]. These tests pin the
//! unified contract:
//!
//! - direct grammar pins: each lane keeps its exact admission table through
//!   the shared parser (literals refuse on the plain lane, admit on the
//!   member lane; same-name rest collisions refuse on the plain lane);
//! - cross-lane agreement pins: dotted-chain-mixed and flat-call-mixed faces
//!   answer identical line sets (the drift the triplication invited);
//! - faithful-binding pins: multi-segment `$O.$M` receivers bind through
//!   the unified resolver exactly as before.

use ast_sgrep_lang::{match_pattern, parse_arg_slots, ArgSlot, ArgSlotGrammar, Language};

fn lines_of(hits: &[ast_sgrep_lang::PatternMatch]) -> Vec<u32> {
    let mut lines: Vec<u32> = hits.iter().map(|h| h.line_start).collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

// --- S3: one parser, per-lane admission preserved ---

#[test]
fn plain_lane_refuses_literal_slots_while_member_lane_admits_them() {
    assert!(parse_arg_slots("1, $B", ArgSlotGrammar::PlainCall).is_none());
    assert_eq!(
        parse_arg_slots("1, $B", ArgSlotGrammar::Member),
        Some(vec![
            ArgSlot::Literal("1".to_string()),
            ArgSlot::Meta("B".to_string()),
        ])
    );
}

#[test]
fn plain_lane_rest_rules_preserved_through_unified_parser() {
    // Trailing rest + single classifies; non-trailing rest + single too
    // (positional matcher pins the arity, not the parser).
    assert_eq!(
        parse_arg_slots("$A, $$$B", ArgSlotGrammar::PlainCall),
        Some(vec![
            ArgSlot::Meta("A".to_string()),
            ArgSlot::Rest("B".to_string()),
        ])
    );
    // k >= 2 rests with no singles classify with distinct names.
    assert_eq!(
        parse_arg_slots("$$$A, $$$B", ArgSlotGrammar::PlainCall),
        Some(vec![
            ArgSlot::Rest("A".to_string()),
            ArgSlot::Rest("B".to_string()),
        ])
    );
    // Same-name rest collision refuses (fail-closed, never a silent empty).
    assert!(parse_arg_slots("$$$A, $A", ArgSlotGrammar::PlainCall).is_none());
    assert!(parse_arg_slots("$$$A, $$$A", ArgSlotGrammar::PlainCall).is_none());
    // Sole rest never reaches the slot parser on either lane.
    assert!(parse_arg_slots("$$$A", ArgSlotGrammar::PlainCall).is_none());
    // At most one rest with singles on the plain lane.
    assert!(parse_arg_slots("$$$A, $$$B, $C", ArgSlotGrammar::PlainCall).is_none());
}

#[test]
fn member_lane_single_rest_cap_preserved_through_unified_parser() {
    // The member/RHS grammar admits literals and caps rests at one.
    assert_eq!(
        parse_arg_slots("$$$A, $B", ArgSlotGrammar::Member),
        Some(vec![
            ArgSlot::Rest("A".to_string()),
            ArgSlot::Meta("B".to_string()),
        ])
    );
    assert!(parse_arg_slots("$$$A, $$$B", ArgSlotGrammar::Member).is_none());
    // A whole-list rest parses to its single slot (callers route it to the
    // whole-list arm before the slots; the parser itself accepts it —
    // identical to the former `parse_rhs_arg_slots`).
    assert_eq!(
        parse_arg_slots("$$$A", ArgSlotGrammar::Member),
        Some(vec![ArgSlot::Rest("A".to_string())])
    );
}

// --- S3+S4: cross-lane agreement (the drift the triplication invited) ---

const JAVA_CALLS: &str = "class T {\n  void m() {\n    q(1);\n    q(1, 2);\n    q(1, 2, 3);\n  }\n}\n";

#[test]
fn flat_rest_slots_answer_trailing_rest_at_n_ge_2() {
    let hits = match_pattern(Language::Java, JAVA_CALLS, "q($A, $$$B)").unwrap();
    assert_eq!(lines_of(&hits), vec![4, 5]);
    let captures = &hits[0].captures;
    assert_eq!(captures.get("A").map(String::as_str), Some("1"));
    assert_eq!(captures.get("$$$B").map(String::as_str), Some("2"));
}

const JAVA_DOTTED: &str = "class T {\n  void m() {\n    a.b(1);\n    a.b(1, 2);\n    a.b(1, 2, 3);\n  }\n}\n";

#[test]
fn dotted_chain_mixed_rest_agrees_with_flat_call_lane() {
    // `a.b($A, $$$B)` refuses chain classification and rides the flat Call
    // lane through the same slot grammar: identical line sets, identical
    // bindings. Any future drift between the chain and flat tables breaks
    // this pin.
    let chained = match_pattern(Language::Java, JAVA_DOTTED, "a.b($A, $$$B)").unwrap();
    let flat = match_pattern(Language::Java, JAVA_CALLS, "q($A, $$$B)").unwrap();
    assert_eq!(lines_of(&chained), lines_of(&flat));
    assert_eq!(lines_of(&chained), vec![4, 5]);
    assert_eq!(
        chained[0].captures.get("A").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        chained[0].captures.get("$$$B").map(String::as_str),
        Some("2")
    );
}

#[test]
fn dotted_chain_meta_lists_bind_positionally() {
    let source = "class T {\n  void m() {\n    a.b(1, 2);\n    a.b(1).c(2);\n  }\n}\n";
    let hits = match_pattern(Language::Java, source, "a.b($X).c($Y)").unwrap();
    assert_eq!(lines_of(&hits), vec![4]);
    assert_eq!(hits[0].captures.get("X").map(String::as_str), Some("1"));
    assert_eq!(hits[0].captures.get("Y").map(String::as_str), Some("2"));
}

// --- S2: faithful receiver binding through the unified resolver ---

#[test]
fn leading_metavar_receiver_binds_whole_prefix() {
    let source = "class T {\n  void m() {\n    a.b.c(2);\n  }\n}\n";
    let hits = match_pattern(Language::Java, source, "$O.$M($$$A)").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].captures.get("O").map(String::as_str), Some("a.b"));
    assert_eq!(hits[0].captures.get("M").map(String::as_str), Some("c"));
}

#[test]
fn literal_leading_segment_pins_chain_length() {
    let source = "class T {\n  void m() {\n    a.b.c(2);\n  }\n}\n";
    assert!(
        match_pattern(Language::Java, source, "obj.$M($$$A)")
            .unwrap()
            .is_empty()
    );
}
