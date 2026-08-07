//! Repository-derived semantic lexicon (bead ast-sgrep-tef-semantic-lexicon-ufk7).
//!
//! The shipped semantic fallback expands queries through a handful of
//! hand-written concept groups (auth, refresh, ...). Those generalize badly:
//! they know nothing about the vocabulary of the repository actually being
//! searched, where `hydrator` may mean deserialize and `lease` may mean
//! distributed lock.
//!
//! This module learns those associations from the corpus itself, during
//! indexing, with no network and no model download. It pairs identifier
//! subtokens with the prose terms that co-occur with them (doc comments, test
//! names, and other identifiers in the same symbol) and scores each pair with
//! positive pointwise mutual information under a minimum-support floor.
//!
//! PPMI is used because raw co-occurrence is dominated by frequent, useless
//! terms: `the`, `self`, `value` co-occur with everything. PPMI divides out
//! that background rate and keeps only pairs that occur together *more than
//! chance*, which is precisely the "this repository uses these two words to
//! mean the same thing" signal we want.

use crate::Result;
use std::collections::HashMap;

/// A learned association between two repository terms.
#[derive(Debug, Clone, PartialEq)]
pub struct Association {
    pub term: String,
    pub related: String,
    /// Positive pointwise mutual information. Higher is a stronger, rarer link.
    pub ppmi: f64,
    /// How many distinct symbols contributed this pairing. This is the number
    /// an explanation quotes, because it is the one a human can check.
    pub support: u32,
}

/// Minimum co-occurrence count before a pair is allowed into the lexicon.
///
/// Without a floor, PPMI is maximized by pairs that appear exactly once
/// together and nowhere else -- the noisiest possible evidence.
pub const MIN_SUPPORT: u32 = 3;

/// Cap on stored associations per term, keeping the strongest.
pub const MAX_PER_TERM: usize = 8;

/// Terms too generic to carry meaning in any repository.
const STOP_TERMS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "is", "it", "for", "on", "with", "self",
    "this", "that", "new", "get", "set", "value", "return", "returns", "type", "use", "used",
    "fn", "let", "mut", "pub", "impl", "struct", "enum", "test", "tests", "none", "some", "ok",
    "err", "err", "result", "option", "string", "str", "usize", "i32", "u32", "bool",
];

/// Split an identifier into lowercase subtokens: `refresh_token` and
/// `refreshToken` both yield ["refresh", "token"].
pub fn subtokens(identifier: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for ch in identifier.chars() {
        if ch == '_' || ch == '-' || ch == ':' || ch == '.' || ch.is_whitespace() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        if ch.is_uppercase() && previous_lower && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        previous_lower = ch.is_lowercase() || ch.is_numeric();
        current.extend(ch.to_lowercase());
    }
    if !current.is_empty() {
        out.push(current);
    }
    out.retain(|token| token.len() >= 3 && !STOP_TERMS.contains(&token.as_str()));
    out
}

/// Split prose (doc comment, test name) into lowercase terms.
pub fn prose_terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .flat_map(subtokens)
        .collect()
}

/// One unit of evidence: the identifier subtokens and prose terms that were
/// observed together on the same symbol.
#[derive(Debug, Default, Clone)]
pub struct Observation {
    pub identifier_terms: Vec<String>,
    pub prose_terms: Vec<String>,
}

/// Accumulates co-occurrence counts and turns them into PPMI associations.
#[derive(Debug, Default)]
pub struct LexiconBuilder {
    pair_counts: HashMap<(String, String), u32>,
    term_counts: HashMap<String, u32>,
    observations: u64,
}

impl LexiconBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one symbol's vocabulary. Pairs are directed identifier -> prose,
    /// and also identifier -> identifier, so `rotate_session` can learn from
    /// both its doc comment and its neighbours.
    pub fn observe(&mut self, observation: &Observation) {
        let mut seen_terms: Vec<&String> = Vec::new();
        seen_terms.extend(observation.identifier_terms.iter());
        seen_terms.extend(observation.prose_terms.iter());
        seen_terms.sort();
        seen_terms.dedup();
        if seen_terms.len() < 2 {
            return;
        }
        self.observations += 1;
        for term in &seen_terms {
            *self.term_counts.entry((*term).clone()).or_default() += 1;
        }
        for left in &observation.identifier_terms {
            for right in &seen_terms {
                if left == *right {
                    continue;
                }
                *self
                    .pair_counts
                    .entry((left.clone(), (*right).clone()))
                    .or_default() += 1;
            }
        }
    }

    /// Score pairs with PPMI and keep the strongest per term.
    pub fn finish(&self) -> Vec<Association> {
        if self.observations == 0 {
            return Vec::new();
        }
        let total = self.observations as f64;
        let mut by_term: HashMap<&str, Vec<Association>> = HashMap::new();
        for ((left, right), count) in &self.pair_counts {
            if *count < MIN_SUPPORT {
                continue;
            }
            let left_count = *self.term_counts.get(left).unwrap_or(&0) as f64;
            let right_count = *self.term_counts.get(right).unwrap_or(&0) as f64;
            if left_count <= 0.0 || right_count <= 0.0 {
                continue;
            }
            let joint = *count as f64 / total;
            let expected = (left_count / total) * (right_count / total);
            if expected <= 0.0 {
                continue;
            }
            let pmi = (joint / expected).ln();
            if pmi <= 0.0 {
                // Negative PMI means the pair co-occurs LESS than chance, which
                // is evidence against an association, not for one.
                continue;
            }
            by_term.entry(left.as_str()).or_default().push(Association {
                term: left.clone(),
                related: right.clone(),
                ppmi: pmi,
                support: *count,
            });
        }
        let mut out = Vec::new();
        for (_, mut associations) in by_term {
            associations.sort_by(|a, b| {
                b.ppmi
                    .partial_cmp(&a.ppmi)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.support.cmp(&a.support))
                    // Deterministic tie-break so the stored lexicon is stable.
                    .then_with(|| a.related.cmp(&b.related))
            });
            associations.truncate(MAX_PER_TERM);
            out.extend(associations);
        }
        out.sort_by(|a, b| a.term.cmp(&b.term).then_with(|| a.related.cmp(&b.related)));
        out
    }
}

/// Persisted lexicon, loaded for query expansion.
#[derive(Debug, Default, Clone)]
pub struct Lexicon {
    by_term: HashMap<String, Vec<Association>>,
}

impl Lexicon {
    pub fn from_associations(associations: Vec<Association>) -> Self {
        let mut by_term: HashMap<String, Vec<Association>> = HashMap::new();
        for association in associations {
            by_term
                .entry(association.term.clone())
                .or_default()
                .push(association);
        }
        Self { by_term }
    }

    pub fn is_empty(&self) -> bool {
        self.by_term.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_term.values().map(Vec::len).sum()
    }

    /// Terms this repository associates with `term`, strongest first.
    pub fn related(&self, term: &str) -> &[Association] {
        self.by_term
            .get(term)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Expand a query with repository-learned terms, returning the added terms
    /// paired with the evidence that justified them.
    ///
    /// Evidence travels with the expansion so the engine can say *why* it
    /// widened a query instead of silently changing what the user asked for.
    pub fn expand(&self, query_terms: &[String], max_added: usize) -> Vec<Association> {
        let mut added: Vec<Association> = Vec::new();
        for term in query_terms {
            for association in self.related(term) {
                if query_terms.contains(&association.related)
                    || added.iter().any(|a| a.related == association.related)
                {
                    continue;
                }
                added.push(association.clone());
            }
        }
        added.sort_by(|a, b| {
            b.ppmi
                .partial_cmp(&a.ppmi)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.related.cmp(&b.related))
        });
        added.truncate(max_added);
        added
    }
}

/// Human-readable justification for one learned association (ufk7).
pub fn explain(association: &Association) -> String {
    format!(
        "repository association: \"{}\" → \"{}\" through {} co-occurrences",
        association.term, association.related, association.support
    )
}

/// Build a lexicon from symbol observations and persist it (ufk7).
pub fn store_lexicon(store: &crate::store::IndexStore, associations: &[Association]) -> Result<()> {
    store.replace_lexicon(associations)
}

/// Load the persisted lexicon (ufk7).
pub fn load_lexicon(store: &crate::store::IndexStore) -> Result<Lexicon> {
    Ok(Lexicon::from_associations(store.all_lexicon_rows()?))
}
