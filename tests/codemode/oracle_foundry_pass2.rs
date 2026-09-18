//! Pass 2 (oracle-foundry, Mission 4): L2 mutation-discriminating oracles for
//! codemode plan/batch/catalog/session contracts, plugin budget/rendering,
//! and testkit golden/scrub helpers.
//!
//! Each test names the mutant class it kills: alias-table drops, budget
//! `>=`-vs-`>` flips, validation-guard removal, evidence-dropping budgets,
//! version-field over/under-scrubs, and index-path jail escapes. The N-API
//! crate itself is cdylib-only (no Rust-linkable surface), so its
//! Rust-side contracts (session config, root jailing) are pinned here.
//! Failures assert discriminants via `matches!`, never Display text.

use ast_sgrep_codemode::{
    catalog_describe, catalog_search, parse_plan, run_batch, run_plan,
    tool_catalog, BatchCall, BatchRequest, CallError, CodeModeSession, SessionConfig,
    ToolName, MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES,
};
use ast_sgrep_codemode::plan::single_result;
use ast_sgrep_core::search::HitSignal;
use ast_sgrep_core::{HitKind, SearchHit};
use ast_sgrep_plugins::budget::{plan_cost, render, select};
use ast_sgrep_plugins::{DetailLevel, MissReason, OutputBudget, OutputFormat};
use ast_sgrep_testkit::{canonicalize_text, Scrubber};
use serde_json::json;

fn session_at(root: &std::path::Path) -> CodeModeSession {
    CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: None,
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    })
}

#[test]
fn tool_name_aliases_resolve_exactly() {
    // Kills: alias-arm drops, case-folding, whitespace trimming, and
    // as_str/parse skew (roundtrip over all 15 canonical names).
    let cases: &[(&str, Option<ToolName>)] = &[
        ("search", Some(ToolName::Search)),
        ("code_search", Some(ToolName::Search)),
        ("find", Some(ToolName::Find)),
        ("grep", Some(ToolName::Find)),
        ("keyword", Some(ToolName::Find)),
        ("read", Some(ToolName::Read)),
        ("code_read", Some(ToolName::Read)),
        ("edit", Some(ToolName::Edit)),
        ("code_edit", Some(ToolName::Edit)),
        ("semantic", Some(ToolName::Semantic)),
        ("chain", Some(ToolName::Chain)),
        ("defs", Some(ToolName::Defs)),
        ("define", Some(ToolName::Defs)),
        ("definitions", Some(ToolName::Defs)),
        ("callers", Some(ToolName::Callers)),
        ("references", Some(ToolName::Callers)),
        ("imports", Some(ToolName::Imports)),
        ("index_status", Some(ToolName::IndexStatus)),
        ("indexStatus", Some(ToolName::IndexStatus)),
        ("index_repo", Some(ToolName::IndexRepo)),
        ("indexRepo", Some(ToolName::IndexRepo)),
        ("filter_hits", Some(ToolName::FilterHits)),
        ("select", Some(ToolName::Select)),
        ("catalog_search", Some(ToolName::CatalogSearch)),
        ("catalogSearch", Some(ToolName::CatalogSearch)),
        ("catalog_describe", Some(ToolName::CatalogDescribe)),
        ("catalogDescribe", Some(ToolName::CatalogDescribe)),
        ("", None),
        ("Search", None),
        ("SEARCH", None),
        (" search", None),
        ("bogus", None),
    ];
    for (raw, expected) in cases {
        assert_eq!(ToolName::parse(raw), *expected, "raw={raw:?}");
    }
    for def in tool_catalog() {
        let parsed = ToolName::parse(&def.name).expect("catalog name parses");
        assert_eq!(parsed.as_str(), def.name);
    }
}

#[test]
fn call_budget_boundary_is_calls_gte_max() {
    // Kills: `>=` flipped to `>` (a 3rd call at max 2 would succeed) and
    // zero-budget blindness (max 0 must refuse immediately, not once).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 2;
    let args = || json!({"query": "search"});
    assert!(session.call("catalog_search", args()).is_ok());
    assert!(session.call("catalog_search", args()).is_ok());
    assert!(session.exhausted());
    let err = session.call("catalog_search", args()).expect_err("budget");
    assert!(matches!(err, CallError::BudgetExhausted(2)), "got {err:?}");
    let mut zero = session_at(temp.path());
    zero.max_calls = 0;
    assert!(zero.exhausted());
    let err = zero.call("catalog_search", args()).expect_err("zero budget");
    assert!(matches!(err, CallError::BudgetExhausted(0)), "got {err:?}");
}

#[test]
fn run_plan_refuses_empty_duplicate_and_dangling() {
    // Kills: empty-steps acceptance (parse allows it, run must refuse),
    // duplicate-id blindness, unknown-$ref blindness, and ok/call_count lies.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let empty = parse_plan(&json!({"steps": []})).expect("empty parses");
    let err = run_plan(&mut session, &empty).expect_err("empty runs never");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let dup = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "catalog_search", "args": {"query": "a"}},
        {"id": "s", "tool": "catalog_search", "args": {"query": "b"}},
    ]}))
    .expect("dup parses");
    let err = run_plan(&mut session, &dup).expect_err("dup id");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let dangling = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "catalog_search", "args": {"query": "a"}},
    ], "return": "$missing"}))
    .expect("dangling parses");
    let err = run_plan(&mut session, &dangling).expect_err("dangling ref");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let good = parse_plan(&json!({"steps": [
        {"id": "s", "tool": "catalog_search", "args": {"query": "search"}},
    ], "return": "$s"}))
    .expect("good parses");
    let mut fresh = session_at(temp.path());
    let result = run_plan(&mut fresh, &good).expect("good runs");
    assert!(result.ok);
    assert_eq!(result.call_count, 1);
    assert!(result.return_value.get("tools").is_some());
}

fn batch_request(calls: Vec<BatchCall>) -> BatchRequest {
    BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit: None,
        parallel: None,
        parallel_mode: None,
        calls,
    }
}

fn catalog_call(id: &str) -> BatchCall {
    BatchCall {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    }
}

#[test]
fn batch_validation_rejects_before_execution() {
    // Kills: empty-batch acceptance, the 32-call ceiling dropped (33 must
    // fail), empty/oversize id and tool guards dropped. Runs no index I/O:
    // every accepted call below is a pure catalog lookup.
    assert_eq!((MAX_BATCH_CALLS, MAX_BATCH_ID_BYTES, MAX_BATCH_TOOL_BYTES), (32, 128, 128));
    let temp = tempfile::tempdir().expect("tempdir");
    let config = SessionConfig {
        root: temp.path().to_path_buf(),
        index_path: None,
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    };
    let err = run_batch(config.clone(), &batch_request(vec![])).expect_err("empty");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let too_many: Vec<BatchCall> =
        (0..33).map(|i| catalog_call(&format!("c{i}"))).collect();
    let err = run_batch(config.clone(), &batch_request(too_many)).expect_err("33 calls");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let err = run_batch(config.clone(), &batch_request(vec![catalog_call("")])).expect_err("id");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let err = run_batch(
        config.clone(),
        &batch_request(vec![catalog_call(&"i".repeat(129))]),
    )
    .expect_err("long id");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let mut no_tool = catalog_call("t");
    no_tool.tool.clear();
    let err = run_batch(config.clone(), &batch_request(vec![no_tool])).expect_err("tool");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    let mut long_tool = catalog_call("t");
    long_tool.tool = "t".repeat(129);
    let err = run_batch(config.clone(), &batch_request(vec![long_tool])).expect_err("long tool");
    assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    // Accept sides: exactly-32 and exactly-128-byte id execute (pure calls).
    let full: Vec<BatchCall> = (0..32).map(|i| catalog_call(&format!("c{i}"))).collect();
    let ok = run_batch(config.clone(), &batch_request(full)).expect("32 run");
    assert!(ok.all_ok);
    assert_eq!(ok.call_count, 32);
    let edge = run_batch(
        config.clone(),
        &batch_request(vec![catalog_call(&"i".repeat(128))]),
    )
    .expect("128-byte id runs");
    assert!(edge.all_ok);
    // A 128-byte unknown tool passes validation but fails per-call dispatch.
    let mut unknown = catalog_call("u");
    unknown.tool = "u".repeat(128);
    let dispatched = run_batch(config, &batch_request(vec![unknown])).expect("dispatch runs");
    assert!(!dispatched.all_ok);
    assert!(!dispatched.results[0].ok);
    assert_eq!(dispatched.results[0].id, "u");
}

#[test]
fn catalog_search_empty_all_case_fold_miss() {
    // Kills: empty-query filtering (must return the whole catalog),
    // case-fold removal, filter removal (junk must match nothing), and
    // describe case-fold/empty acceptance.
    let all = tool_catalog();
    assert!(!all.is_empty());
    assert_eq!(catalog_search("").len(), all.len());
    assert!(catalog_search("SEARCH").iter().any(|t| t.name == "search"));
    assert!(catalog_search("zzz-no-such-tool").is_empty());
    assert_eq!(catalog_describe("search").expect("search").name, "search");
    assert!(catalog_describe("Search").is_none());
    assert!(catalog_describe("").is_none());
}

fn sample_hit(excerpt: &str) -> SearchHit {
    SearchHit {
        kind: HitKind::Def,
        file: "src/lib.rs".to_string(),
        line_start: 1,
        line_end: 3,
        symbol: Some("foo".to_string()),
        caller: None,
        callee: None,
        language: Some("rust".to_string()),
        score: 3.0,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Def],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: excerpt.to_string(),
        byte_span: None,
    }
}

#[test]
fn output_budget_default_and_select_floor() {
    // Kills: default-value drift, evidence dropping under a zero budget
    // (must degrade to Metadata, never drop rows), upgrade removal, and
    // cost-accounting mutants.
    let default = OutputBudget::default();
    assert_eq!(default.max_tokens, 900);
    assert_eq!(default.default_detail, DetailLevel::Block);
    assert!(select(&[], default).is_empty());
    let hits = vec![sample_hit("fn foo() {\nbar();\n}\n"), sample_hit("fn bar() {}\n")];
    let starved = select(
        &hits,
        OutputBudget {
            max_tokens: 0,
            default_detail: DetailLevel::Metadata,
        },
    );
    assert_eq!(starved.len(), 2);
    assert!(starved.iter().all(|r| r.detail == DetailLevel::Metadata));
    let funded = select(
        &hits,
        OutputBudget {
            max_tokens: 100_000,
            default_detail: DetailLevel::Full,
        },
    );
    assert!(funded.iter().all(|r| r.detail == DetailLevel::Full));
    assert_eq!(plan_cost(&funded), funded.iter().map(|r| r.cost).sum::<usize>());
    let meta = render(&hits[0], DetailLevel::Metadata);
    assert_eq!(meta.body, "");
    let full = render(&hits[0], DetailLevel::Full);
    assert_eq!(full.body, "fn foo() {\nbar();\n}");
    let sig = render(&hits[0], DetailLevel::Signature);
    assert_eq!(sig.body, "fn foo() {\n… bar();");
}

#[test]
fn miss_reason_labels_and_next_steps() {
    // Kills: label swaps and next-step branch/format mutants.
    assert_eq!(
        [
            MissReason::EmptyIndex.as_str(),
            MissReason::FiltersExcludedAll.as_str(),
            MissReason::ChannelUnavailable.as_str(),
            MissReason::NoMatch.as_str(),
        ],
        ["empty_index", "filters_excluded_all", "channel_unavailable", "no_match"]
    );
    assert_eq!(
        MissReason::EmptyIndex.next_step(None),
        "index this project, then search again"
    );
    assert_eq!(
        MissReason::FiltersExcludedAll.next_step(Some("lang")),
        "drop the lang filter"
    );
    assert_eq!(
        MissReason::FiltersExcludedAll.next_step(None),
        "widen the search scope"
    );
    assert_eq!(
        MissReason::ChannelUnavailable.next_step(None),
        "retry on another channel, or repair the unavailable one"
    );
    assert_eq!(
        MissReason::NoMatch.next_step(None),
        "try a shorter or more distinctive term"
    );
}

#[test]
fn golden_canonicalize_trims_unifies_terminates() {
    // Kills: CRLF unification dropped, per-line trim dropped, trailing-blank
    // popping dropped, and the terminating-newline guarantee flipped.
    assert_eq!(canonicalize_text("a  \r\nb\t\n\n"), "a\nb\n");
    assert_eq!(canonicalize_text(""), "");
    assert_eq!(canonicalize_text("x"), "x\n");
    assert_eq!(canonicalize_text("\n\n"), "");
    assert_eq!(canonicalize_text("a\r\n"), "a\n");
    assert_eq!(canonicalize_text("  "), "");
}

#[test]
fn scrubber_machine_contract_splits_version_fields() {
    // Kills: version-field under-scrub (golden flakes on release) and
    // schema_version over-scrub (destroys the contract signal), plus
    // UUID/pass-through mutants in the standard and none presets.
    let scrubbed =
        Scrubber::machine_contract().apply(r#"{"version": "2.0.0", "schema_version": 3}"#);
    assert!(scrubbed.contains(r#""version": "<version>""#), "{scrubbed}");
    assert!(scrubbed.contains(r#""schema_version": 3"#), "{scrubbed}");
    let uuid = Scrubber::standard().apply("id 550e8400-e29b-41d4-a716-446655440000 ok");
    assert!(uuid.contains("<UUID>"), "{uuid}");
    assert!(!uuid.contains("550e"), "{uuid}");
    assert_eq!(Scrubber::standard().apply("hello"), "hello");
    assert_eq!(Scrubber::none().apply(r#"{"version": "2.0.0"}"#), r#"{"version": "2.0.0"}"#);
}

#[test]
fn single_result_wraps_main_with_count_1() {
    // Kills: ok-flag, step-key, return-echo, and call_count mutants.
    let value = json!({"a": 1});
    let result = single_result("search", value.clone());
    assert!(result.ok);
    assert_eq!(result.call_count, 1);
    assert_eq!(result.return_value, value);
    assert_eq!(result.steps.get("main"), Some(&value));
}

#[test]
fn session_pins_relative_index_under_root() {
    // Kills: resolve_session_index_path removal (relative index escapes to
    // process cwd) and absolute-path rewriting. Rust-side pin for the
    // R-CM-ROOT-POLICY contract NAPI inherits.
    let temp = tempfile::tempdir().expect("tempdir");
    let canonical = temp.path().canonicalize().expect("canonical");
    let relative = CodeModeSession::new(SessionConfig {
        root: temp.path().to_path_buf(),
        index_path: Some(std::path::PathBuf::from("custom-index")),
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    });
    assert_eq!(
        relative.config().index_path,
        Some(canonical.join("custom-index"))
    );
    let absolute = CodeModeSession::new(SessionConfig {
        root: temp.path().to_path_buf(),
        index_path: Some(std::path::PathBuf::from("/tmp/abs-index-pass2")),
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    });
    assert_eq!(
        absolute.config().index_path,
        Some(std::path::PathBuf::from("/tmp/abs-index-pass2"))
    );
}
