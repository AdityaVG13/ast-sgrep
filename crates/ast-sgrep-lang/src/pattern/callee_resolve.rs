//! Callee resolution and call-junction gates.

use super::*;
use crate::extract::{is_ident_kind, is_in_comment_or_string, node_text};
use crate::Language;
use tree_sitter::Node;

/// When `node` is a call outside trivia that matches `path`, return its callee segments.
pub(crate) fn call_match_path(
    node: &Node,
    source: &str,
    path: &[Option<String>],
) -> Option<Vec<String>> {
    // Non-call nodes need no ancestor lookup; call candidates retain the
    // complete comment/string veto before callee resolution.
    if !is_call_kind(node.kind()) || is_in_comment_or_string(node) {
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

/// Which receiver fidelity the unified callee resolver enforces — the one
/// resolution path for the S2 extractor family (`call_callee` /
/// `call_target_path` / `call_target_path_faithful`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CalleeStrictness {
    /// Match/index resolution: receivers flatten through the last
    /// identifier (`a["b"]` contributes `a`), keeping every registered
    /// match and `call:` row.
    Lenient,
    /// Metavariable-binding resolution: a receiver chain that flattens
    /// through a call/paren/index node has no faithful per-segment text,
    /// so the path vetoes instead of unifying on flattened tails.
    Faithful,
}

/// Resolve a call node's callee into dotted path segments plus whether the
/// chain was reassembled from split fields rather than read off one node —
/// the single dispatch behind the former `call_callee` /
/// `call_target_path_faithful` pair. The split-field arms (java
/// `method_invocation`, ruby receiver-dot `call`, php
/// `member_call_expression`, php `scoped_call_expression`) live here once;
/// the strictness flag only selects the leaf path builder (`resolve_path`).
/// MoonBit `dot_apply_expression` stays with `call_target_path`: its
/// match-path shape (`call_field_node` + lenient path) is registered
/// separately from its index-row shape.
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
pub(crate) fn resolve_callee<'a>(
    node: &Node<'a>,
    source: &'a str,
    strictness: CalleeStrictness,
) -> Option<(Vec<String>, bool)> {
    let strict = matches!(strictness, CalleeStrictness::Faithful);
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
            let mut segs = resolve_path(&obj, source, strict)?;
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
    //
    // The nullsafe spelling shares the object/name field split, but only
    // the lenient path resolves it here — the faithful path serves the
    // nullsafe faces through its own lane, which never reaches this arm.
    if node.kind() == "member_call_expression"
        || (!strict && node.kind() == "nullsafe_member_call_expression")
    {
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
                if !strict {
                    return Some((Vec::new(), true));
                }
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
    resolve_path(&field, source, strict).map(|segs| (segs, false))
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
    resolve_callee(node, source, CalleeStrictness::Lenient)
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
