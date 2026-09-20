use ast_sgrep_core::intent::QueryIntent;
use ast_sgrep_core::search::field_weight::*;
use ast_sgrep_core::semantic_chunk::SemanticFieldVectors;

fn blob(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn populated_fields() -> SemanticFieldVectors {
    SemanticFieldVectors {
        name: Some(blob(&[1.0, 0.0])),
        docs: Some(blob(&[0.0, 1.0])),
        body: Some(blob(&[1.0, 1.0])),
        graph: Some(blob(&[0.5, 0.5])),
        tests_examples: Some(blob(&[0.0, 0.0])),
    }
}

#[test]
fn literal_intent_skips_zero_weight_why_terms() {
    let query = [1.0, 0.0];
    let (score, notes) =
        rescore_similarity(0.42, &query, &populated_fields(), QueryIntent::Literal);
    assert_eq!(score, 0.42);
    assert!(
        notes.is_none(),
        "literal why must not emit unweighted embed_field terms: {notes:?}"
    );
    assert!(!field_weights(QueryIntent::Literal).mask().any());
}

#[test]
fn symbol_intent_scores_only_name() {
    let query = [1.0, 0.0];
    let (score, notes) = rescore_similarity(0.1, &query, &populated_fields(), QueryIntent::Symbol);
    let notes = notes.expect("symbol queries expose the name field");
    assert!(notes.name.is_some());
    assert!(notes.docs.is_none());
    assert!(notes.body.is_none());
    assert!(notes.graph.is_none());
    assert!(notes.tests_examples.is_none());
    assert!(
        score > 0.9,
        "name-only mix should keep the name cosine, got {score}"
    );
    let why = notes.why_terms();
    assert!(
        why.iter().any(|t| t.starts_with("embed_field:name=")),
        "{why:?}"
    );
    assert!(
        why.iter().all(|t| t.starts_with("embed_field:name=")),
        "{why:?}"
    );
}

#[test]
fn conceptual_intent_includes_name() {
    let weights = field_weights(QueryIntent::Conceptual);
    assert!(
        weights.name > 0.0,
        "conceptual name weight must participate"
    );
    assert!(weights.body > 0.0 && weights.docs > 0.0);
    let query = [1.0, 0.0];
    let (score, notes) =
        rescore_similarity(0.1, &query, &populated_fields(), QueryIntent::Conceptual);
    let notes = notes.expect("conceptual queries expose weighted fields");
    assert!(notes.name.is_some(), "{notes:?}");
    assert!(
        score > 0.1,
        "name-inclusive mix should beat the dummy primary, got {score}"
    );
}
