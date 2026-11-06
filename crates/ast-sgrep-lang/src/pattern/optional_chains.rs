//! Optional-chain call matching and decomposition.

use super::*;
use crate::extract::{is_ident_kind, is_in_comment_or_string, is_member_expr_kind, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Match the two-segment optional-chain template `HEAD?.$TAIL(...)`
/// against candidate calls whose callee member chain links into its tail
/// segment through an anonymous `?.` token — the token-exact rule (the
/// optional template answers exactly the optional faces, zero plain faces;
/// the plain template's opposite contract lives in the path veto). A
/// wildcard head folds the WHOLE receiver expression like the
/// member-object metavariable: `$O` = `user?.profile` on
/// `user?.profile?.load()`, `maybe()` on `maybe()?.load()`, `a.b` on
/// `a.b?.c()`. Nested chains emit inner+outer rows
/// (`conn?.open()?.send(1)` answers both calls).
// Matcher-lane shape: (lang/node/source/pattern/captures/...) is threaded
// deliberately; bundling would churn every lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_optional_calls(
    lang: Language,
    node: Node,
    source: &str,
    pattern: &str,
    head_capture: Option<&str>,
    head_literal: Option<&str>,
    tail_capture: Option<&str>,
    tail_literal: Option<&str>,
    arguments: Option<&ArgumentTemplate>,
    out: &mut Vec<PatternMatch>,
) {
    if let Some(captures) = optional_call_matches(
        lang,
        &node,
        source,
        pattern,
        head_capture,
        head_literal,
        tail_capture,
        tail_literal,
        arguments,
    ) {
        let mut captures = captures;
        if !captures.contains_key("MATCH") {
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
        }
        out.push(hit_for_node(&node, source, pattern, captures));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_optional_calls(
            lang,
            child,
            source,
            pattern,
            head_capture,
            head_literal,
            tail_capture,
            tail_literal,
            arguments,
            out,
        );
    }
}

/// Segment unification + argument-template check for one optional-chain
/// candidate. `None` when the candidate is not a call, its chain does not
/// link into the tail through `?.`, or a head/tail spelling (literal or
/// same-name unification) fails.
// Matcher-lane shape: (lang/node/source/pattern/captures/...) is threaded
// deliberately; bundling would churn every lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn optional_call_matches(
    lang: Language,
    node: &Node,
    source: &str,
    pattern: &str,
    head_capture: Option<&str>,
    head_literal: Option<&str>,
    tail_capture: Option<&str>,
    tail_literal: Option<&str>,
    arguments: Option<&ArgumentTemplate>,
) -> Option<BTreeMap<String, String>> {
    if !is_call_kind(node.kind()) || is_in_comment_or_string(node) {
        return None;
    }
    // The optional lanes answer through this function and never reach
    // `capture_call_path` (the `simple_call` guard excludes `?.`-spelled
    // patterns from its gate), so the SAME junction rule must be consulted
    // here: a comment extra between the callee and the argument list
    // (`a?.b /*c*/ (1)`) breaks the exact-children match for `?.`-spelled
    // patterns exactly as for plain ones (js+ts: `a?.b($X)`, `$A?.b($X)`,
    // `a?.b($A)`, and `a?.b($$$A)` at both arities all answer []).
    if !call_junction_exact(node) {
        return None;
    }
    if !arguments_match(node, arguments, &["arguments"]) {
        return None;
    }
    // Resolve grammar-specific receiver spans once; literal admission and
    // capture publication below are identical for optional and nullsafe calls.
    let (head_text, tail_text) = match node.kind() {
        "member_call_expression" => return None,
        "nullsafe_member_call_expression" => {
            // PHP's connector is the node kind; the object is already the
            // whole folded receiver (`g?->h` on `g?->h?->i()`).
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            (node_text(&object, source)?, node_text(&name, source)?)
        }
        _ => {
            let field = call_field_node(node)?;
            let (segments, connectors) = optional_chain_decompose(lang, &field)?;
            if segments.len() < 2 || !*connectors.last()? {
                return None;
            }
            let tail_text = node_text(segments.last()?, source)?;
            // The folded head ends before the final connector; `user?.profile`
            // must never acquire the trailing `?` from `user?.profile?.load()`.
            let head_end = segments[segments.len() - 2].end_byte();
            let head_text = source.get(segments[0].start_byte()..head_end)?;
            (head_text, tail_text)
        }
    };
    if tail_literal.is_some_and(|literal| literal != tail_text) {
        return None;
    }
    if head_literal.is_some_and(|literal| literal != head_text) {
        return None;
    }
    // MATCH + argument captures through the shared pattern-text contract;
    // `capture_call_path` no-ops for optional spellings (its simple_call
    // guard refuses `?`-bearing segments), so head/tail bind here.
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    capture_arguments(node, source, pattern, &mut captures)?;
    if let Some(name) = head_capture {
        bind_capture(&mut captures, name, head_text)?;
    }
    if let Some(name) = tail_capture {
        bind_capture(&mut captures, name, tail_text)?;
    }
    Some(captures)
}

/// Decompose a candidate callee into (segment nodes, connector flags):
/// `segments[0]` may be ANY expression (the folded head); every later
/// segment is a plain identifier leaf; `connectors[i]` is true when the
/// link INTO segment i+1 is `?.` (anonymous token or the named
/// `optional_chain` wrapper), false for `.`. A member link that is not a
/// clean base+connector+leaf triple, or a non-identifier tail (kotlin/
/// swift navigation suffixes), vetoes the chain — those grammars keep
/// their fail-closed contract.
pub(crate) fn optional_chain_decompose<'a>(
    lang: Language,
    node: &Node<'a>,
) -> Option<(Vec<Node<'a>>, Vec<bool>)> {
    if !is_member_expr_kind(node.kind()) {
        return Some((vec![*node], Vec::new()));
    }
    // Kotlin's flat member links carry comment children
    // (`member_link_parts` skips them to fill its slots), so the 2-link
    // lane answered link-STRUCTURAL trivia faces the reference refuses.
    // Apply the ONE position-scoped doctrine the kt general arm already
    // uses — the same predicate `chain_candidate_exact` consults — so
    // both kt optional lanes hold it too (js/ts keep their transparency
    // contracts; swift never decomposes here).
    if kt_link_structural(lang, node) {
        return None;
    }
    let link = member_link_parts(lang, node)?;
    if !is_ident_kind(link.leaf.kind()) && !KEYWORD_RECEIVER_KINDS.contains(&link.leaf.kind()) {
        return None;
    }
    let (mut segments, mut connectors) = optional_chain_decompose(lang, &link.base)?;
    segments.push(link.leaf);
    connectors.push(link.optional);
    Some((segments, connectors))
}

/// Chain-lane decomposition — like [`optional_chain_decompose`] but the
/// recursion CONTINUES through call receivers (a method-call link
/// contributes its callee leaf as the segment, so `$M1` binds the method
/// NAME and the connector INTO the call is the link into its callee). The
/// two-segment lane keeps the stop-at-call folding in
/// [`optional_chain_decompose`]: its registered folded-head captures
/// (O=`conn?.open()`, O=`maybe()`) depend on it. Returns parallel
/// (segment leaves, connector flags, EXPRESSION ends): `ends[k]` is the end
/// byte of the whole expression contributing segment k — a call segment
/// ends at its argument list's closing paren (the reference binds folded
/// heads like `a.b()`), a plain member leaf at its member node.
pub(crate) fn optional_call_chain_decompose<'a>(
    lang: Language,
    node: &Node<'a>,
) -> Option<(Vec<Node<'a>>, Vec<bool>, Vec<usize>)> {
    if is_call_kind(node.kind()) {
        let field = call_field_node(node)?;
        let mut decomp = optional_call_chain_decompose(lang, &field)?;
        if let Some(end) = decomp.2.last_mut() {
            *end = node.end_byte();
        }
        return Some(decomp);
    }
    if !is_member_expr_kind(node.kind()) {
        return Some((vec![*node], Vec::new(), vec![node.end_byte()]));
    }
    // Same kotlin candidate-link veto as [`optional_chain_decompose`] —
    // the all-call lane decomposes trivia children away and answered the
    // trivia×all-call intersection (`a?.b(1) /*c*/ ?.c(2)`,
    // `a /*c*/ ?.b(1)?.c(2)`, 2-link receiver twin) where the reference
    // refuses. Callee-internal trivia (only named siblings after it) keeps
    // answering — the position scope is the ONE registered rule.
    if kt_link_structural(lang, node) {
        return None;
    }
    let link = member_link_parts(lang, node)?;
    if !is_ident_kind(link.leaf.kind()) && !KEYWORD_RECEIVER_KINDS.contains(&link.leaf.kind()) {
        return None;
    }
    let (mut segments, mut connectors, mut ends) = optional_call_chain_decompose(lang, &link.base)?;
    segments.push(link.leaf);
    connectors.push(link.optional);
    ends.push(node.end_byte());
    Some((segments, connectors, ends))
}

/// Enforce ONE call segment's argument contract. Mixed-rest slot lists ride
/// the registered plain-call rest-slot semantics (`match_arg_slots` with
/// `ArgSlotMode::Positional`: trailing rest + single ⇒ n ≥ 2, k ≥ 2 rests
/// ⇒ n ≥ k-1, non-trailing rest ⇒ n == 1 — the `?.`-chain faces answer at
/// exactly those arities).
/// Plain templates enforce arity and bind both whole-list and positional
/// captures, including repeated-name equality across segments.
pub(crate) fn chain_segment_args_contract(
    node: &Node,
    segment: &CallChainSegment,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    if let Some(slots) = &segment.arg_slots {
        let nodes = argument_nodes(node, &["arguments"])?;
        return match_arg_slots(slots, &nodes, source, captures, ArgSlotMode::Positional);
    }
    if !arguments_match(node, segment.args.as_ref(), &["arguments"]) {
        return None;
    }
    if let Some((name, multi)) = &segment.args_capture {
        if let Some(text) = capture_arguments_text(node, source) {
            bind_capture_kind(captures, name, strip_container(&text), *multi)?;
        }
    }
    if let Some(names) = &segment.arg_metas {
        let nodes = argument_nodes(node, &["arguments"])?;
        if names.len() != nodes.len() {
            return None;
        }
        for (name, argument) in names.iter().zip(nodes.iter()) {
            bind_capture(captures, name, node_text(argument, source)?)?;
        }
    }
    Some(())
}

/// Match the N-segment optional chain template `HEAD?.M1(...).M2(...)`
/// against candidate calls whose callee decomposes with EVERY ALIGNED
/// connector equal to the template's per-position flag — the head link
/// `$O?.…`, the mid-chain spellings `$O.$M1()?.$M2($$$B)`, and their
/// combinations all reduce to flag equality (the token-exact rule; the
/// optional-mid sources never answer the all-plain template). The wildcard
/// head folds the whole receiver prefix, ending at the last absorbed
/// segment's EXPRESSION end so no connector bytes glue into the capture.
/// Segment names unify through `bind_capture` (the same-name veto) and
/// every call segment's argument list is checked along the receiver walk,
/// exactly like [`chain_matches`].
pub(crate) fn optional_chain_matches(
    lang: Language,
    node: &Node,
    source: &str,
    _pattern: &str,
    segments: &[CallChainSegment],
    optional_flags: &[bool],
) -> Option<BTreeMap<String, String>> {
    if !is_call_kind(node.kind()) || is_in_comment_or_string(node) {
        return None;
    }
    // The N-segment optional chain lane answers here and never reaches
    // `capture_call_path`'s junction gate — consult the SAME junction rule
    // (see the twin hunk in `optional_call_matches`): a comment extra in the
    // callee→arguments gap refuses the candidate for `?.`-spelled patterns
    // too, while trivia INSIDE the member-chain callee
    // (`a?.b /*c*/ .c(1)`) sits outside the gap and keeps answering.
    if !call_junction_exact(node) {
        return None;
    }
    let field = call_field_node(node)?;
    let (segs, connectors, ends) = optional_call_chain_decompose(lang, &field)?;
    let head = &segments[0];
    let links = &segments[1..];
    if segs.len() < links.len() + 1 || connectors.len() + 1 != segs.len() {
        return None;
    }
    // Alignment: the pattern's segments pair with the candidate's LAST
    // segments; the head folds everything before the first aligned segment.
    let absorb = segs.len() - (links.len() + 1);
    // Token-exact per aligned connector: the link into candidate segment
    // absorb+j must EQUAL the template's flag for pattern segment j.
    for j in 1..segments.len() {
        if connectors[absorb + j - 1] != optional_flags[j] {
            return None;
        }
    }
    // The folded head span ends at the last absorbed segment's EXPRESSION
    // end — trailing connector bytes stay out of the capture (the
    // no-glued-? rule) while a folded call keeps its argument list (the
    // reference binds `a.b()`, never `a.b`).
    let head_text = source.get(segs[0].start_byte()..ends[absorb])?;
    // A LITERAL head anchors the pattern chain at the head segment's LEAF
    // text — the segment NAME (`fetch`), not the folded EXPRESSION span
    // (`fetch()` carries the argument list). Comparing the name against
    // the span vetoed every literal-call-head candidate where the reference
    // answers (`fetch()?.$M($$$A).$M2($$$B)` on `fetch()?.g().h()`). And a
    // literal head admits no absorbed receiver prefix: the pattern chain
    // roots at the bare head, so a deeper candidate chain is a different
    // node (the plain lane's exact-length rule; `x.fetch()?…` never
    // answers).
    if head.literal.is_some() {
        if absorb > 0 {
            return None;
        }
        let head_leaf = node_text(&segs[absorb], source)?;
        if head.literal.as_deref() != Some(head_leaf) {
            return None;
        }
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    // NO pattern-level argument capture here. The legacy
    // `capture_arguments(node, …, pattern, …)` call extracted the FIRST call
    // segment's `$$$A` (its `(`..`)` slice starts at the first parenthesis of
    // the chain) and bound it against the OUTER node's argument list — the
    // LAST segment's — so on `fetch()?.g(1).h()` the multi key `$$$A` was
    // pre-bound to "" and the receiver walk's correct A=[1] binding collided
    // into a veto: the whole face silently missed. Every segment's argument
    // template and capture is enforced below by the segment-specific
    // machinery (head-args block, receiver walk, last-segment bind), which
    // binds each name exactly once against its own list.
    if let Some(name) = head.capture.as_deref() {
        bind_capture(&mut captures, name, head_text)?;
    }
    for (segment, seg_node) in links.iter().zip(segs[absorb + 1..].iter()) {
        let text = node_text(seg_node, source)?;
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want != text => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, text)?,
            (None, None) => return None,
        }
    }
    // Alignment of segment SHAPES + per-position argument contracts: every
    // non-terminal link consumes one receiver hop, landing on the candidate
    // expression that ends at that link. A pattern CALL segment requires a
    // call there (arity/args-capture checked on it); a pattern PROPERTY
    // segment requires a PLAIN member link — a call in the candidate
    // breaks the exact-children match exactly as a comment does
    // (`a?.b(1)?.c(2)` never answers `a?.b?.c($X)`).
    // Arguments: the candidate node carries the LAST link's list; every
    // earlier CALL link consumes one receiver link along the walk.
    let last = links.last()?;
    let segment_is_call = |segment: &CallChainSegment| {
        segment.args.is_some() || segment.args_capture.is_some() || segment.arg_slots.is_some()
    };
    // ONE hop inward: a call node hops its callee's object; a member node
    // (the landing shape of a PROPERTY link) hops its own object. Mixed
    // call/property chains alternate the two.
    let mut receiver = *node;
    for segment in links[..links.len() - 1].iter().rev() {
        receiver = if is_call_kind(receiver.kind()) {
            chain_receiver(&receiver)?
        } else {
            member_receiver(&receiver)?
        };
        if segment_is_call(segment) {
            if !is_call_kind(receiver.kind()) {
                return None;
            }
            // EVERY consumed call level obeys the ONE junction rule — a comment
            // extra in a mid-link callee→arguments gap refuses the candidate
            // (the terminal consult at the top covered only the outer call).
            if !call_junction_exact(&receiver) {
                return None;
            }
            chain_segment_args_contract(&receiver, segment, source, &mut captures)?;
        } else if is_call_kind(receiver.kind()) {
            return None;
        }
    }
    // A CALL head carries its own argument contract — reachable as one more
    // receiver link when nothing was absorbed (the head segment IS the
    // chain root then). Without this block the head's arity/args-capture
    // template was never consulted (`fetch(1)?…` over-answered the
    // arity-mismatched faces). The head call is a consumed call level too
    // — same junction consult, same slot-aware contract (`a(0)?.b?.c($X)`
    // refuses `a /*c*/ (0)?.b?.c(1)`).
    if absorb == 0 && segment_is_call(head) {
        receiver = if is_call_kind(receiver.kind()) {
            chain_receiver(&receiver)?
        } else {
            member_receiver(&receiver)?
        };
        if !is_call_kind(receiver.kind()) {
            return None;
        }
        if !call_junction_exact(&receiver) {
            return None;
        }
        chain_segment_args_contract(&receiver, head, source, &mut captures)?;
    }
    chain_segment_args_contract(node, last, source, &mut captures)?;
    Some(captures)
}

/// Walker for the N-segment optional chain — one row per matching call node
/// (nested chains emit inner+outer rows like the reference), the same
/// emission shape as [`walk_optional_calls`].
pub(crate) fn walk_optional_call_chains(
    lang: Language,
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    optional_flags: &[bool],
    out: &mut Vec<PatternMatch>,
) {
    if let Some(captures) =
        optional_chain_matches(lang, &node, source, pattern, segments, optional_flags)
    {
        let mut captures = captures;
        if !captures.contains_key("MATCH") {
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
        }
        out.push(hit_for_node(&node, source, pattern, captures));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_optional_call_chains(lang, child, source, pattern, segments, optional_flags, out);
    }
}
