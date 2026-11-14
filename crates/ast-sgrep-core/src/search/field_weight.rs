//! Intent-weighted combination of per-field embedding similarities (7d5x.3).
use crate::intent::QueryIntent;
use crate::semantic_chunk::{FieldVectorMask, SemanticFieldVectors};
use ast_sgrep_embed::cosine_similarity;

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
    crate::store::bundle_semantic::decode_semantic_blob(bytes).ok()
}

/// SQL and bundle fields now share the same f16 bytes and widen only for
/// the f32 cosine kernel.
fn vector_for_query(query: &[f32], bytes: &[u8]) -> Option<Vec<f32>> {
    decode_field_vector(Some(bytes)).filter(|v| v.len() == query.len())
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
        let vector = vector_for_query(query, blob.map(Vec::as_slice)?)?;
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

/// Combine a primary similarity with the intent-weighted field mix.
///
/// The primary vector is the trained joint embedding of the chunk's
/// name+docs+body text, so it already encodes the fields together; the
/// per-field mix is an auxiliary estimate of the same pair. Replacing the
/// primary with the mix let one weak field (a generic body, a sparse
/// docstring) veto a strong joint match — a holdover from the hashed
/// backend, where the primary was a hash projection and the name-field
/// lift had to rescue it. Fields now only lift: the score is the stronger
/// of the two estimates.
pub fn rescore_similarity(
    primary: f32,
    query: &[f32],
    fields: &SemanticFieldVectors,
    intent: QueryIntent,
) -> (f32, Option<EmbedFieldScores>) {
    let weights = field_weights(intent);
    let scores = score_fields(query, fields, weights);
    match combine_field_scores(weights, &scores) {
        Some(mixed) => (primary.max(mixed), Some(scores)),
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

/// Query-side field rank union (`ASGREP_FIELD_RANK_UNION=off|docs|docs-python|all`,
/// default `off`).
///
/// Per-field cosines arrive intent-masked in `notes` (score_fields zeroes
/// weight<=0 fields). Each list — the input concat order plus one ranking
/// per participating field — assigns ranks; a row's order key is its BEST
/// rank across lists, tie-broken by the winning list's cosine (not the
/// joint similarity, which is what body dilution corrupts). Unlike
/// max-over-cosines combination (measured dead on CoSQA 2026-10-17,
/// mixed-sign on documented corpora), rank arithmetic is monotone per row:
/// a row never falls below its concat position.
///
/// `docs` joins only the docs list: gold NL-query separation lives in the
/// docs field (documented-repo corpora), while name/body lists arm
/// identifier-ish distractors — the all-fields union measured
/// ripgrep .582→.531 despite offline gains. `docs-python` additionally
/// gates list membership to `.py` rows: python docs fields carry PEP 257
/// docstrings (orthogonal NL prose below the def), while rust docs fields
/// carry `///` comments that echo the signature — unioning those armed the
/// round-D ripgrep regression (.582→.568, rg-stdin lost). With the gate,
/// a corpus with no `.py` docs rows can never form a docs list, so
/// non-python corpora are bit-identical to `off` by construction.
/// `off` restores the shipped mean-combine similarity order for paired
/// arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRankUnionMode {
    Off,
    Docs,
    DocsPython,
    All,
}

pub fn field_rank_union_mode() -> FieldRankUnionMode {
    // Default Off: both round-D measured variants lost the keep gate
    // (symmetric ripgrep .582→.531 / flask +.009; docs-only
    // ripgrep .582→.568 / flask ±0). The env surface stays for paired
    // arms; see the perf-negative-results ledger entry for the
    // transfer-gap mechanism before re-enabling.
    match std::env::var_os("ASGREP_FIELD_RANK_UNION") {
        Some(v) => match v.to_string_lossy().as_ref() {
            "off" | "0" | "false" => FieldRankUnionMode::Off,
            "all" => FieldRankUnionMode::All,
            "docs" => FieldRankUnionMode::Docs,
            "docs-python" => FieldRankUnionMode::DocsPython,
            _ => FieldRankUnionMode::Off,
        },
        None => FieldRankUnionMode::Off,
    }
}

/// Keys are (best rank across lists, cosine in the list that produced that
/// rank). See [`FieldRankUnionMode`] for the mode semantics. `files` is
/// indexed by chunk idx (aligned with `notes`) and consulted only by
/// [`FieldRankUnionMode::DocsPython`] membership gating.
pub fn field_rank_union_keys(
    ranked: &[(usize, f32)],
    files: &[&str],
    notes: &[Option<EmbedFieldScores>],
    mode: FieldRankUnionMode,
) -> Vec<(u32, f32)> {
    let mut best: Vec<(u32, f32)> = (0..ranked.len() as u32)
        .zip(ranked.iter().map(|(_, score)| *score))
        .collect();
    if mode == FieldRankUnionMode::Off || notes.is_empty() {
        return best;
    }
    let participating: &[usize] = match mode {
        FieldRankUnionMode::Docs | FieldRankUnionMode::DocsPython => &[1], // docs
        FieldRankUnionMode::All => &[0, 1, 2, 3, 4],
        FieldRankUnionMode::Off => &[],
    };
    for &field in participating {
        let mut list: Vec<(f32, usize)> = Vec::new();
        for (pos, (idx, _score)) in ranked.iter().enumerate() {
            if mode == FieldRankUnionMode::DocsPython
                && !files.get(*idx).is_some_and(|f| f.ends_with(".py"))
            {
                continue;
            }
            let Some(cos) = notes
                .get(*idx)
                .and_then(|n| n.as_ref())
                .and_then(|n| match field {
                    0 => n.name,
                    1 => n.docs,
                    2 => n.body,
                    3 => n.graph,
                    _ => n.tests_examples,
                })
            else {
                continue;
            };
            list.push((cos, pos));
        }
        if list.len() < 2 {
            continue;
        }
        list.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (rank, (cos, pos)) in list.into_iter().enumerate() {
            let rank = rank as u32;
            if rank < best[pos].0 || (rank == best[pos].0 && cos > best[pos].1) {
                best[pos] = (rank, cos);
            }
        }
    }
    best
}
