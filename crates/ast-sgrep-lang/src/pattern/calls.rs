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
                arg_slots_match(slots, nodes, source, &mut captures)?;
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
                .map(|text| strip_container(&text).to_string());
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
                .map(|text| strip_container(&text).to_string());
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
    // Php's member calls split the chain across fields — the callee is a
    // `name` node, not a member expression, and the nullsafe connector
    // gets its own named kind `nullsafe_member_call_expression` (plain
    // `->` stays `member_call_expression`). The kind IS the token-exact
    // discriminator: the plain spelling never answers here, and the
    // wildcard head folds the whole object expression like the reference
    // (O= `g?->h` on `g?->h?->i()`).
    if matches!(
        node.kind(),
        "member_call_expression" | "nullsafe_member_call_expression"
    ) {
        let object = node.child_by_field_name("object")?;
        let name = node.child_by_field_name("name")?;
        let nullsafe = node.kind() == "nullsafe_member_call_expression";
        if !nullsafe {
            return None;
        }
        let head_text = node_text(&object, source)?;
        let tail_text = node_text(&name, source)?;
        if tail_literal.is_some_and(|literal| literal != tail_text) {
            return None;
        }
        if head_literal.is_some_and(|literal| literal != head_text) {
            return None;
        }
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
        return Some(captures);
    }
    let field = call_field_node(node)?;
    let (segments, connectors) = optional_chain_decompose(lang, &field)?;
    // A two-segment template needs a member chain: head + `?.` + tail.
    if segments.len() < 2 || !*connectors.last()? {
        return None;
    }
    let tail_node = segments.last()?;
    let tail_text = node_text(tail_node, source)?;
    if tail_literal.is_some_and(|literal| literal != tail_text) {
        return None;
    }
    // Wildcard-head folding: the receiver span covers segments 0..last,
    // ending at the LAST FOLDED segment's end — the trailing `?.` connector
    // bytes stay out of the capture (the reference binds
    // `user?.profile`, never `user?.profile?`; the proven codemod
    // corruption class).
    let head_node = &segments[0];
    let head_end = segments[segments.len() - 2].end_byte();
    let head_text = source.get(head_node.start_byte()..head_end)?;
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
/// the registered plain-call rest-slot semantics (`call_arg_slots_match`:
/// trailing rest + single ⇒ n ≥ 2, k ≥ 2 rests ⇒ n ≥ k-1, non-trailing
/// rest ⇒ n == 1 — the `?.`-chain faces answer at exactly those arities).
/// Plain templates keep `arguments_match` + the args-capture bind,
/// byte-identical.
pub(crate) fn chain_segment_args_contract(
    node: &Node,
    segment: &CallChainSegment,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    if let Some(slots) = &segment.arg_slots {
        let nodes = argument_nodes(node, &["arguments"])?;
        return call_arg_slots_match(slots, &nodes, source, captures);
    }
    if !arguments_match(node, segment.args.as_ref(), &["arguments"]) {
        return None;
    }
    if let Some((name, multi)) = &segment.args_capture {
        if let Some(text) = capture_arguments_text(node, source) {
            bind_capture_kind(captures, name, strip_container(&text), *multi)?;
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
        if let Some((name, multi)) = &segment.args_capture {
            if let Some(text) = capture_arguments_text(&receiver, source) {
                // The argument capture binds the LIST CONTENT (container
                // stripped), the same convention as the single-call lane's
                // `$$$` capture — the reference's multi list for `()` is empty.
                bind_capture_kind(&mut captures, name, strip_container(&text), *multi)?;
            }
        }
        // Positional `$A, $B` binding, same contract as the member-chain lane
        // (`r.m1($A, $B).m2()` binds A=1, B=2).
        if let Some(names) = &segment.arg_metas {
            if let Some(nodes) = argument_nodes(&receiver, &["arguments"]) {
                for (name, argument) in names.iter().zip(nodes.iter()) {
                    if let Some(text) = node_text(argument, source) {
                        bind_capture(&mut captures, name, text)?;
                    }
                }
            }
        }
    }
    if !arguments_match(node, last.args.as_ref(), &["arguments"]) {
        return None;
    }
    if let Some((name, multi)) = &last.args_capture {
        if let Some(text) = capture_arguments_text(node, source) {
            bind_capture_kind(&mut captures, name, strip_container(&text), *multi)?;
        }
    }
    // The tail call carries the positional contract.
    if let Some(names) = &last.arg_metas {
        if let Some(nodes) = argument_nodes(node, &["arguments"]) {
            for (name, argument) in names.iter().zip(nodes.iter()) {
                if let Some(text) = node_text(argument, source) {
                    bind_capture(&mut captures, name, text)?;
                }
            }
        }
    }
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
    let field = call_field_node(node)?;
    member_receiver(&field)
}

/// The candidate call's argument-list text between its parentheses.
pub(crate) fn capture_arguments_text(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("arguments")
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

/// When `node` is a call outside trivia that matches `path`, return its callee segments.
pub(crate) fn call_match_path(
    node: &Node,
    source: &str,
    path: &[Option<String>],
) -> Option<Vec<String>> {
    if is_in_comment_or_string(node) || !is_call_kind(node.kind()) {
        return None;
    }
    let (callee, synthetic_chain) = call_callee(node, source)?;
    // A synthetic chain (java/php `object`+`name` splits, ruby receiver-dot
    // calls) spans several AST fields, so there is no single callee node for a
    // lone metavariable to bind: the reference keeps `$F($$$A)` /
    // `helper($$$A)` empty on those calls (pinned java/ruby probes).
    // Two-segment-plus patterns resolve normally.
    if synthetic_chain && path.len() < 2 {
        return None;
    }
    path_matches(&callee, path).then_some(callee)
}

/// Callee resolution for call-pattern matching and `call:` index rows.
///
/// Returns the dotted path segments, the full callee source text, and whether
/// the chain was reassembled from split fields rather than read off one node.
///
/// - Grammars that split a member call across `object` + `name` fields
///   (java `method_invocation`) previously exposed only the trailing `name`
///   identifier: literal dotted patterns (`System.out.println($$$A)`) were
///   silently empty while trailing-name patterns (`println($$$A)`)
///   over-matched.
/// - Ruby receiver-dot calls carry the callee as `receiver` + `method`; they
///   are path calls only with a `.` operator AND a parenthesized argument
///   list. Bare `text.upcase` and operator `"a" + "b"` calls stay unmatched,
///   exactly matching the reference (pinned probes).
pub(crate) fn call_callee<'a>(node: &Node<'a>, source: &'a str) -> Option<(Vec<String>, bool)> {
    // `method_invocation` is java(/kotlin-family) member calls: the member
    // operator is `.`, so dot-separated patterns express them. php's
    // `member_call_expression` also splits object/name but its operator is
    // `->`, which dot patterns can never spell — the reference keeps
    // every dot-pattern empty there, so php must keep trailing-name shape.
    if node.kind() == "method_invocation" {
        if let (Some(obj), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            let mut segs = path_from_node(&obj, source)?;
            segs.push(node_text(&name, source)?.to_string());
            return Some((segs, true));
        }
    }
    if node.kind() == "call" {
        if let Some(segs) = ruby_receiver_callee(node, source) {
            return Some((segs, true));
        }
    }
    // Php `member_call_expression` (plain `->` calls) is served by the
    // dedicated [`NativeKind::MemberCall`] lane ONLY — the plain Call lane
    // must never see a member-call candidate, because a member callee
    // resolves to the full object->name segment chain and a DOT template
    // would over-match the `->` call site (matching is connector
    // token-exact). The synthetic-chain flag keeps the lone-metavar veto,
    // and an object the strict resolver cannot spell yields the EMPTY veto
    // shape instead of the trailing-name shape — every bare-name pattern
    // stays empty on member calls. `call_target` falls back to the raw
    // callee bytes for the index row.
    if node.kind() == "member_call_expression" || node.kind() == "nullsafe_member_call_expression" {
        // The nullsafe spelling shares the object/name field split; its
        // segments serve the flat nullsafe slot faces (the receiver-side
        // `nullsafe_member_access` link still vetoes empty —
        // unresolvable-object discipline unchanged).
        if let (Some(object), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            if let Some(name_text) = node_text(&name, source) {
                if let Some(mut segments) = php_member_object_segments(&object, source) {
                    segments.push(name_text.to_string());
                    return Some((segments, true));
                }
                return Some((Vec::new(), true));
            }
        }
    }
    // Php `scoped_call_expression` (static `::` calls) splits the callee
    // across `scope` + `name` fields with no single callee node. The
    // reference matches scope-to-scope and name-to-name (`Foo::bar($A)`
    // answers; `$A::bar($B)` binds the RAW scope text — `Foo`, `self`,
    // `$inst`; `Foo::$M($A)` binds `$dyn` with the dollar), so the dotted
    // two-segment chain IS the shape here (patterns normalize `::` to `.`).
    // The synthetic-chain flag keeps the lone-metavar veto: `$F($$$A)`
    // stays empty on `Foo::bar(1)` exactly like the reference (only plain
    // `helper(...)` calls answer). Php `->` member calls keep the
    // registered trailing-name shape — this arm keys on
    // `scoped_call_expression` only.
    if node.kind() == "scoped_call_expression" {
        if let (Some(scope), Some(name)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(scope_text), Some(name_text)) =
                (node_text(&scope, source), node_text(&name, source))
            {
                return Some((vec![scope_text.to_string(), name_text.to_string()], true));
            }
        }
    }
    let field = call_field_node(node)?;
    path_from_node(&field, source).map(|segs| (segs, false))
}

/// Strict object-side segments for a php member call. A `variable_name`
/// (`$svc`) is ONE raw-text segment; a plain `member_access_expression`
/// descends (`$a->b` → `["$a", "b"]`); identifier kinds pass through.
/// Nullsafe-access and call objects are refused — their connector/shape
/// fidelity has no contract here, so those faces stay off the plain-`->`
/// chain lane (fail-closed, never an over-match).
pub(crate) fn php_member_object_segments<'a>(
    node: &Node<'a>,
    source: &'a str,
) -> Option<Vec<String>> {
    match node.kind() {
        "member_access_expression" => {
            let mut segments =
                php_member_object_segments(&node.child_by_field_name("object")?, source)?;
            segments.push(node_text(&node.child_by_field_name("name")?, source)?.to_string());
            Some(segments)
        }
        "nullsafe_member_access_expression" => None,
        // A scoped-call object (`Foo::bar($u)->baz()`) decomposes into its
        // scope::name segments — the chain shape.
        "scoped_call_expression" => call_callee(node, source).map(|(segments, _)| segments),
        // A static-prop object (`C::$s->m()`, `self::$s->m()`) decomposes
        // into its scope::prop segments — the same faithful two-segment head
        // the scoped-call arm spells.
        "scoped_property_access_expression" => {
            let scope = node.child_by_field_name("scope")?;
            let prop = node.child_by_field_name("name")?;
            Some(vec![
                node_text(&scope, source)?.to_string(),
                node_text(&prop, source)?.to_string(),
            ])
        }
        _ => {
            if is_ident_kind(node.kind())
                || KEYWORD_RECEIVER_KINDS.contains(&node.kind())
                || node.kind() == "variable_name"
            {
                node_text(node, source).map(|text| vec![text.to_string()])
            } else {
                None
            }
        }
    }
}

/// Ruby `receiver.method(...)` chain segments. The top-level call must carry a
/// `.` operator AND a parenthesized argument list — bare `text.upcase` and
/// operator `"a" + "b"` calls are not call-pattern shapes (the reference
/// matches nothing on them). Nested chains recurse through bare receiver
/// calls (`a.b.c(1)` → `["a", "b", "c"]`), where the intermediate `a.b`
/// has no arguments of its own.
pub(crate) fn ruby_receiver_callee(node: &Node, source: &str) -> Option<Vec<String>> {
    if node.kind() != "call" {
        return None;
    }
    let operator = node.child_by_field_name("operator")?;
    if node_text(&operator, source) != Some(".") {
        return None;
    }
    node.child_by_field_name("arguments")?;
    let receiver = node.child_by_field_name("receiver")?;
    let method = node.child_by_field_name("method")?;
    let mut segs = ruby_receiver_base(&receiver, source)?;
    segs.push(node_text(&method, source)?.to_string());
    Some(segs)
}

/// Receiver-side resolution: like [`ruby_receiver_callee`] but without the
/// argument-list requirement, descending through nested dot calls; non-call
/// receivers (identifiers, constants, `self`, member kinds) fall through to
/// the shared path builder.
pub(crate) fn ruby_receiver_base(node: &Node, source: &str) -> Option<Vec<String>> {
    if node.kind() == "call" {
        let operator = node.child_by_field_name("operator")?;
        if node_text(&operator, source) != Some(".") {
            return None;
        }
        let receiver = node.child_by_field_name("receiver")?;
        let method = node.child_by_field_name("method")?;
        let mut segs = ruby_receiver_base(&receiver, source)?;
        segs.push(node_text(&method, source)?.to_string());
        return Some(segs);
    }
    path_from_node(node, source)
}

pub(crate) fn call_field_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    ["function", "name"]
        .into_iter()
        .find_map(|f| node.child_by_field_name(f))
        // Swift/Kotlin call_expression has no function/name fields; callee is the
        // first named child. MoonBit apply/dot-apply calls are fieldless too. Safe
        // for other langs: C# uses `invocation_expression`, and field-bearing
        // grammars hit find_map first.
        .or_else(|| {
            matches!(
                node.kind(),
                "call_expression" | "apply_expression" | "dot_apply_expression"
            )
            .then(|| node.named_child(0))
            .flatten()
        })
        // Ruby's call kind is `call` with the callee in the `method`
        // field, so neither probe above ever fired — every ruby
        // call pattern (index rows AND native matches) was silently empty.
        // The callee is honored only on receiver-free calls: the reference
        // agrees with `$F($$$A)`/`upcase($$$A)` on `greet("world")` but matches
        // nothing on `text.upcase` or the operator call `"hello " + name`
        // (both are `call` nodes carrying a `receiver`).
        .or_else(
            || match node.kind() == "call" && node.child_by_field_name("receiver").is_none() {
                true => node.child_by_field_name("method"),
                false => None,
            },
        )
}

/// Exact-children discipline at the matched call node, in complement form:
/// every child must end at/before the callee's end byte, start at/after the
/// argument list's start byte, or be an UNNAMED token without `?`. Gap
/// whitespace is invisible to this scan. A named extra in the open
/// `(callee.end, arguments.start)` gap (a comment, a ts `type_arguments`
/// sibling) or the anonymous `?.` token breaks the match for LITERAL and
/// META heads alike, while comments inside the argument list, the
/// member-chain callee, or outside the call node are transparent trivia.
/// Unresolvable callee/argument nodes skip the check. The one junction
/// rule, consulted from the capture path and the lanes that bypass it.
/// NOT covered: java `a.<T>b(1)` — its `type_arguments` precedes `name`.
pub(crate) fn call_junction_exact(node: &Node) -> bool {
    let Some(callee) = call_field_node(node) else {
        return true;
    };
    let Some(arguments) = argument_container(node, &["arguments"]) else {
        return true;
    };
    let mut cursor = node.walk();
    let exact = node.children(&mut cursor).all(|child| {
        child.end_byte() <= callee.end_byte()
            || child.start_byte() >= arguments.start_byte()
            || (!child.is_named() && !child.kind().contains('?'))
    });
    exact
}

/// The cs callee→`(` junction gap law — the junction bytes must be
/// comment-free AND all [`is_sg_cs_trivia`]: a comment or a Rust-ws outsider
/// run (U+2028/U+2029) refuses while FEFF/NBSP/ASCII-ws gaps bind (the cs
/// `\s` is Unicode there). A zero-length gap trivially binds. Comments INSIDE
/// the argument list sit outside the junction and keep binding.
pub(crate) fn cs_call_junction_trivia_free(node: &Node, source: &str) -> bool {
    let Some(callee) = call_field_node(node) else {
        return true;
    };
    let Some(arguments) = argument_container(node, &["arguments"]) else {
        return true;
    };
    let gap = &source[callee.end_byte()..arguments.start_byte()];
    strip_comment_spans(gap) == gap && gap.chars().all(is_sg_cs_trivia)
}

/// The AFTER-`>` junction consult for type-args-spelled calls. The general
/// lane serves `g<T>($A)` faces (the simple-call gate excludes the typeargs
/// spelling), so the exact-children doctrine of [`call_junction_exact`]
/// must be consulted here too: a trivia child fully inside the
/// typeargs→arguments gap breaks the structural match and the candidate is
/// refused, descent-only. A whitespace-only gap presents no child node and
/// stays admitted. Scoped to js/ts, to templates whose root carries a
/// DIRECT type_arguments child, and to candidates carrying both a
/// type_arguments child and an arguments container.
pub(crate) fn ts_typeargs_junction_refused(
    lang: Language,
    template_root: Node,
    node: &Node,
    _source: &str,
) -> bool {
    if !matches!(lang, Language::TypeScript | Language::JavaScript) {
        return false;
    }
    let mut tcur = template_root.walk();
    if !template_root
        .children(&mut tcur)
        .any(|child| child.kind() == "type_arguments")
    {
        return false;
    }
    let mut ccur = node.walk();
    let children: Vec<Node> = node.children(&mut ccur).collect();
    let Some(typeargs) = children
        .iter()
        .find(|child| child.kind() == "type_arguments")
    else {
        return false;
    };
    let Some(arguments) = argument_container(node, &["arguments"]) else {
        return false;
    };
    children.iter().any(|child| {
        child.end_byte() > typeargs.end_byte() && child.start_byte() < arguments.start_byte()
    })
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
    if node.kind() == "method_invocation" {
        if let (Some(obj), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            let mut segs = faithful_path_from_node(&obj, source)?;
            segs.push(node_text(&name, source)?.to_string());
            return Some(segs);
        }
    }
    if node.kind() == "call" {
        // Ruby receiver-dot calls keep their registered flattening
        // (pinned ruby probes); only the member-chain decomposition below
        // is refined here.
        if ruby_receiver_callee(node, source).is_some() {
            return call_target_path(node, source);
        }
    }
    // Php static calls decompose into exactly the scope and name positions
    // the reference unifies against (raw texts — `$inst`/`$dyn` included).
    // The scope is ONE node, so its text is faithful by construction;
    // there is no flattening to veto. The plain `->` member-call twin —
    // object and name are single faithful nodes/strict chains (the strict
    // object resolver refuses flattening), so the raw texts unify exactly
    // like the reference.
    if node.kind() == "member_call_expression" {
        if let (Some(object), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(mut segments), Some(name_text)) = (
                php_member_object_segments(&object, source),
                node_text(&name, source),
            ) {
                segments.push(name_text.to_string());
                return Some(segments);
            }
        }
    }
    if node.kind() == "scoped_call_expression" {
        if let (Some(scope), Some(name)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(scope_text), Some(name_text)) =
                (node_text(&scope, source), node_text(&name, source))
            {
                return Some(vec![scope_text.to_string(), name_text.to_string()]);
            }
        }
    }
    let field = call_field_node(node)?;
    faithful_path_from_node(&field, source)
}

/// Like [`path_from_node`], but returning `None` instead of falling back to
/// `last_identifier_in_chain` — a chain segment that is not a plain
/// identifier or member expression (a call receiver, paren, subscript…)
/// has no faithful segment text. The swift/kotlin grammars wrap each member
/// link in a `navigation_suffix` node — the suffix is part of the
/// member-chain decomposition, so the faithful resolver reads straight
/// through it (its named child contributes the next segment). The wrapper
/// never hides a `?.` connector: swift spells the optional link as a
/// separate named `?` SIBLING before the suffix, and the loop's `?`-kind
/// veto below fires on it — the `.`-template stays connector token-exact.
pub(crate) fn faithful_path_from_node(node: &Node, source: &str) -> Option<Vec<String>> {
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        return node_text(node, source).map(|t| vec![t.to_string()]);
    }
    if !is_member_expr_kind(node.kind()) && !is_navigation_suffix_kind(node.kind()) {
        return None;
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
        // Chain decomposition is comment-transparent — `a /*c*/ .b(1)`,
        // `a. /*c*/ b(1)`, `a /*c*/ . /*d*/ b(1)` and the 3-link
        // `a /*c*/ .b /*d*/ .c(1)` all answer `a.b(...)`/`a.b.c(...)`, and
        // a meta head binds the CLEAN identifier text (`$A` = `a` at the
        // identifier's byte range). Skip trivia/extra children so
        // comment-bearing chains stay on the faithful arm instead of
        // routing to the nonfaithful receiver slice (mirroring
        // `argument_nodes`' filters); the `?`-kind veto above runs FIRST,
        // so `a?. /*c*/ b(1)` keeps its registered refusal, and non-member
        // children (subscript/call receivers) still veto below.
        if is_trivia_kind(child.kind()) || child.is_extra() {
            continue;
        }
        // Unlike `path_from_node`, a NAMED child with no faithful path
        // (a call/paren/subscript receiver) VETOES the whole chain
        // instead of being silently skipped — its "segments" would be
        // flattened lies about the receiver shape.
        let mut p = faithful_path_from_node(&child, source)?;
        segs.append(&mut p);
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
    if let Some(accessor) = crate::extract::dot_accessor_text(node, source) {
        return Some(vec![accessor.to_string()]);
    }
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        return node_text(node, source).map(|t| vec![t.to_string()]);
    }
    if !is_member_expr_kind(node.kind()) {
        return last_identifier_in_chain(node, source).map(|s| vec![s]);
    }
    let mut segs = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Optional-chain separators veto the path (see
        // `faithful_path_from_node`) — anonymous `?.` tokens and named
        // `optional_chain` nodes alike.
        if child.kind().contains('?') || child.kind().contains("optional") {
            return None;
        }
        if let Some(mut p) = path_from_node(&child, source) {
            segs.append(&mut p);
        }
    }
    (!segs.is_empty()).then_some(segs)
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
