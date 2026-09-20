//! Capture binding and hit pushing.

use super::*;
use crate::extract::{is_ident_kind, is_member_expr_kind, node_lines, node_text};
use crate::PatternNode;
use std::borrow::Cow;
use std::collections::BTreeMap;
use tree_sitter::Node;

// e2hc/difu.5: invocation_expression is the C# tree-sitter grammar's call node.
// MoonBit calls are `apply_expression` (bare) / `dot_apply_expression` (`.m(...)`);
// the callee is the first named child in both.
pub(crate) const CALL_KINDS: &[&str] = &[
    "call_expression",
    "call",
    "method_invocation",
    "invocation_expression",
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
    "apply_expression",
    "dot_apply_expression",
];

pub(crate) fn is_call_kind(kind: &str) -> bool {
    CALL_KINDS.contains(&kind)
}

/// Full callee text for `call:` index rows. Split-field chains (java/php
/// `object`+`name`, ruby receiver-dot calls) have no single callee node, so
/// their text is reassembled from the resolved segments; every other grammar
/// keeps the callee node's exact source bytes.
pub(crate) fn call_target<'a>(node: &Node<'a>, source: &'a str) -> Option<Cow<'a, str>> {
    // MoonBit `obj.method()` is fieldless: the trailing accessor is only the
    // method, so a trailing-name `call:trim` row made `trim($$$)` index-serve
    // member calls (an over-match) and left `name.trim($$$)` with no
    // `call:name.trim` row (silent miss). Join the same path the matcher uses.
    if node.kind() == "dot_apply_expression" {
        return call_target_path(node, source).map(|segs| Cow::Owned(segs.join(".")));
    }
    // Php static-call rows key on the exact source callee bytes (`Foo::bar`),
    // NOT the trailing name. The old `call:bar` key made `bar($$$A)`
    // index-serve every `Foo::bar(1)` line (silent over-match) and left
    // `Foo::bar($$$A)` — whose signature derives the raw `call:Foo::bar` —
    // with no rows to hit (silent miss). The callee spelling is
    // `scope::name`.
    if node.kind() == "scoped_call_expression" {
        if let (Some(scope), Some(name)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(scope_text), Some(name_text)) =
                (node_text(&scope, source), node_text(&name, source))
            {
                return Some(Cow::Owned(format!("{scope_text}::{name_text}")));
            }
        }
    }
    let (segs, synthetic) = call_callee(node, source)?;
    // An empty synthetic chain is the member-call veto shape (an object the
    // strict resolver cannot spell) — the index row keeps the registered raw
    // callee bytes instead of a junk key.
    if synthetic && !segs.is_empty() {
        return Some(Cow::Owned(segs.join(".")));
    }
    if synthetic && segs.is_empty() {
        return call_field_node(node)
            .and_then(|t| node_text(&t, source))
            .map(Cow::Borrowed);
    }
    call_field_node(node)
        .and_then(|t| node_text(&t, source))
        .map(Cow::Borrowed)
}

pub(crate) fn push_pattern_node(
    node: Node,
    source: &str,
    signature: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    let (line_start, line_end) = node_lines(&node, source);
    if !seen.insert((signature.to_string(), line_start)) {
        return;
    }
    out.push(PatternNode {
        signature: signature.to_string(),
        line_start,
        line_end,
        excerpt: excerpt_for_node(&node, source, signature),
    });
}

pub(crate) fn push_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_text: Option<&str>,
    out: &mut Vec<PatternMatch>,
) {
    let Some(captures) = captures_for_node(node, source, pattern, name_text) else {
        // A repeated metavariable name is bound to two different texts;
        // the reference unification semantics reject the candidate.
        return;
    };
    push_match_with_captures(node, source, pattern, captures, out);
}

/// Push a fully-built capture map (the slot-matching lanes build captures
/// directly — the generic pattern-text path would mis-bind mixed argument
/// lists). Same byte-range dedup as the derived path.
pub(crate) fn push_match_with_captures(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: BTreeMap<String, String>,
    out: &mut Vec<PatternMatch>,
) {
    let byte_start = node.start_byte();
    let byte_end = node.end_byte();
    if out
        .iter()
        .any(|matched| matched.byte_start == byte_start && matched.byte_end == byte_end)
    {
        return;
    }
    out.push(hit_for_node(node, source, pattern, captures));
}

pub(crate) fn captures_for_node(
    node: &Node,
    source: &str,
    pattern: &str,
    name_text: Option<&str>,
) -> Option<BTreeMap<String, String>> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(variable), Some(name)) = (declaration_name_capture(pattern), name_text) {
        bind_capture_kind(
            &mut captures,
            variable,
            name,
            declaration_head_is_multi_meta(pattern),
        )?;
    }
    // An if-prefixed pattern has NO argument template —
    // `pattern_argument_text` misreads the condition section (`if ($X) { $B }`
    // → `$X`) as one, `argument_container` descends into the condition's inner
    // call, and $X first binds the ARGUMENT text; `if_condition_capture` then
    // binds the whole condition text and the `bind_capture` conflict silently
    // dropped every candidate whose condition holds a call with >=1 argument
    // (js/ts/php + paren-spelled go/py). Skip the argument capture on the if
    // lane — the same is_if_prefixed special-case the body capture applies
    // two arms below.
    if !is_if_prefixed(pattern.trim()) {
        capture_arguments(node, source, pattern, &mut captures)?;
    }
    if let Some((variable, multi)) = body_capture(pattern) {
        let body = if is_if_prefixed(pattern.trim()) {
            if_consequence(node)
        } else {
            function_body_node(node)
        };
        if let Some(text) = body.and_then(|body| node_text(&body, source)) {
            bind_capture_kind(&mut captures, variable, strip_container(text), multi)?;
        }
    }
    if let Some(variable) = if_condition_capture(pattern) {
        if let Some(condition) = node.child_by_field_name("condition") {
            if let Some(text) = node_text(&condition, source) {
                bind_capture(&mut captures, variable, strip_container(text))?;
            }
        }
    }
    capture_call_path(node, source, pattern, &mut captures)?;
    Some(captures)
}

/// Unified metavariable binding. A repeated name is ONE variable bound
/// once (reference semantics): binding an already-bound name to
/// a different text returns `None` and rejects the whole candidate match;
/// equal text re-affirms the binding. The reserved envelope key `MATCH`
/// (full node text) keeps its historical overwrite behavior so a pattern
/// that spells `$MATCH` as an ordinary metavariable keeps today's envelope
/// bytes.
pub(crate) fn bind_capture(
    captures: &mut BTreeMap<String, String>,
    name: &str,
    text: &str,
) -> Option<()> {
    if name == "MATCH" {
        captures.insert(name.to_string(), text.to_string());
        return Some(());
    }
    match captures.get(name) {
        Some(existing) => (existing == text).then_some(()),
        None => {
            captures.insert(name.to_string(), text.to_string());
            Some(())
        }
    }
}

/// The reference keeps single and multi metavariable namespaces DISTINCT
/// (`metaVariables.single.B` and `metaVariables.multi.B` coexist), so a name
/// bound BOTH as `$B` and `$$$B` is two variables. Multi bindings key as
/// `$$$NAME` in the capture map and never unify with the single binding of
/// the same name; unification within one namespace is unchanged.
pub(crate) fn bind_capture_kind(
    captures: &mut BTreeMap<String, String>,
    name: &str,
    text: &str,
    multi: bool,
) -> Option<()> {
    if !multi || name == "MATCH" {
        return bind_capture(captures, name, text);
    }
    let key = format!("$$${name}");
    match captures.get(&key) {
        Some(existing) => (existing == text).then_some(()),
        None => {
            captures.insert(key, text.to_string());
            Some(())
        }
    }
}

pub(crate) fn capture_name(token: &str) -> Option<&str> {
    let token = token.trim();
    // `$$NAME` binds in the SINGLE namespace under `NAME`, so it unifies
    // with a same-name `$NAME` — `$A::bar($$A)` answers [] on php exactly
    // like the reference.
    let name = token
        .strip_prefix("$$$")
        .or_else(|| token.strip_prefix("$$"))
        .or_else(|| token.strip_prefix('$'))?;
    is_metavar_name(name).then_some(name)
}

pub(crate) fn declaration_name_capture(pattern: &str) -> Option<&str> {
    let (declaration, _) = strip_declaration_modifiers(pattern);
    DECL_PATTERN_PREFIXES.iter().find_map(|(prefix, _)| {
        let rest = declaration.strip_prefix(prefix)?;
        let head = rest
            .split(|c: char| c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace())
            .next()?;
        capture_name(head)
    })
}

/// Whether the declaration-head meta is `$$$`-prefixed — the capture binds
/// in the MULTI namespace (`metaVariables.multi`); `$$`/`$` heads stay
/// single.
pub(crate) fn declaration_head_is_multi_meta(pattern: &str) -> bool {
    let (declaration, _) = strip_declaration_modifiers(pattern);
    DECL_PATTERN_PREFIXES
        .iter()
        .find_map(|(prefix, _)| {
            let rest = declaration.strip_prefix(prefix)?;
            let head = rest
                .split(|c: char| c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace())
                .next()?;
            Some(head.trim().starts_with("$$$"))
        })
        .unwrap_or(false)
}

pub(crate) fn capture_arguments(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let Some(arguments) = pattern_argument_text(pattern) else {
        return Some(());
    };
    let fields = if is_call_kind(node.kind()) {
        &["arguments"][..]
    } else {
        &["parameters"][..]
    };
    if arguments.starts_with("$$$") {
        if let Some(variable) = capture_name(arguments) {
            let text = argument_container(node, fields)
                .and_then(|container| node_text(&container, source))
                .map(strip_container)
                .unwrap_or_default();
            // `$$$` arguments bind in the multi namespace.
            bind_capture_kind(captures, variable, text, true)?;
        }
        return Some(());
    }
    let names = arguments
        .split(',')
        .filter_map(capture_name)
        .collect::<Vec<_>>();
    let Some(nodes) = argument_nodes(node, fields) else {
        return Some(());
    };
    for (name, argument) in names.into_iter().zip(nodes) {
        if let Some(text) = node_text(&argument, source) {
            bind_capture(captures, name, text)?;
        }
    }
    Some(())
}

/// Returns `(name, multi)` — `multi` is true when the body template capture
/// was spelled `$$$NAME`, so the binding lands in the multi namespace and
/// never unifies with a same-named `$NAME` capture.
pub(crate) fn body_capture(pattern: &str) -> Option<(&str, bool)> {
    if let Some(open) = pattern.find('{') {
        let close = pattern.rfind('}')?;
        let section = pattern.get(open + 1..close)?.trim();
        let name = capture_name(section)?;
        return Some((name, section.starts_with("$$$")));
    }
    let trimmed = pattern.trim();
    if is_if_prefixed(trimmed) {
        let body = trimmed.rsplit_once(':')?.1.trim();
        let name = capture_name(body)?;
        return Some((name, body.starts_with("$$$")));
    }
    // Python-style suite body — `def $A($B): $$$C`. The section after the
    // parameter list is a body template, so a `$$$` capture there must bind
    // for rewrites instead of erroring unbound.
    suite_body_capture(trimmed)
}

/// `(name, multi)` for `$$$NAME` / `$NAME` in the `: body` suite after a
/// declaration's parameter list.
pub(crate) fn suite_body_capture(pattern: &str) -> Option<(&str, bool)> {
    let (declaration, _) = strip_declaration_modifiers(pattern);
    let open = declaration.find('(')?;
    let close = open + declaration[open..].find(')')?;
    let after = declaration[close + 1..].trim();
    let body = after.strip_prefix(':')?.trim();
    let name = capture_name(body)?;
    Some((name, body.starts_with("$$$")))
}

pub(crate) fn if_condition_capture(pattern: &str) -> Option<&str> {
    let rest = pattern.trim().strip_prefix("if")?.trim_start();
    if let Some(inner) = rest.strip_prefix('(') {
        let close = inner.find(')')?;
        capture_name(&inner[..close])
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '{' || c == ':')
            .unwrap_or(rest.len());
        capture_name(&rest[..end])
    }
}

pub(crate) fn strip_container(text: &str) -> &str {
    let trimmed = text.trim();
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        if trimmed.starts_with(open) && trimmed.ends_with(close) {
            return trimmed[open.len_utf8()..trimmed.len() - close.len_utf8()].trim();
        }
    }
    trimmed
}

pub(crate) fn capture_call_path(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    // The receiver-path binding only makes sense for patterns that ARE a
    // call (`callee(args)` with a single argument list). Any other shape
    // — a let/assignment whose value is a member call (`let r2 = q.len();`,
    // `hh = u.len()`), a statement compound — parses a bogus callee out of
    // the first `(` and its veto discarded matches the literal lane had
    // answered correctly while the reference answers those faces. Skip the
    // binding for non-call shapes instead of vetoing.
    let simple_call = match (pattern.trim().find('('), pattern.trim().rfind(')')) {
        (Some(open), Some(close)) if close > open => {
            let trimmed = pattern.trim();
            // Normalize `::` like `parse_call_path`, so a php static-call
            // callee (`$A::bar`) decomposes into its segments here too —
            // without this the guard saw one non-ident segment (`$A::bar`)
            // and skipped scope/name capture binding entirely. The php `->`
            // member connector normalizes the same way (`$O->m` binds the
            // object capture through the faithful member arm).
            trimmed[close + 1..].trim().is_empty()
                && !trimmed[open + 1..close].contains(['(', ')'])
                && trimmed[..open]
                    .replace("::", ".")
                    .replace("->", ".")
                    .split('.')
                    .all(|segment| {
                        let segment = segment.trim();
                        is_pure_metavariable(segment)
                            || is_pattern_ident(segment)
                            || namespace_qualified_segment(segment)
                            || dollar_name_class(segment.strip_prefix('$').unwrap_or(""))
                                == Some(DollarTokenClass::LowercaseLed)
                    })
        }
        _ => false,
    };
    if !simple_call {
        return Some(());
    }
    let Some(open) = pattern.find('(') else {
        return Some(());
    };
    let callee = pattern[..open]
        .trim()
        .strip_prefix("::")
        .unwrap_or(pattern[..open].trim())
        .replace("::", ".")
        .replace("->", ".");
    let pattern_segments = callee.split('.').collect::<Vec<_>>();
    // The match is exact-children at the call node — a `?.` optional-call
    // token, a comment extra, or a `type_arguments` sibling between the
    // callee and the argument list breaks the match for LITERAL and META
    // heads alike, so refuse the candidate here, before any segment
    // binding. The `?.`- and type-args-SPELLED patterns answer their
    // aligned sites through their dedicated lanes, which never reach this
    // gate.
    if !call_junction_exact(node) {
        return None;
    }
    // Multi-segment callees bind through the FAITHFUL receiver path — when
    // the receiver chain flattens through a call, the segments are not
    // faithful node texts, so refuse instead of unifying on flattened
    // tails. The veto fires only when equality could actually be violated:
    // a wildcard receiver metavar binds the receiver's literal text, with
    // duplicate-name equality keeping every same-name veto. A literal
    // single segment is accepted only when the candidate's WHOLE callee
    // text byte-equals it (flattening `a["b"]` to `a` would wrongly
    // accept); metavariable heads keep the flattened-tail binding.
    let actual = if pattern_segments.len() >= 2 {
        match call_target_path_faithful(node, source) {
            Some(path) => path,
            None => return bind_nonfaithful_receiver(node, source, &pattern_segments, captures),
        }
    } else {
        if capture_name(pattern_segments[0]).is_none() {
            let callee_node = call_field_node(node)?;
            let callee_text = node_text(&callee_node, source)?;
            if callee_text != pattern_segments[0].trim() {
                return None;
            }
        }
        match call_target_path(node, source) {
            Some(path) => path,
            None => return Some(()),
        }
    };
    if pattern_segments.len() == actual.len() {
        for (pattern_segment, actual_segment) in pattern_segments.iter().zip(&actual) {
            if let Some(variable) = capture_name(pattern_segment) {
                bind_capture(captures, variable, actual_segment)?;
            }
        }
    } else if pattern_segments.len() == 1 {
        if let (Some(variable), Some(actual_segment)) =
            (capture_name(pattern_segments[0]), actual.last())
        {
            bind_capture(captures, variable, actual_segment)?;
        }
    } else if actual.len() > pattern_segments.len() && capture_name(pattern_segments[0]).is_some() {
        // A leading metavariable segment absorbs the WHOLE multi-segment
        // receiver — bind it so `bind_capture` enforces duplicate-name
        // equality. `$O.$O($$$A)` on `a.b.c(1)` binds O=`a.b` then O=`c` and
        // rejects, matching the reference.
        let head_len = actual.len() - pattern_segments.len() + 1;
        let head = actual[..head_len].join(".");
        bind_capture(
            captures,
            capture_name(pattern_segments[0]).unwrap_or_default(),
            &head,
        )?;
        for (pattern_segment, actual_segment) in
            pattern_segments[1..].iter().zip(&actual[head_len..])
        {
            if let Some(variable) = capture_name(pattern_segment) {
                bind_capture(captures, variable, actual_segment)?;
            }
        }
    }
    Some(())
}

/// Whole-text receiver binding for wildcard-led two-segment patterns whose
/// callee chain is NOT a plain member chain (subscript receivers `arr[0]`,
/// macro-call receivers `vec![1]`, call receivers `y.first()`). The
/// receiver metavariable binds to the receiver node's literal text and the
/// tail metavariable to the final identifier; a duplicate head/tail name
/// (`$O.$O`) then fails `bind_capture` equality, which is exactly how the
/// reference rejects the same-name chains. A literal segment must
/// byte-equal its text slice (head vs the receiver, tail vs the chain-tail
/// identifier) — the reference answers `a.b($X)` with nothing on
/// `a[0].b(1)`. Split-callee grammars (java/php `object`+`name` fields,
/// ruby receiver calls) keep the veto (`None`).
pub(crate) fn bind_nonfaithful_receiver(
    node: &Node,
    source: &str,
    pattern_segments: &[&str],
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    // The interrupted pass exempted LITERAL heads from this veto for the
    // member-call-valued faces, but no pinned face reaches this arm anymore
    // — `capture_call_path`'s `simple_call` guard skips non-call shapes
    // and the `$`-less faces never run captures at all. The registered
    // veto semantics are restored verbatim; the member-call contract lives
    // in the `simple_call` guard and is killed by its regression test.
    if pattern_segments.len() != 2 {
        return None;
    }
    let field = call_field_node(node)?;
    let (tail_node, tail_text) = chain_tail_identifier(&field, source)?;
    if tail_node.id() == field.id() {
        // A single-identifier callee always has a faithful path; reaching
        // here means the callee shape is not a member chain — keep the veto.
        return None;
    }
    let receiver = source
        .get(field.start_byte()..tail_node.start_byte())?
        .trim_end()
        .trim_end_matches('.')
        .trim_end();
    if receiver.is_empty() {
        return None;
    }
    // Optional-link shapes (`a?.b`) never reach this arm — the path
    // resolvers' `?`-kind veto refuses them before captures, so the receiver
    // slice is `?`-free by construction here. A non-metavariable segment
    // must BYTE-EQUAL the text it is accepted against (a subscript receiver
    // is not the literal identifier `a`), so a literal head vetoes the
    // candidate on receiver mismatch instead of binding through the empty
    // capture name. Symmetrically the literal tail compares against the
    // chain-tail identifier. Metavariable segments keep the whole-text
    // binding (`$O.b($Y)` answers with O = the receiver's literal text).
    match capture_name(pattern_segments[0]) {
        Some(name) => bind_capture(captures, name, receiver)?,
        None => {
            if receiver != pattern_segments[0].trim() {
                return None;
            }
        }
    }
    match capture_name(pattern_segments[1]) {
        Some(name) => bind_capture(captures, name, &tail_text)?,
        None => {
            if tail_text != pattern_segments[1].trim() {
                return None;
            }
        }
    }
    Some(())
}

/// The final identifier of a callee chain node: the node itself when it is
/// an identifier, else the last named child's own tail (member chains end in
/// the property identifier). The swift `navigation_suffix` wrapper is
/// transparent here — its named child IS the chain tail (see
/// [`faithful_path_from_node`]); `?.` sites never reach captures at all (the
/// path resolvers' `?`-kind veto fires first).
pub(crate) fn chain_tail_identifier<'a>(
    node: &Node<'a>,
    source: &str,
) -> Option<(Node<'a>, String)> {
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        let text = node_text(node, source)?.to_string();
        return Some((*node, text));
    }
    if !is_member_expr_kind(node.kind()) && !is_navigation_suffix_kind(node.kind()) {
        return None;
    }
    let mut cursor = node.walk();
    let last = node.named_children(&mut cursor).last()?;
    chain_tail_identifier(&last, source)
}

/// The swift/kotlin `navigation_suffix` member-link wrapper kind. Absent
/// from [`MEMBER_EXPR_KINDS`] (that table also drives index extraction), but
/// part of the member-chain decomposition — the two faithful resolvers above
/// read through it.
pub(crate) fn is_navigation_suffix_kind(kind: &str) -> bool {
    kind == "navigation_suffix"
}

pub(crate) fn excerpt_for_node(node: &Node, source: &str, pattern: &str) -> String {
    if let Some(text) = node_text(node, source) {
        if text.lines().count() <= 6 {
            return text.to_string();
        }
    }
    // The previous `source.lines().nth(line - 1)` fallback re-scanned the
    // whole source for every >6-line node (quadratic on the universal
    // lane). Backward/forward newline scan around start_byte yields the
    // identical first-line text in O(line) — including `lines()`'s
    // trailing-`\r` strip.
    let start = node.start_byte().min(source.len());
    if start >= source.len() && (source.is_empty() || source.ends_with('\n')) {
        // Zero-width node at EOF: `lines()` has no piece for
        // `byte_to_line(len)` here, and the old `nth(...)` fell back to the
        // pattern. (When the source does not end in `\n`, the last piece is
        // exactly the tail line — the scan below already yields it.)
        return pattern.to_string();
    }
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |i| line_start + i);
    let line = &source[line_start..line_end];
    let line = line.strip_suffix('\r').unwrap_or(line);
    line.to_string()
}
