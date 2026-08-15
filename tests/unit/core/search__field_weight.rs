use super::*;
use crate::intent::QueryIntent;
use crate::semantic_chunk::SemanticFieldVectors;
use ast_sgrep_embed::embed_to_bytes;

fn unit(x: f32, y: f32) -> Vec<u8> {
    embed_to_bytes(&[x, y])
}

#[test]
fn conceptual_weights_docs_body_and_examples() {
    let w = field_weights(QueryIntent::Conceptual);
    assert!(w.docs > 0.0 && w.body > 0.0 && w.tests_examples > 0.0);
    assert_eq!(w.name, 0.0);
    assert_eq!(w.graph, 0.0);
}

#[test]
fn symbol_weights_name_only() {
    let w = field_weights(QueryIntent::Symbol);
    assert!(w.name > 0.0);
    assert_eq!(w.docs, 0.0);
    assert_eq!(w.body, 0.0);
    assert_eq!(w.graph, 0.0);
    assert_eq!(w.tests_examples, 0.0);
}

#[test]
fn structural_weights_body_graph_and_examples() {
    let w = field_weights(QueryIntent::Structural);
    assert!(w.body > 0.0 && w.graph > 0.0 && w.tests_examples > 0.0);
    assert_eq!(w.name, 0.0);
    assert_eq!(w.docs, 0.0);
}

#[test]
fn combine_renormalizes_over_present_fields() {
    let scores = EmbedFieldScores {
        name: Some(1.0),
        docs: Some(0.2),
        body: None,
        graph: None,
        tests_examples: None,
    };
    let mixed = combine_field_scores(field_weights(QueryIntent::Conceptual), &scores).unwrap();
    assert!(
        (mixed - 0.2).abs() < 1e-5,
        "docs-only conceptual mix, got {mixed}"
    );
}

#[test]
fn symbol_intent_prefers_name_over_docs() {
    let query = [1.0f32, 0.0];
    let fields = SemanticFieldVectors {
        name: Some(unit(1.0, 0.0)),
        docs: Some(unit(0.0, 1.0)),
        body: None,
        graph: None,
        tests_examples: None,
    };
    let (symbol_score, _) = rescore_similarity(0.1, &query, &fields, QueryIntent::Symbol);
    let (conceptual_score, _) = rescore_similarity(0.1, &query, &fields, QueryIntent::Conceptual);
    assert!(
        symbol_score > conceptual_score,
        "symbol={symbol_score} conceptual={conceptual_score}"
    );
}

#[test]
fn missing_fields_keep_primary_similarity() {
    let fields = SemanticFieldVectors::default();
    let (score, reported) = rescore_similarity(0.42, &[1.0, 0.0], &fields, QueryIntent::Symbol);
    assert!((score - 0.42).abs() < 1e-6);
    assert!(reported.is_none());
}

#[test]
fn why_terms_include_present_fields() {
    let why = EmbedFieldScores {
        name: Some(0.5),
        docs: None,
        body: Some(0.25),
        graph: None,
        tests_examples: Some(0.75),
    }
    .why_terms();
    assert!(why.iter().any(|t| t.starts_with("embed_field:name=")));
    assert!(why.iter().any(|t| t.starts_with("embed_field:body=")));
    assert!(why
        .iter()
        .any(|t| t.starts_with("embed_field:tests_examples=")));
    assert!(why.iter().all(|t| !t.contains("docs")));
}

#[test]
fn hit_why_appends_embed_field_terms() {
    use crate::search::types::{hit_why, HitKind, SearchHit, SpanHitInput};
    let mut hit = SearchHit::span(SpanHitInput {
        kind: HitKind::Embed,
        file: "a.rs".into(),
        line_start: 1,
        line_end: 1,
        score: 0.9,
        excerpt: "body".into(),
        symbol: Some("foo".into()),
        language: Some("rust".into()),
    });
    hit.embed_fields = Some(EmbedFieldScores {
        name: Some(0.5),
        docs: None,
        body: Some(0.25),
        graph: None,
        tests_examples: None,
    });
    let why = hit_why(&hit);
    assert!(why.iter().any(|t| t == "semantic_similarity"));
    assert!(why.iter().any(|t| t.starts_with("embed_field:name=")));
    assert!(why.iter().any(|t| t.starts_with("embed_field:body=")));
}
