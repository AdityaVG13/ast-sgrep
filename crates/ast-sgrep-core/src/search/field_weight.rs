//! Intent-weighted combination of per-field embedding similarities (7d5x.3).
use crate::intent::QueryIntent;
use crate::semantic_chunk::SemanticFieldVectors;
use ast_sgrep_embed::{cosine_similarity, embed_from_bytes};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldWeights {
    pub name: f32,
    pub docs: f32,
    pub body: f32,
    pub graph: f32,
    pub tests_examples: f32,
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
            name: 0.0,
            docs: 1.0,
            body: 1.0,
            graph: 0.0,
            tests_examples: 1.0,
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

pub fn score_fields(query: &[f32], fields: &SemanticFieldVectors) -> EmbedFieldScores {
    let sim = |blob: Option<&Vec<u8>>| {
        let vector = decode_field_vector(blob.map(Vec::as_slice))?;
        if vector.len() != query.len() {
            return None;
        }
        Some(cosine_similarity(query, &vector))
    };
    EmbedFieldScores {
        name: sim(fields.name.as_ref()),
        docs: sim(fields.docs.as_ref()),
        body: sim(fields.body.as_ref()),
        graph: sim(fields.graph.as_ref()),
        tests_examples: sim(fields.tests_examples.as_ref()),
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
    let scores = score_fields(query, fields);
    match combine_field_scores(field_weights(intent), &scores) {
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
#[path = "../../../../tests/unit/core/search__field_weight.rs"]
mod tests;
