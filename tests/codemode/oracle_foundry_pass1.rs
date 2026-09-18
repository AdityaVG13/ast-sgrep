//! Pass 1 (oracle-foundry, Mission 4): independent L1 oracles for plugin
//! format/budget contracts, codemode plan parsing (FFI-adjacent Result
//! discriminants), and testkit fixture helpers.
//!
//! Expectations are hand-computed tables. Failures assert enum
//! discriminants via `matches!`, never Display text — the same discipline
//! the N-API boundary needs (discriminants preserved, no panics cross it).

use ast_sgrep_codemode::tools::CallError;
use ast_sgrep_codemode::{example_plan, parse_plan};
use ast_sgrep_plugins::{CompactBudget, DetailLevel, OutputFormat};
use ast_sgrep_testkit::{sample_file, sample_root};
use serde_json::json;

#[test]
fn output_format_parse_matches_hand_table() {
    let cases: &[(&str, Option<OutputFormat>)] = &[
        ("native", Some(OutputFormat::Native)),
        ("asgrep", Some(OutputFormat::Native)),
        ("github", Some(OutputFormat::GitHub)),
        ("GH", Some(OutputFormat::GitHub)),
        ("gitlab", Some(OutputFormat::GitLab)),
        ("gl", Some(OutputFormat::GitLab)),
        ("agent", Some(OutputFormat::Agent)),
        ("LLM", Some(OutputFormat::Agent)),
        ("ai", Some(OutputFormat::Agent)),
        ("agent-capsule", Some(OutputFormat::AgentCapsule)),
        ("capsule", Some(OutputFormat::AgentCapsule)),
        ("compact", Some(OutputFormat::Compact)),
        ("COMPACT", Some(OutputFormat::Compact)),
        ("bogus", None),
        ("", None),
    ];
    for (raw, expected) in cases {
        assert_eq!(OutputFormat::parse(raw), *expected, "raw={raw:?}");
    }
}

#[test]
fn compact_budget_default_matches_hand_values() {
    let budget = CompactBudget::default();
    assert_eq!(budget.per_result_tokens, 96);
    assert_eq!(budget.response_tokens, 768);
}

#[test]
fn detail_levels_order_and_label_by_hand() {
    assert_eq!(DetailLevel::ALL.len(), 4);
    let labels: Vec<&str> = DetailLevel::ALL.iter().map(|d| d.as_str()).collect();
    assert_eq!(labels, vec!["metadata", "signature", "block", "full"]);
    assert!(
        DetailLevel::Metadata < DetailLevel::Signature
            && DetailLevel::Signature < DetailLevel::Block
            && DetailLevel::Block < DetailLevel::Full
    );
}

#[test]
fn example_plan_parses_with_nonempty_steps() {
    let plan = parse_plan(&example_plan()).expect("example parses");
    assert!(!plan.steps.is_empty());
    for step in &plan.steps {
        assert!(!step.id.is_empty(), "step id must be set");
        assert!(!step.tool.is_empty(), "step tool must be set");
    }
}

#[test]
fn plan_parse_failures_carry_invalid_args_discriminant() {
    for bad in [
        json!({"steps": "nope"}),
        json!({}),
        json!({"steps": [{"id": "a"}]}),
        json!({"steps": "nope", "return": "$a"}),
    ] {
        let err = parse_plan(&bad).expect_err("must fail");
        assert!(
            matches!(err, CallError::InvalidArgs(_)),
            "expected InvalidArgs, got {err:?}"
        );
    }
    // Empty steps are absence (Ok), not a failure.
    let empty = parse_plan(&json!({"steps": []})).expect("empty ok");
    assert!(empty.steps.is_empty());
    assert!(empty.return_ref.is_none());
}

#[test]
fn testkit_fixture_helpers_resolve_to_real_content() {
    // The shared oracle helpers must be anchored, not self-fulfilling:
    // the sample root exists on disk and the flagship file is non-empty.
    let root = sample_root();
    assert!(root.is_dir(), "missing sample root: {}", root.display());
    let source = sample_file("src/main.rs");
    assert!(!source.is_empty());
}
