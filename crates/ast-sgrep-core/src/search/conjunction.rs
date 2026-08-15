//! Two-channel conjunction queries (P0 channel-conjunction).
//!
//! `asgrep 'callers:process_request AND pattern:fn $NAME($$$)'` intersects
//! two prefixed channels inside the index: graph, pattern, literal, and
//! semantic evidence combine in one shot instead of the caller running two
//! commands and joining by hand. `AND NOT` subtracts the right channel.
//!
//! Semantics (v1, deliberately bounded):
//!
//! - Exactly **two** channels. A query with more than one `AND` does not parse
//!   as a conjunction and falls through to ordinary search.
//! - Both sides must be prefixed channel queries (`defs:`, `callers:`,
//!   `imports:`, `pattern:`, `literal:`, `regex:`, `word:`, or
//!   `semantic:"..."`). Unprefixed text on either side falls through, so plain
//!   English containing "AND" never changes meaning.
//! - The **left** channel is the result identity: hits come from the left
//!   side. `AND` keeps left hits whose file also matched the right channel;
//!   `AND NOT` keeps left hits whose file did not. Right-channel evidence that
//!   overlaps a kept hit's span is merged into its contributor set.
//! - Empty channels stay honest: an empty left result is empty; an empty
//!   right result makes `AND` empty and `AND NOT` a no-op.
//!
//! The join is file-level. Span-level joins (pattern-graph-join) are a later
//! campaign; this module never widens beyond what each channel returned.

use crate::query::{ParsedQuery, QueryMode};
use crate::search::types::{merge_channel_evidence, SearchHit};
use crate::Result;

/// One side of a conjunction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChannelQuery {
    /// A prefixed query mode (`defs:`, `pattern:`, `literal:`, ...).
    Mode(ParsedQuery),
    /// Embedding-only retrieval (`semantic:"parameterized query"`).
    Semantic(String),
}

impl ChannelQuery {
    fn parse(side: &str) -> Option<Self> {
        let side = side.trim();
        if let Some(rest) = side.strip_prefix("semantic:") {
            let payload = strip_wrapping_quotes(rest.trim());
            if payload.is_empty() {
                return None;
            }
            return Some(Self::Semantic(payload.to_string()));
        }
        let parsed = ParsedQuery::parse(side);
        if parsed.mode == QueryMode::Hybrid {
            // Unprefixed text is not a channel: the whole query falls through
            // to ordinary search so plain English "AND" keeps its meaning.
            return None;
        }
        if parsed.target.as_deref().unwrap_or("").is_empty() {
            return None;
        }
        Some(Self::Mode(parsed))
    }
}

/// A parsed `<left> AND [NOT] <right>` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Conjunction {
    pub left: ChannelQuery,
    pub right: ChannelQuery,
    /// True for `AND NOT`: subtract the right channel instead of intersecting.
    pub negated: bool,
}

fn strip_wrapping_quotes(s: &str) -> &str {
    let s = s.trim();
    let quoted =
        (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''));
    if s.len() >= 2 && quoted {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Parse a two-channel conjunction. Returns `None` (fall through to ordinary
/// search) unless the query is exactly `<channel> AND [NOT] <channel>`.
pub(crate) fn parse(raw: &str) -> Option<Conjunction> {
    let raw = raw.trim();
    let (lhs, rhs) = raw.split_once(" AND ")?;
    if rhs.contains(" AND ") {
        // Two channels only in v1.
        return None;
    }
    let rhs = rhs.trim();
    let (negated, rhs) = match rhs
        .strip_prefix("NOT ")
        .or_else(|| rhs.strip_prefix("not "))
    {
        Some(rest) => (true, rest),
        None => (false, rhs),
    };
    let left = ChannelQuery::parse(lhs)?;
    let right = ChannelQuery::parse(rhs)?;
    Some(Conjunction {
        left,
        right,
        negated,
    })
}

/// The `ParsedQuery` that `finish_response` should rank and gate with: the
/// left channel's mode and terms under the full conjunction's raw text, so
/// the response reports the query the caller actually asked.
pub(crate) fn response_query(raw: &str, conjunction: &Conjunction) -> ParsedQuery {
    let mut parsed = match &conjunction.left {
        ChannelQuery::Mode(parsed) => parsed.clone(),
        ChannelQuery::Semantic(query) => ParsedQuery::parse(query),
    };
    parsed.raw = raw.trim().to_string();
    parsed
}

fn spans_overlap(a: &SearchHit, b: &SearchHit) -> bool {
    a.line_start <= b.line_end && b.line_start <= a.line_end
}

/// Combine channel results. Left hits are the identity; the right channel
/// filters (`AND`) or subtracts (`AND NOT`). Overlapping right evidence is
/// merged into kept hits so the contributor set shows the agreement.
pub(crate) fn combine(
    left_hits: Vec<SearchHit>,
    right_hits: Vec<SearchHit>,
    negated: bool,
) -> Vec<SearchHit> {
    use std::collections::HashSet;
    let right_files: HashSet<&str> = right_hits.iter().map(|hit| hit.file.as_str()).collect();
    if negated {
        return left_hits
            .into_iter()
            .filter(|hit| !right_files.contains(hit.file.as_str()))
            .collect();
    }
    let mut kept: Vec<SearchHit> = left_hits
        .into_iter()
        .filter(|hit| right_files.contains(hit.file.as_str()))
        .collect();
    for right in right_hits {
        if let Some(target) = kept
            .iter_mut()
            .find(|kept| kept.file == right.file && spans_overlap(kept, &right))
        {
            merge_channel_evidence(target, right);
        }
    }
    kept
}

/// Execute both channels through the searcher and combine.
pub(crate) fn run(searcher: &super::Searcher, conjunction: &Conjunction) -> Result<Vec<SearchHit>> {
    let left_hits = searcher.channel_hits(&conjunction.left)?;
    let right_hits = searcher.channel_hits(&conjunction.right)?;
    Ok(combine(left_hits, right_hits, conjunction.negated))
}

#[cfg(test)]
#[path = "../../../../tests/unit/core/search__conjunction.rs"]
mod tests;
