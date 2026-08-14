//! Post-fusion deterministic critic (P0 critic-on-shortlist).
//!
//! The critic is the in-process "second model": after fusion it checks whether
//! each hit carries the kind of evidence the query asked for, instead of
//! trusting cosine similarity or any single channel.
//!
//! Rules (all deterministic, no model, no network):
//!
//! 1. **Corroboration gate.** An embed-only hit whose parent span has no
//!    lexical or structural corroboration in the shortlist is dropped, unless
//!    the query is conceptual and the structural stage produced nothing (the
//!    explicitly allowed semantic path). Kept-but-uncorroborated hits are
//!    labeled so the caller can see the weaker evidence class.
//! 2. **Agreement boost.** A hit where semantic and structural channels fused
//!    on the same span is boosted; definition + usage + semantic agreement is
//!    boosted further. Signal provenance is untouched: a boosted semantic hit
//!    stays `semantic`.
//! 3. **Identifier-collision penalty.** When the query names a compound
//!    identifier (`auth_refresh`), a hit whose symbol is only a fragment of it
//!    (`refresh`) is penalized unless the hit itself evidences the full
//!    identifier.
//!
//! The critic adjusts scores and annotates hits before `finish_response`
//! assigns margins and confidence, so downstream honesty fields reflect the
//! critiqued ordering.

use crate::intent::QueryIntent;
use crate::query::ParsedQuery;
use crate::search::types::{HitKind, SearchHit};
use std::collections::{HashMap, HashSet};

/// Score multiplier when semantic evidence agrees with any structural channel.
pub const AGREEMENT_BOOST: f64 = 1.15;
/// Score multiplier when definition, usage, and semantic evidence all agree.
pub const FULL_AGREEMENT_BOOST: f64 = 1.25;
/// Score multiplier for identifier-fragment collisions.
pub const COLLISION_PENALTY: f64 = 0.85;

/// Engine-derived critic annotation. Never trusted from the wire; JSON decode
/// re-derives an empty set (same policy as `resolution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriticNote {
    /// Structural and semantic channels fused on the same span.
    ChannelAgreement,
    /// Definition, usage (caller/graph), and semantic evidence all agree.
    FullAgreement,
    /// Embed-only hit kept without corroboration (allowed conceptual path).
    SemanticUncorroborated,
    /// Symbol is a fragment of a longer query identifier.
    IdentifierCollision,
}

impl CriticNote {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChannelAgreement => "channel_agreement",
            Self::FullAgreement => "full_agreement",
            Self::SemanticUncorroborated => "semantic_uncorroborated",
            Self::IdentifierCollision => "identifier_collision",
        }
    }
}

fn has_kind(hit: &SearchHit, want: impl Fn(HitKind) -> bool) -> bool {
    hit.contributors.iter().copied().any(&want) || want(hit.kind)
}

fn is_structural_kind(kind: HitKind) -> bool {
    matches!(
        kind,
        HitKind::Def
            | HitKind::Caller
            | HitKind::Graph
            | HitKind::Anchor
            | HitKind::Import
            | HitKind::Pattern
    )
}

fn is_usage_kind(kind: HitKind) -> bool {
    matches!(kind, HitKind::Caller | HitKind::Graph)
}

fn embed_only(hit: &SearchHit) -> bool {
    hit.kind == HitKind::Embed && hit.contributors.iter().all(|k| *k == HitKind::Embed)
}

/// Non-embed evidence in one file, used to corroborate embed-only spans.
struct Corroborator {
    line_start: u32,
    line_end: u32,
    symbol: Option<String>,
    callee: Option<String>,
}

fn spans_overlap(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> bool {
    a_start <= b_end && b_start <= a_end
}

fn corroborates(witness: &Corroborator, hit: &SearchHit) -> bool {
    if spans_overlap(
        witness.line_start,
        witness.line_end,
        hit.line_start,
        hit.line_end,
    ) {
        return true;
    }
    match hit.symbol.as_deref() {
        Some(symbol) => {
            let symbol = symbol.to_lowercase();
            let matches =
                |value: Option<&str>| value.is_some_and(|value| value.to_lowercase() == symbol);
            matches(witness.symbol.as_deref()) || matches(witness.callee.as_deref())
        }
        None => false,
    }
}

/// Compound query identifiers and their fragments (`auth_refresh` -> {auth, refresh}).
fn identifier_fragments(parsed: &ParsedQuery) -> HashMap<String, String> {
    let mut fragment_to_ident = HashMap::new();
    for term in &parsed.terms {
        if !term.contains('_') {
            continue;
        }
        for fragment in term.split('_').filter(|f| f.len() > 1) {
            fragment_to_ident.insert(fragment.to_string(), term.clone());
        }
    }
    fragment_to_ident
}

fn push_note(hit: &mut SearchHit, note: CriticNote) {
    if !hit.critic.contains(&note) {
        hit.critic.push(note);
    }
}

/// Apply the deterministic critic to a fused hybrid shortlist.
///
/// Runs after `fusion::apply_weighted_rrf` (contributor sets are final) and
/// before `finish_response` (margins/confidence see critiqued scores).
pub(crate) fn apply_critic(parsed: &ParsedQuery, intent: QueryIntent, hits: &mut Vec<SearchHit>) {
    if hits.is_empty() {
        return;
    }
    let structural_present = hits
        .iter()
        .any(|hit| hit.contributors.iter().copied().any(is_structural_kind));
    let semantic_only_allowed = intent == QueryIntent::Conceptual && !structural_present;

    // Non-embed witnesses per file: spans and symbols that can corroborate an
    // embed-only parent (child chunks map to parent symbols).
    let mut witnesses: HashMap<&str, Vec<Corroborator>> = HashMap::new();
    for hit in hits.iter() {
        if embed_only(hit) {
            continue;
        }
        witnesses
            .entry(hit.file.as_str())
            .or_default()
            .push(Corroborator {
                line_start: hit.line_start,
                line_end: hit.line_end,
                symbol: hit.symbol.clone(),
                callee: hit.callee.clone(),
            });
    }
    let mut drop: HashSet<usize> = HashSet::new();
    let mut uncorroborated: HashSet<usize> = HashSet::new();
    for (index, hit) in hits.iter().enumerate() {
        if !embed_only(hit) {
            continue;
        }
        let corroborated = witnesses
            .get(hit.file.as_str())
            .is_some_and(|file_witnesses| {
                file_witnesses
                    .iter()
                    .any(|witness| corroborates(witness, hit))
            });
        if corroborated {
            continue;
        }
        if semantic_only_allowed {
            uncorroborated.insert(index);
        } else {
            drop.insert(index);
        }
    }

    let fragments = identifier_fragments(parsed);
    let mut kept = Vec::with_capacity(hits.len());
    for (index, mut hit) in hits.drain(..).enumerate() {
        if drop.contains(&index) {
            continue;
        }
        if uncorroborated.contains(&index) {
            push_note(&mut hit, CriticNote::SemanticUncorroborated);
        }
        let has_embed = has_kind(&hit, |k| k == HitKind::Embed);
        let has_structural = has_kind(&hit, is_structural_kind);
        if has_embed && has_structural {
            let has_def = has_kind(&hit, |k| k == HitKind::Def);
            let has_usage = has_kind(&hit, is_usage_kind);
            if has_def && has_usage {
                hit.score *= FULL_AGREEMENT_BOOST;
                push_note(&mut hit, CriticNote::FullAgreement);
            } else {
                hit.score *= AGREEMENT_BOOST;
                push_note(&mut hit, CriticNote::ChannelAgreement);
            }
        }
        if let Some(symbol) = hit.symbol.as_deref() {
            let symbol = symbol.to_lowercase();
            if let Some(full_ident) = fragments.get(&symbol) {
                let evidences_full = hit.excerpt.to_lowercase().contains(full_ident.as_str());
                if !evidences_full {
                    hit.score *= COLLISION_PENALTY;
                    push_note(&mut hit, CriticNote::IdentifierCollision);
                }
            }
        }
        kept.push(hit);
    }
    *hits = kept;
}

#[cfg(test)]
#[path = "../../../../tests/unit/core/search__critic.rs"]
mod tests;
