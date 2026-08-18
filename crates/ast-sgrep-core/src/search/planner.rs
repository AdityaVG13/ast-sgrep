//! Causal query planner (P0 causal-follow-ups).
//!
//! `follow_up_queries` and `suggested_next` used to be templates: every hit
//! with a symbol got `defs:` and `callers:` stamped on it. The planner makes
//! them causal. It reads the evidence each hit actually carries (contributor
//! channels, critic notes, within-signal margin) and emits only the
//! executable queries the engine itself would run next to close the remaining
//! evidence gap. A hit whose definition, usage, and ordering are already
//! settled gets no follow-ups: the plan is "you are done", not a template.
//!
//! Everything here is deterministic and runs on the finished response, after
//! `finish_response` has assigned margins, so the plan reflects exactly what
//! the caller was shown.

use crate::search::critic::CriticNote;
use crate::search::types::{HitKind, SearchHit, SearchResponse};

/// Quote one arbitrary argument for a POSIX shell without interpolation.
fn quote_shell_arg(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', "'\\''"))
}

/// A hit's margin must be at least this fraction of its score before the
/// ordering counts as decisive. Margins are absolute within-signal score gaps
/// (`assign_signal_margins`), so they are compared relative to the hit's own
/// score rather than against a fixed constant. Engine default, not a
/// certified weight.
pub const DECISIVE_MARGIN_RATIO: f64 = 0.10;

/// True when the hit is decisively separated from the next hit in its signal
/// group. Singletons and ties carry margin 0 and are never decisive.
pub fn margin_is_decisive(hit: &SearchHit) -> bool {
    hit.score > 0.0 && hit.margin >= DECISIVE_MARGIN_RATIO * hit.score
}

fn hit_symbol(hit: &SearchHit) -> Option<&str> {
    hit.symbol
        .as_deref()
        .or(hit.callee.as_deref())
        .or(hit.caller.as_deref())
}

fn has_kind(hit: &SearchHit, want: impl Fn(HitKind) -> bool) -> bool {
    hit.contributors.iter().copied().any(&want) || want(hit.kind)
}

/// The compound query identifier (contains `_`) that a fragment symbol
/// collides with, if any. Mirrors the critic's fragment rule.
fn compound_query_identifier<'q>(query: &'q str, fragment: &str) -> Option<&'q str> {
    let fragment = fragment.to_lowercase();
    query
        .split_whitespace()
        .filter(|term| term.contains('_'))
        .find(|term| term.to_lowercase().split('_').any(|part| part == fragment))
}

/// Executable follow-up queries the engine itself would run next for this
/// hit. Empty when the hit's evidence is complete and its ordering decisive.
pub fn follow_ups_for_hit(query: &str, hit: &SearchHit) -> Vec<String> {
    // Identifier collision: the drill-down target is the identifier the
    // query actually named, not the colliding fragment.
    if hit.critic.contains(&CriticNote::IdentifierCollision) {
        if let Some(fragment) = hit_symbol(hit) {
            if let Some(full) = compound_query_identifier(query, fragment) {
                return vec![format!("defs:{full}"), format!("callers:{full}")];
            }
        }
    }
    let Some(symbol) = hit_symbol(hit) else {
        // No symbol to drill into: there is no query the engine would run.
        return Vec::new();
    };
    let has_def = has_kind(hit, |kind| kind == HitKind::Def);
    let has_usage = has_kind(hit, |kind| matches!(kind, HitKind::Caller | HitKind::Graph));
    if has_def && has_usage && margin_is_decisive(hit) {
        // Definition, usage, and ordering are all settled.
        return Vec::new();
    }
    let mut follow_ups = Vec::new();
    if !has_def {
        follow_ups.push(format!("defs:{symbol}"));
    }
    if !has_usage {
        follow_ups.push(format!("callers:{symbol}"));
    }
    if follow_ups.is_empty() {
        // Evidence is complete but the ordering is not decisive: confirm with
        // exact text instead of re-running graph channels that already agreed.
        follow_ups.push(format!("literal:{symbol}"));
    }
    follow_ups
}

/// Causal `suggested_next` for the agent envelope, derived from the actual
/// top hit instead of a static template. Every entry is an executable
/// `asgrep` command.
pub fn plan_suggested_next(response: &SearchResponse) -> Vec<String> {
    let mut suggested = Vec::new();
    match response.hits.first() {
        None => {
            // Nothing came back: the semantic channel is the widest remaining
            // probe for an indexed corpus.
            suggested.push(format!(
                "asgrep semantic {}",
                quote_shell_arg(&response.query)
            ));
        }
        Some(top) => {
            for follow_up in follow_ups_for_hit(&response.query, top) {
                suggested.push(format!("asgrep {}", quote_shell_arg(&follow_up)));
            }
            let has_semantic = response
                .hits
                .iter()
                .any(|hit| has_kind(hit, |kind| kind == HitKind::Embed));
            if !has_semantic {
                // No semantic evidence anywhere in the shortlist: a semantic
                // re-run is the one channel the caller has not seen.
                suggested.push(format!(
                    "asgrep semantic {}",
                    quote_shell_arg(&response.query)
                ));
            }
        }
    }
    suggested.push(format!(
        "asgrep --json --format agent {}",
        quote_shell_arg(&response.query)
    ));
    suggested.dedup();
    suggested
}

#[cfg(test)]
#[path = "../../../../tests/unit/core/search__planner.rs"]
mod tests;
