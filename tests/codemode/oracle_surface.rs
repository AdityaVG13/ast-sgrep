//! Oracle SURFACE for codemode (consolidated suite): session root policy,
//! error taxonomy, name/alias tables, and shared golden/edge helpers.
//!
//! Consolidates the `other`/`session` facets of `oracle_foundry_pass1.rs`
//! (formats, fixtures), `pass2.rs` (aliases, catalog, miss reasons,
//! canonicalize, scrubber, pinning) and `pass3.rs` (taxonomy, empty/unicode
//! edges, cross-path agreement) into 3 intent-grouped tests: each `#[test]`
//! owns ONE intent with multiple facets. Plan/batch/budget intents live in
//! `oracle_core.rs`; end-to-end compositions live in `oracle_e2e.rs`.
//! Catalog: `tests/catalog/oracle-codemode.md`.
//!
//! Discipline (inherited): hand-computed expectations; failures assert enum
//! discriminants via `matches!`, never Display text. Session/batch builders
//! come from testkit (`session_at`, `config_at`, `batch_request`,
//! `catalog_call` 2-arg canonical form); golden/fixture helpers come from
//! testkit (`canonicalize_text`, `Scrubber`, `sample_root`, `sample_file`).
//! This file needs no file-local helpers — every builder it uses already
//! exists in testkit.

use ast_sgrep_codemode::{
    catalog_describe, catalog_search, parse_plan, run_batch, run_plan, tool_catalog, BatchCall,
    CallError, ToolName,
};
use ast_sgrep_plugins::{MissReason, OutputFormat};
use ast_sgrep_testkit::{
    batch_request, canonicalize_text, catalog_call, config_at, sample_file, sample_root,
    session_at, Scrubber,
};
use serde_json::{json, Value};

/// INTENT=session root policy + error taxonomy: relative index paths pin
/// under the session root while absolute paths pass through untouched;
/// unknown tools fail as UnknownTool, bad args as InvalidArgs, and unknown
/// tools inside a batch stay per-call failures (Ok envelope, never top-level
/// Err).
/// KILLS=jail-escape, absolute-rewrite, taxonomy-collapse (unknown vs
/// invalid-args confusion), batch-dispatch-as-validation mutants.
/// ABSORBS=session_pins_relative_index_under_root,
/// error_taxonomy_splits_unknown_tool_from_invalid_args.
#[test]
fn session_pins_index_and_taxonomy_splits_unknown_from_invalid() {
    // Facet 1 (index pinning): a relative index resolves under the root
    // (never the process cwd); an absolute index is left untouched.
    // Rust-side pin for the R-CM-ROOT-POLICY contract NAPI inherits.
    let temp = tempfile::tempdir().expect("tempdir");
    let canonical = temp.path().canonicalize().expect("canonical");
    let relative = ast_sgrep_codemode::CodeModeSession::new(ast_sgrep_codemode::SessionConfig {
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
    let absolute = ast_sgrep_codemode::CodeModeSession::new(ast_sgrep_codemode::SessionConfig {
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

    // Facet 2 (taxonomy): dispatch failures are UnknownTool; guard failures
    // are InvalidArgs; batch dispatch failures stay per-call (Ok envelope)
    // while validation failures are top-level Err. Discriminants only.
    let mut session = session_at(temp.path());
    for unknown in ["no-such-tool", "Search", " search", ""] {
        let err = session
            .call(unknown, json!({"query": "x"}))
            .expect_err("unknown tool");
        assert!(
            matches!(err, CallError::UnknownTool(_)),
            "tool {unknown:?}: got {err:?}"
        );
    }
    let invalid: &[(&str, Value)] = &[
        ("catalog_search", json!({})),
        ("catalog_search", json!({"query": ""})),
        ("catalog_search", json!({"query": "   "})),
        ("catalog_describe", json!({"name": "no-such-tool"})),
        ("catalog_describe", json!({})),
        ("defs", json!({})),
        ("defs", json!({"symbol": "  "})),
        ("imports", json!({})),
        ("select", json!({})),
        ("select", json!({"value": {"a": 1}})),
        ("select", json!({"value": {"a": 1}, "fields": []})),
        ("select", json!({"value": 42, "fields": ["a"]})),
        ("filter_hits", json!({})),
        ("filter_hits", json!({"hits": {"not": "hits"}})),
    ];
    for (tool, args) in invalid {
        let err = session.call(tool, args.clone()).expect_err("invalid args");
        assert!(
            matches!(err, CallError::InvalidArgs(_)),
            "tool {tool}: got {err:?}"
        );
    }
    let mut bad = catalog_call("u", "search");
    bad.tool = "no-such-tool".to_string();
    let dispatched = run_batch(config_at(temp.path()), &batch_request(vec![bad])).expect("runs");
    assert!(!dispatched.all_ok);
    assert!(!dispatched.results[0].ok);
    assert!(dispatched.results[0].error.is_some());
}

/// INTENT=name resolution contracts: output formats map 13 aliases
/// case-insensitively and reject bogus/empty; tool names resolve 26
/// alias/case/whitespace cases exactly with a catalog roundtrip;
/// catalog_search returns all on empty, folds case, misses junk, and describe
/// is exact-only; miss reasons carry exact labels and next-step branches.
/// KILLS=alias-arm-drop, case-fold/trim mutants, as_str/parse skew,
/// filter-removal, describe case-fold/empty acceptance, label-swap,
/// branch-mutant.
/// ABSORBS=output_format_parse_matches_hand_table,
/// tool_name_aliases_resolve_exactly, catalog_search_empty_all_case_fold_miss,
/// miss_reason_labels_and_next_steps.
#[test]
fn names_aliases_catalog_lookup_and_miss_labels_resolve_exactly() {
    // Facet 1 (output formats): 13 aliases map case-insensitively;
    // bogus/empty reject.
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

    // Facet 2 (tool aliases): 26 alias/case/whitespace cases resolve exactly;
    // every catalog name roundtrips through parse/as_str.
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
        let parsed = ToolName::parse(def.name).expect("catalog name parses");
        assert_eq!(parsed.as_str(), def.name);
    }

    // Facet 3 (catalog lookup): empty query returns the whole catalog;
    // search folds case; junk matches nothing; describe is exact-only
    // (case-sensitive, rejects empty).
    let all = tool_catalog();
    assert!(!all.is_empty());
    assert_eq!(catalog_search("").len(), all.len());
    assert!(catalog_search("SEARCH").iter().any(|t| t.name == "search"));
    assert!(catalog_search("zzz-no-such-tool").is_empty());
    assert_eq!(catalog_describe("search").expect("search").name, "search");
    assert!(catalog_describe("Search").is_none());
    assert!(catalog_describe("").is_none());

    // Facet 4 (miss reasons): 4 labels + 5 next_step branches exact.
    assert_eq!(
        [
            MissReason::EmptyIndex.as_str(),
            MissReason::FiltersExcludedAll.as_str(),
            MissReason::ChannelUnavailable.as_str(),
            MissReason::NoMatch.as_str(),
        ],
        [
            "empty_index",
            "filters_excluded_all",
            "channel_unavailable",
            "no_match"
        ]
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

/// INTENT=shared-helper anchors and adversarial edges: the sample fixture
/// root resolves to real on-disk content; canonicalize trims/unifies/
/// terminates per 6 point-values and is an idempotent fixpoint over a
/// 10-input corpus; the scrubber splits version from schema_version and
/// placeholders UUIDs; empty plan/batch failures are repeat-stable and empty
/// transform inputs yield zero shapes; unicode ids roundtrip, unicode tools
/// are unknown, unicode queries are deterministic-empty, and batch ids echo;
/// filter_hits/select agree across call/plan/batch paths with monotone
/// limits and hand-computed select shapes.
/// KILLS=fixture-rot, trim-drop, CRLF-drop, pop-drop, terminate-flip,
/// non-fixpoint-mutant, under-scrub, over-scrub, error-caching, empty-flap,
/// unicode-roundtrip/case-mutant, dispatch-path-divergence.
/// ABSORBS=testkit_fixture_helpers_resolve_to_real_content,
/// golden_canonicalize_trims_unifies_terminates,
/// golden_canonicalize_is_idempotent_fixpoint (fixpoint leg),
/// scrubber_machine_contract_splits_version_fields,
/// empty_plan_and_batch_edges_fail_stably, unicode_and_empty_adversarial_inputs,
/// pure_transforms_agree_across_call_plan_batch_paths.
#[test]
fn helpers_anchor_golden_edges_and_transforms_agree_across_paths() {
    // Facet 1 (fixture anchor): the shared oracle helpers are anchored, not
    // self-fulfilling — the sample root exists and the flagship file is
    // non-empty. Sole filesystem-anchor pin.
    let root = sample_root();
    assert!(root.is_dir(), "missing sample root: {}", root.display());
    let source = sample_file("src/main.rs");
    assert!(!source.is_empty());

    // Facet 2 (canonicalize point-values + fixpoint): trim/CRLF-unify/
    // blank-pop/terminate per 6 hand values; re-canonicalizing never changes
    // output over a CRLF/tab/unicode/blank corpus; leading whitespace is
    // preserved while trailing is cut.
    assert_eq!(canonicalize_text("a  \r\nb\t\n\n"), "a\nb\n");
    assert_eq!(canonicalize_text(""), "");
    assert_eq!(canonicalize_text("x"), "x\n");
    assert_eq!(canonicalize_text("\n\n"), "");
    assert_eq!(canonicalize_text("a\r\n"), "a\n");
    assert_eq!(canonicalize_text("  "), "");
    let corpus = [
        "a  \r\nb\t\n\n",
        "",
        "x",
        "\n\n",
        "a\r\n",
        "  ",
        "héllo  \r\nwörld\t\n\n\n",
        "a\nb\n",
        "  indented\n\ttabbed  \n",
        "emoji 🔍 trailing   \r\n",
    ];
    for input in corpus {
        let once = canonicalize_text(input);
        let twice = canonicalize_text(&once);
        assert_eq!(once, twice, "not idempotent for {input:?}");
    }
    assert_eq!(
        canonicalize_text("  indented\n\ttabbed  \n"),
        "  indented\n\ttabbed\n"
    );
    assert_eq!(
        canonicalize_text("héllo  \r\nwörld\t\n\n\n"),
        "héllo\nwörld\n"
    );

    // Facet 3 (scrubber): version scrubs but schema_version survives;
    // UUIDs become placeholders; clean input and the none preset pass
    // through byte-identical.
    let scrubbed =
        Scrubber::machine_contract().apply(r#"{"version": "2.0.0", "schema_version": 3}"#);
    assert!(scrubbed.contains(r#""version": "<version>""#), "{scrubbed}");
    assert!(scrubbed.contains(r#""schema_version": 3"#), "{scrubbed}");
    let uuid = Scrubber::standard().apply("id 550e8400-e29b-41d4-a716-446655440000 ok");
    assert!(uuid.contains("<UUID>"), "{uuid}");
    assert!(!uuid.contains("550e"), "{uuid}");
    assert_eq!(Scrubber::standard().apply("hello"), "hello");
    assert_eq!(
        Scrubber::none().apply(r#"{"version": "2.0.0"}"#),
        r#"{"version": "2.0.0"}"#
    );

    // Facet 4 (empty edges): empty-plan and empty-batch runs fail with the
    // same discriminant on repeat (no state bleed); empty transform inputs
    // stay Ok with hand-computed zero shapes.
    let temp = tempfile::tempdir().expect("tempdir");
    let empty_plan = parse_plan(&json!({"steps": []})).expect("empty parses");
    for _ in 0..2 {
        let mut session = session_at(temp.path());
        let err = run_plan(&mut session, &empty_plan).expect_err("empty never runs");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
        let err = run_batch(config_at(temp.path()), &batch_request(vec![])).expect_err("empty");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    }
    let mut session = session_at(temp.path());
    let filtered = session
        .call("filter_hits", json!({"hits": []}))
        .expect("empty hits ok");
    assert_eq!(filtered["hit_count"], json!(0));
    assert_eq!(filtered["hits"], json!([]));
    let projected = session
        .call("select", json!({"value": [], "fields": ["a"]}))
        .expect("empty array ok");
    assert_eq!(projected, json!([]));

    // Facet 5 (unicode): unicode step ids roundtrip byte-exact; unicode tool
    // names are UnknownTool; unicode queries are deterministic with a
    // hand-computed empty match; batch ids echo byte-exact; the direct
    // catalog fn agrees with the session path.
    let plan = parse_plan(&json!({"steps": [
        {"id": "étape-🔍", "tool": "catalog_search", "args": {"query": "search"}},
    ], "return": "$étape-🔍.tools.0.name"}))
    .expect("unicode plan parses");
    let mut session = session_at(temp.path());
    let result = run_plan(&mut session, &plan).expect("unicode plan runs");
    assert!(result.steps.contains_key("étape-🔍"));
    assert_eq!(result.return_value, json!("search"));
    let err = session
        .call("searχ", json!({"query": "x"}))
        .expect_err("unicode tool unknown");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    let v1 = session
        .call("catalog_search", json!({"query": "héllo"}))
        .expect("unicode query ok");
    let v2 = session
        .call("catalog_search", json!({"query": "héllo"}))
        .expect("unicode query ok twice");
    assert_eq!(v1, v2);
    assert_eq!(v1["tools"], json!([]));
    assert_eq!(catalog_search("héllo").len(), 0);
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![catalog_call("🔍-id-✓", "search")]),
    )
    .expect("unicode batch id runs");
    assert!(batch.all_ok);
    assert_eq!(batch.results[0].id, "🔍-id-✓");

    // Facet 6 (cross-path agreement): filter_hits/select agree across
    // session.call, plan steps, and batch calls — three independent dispatch
    // paths, one semantics. Limits are monotone; select drops unknown fields
    // and truncates by limit; the session catalog payload wraps the direct fn.
    let hits = json!([
        {"kind": "def", "file": "src/a.rs", "score": 9.0},
        {"kind": "ref", "file": "src/b.rs", "score": 1.0},
        {"kind": "def", "file": "tests/c.rs", "score": 5.0},
    ]);
    let filter_args = json!({"hits": hits, "kind": "def", "path_contains": "src/", "min_score": 2.0, "limit": 10});
    let mut session = session_at(temp.path());
    let direct = session
        .call("filter_hits", filter_args.clone())
        .expect("direct");
    assert_eq!(direct["hit_count"], json!(1));
    assert_eq!(direct["hits"][0]["file"], json!("src/a.rs"));
    let tight = session
        .call("filter_hits", json!({"hits": hits, "limit": 1}))
        .expect("limit 1");
    let loose = session
        .call("filter_hits", json!({"hits": hits}))
        .expect("no limit");
    assert_eq!(tight["hit_count"], json!(1));
    assert_eq!(loose["hit_count"], json!(3));
    let plan = parse_plan(&json!({"steps": [
        {"id": "f", "tool": "filter_hits", "args": filter_args},
    ]}))
    .expect("filter plan parses");
    let mut planned = session_at(temp.path());
    let pr = run_plan(&mut planned, &plan).expect("filter plan runs");
    assert_eq!(pr.return_value, direct);
    let batch = run_batch(
        config_at(temp.path()),
        &batch_request(vec![BatchCall {
            id: "f".to_string(),
            tool: "filter_hits".to_string(),
            args: filter_args,
        }]),
    )
    .expect("filter batch runs");
    assert!(batch.all_ok);
    assert_eq!(batch.results[0].value.as_ref().expect("value"), &direct);
    let projected = session
        .call(
            "select",
            json!({"value": {"a": 1, "b": 2}, "fields": ["a", "zzz"]}),
        )
        .expect("select");
    assert_eq!(projected, json!({"a": 1}));
    let truncated = session
        .call(
            "select",
            json!({"value": [{"a": 1}, {"a": 2}], "fields": ["a"], "limit": 1}),
        )
        .expect("select limit");
    assert_eq!(truncated, json!([{"a": 1}]));
    let via_session = session
        .call("catalog_search", json!({"query": "chain"}))
        .expect("catalog via session");
    let direct_tools = catalog_search("chain");
    assert_eq!(
        via_session["tools"].as_array().expect("array").len(),
        direct_tools.len()
    );
    assert_eq!(via_session["tools"][0]["name"], json!(direct_tools[0].name));
    assert!(!direct_tools.is_empty());
    let catalog_names: Vec<&str> = tool_catalog().iter().map(|t| t.name).collect();
    assert!(direct_tools.iter().all(|t| catalog_names.contains(&t.name)));
}
