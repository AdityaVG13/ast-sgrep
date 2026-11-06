//! Call-chain resolution and optional-call matching.

use super::*;
use crate::extract::{
    is_ident_kind, is_member_expr_kind, last_identifier_in_chain, node_text,
};
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
