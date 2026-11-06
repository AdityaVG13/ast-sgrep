//! Call-chain resolution and optional-call matching.

use super::*;
use crate::extract::{
    is_ident_kind, is_in_comment_or_string, is_member_expr_kind, last_identifier_in_chain,
    node_text,
};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// The parsed RHS pattern's root sits under `program` (and possibly
/// `expression_statement` — its trailing `;` is an anonymous child) wrappers
/// — descend through single-NAMED-child wrappers to the expression node the
/// structural matcher compares against the candidate. A multi-statement
/// pattern stops the descent and refuses (kind mismatch — fail-closed).
pub(crate) fn unwrap_pattern_expression(mut node: Node) -> Node {
    while matches!(node.kind(), "program" | "expression_statement") {
        let mut cursor = node.walk();
        let named: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|child| child.kind() != "php_tag")
            .collect();
        match named.as_slice() {
            [only] => node = *only,
            _ => break,
        }
    }
    node
}

/// One decomposed candidate chain segment: its name text, the argument NODES
/// when the segment is a real call (`None` = receiver/property link — no
/// call happened, so a pattern call segment can never sit here), and the
/// argument-list content text for whole-list captures.
pub(crate) struct MemberChainSegment<'a> {
    text: String,
    args: Option<Vec<Node<'a>>>,
    args_content: Option<String>,
}

/// Exact-depth chain unification: the candidate's member-call decomposition
/// must have EXACTLY the pattern's segment count (inner prefix subnodes are
/// separate candidates the walk visits, never an absorb here), every link's
/// nullsafe flag must equal the pattern's (token exactness),
/// every segment name unifies through `bind_capture` (the same-name veto),
/// a pattern property segment must land on a property link and every
/// pattern call segment on a REAL call link with matching arity (a property
/// access in the receiver chain is not a call: `$a->b->c($u)` has no `b()`
/// link, so `$a->b()->c($A)` must not answer it, and `$a->b()->c($A)` has
/// no `->b` property link, so `$a->b->c($A)` must not answer it either),
/// and argument metavars bind like the flat lane (`$A`/`$$A` → single
/// namespace key `A`; `$$$A` → multi). A receiver (argument-free) pattern
/// segment carries no argument contract, mirroring [`chain_matches`].
pub(crate) fn member_chain_matches(
    node: &Node,
    source: &str,
    segments: &[CallChainSegment],
    nullsafe_flags: &[bool],
) -> Option<BTreeMap<String, String>> {
    let mut actual = Vec::new();
    let mut actual_flags = Vec::new();
    member_chain_segments(node, source, &mut actual, &mut actual_flags)?;
    if actual.len() != segments.len() {
        return None;
    }
    let mut captures = BTreeMap::new();
    for (index, (segment, candidate)) in segments.iter().zip(actual.iter()).enumerate() {
        if actual_flags[index] != nullsafe_flags[index] {
            return None;
        }
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want != &candidate.text => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, &candidate.text)?,
            (None, None) => return None,
        }
        if segment.args.is_none() && index > 0 && candidate.args.is_some() {
            // A pattern property link never folds onto a candidate call
            // position (the reference keeps the faces disjoint).
            return None;
        }
        if let Some(template) = &segment.args {
            // A pattern call segment requires a real call link: a property
            // access in the receiver chain is not a call (`$a->b->c($u)` has
            // no `b()` link, so `$a->b()->c($A)` must not answer it).
            let nodes = candidate.args.as_ref()?;
            let arity_ok = match template {
                ArgumentTemplate::Any => true,
                ArgumentTemplate::Exactly(expected) => nodes.len() == *expected,
            };
            if !arity_ok {
                return None;
            }
            if let Some((name, multi)) = &segment.args_capture {
                if let Some(text) = &candidate.args_content {
                    bind_capture_kind(&mut captures, name, text, *multi)?;
                }
            }
            // A `$A, $B` list binds every name to its positional candidate
            // argument — the flat lane's `capture_arguments` contract,
            // without which the codemod rewrite falsely bails "unbound
            // metavariable" on names the reference substitutes (arity above
            // already pinned the equal lengths). Mixed literal/meta lists
            // match through per-position slots instead (literals byte-exact,
            // metas bind, rest binds the remaining source bytes).
            if let Some(slots) = &segment.arg_slots {
                match_arg_slots(slots, nodes, source, &mut captures, ArgSlotMode::Backtracking)?;
            } else if let Some(names) = &segment.arg_metas {
                for (name, argument) in names.iter().zip(nodes.iter()) {
                    if let Some(text) = node_text(argument, source) {
                        bind_capture(&mut captures, name, text)?;
                    }
                }
            }
        }
    }
    Some(captures)
}

/// Recursive `->`-chain decomposition of a candidate member-call node:
/// `$repo->find($id)->hydrate($row)` is [receiver $repo, call find, call
/// hydrate]; a `member_access_expression` object contributes its property as
/// an argument-free link; a `scoped_call_expression` object contributes its
/// `scope::name` texts (the call keeping its argument list). The parallel
/// `flags` vector records, per pushed segment, whether the connector INTO it
/// is the nullsafe `?->` (the nullsafe node kinds carry the named connector
/// kind; plain links push false). Unresolvable receivers (exotic nodes) veto
/// the whole decomposition — such faces stay empty, never an over-match.
pub(crate) fn member_chain_segments<'a>(
    node: &Node<'a>,
    source: &'a str,
    segs: &mut Vec<MemberChainSegment<'a>>,
    flags: &mut Vec<bool>,
) -> Option<()> {
    match node.kind() {
        kind @ ("member_call_expression" | "nullsafe_member_call_expression") => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs, flags)?;
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            flags.push(kind == "nullsafe_member_call_expression");
            Some(())
        }
        kind @ ("member_access_expression" | "nullsafe_member_access_expression") => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs, flags)?;
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args: None,
                args_content: None,
            });
            flags.push(kind == "nullsafe_member_access_expression");
            Some(())
        }
        "scoped_call_expression" => {
            let scope = node.child_by_field_name("scope")?;
            let name = node.child_by_field_name("name")?;
            segs.push(MemberChainSegment {
                text: node_text(&scope, source)?.to_string(),
                args: None,
                args_content: None,
            });
            flags.push(false);
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            flags.push(false);
            Some(())
        }
        _ => {
            // A dynamic-variable link node (`->$$dyn`) pushes its FULL text
            // including the dollars — the classifier's property literal
            // (`$$dyn`) compares it byte-exactly.
            if is_ident_kind(node.kind())
                || KEYWORD_RECEIVER_KINDS.contains(&node.kind())
                || node.kind() == "variable_name"
                || node.kind() == "dynamic_variable_name"
            {
                segs.push(MemberChainSegment {
                    text: node_text(node, source)?.to_string(),
                    args: None,
                    args_content: None,
                });
                flags.push(false);
                Some(())
            } else {
                None
            }
        }
    }
}

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
    if is_call_kind(node.kind()) && !is_in_comment_or_string(&node) {
        if let Some(captures) = chain_matches(lang, &node, source, segments) {
            let mut captures = captures;
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            out.push(hit_for_node(&node, source, pattern, captures));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_call_chains(lang, child, source, pattern, segments, out);
    }
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

/// The candidate call's argument-list text between its parentheses.
pub(crate) fn capture_arguments_text(node: &Node, source: &str) -> Option<String> {
    argument_container(node, &["arguments"])
        .and_then(|list| node_text(&list, source))
        .map(str::to_string)
}

/// The receiver side of a member-expression callee — the object field where
/// the grammar names it, else the first named child (rust `field_expression`,
/// csharp `member_access`). `None` for plain identifiers ends the receiver
/// walk.
pub(crate) fn member_receiver<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if let Some(object) = node.child_by_field_name("object") {
        return Some(object);
    }
    let mut cursor = node.walk();
    let named: Vec<Node> = node.named_children(&mut cursor).collect();
    if named.len() >= 2 {
        return Some(named[0]);
    }
    None
}

pub(crate) fn call_target_path(node: &Node, source: &str) -> Option<Vec<String>> {
    // MoonBit dot-apply calls: callee path is the object chain plus the
    // trailing accessor (`obj.method()` → `[obj, method]`).
    if node.kind() == "dot_apply_expression" {
        let mut segs = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "arguments" {
                break;
            }
            if let Some(accessor) = crate::extract::dot_accessor_text(&child, source) {
                segs.push(accessor.to_string());
                continue;
            }
            if let Some(mut path) = path_from_node(&child, source) {
                segs.append(&mut path);
            }
        }
        return (!segs.is_empty()).then_some(segs);
    }
    call_callee(node, source).map(|(segs, _)| segs)
}

/// The receiver path used for METAVARIABLE BINDING on multi-segment callee
/// patterns. Like `call_target_path`, but refusing paths that flatten
/// through a non-member node (a call/paren/index receiver): the reference
/// decomposes a pattern callee into plain member-chain nodes and rejects
/// call-carrying receivers (probe: `$O.$O($$$A)` matches `b.b(1)` but not
/// `y.first().first()`), so a flattened tail has no faithful per-segment
/// text and must not unify via `bind_capture`.
pub(crate) fn call_target_path_faithful(node: &Node, source: &str) -> Option<Vec<String>> {
    // Ruby receiver-dot calls keep their registered flattening (pinned
    // ruby probes): the unified resolver returns the lenient ruby chain
    // for them even under `Faithful`, exactly like the former dedicated
    // arm that deferred to `call_target_path` here.
    resolve_callee(node, source, CalleeStrictness::Faithful).map(|(segments, _)| segments)
}

/// Resolve one receiver node into dotted path segments — the single path
/// builder behind the former `path_from_node` / `faithful_path_from_node`
/// pair. `strict = false` flattens through the last identifier (match and
/// index resolution); `strict = true` vetoes every non-plain chain link
/// (metavariable binding). The swift/kotlin grammars wrap each member link
/// in a `navigation_suffix` node — the suffix is part of the member-chain
/// decomposition, so the strict resolver reads straight through it (its
/// named child contributes the next segment). The wrapper never hides a
/// `?.` connector: swift spells the optional link as a separate named `?`
/// SIBLING before the suffix, and the loop's `?`-kind veto below fires on
/// it — the `.`-template stays connector token-exact.
pub(crate) fn resolve_path(node: &Node, source: &str, strict: bool) -> Option<Vec<String>> {
    if !strict {
        if let Some(accessor) = crate::extract::dot_accessor_text(node, source) {
            return Some(vec![accessor.to_string()]);
        }
    }
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        return node_text(node, source).map(|t| vec![t.to_string()]);
    }
    if !is_member_expr_kind(node.kind()) && !(strict && is_navigation_suffix_kind(node.kind())) {
        // Lenient: a chain segment that is not a plain member expression
        // flattens to its last identifier. Strict: it has no faithful
        // segment text — veto instead.
        if strict {
            return None;
        }
        return last_identifier_in_chain(node, source).map(|s| vec![s]);
    }
    let mut segs = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // An optional-chain separator (`?.`) is not a plain member segment —
        // the reference refuses `?.` candidates for a `.` template. The
        // separator is an anonymous `?.` token in most grammars and a named
        // `optional_chain` node in others; either spelling vetoes the path
        // instead of being skipped like the `.` punctuation.
        if child.kind().contains('?') || child.kind().contains("optional") {
            return None;
        }
        if !child.is_named() {
            // Anonymous punctuation (`.` separators) is not a chain segment.
            continue;
        }
        if strict {
            // Chain decomposition is comment-transparent — `a /*c*/ .b(1)`,
            // `a. /*c*/ b(1)`, `a /*c*/ . /*d*/ b(1)` and the 3-link
            // `a /*c*/ .b /*c*/ .c(1)` all answer `a.b(...)`/`a.b.c(...)`,
            // and a meta head binds the CLEAN identifier text (`$A` = `a` at
            // the identifier's byte range). Skip trivia/extra children so
            // comment-bearing chains stay on the faithful arm instead of
            // routing to the nonfaithful receiver slice (mirroring
            // `argument_nodes`' filters); the `?`-kind veto above runs
            // FIRST, so `a?. /*c*/ b(1)` keeps its registered refusal. The
            // lenient path needs no skip: a comment child is not an
            // identifier or member node, so the last-identifier fallback
            // below yields nothing for it and it is skipped the same way.
            if is_trivia_kind(child.kind()) || child.is_extra() {
                continue;
            }
        }
        // Unlike the lenient fallback, a NAMED child with no strict path
        // (a call/paren/subscript receiver) VETOES the whole chain in
        // strict mode — its "segments" would be flattened lies about the
        // receiver shape.
        match resolve_path(&child, source, strict) {
            Some(mut p) => segs.append(&mut p),
            None if strict => return None,
            None => {}
        }
    }
    (!segs.is_empty()).then_some(segs)
}

/// Keyword receivers that count as a path segment in `$OBJ.$METHOD($$$)`:
/// `self.helper()` / `this.render()` must match a two-segment wildcard path
/// exactly like `app.tick()` does (the reference agrees). Rust/Ruby use `self`,
/// JS/TS/Java/C++ use `this`, Swift `self_expression`, Kotlin/C#
/// `this_expression`. Not added to `IDENT_KINDS`: that table also drives
/// index extraction, where keyword receivers must stay non-identifiers.
pub(crate) const KEYWORD_RECEIVER_KINDS: &[&str] =
    &["self", "this", "self_expression", "this_expression"];

pub(crate) fn path_from_node(node: &Node, source: &str) -> Option<Vec<String>> {
    resolve_path(node, source, false)
}

pub(crate) fn path_matches(actual: &[String], pattern: &[Option<String>]) -> bool {
    let segment_ok = |a: &String, p: &Option<String>| p.as_ref().is_none_or(|w| w == a);
    if actual.len() == pattern.len() {
        return actual
            .iter()
            .zip(pattern.iter())
            .all(|(a, p)| segment_ok(a, p));
    }
    // The reference binds a leading metavariable to the WHOLE
    // multi-segment receiver — `$O.$M($$$A)` matches `a.b.c(2)` with
    // `$O` = `a.b` (probed on java/python/ts/js/rust/kotlin/swift). A literal
    // leading segment pins the chain to the exact length (`obj.$M($$$A)` does
    // not reach `a.b.c`), and a bare-name pattern never matches a member chain
    // (`helper($$$A)` stays empty everywhere).
    if actual.len() > pattern.len() && pattern.first().is_some_and(|p| p.is_none()) {
        return actual[actual.len() - pattern.len() + 1..]
            .iter()
            .zip(pattern[1..].iter())
            .all(|(a, p)| segment_ok(a, p));
    }
    false
}
