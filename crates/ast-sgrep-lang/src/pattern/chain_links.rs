//! Chain/link/node cluster: gated chain wrapper, exactness gate, link-trivia doctrines, span lookup.

use super::*;
use crate::Language;
use crate::extract::is_member_expr_kind;
use tree_sitter::Node;

/// The general-lane arm for `?.`-chain PROPERTY faces outside
/// {TypeScript, JavaScript}, GATED with the ONE chain rules. The
/// byte-identical preservation arm over-answered kotlin/swift
/// junction-comment and kotlin mid-chain/receiver-trivia faces where the
/// reference refuses. Every candidate the general lane matched must be a
/// chain whose EVERY consumed call level has an exact callee→`(` junction
/// and whose member links carry no trivia child; statement-level trivia
/// BEFORE the chain attaches outside the matched node and keeps answering.
pub(crate) fn match_structural_chain_gated(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Vec<PatternMatch> {
    let mut out = match_structural_general(lang, source, pattern);
    if out.is_empty() {
        return out;
    }
    let Some(tree) = parse_source_silent(lang, source) else {
        return Vec::new();
    };
    let root = tree.root_node();
    out.retain(|m| chain_candidate_exact(root, m.byte_start, m.byte_end, lang));
    out
}

/// The chain-exactness gate for one general-lane candidate — the candidate
/// node (matched at its exact byte span) descends through callee/receiver
/// hops; every call level must pass [`call_junction_exact`] and every
/// member link must be trivia-free. Each hop lands on a strictly smaller
/// child span, so the walk terminates at the chain root. A call hops to
/// its CALLEE first so the member branch trivia-checks the callee link
/// itself. Swift's WRAPPED links let the kt-shaped later-ANONYMOUS rule
/// pass vacuously on this lane (the connector lives inside a NAMED
/// navigation_suffix), so swift — and kt's dotted links — consult the
/// union [`member_link_trivia_structural`] doctrine here too.
pub(crate) fn chain_candidate_exact(root: Node, start: usize, end: usize, lang: Language) -> bool {
    let Some(node) = node_with_span(root, start, end) else {
        return false;
    };
    let mut cursor = node;
    loop {
        if is_call_kind(cursor.kind()) {
            if !call_junction_exact(&cursor) {
                return false;
            }
            match call_field_node(&cursor) {
                Some(next) => cursor = next,
                None => return true,
            }
        } else if is_member_expr_kind(cursor.kind()) {
            let link_structural = if matches!(lang, Language::Swift | Language::Kotlin) {
                member_link_trivia_structural(&cursor)
            } else {
                member_link_has_trivia(&cursor)
            };
            if link_structural {
                return false;
            }
            match member_receiver(&cursor) {
                Some(next) => cursor = next,
                None => return true,
            }
        } else {
            return true;
        }
    }
}

/// Whether a kotlin member link carries link-STRUCTURAL trivia: a trivia
/// child with ANY later anonymous sibling (the `.`/`?.` connector token on
/// every observed face — kotlin-ng flattens `navigation_suffix`, so a
/// post-`?.` comment is a direct link child). Trivia followed only by NAMED
/// siblings sits in the callee-internal position past the link's own
/// connector — transparent. The one shared gate for both kotlin optional
/// decomposers, so a future grammar edit cannot split their conditions.
pub(crate) fn kt_link_structural(lang: Language, node: &Node) -> bool {
    lang == Language::Kotlin && member_link_has_trivia(node)
}

pub(crate) fn member_link_has_trivia(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (index, child) in children.iter().enumerate() {
        if !is_trivia_kind(child.kind()) && !child.is_extra() {
            continue;
        }
        if children[index + 1..].iter().any(|later| !later.is_named()) {
            return true;
        }
    }
    false
}

/// The SWIFT member-link trivia doctrine, same position rule as the kotlin
/// consults but read against swift's WRAPPED links (`a /*c*/ .b(1)` hangs
/// the comment off the navigation_expression; `a. /*c*/ b(1)` puts it
/// INSIDE the navigation_suffix after its `.`). Refuse every candidate
/// whose comment sits BEFORE a later navigation_suffix — receiver-side,
/// mid-link, and doubled alike — while the callee-INTERNAL position
/// (inside the suffix, after the `.`) stays transparent. Swift never
/// enters the kotlin decomposers, so the plain-call and chain lanes consult
/// THIS predicate: any navigation node in the callee subtree carrying a
/// direct trivia/extra child with a LATER navigation_suffix sibling is
/// link-STRUCTURAL → refuse.
pub(crate) fn swift_member_link_structural(node: &Node) -> bool {
    let kind = node.kind();
    if is_navigation_suffix_kind(kind) || kind == "navigation_expression" {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for (index, child) in children.iter().enumerate() {
            if !is_trivia_kind(child.kind()) && !child.is_extra() {
                continue;
            }
            if children[index + 1..]
                .iter()
                .any(|later| is_navigation_suffix_kind(later.kind()))
            {
                return true;
            }
        }
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    children
        .iter()
        .any(|child| swift_member_link_structural(child))
}

/// The UNION member-link trivia doctrine, RECURSIVE over the candidate
/// subtree — at every node, a direct COMMENT-kind trivia child (extras
/// excluded — error-glue recovery rows are extras the reference answers
/// through) with a later ANONYMOUS sibling (flat `?.` links) or a later
/// `navigation_suffix` sibling (WRAPPED dotted links) is link-STRUCTURAL.
/// The two one-level predicates alone miss the nested shapes the
/// per-candidate retain sees (the trivia lives one link down inside the
/// candidate span). Positional scope is unchanged: callee-internal trivia
/// (inside the suffix, after the `.`) and tail trivia (outside the
/// candidate span) never fire.
pub(crate) fn member_link_trivia_structural(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (index, child) in children.iter().enumerate() {
        // COMMENT-kind trivia only: the registered doctrine is comment
        // trivia. Swift's error-glue EXTRA fragments are extras the
        // reference ANSWERS through — folding `is_extra` in here
        // over-refused them (swift faces, caught by the suite).
        if !is_trivia_kind(child.kind()) {
            continue;
        }
        if children[index + 1..]
            .iter()
            .any(|later| !later.is_named() || is_navigation_suffix_kind(later.kind()))
        {
            return true;
        }
    }
    children
        .iter()
        .any(|child| member_link_trivia_structural(child))
}

/// The node whose byte span is exactly `(start, end)`, descending through
/// the children that contain the span.
pub(crate) fn node_with_span<'a>(node: Node<'a>, start: usize, end: usize) -> Option<Node<'a>> {
    if node.start_byte() == start && node.end_byte() == end {
        return Some(node);
    }
    if node.child_count() == 0 || node.start_byte() > start || node.end_byte() < end {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() <= start && child.end_byte() >= end {
            if let Some(found) = node_with_span(child, start, end) {
                return Some(found);
            }
        }
    }
    None
}
