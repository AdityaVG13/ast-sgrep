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

/// A description's best cosine is the evidence. The critic's boosts may not
/// put a hit below the decisive margin above it. A phrase is not a
/// description: the same scores stay in critic order, and a within-margin
/// hit on a description is not clamped either.
/// KILLS: dropping the sentence-boundary gate, or clamping every conceptual hit.
#[test]
fn description_keeps_best_cosine_above_a_far_hit() {
    let far = vec![
        hit(HitKind::Def, "ranges.py", Some("ranges"), 0.08),
        hit(HitKind::Embed, "concurrency.py", Some("install"), 0.04),
    ];
    let far_mags = [Some(0.70), Some(1.0)];
    let description =
        ParsedQuery::parse("1. Purpose: replace the default event loop.\n2. Input: none.\n");
    let mut described = far.clone();
    clamp_far_cosine_on_description(&description, &mut described, &far_mags);
    assert!(
        described[1].score > described[0].score,
        "description must keep the best cosine above a below-margin hit"
    );

    let phrase = ParsedQuery::parse("sort by a token in string python");
    let mut phrased = far.clone();
    clamp_far_cosine_on_description(&phrase, &mut phrased, &far_mags);
    assert!(
        phrased[0].score > phrased[1].score,
        "a phrase query must not clamp; critic order stands"
    );

    let mut close = far.clone();
    clamp_far_cosine_on_description(&description, &mut close, &[Some(0.95), Some(1.0)]);
    assert!(
        close[0].score > close[1].score,
        "within the decisive margin, critic order stands"
    );

    // No embed magnitude: a def the embed channel never scored. Zeroing that
    // vote failed Hit@100. Capping it under the best cosine is the description
    // rule; a phrase leaves the def ahead.
    let mut absent = far.clone();
    clamp_far_cosine_on_description(&description, &mut absent, &[None, Some(1.0)]);
    assert!(
        absent[1].score > absent[0].score,
        "description must keep the best cosine above an embed-absent def"
    );
    let mut absent_phrase = far.clone();
    clamp_far_cosine_on_description(&phrase, &mut absent_phrase, &[None, Some(1.0)]);
    assert!(
        absent_phrase[0].score > absent_phrase[1].score,
        "a phrase must not clamp an embed-absent def"
    );
}

/// The embed channel's own leader is the evidence: when its cosine beats an
/// overrider's by the leader margin, no critic boost may place that
/// overrider above it -- unless fusion itself scored the overrider higher
/// (an earned override). Magnitude ties do not fire: near-duplicates embed
/// equally and there the boost is earned (the frozen-suite failure mode).
/// Hits the embed never scored are never capped. Capped overriders keep
/// their relative order as a staircase under the leader, never a tie blob.
/// KILLS: clamping fusion-earned overrides, clamping on magnitude ties,
/// or collapsing capped overriders onto one score.
#[test]
fn decisive_semantic_leader_keeps_boosted_overriders_below_it() {
    // Leader: embed rank 0 by magnitude, no critic boost. Overrider: weaker
    // cosine by more than the margin, critic boost put it above.
    let leader = hit(HitKind::Embed, "1089.py", Some("numpy"), 0.0464);
    let overrider = hit(HitKind::Embed, "3731.py", Some("read"), 0.0755);
    // Arrays align with the hits vector: [overrider, leader].
    let mags = [Some(0.528), Some(0.640)]; // gap 0.112 >= margin
    let fused = [0.0399, 0.0464]; // fusion ordered the leader first

    let mut hits = vec![overrider.clone(), leader.clone()];
    clamp_boost_overrides_of_semantic_leader(&mut hits, &mags, &fused);
    assert!(
        hits[1].score > hits[0].score,
        "a decisive leader must stay above a boost-manufactured override"
    );

    // Fusion itself scored the overrider higher: earned, stands.
    let mut earned = vec![overrider.clone(), leader.clone()];
    clamp_boost_overrides_of_semantic_leader(
        &mut earned,
        &[Some(0.528), Some(0.640)],
        &[0.0500, 0.0464],
    );
    assert!(
        earned[0].score > earned[1].score,
        "an override fusion itself scored higher must stand"
    );

    // Magnitude tie: near-duplicate embeddings, boost is earned, stands.
    let mut tie = vec![overrider.clone(), leader.clone()];
    clamp_boost_overrides_of_semantic_leader(&mut tie, &[Some(0.635), Some(0.640)], &fused);
    assert!(
        tie[0].score > tie[1].score,
        "a magnitude tie must not fire the clamp"
    );

    // The embed channel never scored the overrider: never capped.
    let mut absent = vec![overrider.clone(), leader.clone()];
    clamp_boost_overrides_of_semantic_leader(&mut absent, &[None, Some(0.640)], &fused);
    assert!(
        absent[0].score > absent[1].score,
        "an embed-absent hit must never be capped"
    );

    // Two overriders: both capped under the leader, relative order kept.
    let second = hit(HitKind::Def, "0456.py", Some("get_list_index"), 0.0700);
    let mut many = vec![overrider.clone(), second, leader.clone()];
    let many_mags = [Some(0.528), Some(0.605), Some(0.640)];
    let many_fused = [0.0399, 0.0414, 0.0464];
    clamp_boost_overrides_of_semantic_leader(&mut many, &many_mags, &many_fused);
    assert!(
        many[2].score > many[0].score && many[2].score > many[1].score,
        "the leader must top both capped overriders"
    );
    assert!(
        many[0].score > many[1].score,
        "capped overriders keep their relative order (staircase, not a tie blob)"
    );
    assert!(
        many[0].score != many[1].score,
        "capped overriders must not land on one shared score"
    );
}

/// The protection generalizes past the leader: the embed channel's second
/// answer (semantic rank 1) also keeps its fusion order against a
/// boost-manufactured override, under the same decisive-margin rule. The
/// near-miss stays honest: a rank-1 hit only just above the overrider in
/// magnitude is still protected (the margin is against the OVERRIDER, not
/// the leader) -- the frozen-suite K-grid showed protecting rank 1 adds
/// +.010 CoSQA / +.003 RepoQA with only borderline drops.
/// KILLS: protecting only the argmax (shipped behavior), or gating the
/// rank-1 protection on a margin against the leader.
#[test]
fn semantic_second_answer_also_keeps_boosted_overriders_below_it() {
    // Magnitudes: leader .640, second .600, overrider .528.
    let leader = hit(HitKind::Embed, "1089.py", Some("numpy"), 0.0464);
    let second = hit(HitKind::Embed, "0512.py", Some("pandas"), 0.0420);
    let overrider = hit(HitKind::Embed, "3731.py", Some("read"), 0.0755);
    let mut hits = vec![overrider.clone(), leader.clone(), second.clone()];
    let mags = [Some(0.528), Some(0.640), Some(0.600)];
    let fused = [0.0399, 0.0464, 0.0420];
    clamp_boost_overrides_of_semantic_leader(&mut hits, &mags, &fused);
    assert!(
        hits[1].score > hits[0].score,
        "the leader must stay above the overrider"
    );
    assert!(
        hits[2].score > hits[0].score,
        "the semantic second answer must also stay above a boost-manufactured override"
    );
    assert!(
        hits[1].score > hits[2].score,
        "the staircase keeps the leader above the second answer"
    );
}
