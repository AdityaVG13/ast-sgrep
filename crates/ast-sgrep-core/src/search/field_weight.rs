//! Intent-weighted combination of per-field embedding similarities (7d5x.3).
use crate::intent::QueryIntent;
use crate::semantic_chunk::{FieldVectorMask, SemanticFieldVectors};
use ast_sgrep_embed::{cosine_similarity, embed_from_bytes};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldWeights {
    pub name: f32,
    pub docs: f32,
    pub body: f32,
    pub graph: f32,
    pub tests_examples: f32,
}

impl FieldWeights {
    pub fn mask(self) -> FieldVectorMask {
        FieldVectorMask::from_positive_weights(
            self.name,
            self.docs,
            self.body,
            self.graph,
            self.tests_examples,
        )
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmbedFieldScores {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_examples: Option<f32>,
}

impl EmbedFieldScores {
    pub fn why_terms(&self) -> Vec<String> {
        let mut out = Vec::new();
        push_why(&mut out, "name", self.name);
        push_why(&mut out, "docs", self.docs);
        push_why(&mut out, "body", self.body);
        push_why(&mut out, "graph", self.graph);
        push_why(&mut out, "tests_examples", self.tests_examples);
        out
    }
}

fn push_why(out: &mut Vec<String>, field: &str, score: Option<f32>) {
    if let Some(score) = score {
        out.push(format!("embed_field:{field}={score:.3}"));
    }
}

pub fn field_weights(intent: QueryIntent) -> FieldWeights {
    match intent {
        QueryIntent::Conceptual => FieldWeights {
            // Names are the conceptual mapping in code (`auth_refresh` for
            // "credential renewal"). Zeroing name made field rescoring ignore
            // the strongest hashed-embed signal on this path.
            name: 0.85,
            docs: 1.0,
            body: 1.0,
            graph: 0.45,
            tests_examples: 0.8,
        },
        QueryIntent::Symbol => FieldWeights {
            name: 1.0,
            docs: 0.0,
            body: 0.0,
            graph: 0.0,
            tests_examples: 0.0,
        },
        QueryIntent::Structural => FieldWeights {
            name: 0.0,
            docs: 0.0,
            body: 1.0,
            graph: 1.0,
            tests_examples: 1.0,
        },
        QueryIntent::Literal => FieldWeights {
            name: 0.0,
            docs: 0.0,
            body: 0.0,
            graph: 0.0,
            tests_examples: 0.0,
        },
    }
}

pub fn decode_field_vector(bytes: Option<&[u8]>) -> Option<Vec<f32>> {
    let bytes = bytes.filter(|b| !b.is_empty())?;
    embed_from_bytes(bytes).ok()
}

pub fn score_fields(
    query: &[f32],
    fields: &SemanticFieldVectors,
    weights: FieldWeights,
) -> EmbedFieldScores {
    let sim = |weight: f32, blob: Option<&Vec<u8>>| {
        if weight <= 0.0 {
            return None;
        }
        let vector = decode_field_vector(blob.map(Vec::as_slice))?;
        if vector.len() != query.len() {
            return None;
        }
        Some(cosine_similarity(query, &vector))
    };
    EmbedFieldScores {
        name: sim(weights.name, fields.name.as_ref()),
        docs: sim(weights.docs, fields.docs.as_ref()),
        body: sim(weights.body, fields.body.as_ref()),
        graph: sim(weights.graph, fields.graph.as_ref()),
        tests_examples: sim(weights.tests_examples, fields.tests_examples.as_ref()),
    }
}

pub fn combine_field_scores(weights: FieldWeights, scores: &EmbedFieldScores) -> Option<f32> {
    let parts = [
        (weights.name, scores.name),
        (weights.docs, scores.docs),
        (weights.body, scores.body),
        (weights.graph, scores.graph),
        (weights.tests_examples, scores.tests_examples),
    ];
    let mut num = 0.0f32;
    let mut den = 0.0f32;
    for (weight, score) in parts {
        if weight > 0.0 {
            if let Some(score) = score {
                num += weight * score;
                den += weight;
            }
        }
    }
    (den > 0.0).then_some(num / den)
}

/// Replace a primary similarity with the intent-weighted field mix when possible.
pub fn rescore_similarity(
    primary: f32,
    query: &[f32],
    fields: &SemanticFieldVectors,
    intent: QueryIntent,
) -> (f32, Option<EmbedFieldScores>) {
    let weights = field_weights(intent);
    let scores = score_fields(query, fields, weights);
    match combine_field_scores(weights, &scores) {
        Some(mixed) => (mixed, Some(scores)),
        None => (
            primary,
            Some(scores).filter(|s| {
                s.name.is_some()
                    || s.docs.is_some()
                    || s.body.is_some()
                    || s.graph.is_some()
                    || s.tests_examples.is_some()
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_chunk::SemanticFieldVectors;

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
        let (score, notes) = rescore_similarity(0.42, &query, &populated_fields(), QueryIntent::Literal);
        assert_eq!(score, 0.42);
        assert!(notes.is_none(), "literal why must not emit unweighted embed_field terms: {notes:?}");
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
        assert!(score > 0.9, "name-only mix should keep the name cosine, got {score}");
        let why = notes.why_terms();
        assert!(why.iter().any(|t| t.starts_with("embed_field:name=")), "{why:?}");
        assert!(why.iter().all(|t| t.starts_with("embed_field:name=")), "{why:?}");
    }

    #[test]
    fn conceptual_intent_includes_name() {
        let weights = field_weights(QueryIntent::Conceptual);
        assert!(weights.name > 0.0, "conceptual name weight must participate");
        assert!(weights.body > 0.0 && weights.docs > 0.0);
        let query = [1.0, 0.0];
        let (score, notes) = rescore_similarity(0.1, &query, &populated_fields(), QueryIntent::Conceptual);
        let notes = notes.expect("conceptual queries expose weighted fields");
        assert!(notes.name.is_some(), "{notes:?}");
        assert!(score > 0.1, "name-inclusive mix should beat the dummy primary, got {score}");
    }
}
