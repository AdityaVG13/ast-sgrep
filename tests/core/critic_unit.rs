use ast_sgrep_core::intent::QueryIntent;
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::critic::*;
use ast_sgrep_core::search::{HitKind, SearchHit};
use std::collections::HashSet;

fn hit(kind: HitKind, file: &str, symbol: Option<&str>, score: f64) -> SearchHit {
    SearchHit {
        kind,
        file: file.into(),
        line_start: 1,
        line_end: 1,
        symbol: symbol.map(str::to_string),
        caller: None,
        callee: None,
        language: Some("rust".into()),
        score,
        signal: kind.signal(),
        contributors: vec![kind],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: String::new(),
        byte_span: None,
    }
}

/// The critic must adjudicate with the concepts retrieval used. The static
/// groups know compact; the repository vocabulary learned compact ->
/// budget. CompactBudget covers both, so it must outrank an equally scored
/// single-concept symbol -- and must NOT do so without the vocabulary,
/// which is what proves where the credit came from.
/// KILLS: affinity computed from static groups only.
#[test]
fn repository_vocabulary_credits_learned_multi_concept_symbols() {
    let parsed = ParsedQuery::parse("compact output path interning");
    let vocabulary: HashSet<String> = ["budget".to_string()].into_iter().collect();
    let shortlist = || {
        vec![
            hit(HitKind::Def, "src/plugins.rs", Some("compact_kind"), 1.0),
            hit(HitKind::Def, "src/plugins.rs", Some("CompactBudget"), 1.0),
        ]
    };
    let mut with_vocabulary = shortlist();
    apply_critic(
        &parsed,
        QueryIntent::Conceptual,
        &mut with_vocabulary,
        Some(&vocabulary),
    );
    with_vocabulary.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(
        with_vocabulary[0].symbol.as_deref(),
        Some("CompactBudget"),
        "a symbol covering a learned association must outrank a single-concept one"
    );

    let mut without_vocabulary = shortlist();
    apply_critic(
        &parsed,
        QueryIntent::Conceptual,
        &mut without_vocabulary,
        None,
    );
    assert_eq!(
        without_vocabulary[0].score, without_vocabulary[1].score,
        "without the vocabulary the two stay equal: the credit came from the learned set"
    );
}

#[test]
fn exact_identifier_outranks_compound_helpers() {
    let parsed = ParsedQuery::parse("Searcher");
    let mut hits = vec![
        hit(HitKind::Def, "src/bench.rs", Some("bench_searcher"), 0.09),
        hit(HitKind::Def, "src/search.rs", Some("Searcher"), 0.04),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(hits[0].symbol.as_deref(), Some("Searcher"));
    assert!(hits[1].critic.contains(&CriticNote::IdentifierCollision));
}

#[test]
fn exact_case_outranks_folded_homonym() {
    let parsed = ParsedQuery::parse("Searcher");
    let mut hits = vec![
        hit(HitKind::Def, "tests/x.rs", Some("searcher"), 0.09),
        hit(HitKind::Def, "src/search.rs", Some("Searcher"), 0.04),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(hits[0].symbol.as_deref(), Some("Searcher"));
    assert!(hits[1].critic.contains(&CriticNote::IdentifierCollision));
}

#[test]
fn markdown_lexical_loses_to_code_on_conceptual_queries() {
    let parsed = ParsedQuery::parse("credential renewal");
    let mut hits = vec![
        hit(HitKind::Asgrep, "README.md", None, 0.02),
        hit(HitKind::Embed, "src/auth.rs", Some("auth_refresh"), 0.011),
    ];
    apply_critic(&parsed, QueryIntent::Conceptual, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(hits[0].symbol.as_deref(), Some("auth_refresh"));
}

fn caller_hit(file: &str, caller: &str, callee: &str, score: f64) -> SearchHit {
    let mut hit = hit(HitKind::Caller, file, Some(callee), score);
    hit.caller = Some(caller.into());
    hit.callee = Some(callee.into());
    hit
}

#[test]
fn conceptual_def_outranks_generic_entrypoint_callers() {
    let parsed = ParsedQuery::parse("credential renewal");
    let mut hits = vec![
        caller_hit("src/bin.rs", "main", "main", 0.028),
        hit(HitKind::Def, "src/auth.rs", Some("auth_refresh"), 0.016),
    ];
    apply_critic(&parsed, QueryIntent::Conceptual, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(hits[0].symbol.as_deref(), Some("auth_refresh"));
}

#[test]
fn partial_identifier_defs_lose_to_exact_spelling() {
    let parsed = ParsedQuery::parse("auth_refresh");
    let mut hits = vec![
        hit(
            HitKind::Def,
            "tests/cli.rs",
            Some("search_does_not_refresh_stale_index"),
            0.05,
        ),
        hit(HitKind::Def, "src/auth.rs", Some("auth_refresh"), 0.04),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(hits[0].symbol.as_deref(), Some("auth_refresh"));
    assert!(hits[1].critic.contains(&CriticNote::IdentifierCollision));
}

#[test]
fn snake_and_camel_same_identifier_are_exact() {
    assert!(matches!(
        identifier_match("semantic_ivf", "SemanticIvf"),
        IdentifierMatch::Exact
    ));
    assert!(matches!(
        identifier_match("semantic_ivf", "load_semantic_ivf"),
        IdentifierMatch::Compound
    ));
    assert!(matches!(
        identifier_match("Searcher", "bench_searcher"),
        IdentifierMatch::Compound
    ));
}

#[test]
fn file_stem_outranks_measure_helper() {
    let parsed = ParsedQuery::parse("semantic_ivf");
    let mut hits = vec![
        hit(
            HitKind::Def,
            "src/bench_suite.rs",
            Some("measure_semantic_ivf_open_p99"),
            0.09,
        ),
        hit(
            HitKind::Def,
            "src/semantic_ivf.rs",
            Some("load_semantic_ivf"),
            0.04,
        ),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert!(
        hits[0].file.ends_with("semantic_ivf.rs"),
        "expected module file first, got {:?}",
        hits[0].file
    );
}

#[test]
fn conceptual_impl_outranks_relative_tests_path_name_dump() {
    // Live corpus paths are `tests/core/...` with no leading slash.
    // A `/tests/` substring check misses them. The test name dumps the
    // query so fusion can start 20× above the impl; TEST_PATH_PENALTY
    // alone is not enough. Mutant: drop the clamp, or restore the
    // `/tests/` substring check.
    let parsed = ParsedQuery::parse("how does hybrid search work");
    let mut hits = vec![
        hit(
            HitKind::Def,
            "tests/core/cascade_planner.rs",
            Some("hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages"),
            0.50,
        ),
        hit(
            HitKind::Def,
            "crates/ast-sgrep-core/src/search/mod.rs",
            Some("search_hybrid"),
            0.025,
        ),
    ];
    apply_critic(&parsed, QueryIntent::Conceptual, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(hits[0].symbol.as_deref(), Some("search_hybrid"));
    assert!(
        !is_test_path(&hits[0].file),
        "conceptual NL must not lead with a test path, got {:?}",
        hits[0].file
    );
}

#[test]
fn symbol_intent_keeps_exact_test_definition() {
    // Mutant: applying the conceptual tests/ clamp on Symbol intent.
    let parsed = ParsedQuery::parse(
        "hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages",
    );
    let mut hits = vec![
        hit(
            HitKind::Def,
            "tests/core/cascade_planner.rs",
            Some("hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages"),
            0.09,
        ),
        hit(
            HitKind::Def,
            "crates/ast-sgrep-core/src/search/mod.rs",
            Some("search_hybrid"),
            0.04,
        ),
    ];
    apply_critic(&parsed, QueryIntent::Symbol, &mut hits, None);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(
        hits[0].symbol.as_deref(),
        Some("hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages")
    );
}
