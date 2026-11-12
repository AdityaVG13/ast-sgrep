//! Dotted member-call chain matching and callee decomposition.

use super::*;
use crate::extract::{is_ident_kind, is_in_comment_or_string, is_member_expr_kind, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Match dotted member-call chains with per-segment argument lists. A
/// candidate call answers when its callee path decomposes into EXACTLY the
/// pattern's segment count (the reference reports the outermost chain, plus
/// inner-prefix subnodes on deeper chains — both are call nodes this walk
/// visits), every segment name unifies through `bind_capture` (the
/// same-name veto), and every call segment's argument list matches its
/// template: the LAST segment's list is the candidate's own arguments, the
/// inner segments' lists come from the nested receiver calls. Captures are
/// built directly (MATCH, segment names, per-call argument metavars) — the
/// shared `captures_for_node` path would re-parse the chain pattern as a
/// single call and mis-bind the truncated callee.
pub(crate) fn walk_call_chains(
    lang: Language,
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    out: &mut Vec<PatternMatch>,
) {
    walk_nodes(
        node,
        out,
        |n: &Node| is_call_kind(n.kind()) && !is_in_comment_or_string(n),
        |n| call_chain_match(n, lang, source, pattern, segments),
    );
}

/// Single-candidate check for [`walk_call_chains`]: the callee path must
/// decompose into exactly the pattern's segment count, every segment name
/// unifies, and every call segment's argument list matches its template.
fn call_chain_match(
    node: &Node,
    lang: Language,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
) -> Option<PatternMatch> {
    let mut captures = chain_matches(lang, node, source, segments)?;
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// Capture arguments at any plain-chain call level after its arity check.
/// Missing containers/text keep this lane's existing permissive behavior;
/// `chain_segment_args_contract` separately enforces strict optional-chain slots.
fn capture_chain_segment_arguments(
    node: &Node,
    segment: &CallChainSegment,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    if let Some((name, multi)) = &segment.args_capture {
        if let Some(text) = capture_arguments_text(node, source) {
            bind_capture_kind(captures, name, strip_container(&text), *multi)?;
        }
    }
    let Some(names) = &segment.arg_metas else {
        return Some(());
    };
    let Some(nodes) = argument_nodes(node, &["arguments"]) else {
        return Some(());
    };
    for (name, argument) in names.iter().zip(nodes.iter()) {
        if let Some(text) = node_text(argument, source) {
            bind_capture(captures, name, text)?;
        }
    }
    Some(())
}

/// Segment unification + argument-template check for one candidate call.
/// A leading METAVARIABLE head absorbs the whole property-access receiver
/// prefix — the pattern's tail aligns with the candidate path's LAST
/// segments and the head binds the absorbed EXPRESSION span. A LITERAL
/// head pins the chain to the exact segment count. The `?.` veto is
/// position-scoped in the registered grammars: an optional connector INTO
/// any ALIGNED call segment refuses, while head-internal optionals fold;
/// every other grammar keeps the wholesale refusal. The receiver walk pairs
/// every pattern CALL segment except the LAST with one receiver link (which
/// must itself be a call), so per-segment argument templates fire on
/// receiver-headed chains — and a receiver head is absorbed into the head
/// text above, consuming no link.
pub(crate) fn chain_matches(
    lang: Language,
    node: &Node,
    source: &str,
    segments: &[CallChainSegment],
) -> Option<BTreeMap<String, String>> {
    // The swift link-STRUCTURAL trivia veto on chain candidates — same
    // predicate as the plain-call consult above (receiver-side/mid-link
    // comment before a later `navigation_suffix` refuses; callee-internal
    // stays transparent).
    if lang == Language::Swift
        && call_field_node(node).is_some_and(|callee| swift_member_link_structural(&callee))
    {
        return None;
    }
    // The ONE junction rule at EVERY consumed call level of the dotted
    // chain — the head call here, every receiver hop in the loop below —
    // for the js/ts grammars whose doctrine is proven (`a.b(1).c /*j*/ (2)`
    // and `a.b /*j*/ (1).c(2)` are both [] while the comment-transparent
    // link positions bind). Comments INSIDE the member-chain callee and
    // INSIDE argument lists never fire this gate.
    if matches!(lang, Language::JavaScript | Language::TypeScript) && !call_junction_exact(node) {
        return None;
    }
    let path = chain_callee_segments(lang, node, source)?;
    if path.len() < segments.len() {
        return None;
    }
    let absorb = path.len() - segments.len();
    if absorb > 0 && segments[0].literal.is_some() {
        return None;
    }
    // Token-exact veto at the aligned positions; head-INTERNAL connectors
    // fold only where the reference's folding is probed (ts/js — the
    // registered optional-connector grammars); everywhere else any `?.`
    // in the candidate keeps the wholesale refusal.
    if path[absorb + 1..].iter().any(|seg| seg.optional)
        || (absorb > 0
            && path[1..=absorb].iter().any(|seg| seg.optional)
            && !matches!(lang, Language::TypeScript | Language::JavaScript))
    {
        return None;
    }
    let mut captures = BTreeMap::new();
    // Head binding: a metavariable head binds the WHOLE absorbed prefix —
    // the expression span when the walk recorded spans (call segments keep
    // their argument list: the reference binds `s.a()`, connector bytes
    // excluded), or the joined ruby path text; `bind_capture`'s same-name
    // veto then
    // rejects any repeated name against the tail bindings.
    let head_text = if path[..=absorb].iter().all(|seg| seg.end.is_some()) {
        source
            .get(path[0].start..path[absorb].end.unwrap())?
            .to_string()
    } else {
        path[..=absorb]
            .iter()
            .map(|seg| seg.text.as_str())
            .collect::<Vec<_>>()
            .join(".")
    };
    match (&segments[0].literal, &segments[0].capture) {
        (Some(want), _) if want != &path[0].text => return None,
        (Some(_), _) => {}
        (None, Some(name)) => bind_capture(&mut captures, name, &head_text)?,
        (None, None) => return None,
    }
    for (segment, actual) in segments[1..]
        .iter()
        .zip(path[absorb + 1..].iter().map(|seg| seg.text.as_str()))
    {
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want.as_str() != actual => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, actual)?,
            (None, None) => return None,
        }
    }
    // Argument lists: the candidate's own arguments are the LAST call
    // segment's; walking the receiver side yields the earlier call segments'
    // lists — each pattern call segment consumes exactly one receiver link
    // (innermost pattern segment first), and every link must be a call.
    let last = segments.last()?;
    let mut receiver = *node;
    for segment in segments[..segments.len() - 1].iter().rev() {
        if segment.args.is_none() {
            // Receiver head: no argument contract, no link consumed.
            continue;
        }
        receiver = chain_receiver(&receiver)?;
        if !is_call_kind(receiver.kind()) {
            return None;
        }
        // The interior link's own callee→`(` junction — the reference applies
        // the exact-children rule per consumed call level, not just the
        // matched head.
        if matches!(lang, Language::JavaScript | Language::TypeScript)
            && !call_junction_exact(&receiver)
        {
            return None;
        }
        if !arguments_match(&receiver, segment.args.as_ref(), &["arguments"]) {
            return None;
        }
        capture_chain_segment_arguments(&receiver, segment, source, &mut captures)?;
    }
    if !arguments_match(node, last.args.as_ref(), &["arguments"]) {
        return None;
    }
    capture_chain_segment_arguments(node, last, source, &mut captures)?;
    Some(captures)
}

/// One decomposed chain path segment: its dotted text, its span start, its
/// EXPRESSION end (absent on the ruby text path), and whether the connector
/// INTO it is the spelled `?.` (recorded per segment so the matcher can
/// scope the token veto to the aligned positions).
pub(crate) struct ChainPathSegment {
    text: String,
    start: usize,
    end: Option<usize>,
    optional: bool,
}

/// Reference-style decomposition of a candidate call's callee into DOTTED
/// COMPONENTS — `alpha.first().second()` is [alpha, first, second], each
/// `name(args)` component contributing its name (the 3-segment template
/// binds O=alpha, M1=first, M2=second). The registered call-lane path
/// (`call_target_path`) flattens a nested call to its LAST identifier,
/// which drops the receiver segment and made every three-segment face
/// silent.
pub(crate) fn chain_callee_segments(
    lang: Language,
    node: &Node,
    source: &str,
) -> Option<Vec<ChainPathSegment>> {
    if node.kind() == "call" {
        if let Some(segs) = ruby_receiver_callee(node, source) {
            return Some(
                segs.into_iter()
                    .map(|text| ChainPathSegment {
                        text,
                        start: 0,
                        end: None,
                        optional: false,
                    })
                    .collect(),
            );
        }
    }
    let mut segs = Vec::new();
    chain_collect_segments(lang, node, source, &mut segs)?;
    Some(segs)
}

/// Recursive dotted-component collection over candidate nodes.
pub(crate) fn chain_collect_segments(
    lang: Language,
    node: &Node,
    source: &str,
    segs: &mut Vec<ChainPathSegment>,
) -> Option<()> {
    // java(/kotlin-family) member calls carry object + name fields directly.
    if node.kind() == "method_invocation" {
        let object = node.child_by_field_name("object")?;
        let name = node.child_by_field_name("name")?;
        chain_collect_segments(lang, &object, source, segs)?;
        segs.push(ChainPathSegment {
            text: node_text(&name, source)?.to_string(),
            start: name.start_byte(),
            end: Some(node.end_byte()),
            optional: false,
        });
        return Some(());
    }
    if is_call_kind(node.kind()) {
        let field = call_field_node(node)?;
        chain_collect_segments(lang, &field, source, segs)?;
        // The call segment's EXPRESSION ends at its argument list (the
        // reference binds folded heads like `s.a()`), never at the bare
        // callee leaf.
        if let Some(last) = segs.last_mut() {
            last.end = Some(node.end_byte());
        }
        return Some(());
    }
    if is_member_expr_kind(node.kind()) {
        // The connector INTO this segment is recorded, not vetoed: the
        // matcher applies the token veto at the ALIGNED positions and folds
        // head-internal optionals in the registered grammars.
        let mut children = node.walk();
        let optional = node
            .children(&mut children)
            .any(|child| child.kind().contains('?') || child.kind().contains("optional"));
        let object = member_receiver(node)?;
        let mut children = node.walk();
        let property = node.named_children(&mut children).find(|child| {
            child.id() != object.id()
                    // Comment trivia between links / at the receiver is
                    // transparent in the js/ts grammars — skip it instead of
                    // letting a comment child pose as the property segment
                    // (`a.b(1)/*m*/.c(2)` used to decompose [a, b, /*m*/]
                    // and refuse where the reference binds). kt/swift keep
                    // their registered link-structural refusals, so the skip
                    // is scoped to the proven grammars.
                    && !(matches!(lang, Language::JavaScript | Language::TypeScript)
                        && (is_trivia_kind(child.kind()) || child.is_extra()))
        })?;
        chain_collect_segments(lang, &object, source, segs)?;
        segs.push(ChainPathSegment {
            text: node_text(&property, source)?.to_string(),
            start: property.start_byte(),
            end: Some(node.end_byte()),
            optional,
        });
        return Some(());
    }
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        segs.push(ChainPathSegment {
            text: node_text(node, source)?.to_string(),
            start: node.start_byte(),
            end: Some(node.end_byte()),
            optional: false,
        });
        return Some(());
    }
    None
}

/// The receiver side of a chain candidate: the `object` of a
/// `method_invocation`, else the object of the callee member expression.
pub(crate) fn chain_receiver<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "method_invocation" {
        return node.child_by_field_name("object");
    }
    // Ruby splits dot calls into receiver/method rather than a callee field.
    // The path resolver already enforces dots; keep every consumed call
    // parenthesized and do not treat a bare property link as a zero-arg call.
    if let ("call", Some(receiver)) = (node.kind(), node.child_by_field_name("receiver")) {
        if !candidate_call_parenthesized(node) {
            return None;
        }
        if receiver.kind() == "call"
            && (receiver.child_by_field_name("arguments").is_none()
                || !candidate_call_parenthesized(&receiver))
        {
            return None;
        }
        return Some(receiver);
    }
    let field = call_field_node(node)?;
    member_receiver(&field)
}
