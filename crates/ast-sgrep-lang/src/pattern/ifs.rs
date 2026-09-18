//! If-statement lane.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// If-node kinds matched by `NativeKind::If` across the 15 indexed languages.
/// Modifier (`x if y` in Ruby) and ternary forms are deliberately excluded.
pub(crate) const IF_KINDS: &[&str] = &["if_statement", "if_expression", "if"];

/// Body/consequence container kinds across grammars.
pub(crate) const BLOCK_KINDS: &[&str] = &[
    "block",
    "statement_block",
    "compound_statement",
    "function_body",
    "body_statement",
    "statements",
    "then",
    "block_expression",
];

/// Wrapper kinds that never hold statements directly; descend into their
/// single block-like child before counting (Swift `function_body { statements }`).
pub(crate) const STMT_WRAPPER_KINDS: &[&str] = &["function_body", "then", "statements"];

pub(crate) fn is_trivia_kind(kind: &str) -> bool {
    kind.contains("comment")
}

pub(crate) fn walk_ifs(
    lang: Language,
    node: Node,
    source: &str,
    pattern: &str,
    cond: Option<&str>,
    body: Option<&BodyTemplate>,
    body_braced: bool,
    alternative: Option<&IfAlternative>,
    out: &mut Vec<PatternMatch>,
) {
    // Three exactness guards on the candidate if node. (1) The node must be
    // NAMED — the anonymous `if` KEYWORD TOKEN carries kind "if" in several
    // grammars and the old scan matched it as a full if-site whenever the
    // template had no body. (2) A DIRECT trivia child sitting BEFORE the
    // consequence breaks the structural match (`if (a) /*c*/ { b(); }` and
    // `if /*c*/ (a) { b(); }` answer `[]`; comments AFTER the consequence
    // — pre-`else` — and inside the condition/body are transparent). (3)
    // Brace-ness — enforced inside `if_body_matches` via `body_braced`.
    let direct_trivia_before_consequence = {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        let consequence_index = children
            .iter()
            .position(|child| Some(*child) == if_consequence(&node));
        children.iter().enumerate().any(|(index, child)| {
            (is_trivia_kind(child.kind()) || child.is_extra())
                && consequence_index.is_some_and(|ci| index < ci)
        })
    };
    // The trivia-position doctrine is GRAMMAR-SCOPED, not language-free.
    // The reference treats if-level comment trivia as TRANSPARENT on swift
    // (pre-condition, pre-`{`, body-start, one-line, call-condition) and on
    // python (the direct-child positions its grammar has — condition-
    // adjacent and body-attached comments); js/ts/c/php and kt/go stay
    // STRUCTURAL — pre-condition/pre-brace trivia refuses. Unprobed
    // grammars keep the conservative structural refusal.
    let if_trivia_structural =
        direct_trivia_before_consequence && !matches!(lang, Language::Swift | Language::Python);
    if IF_KINDS.contains(&node.kind())
        && node.is_named()
        && !is_in_comment_or_string(&node)
        && !if_trivia_structural
        && !cross_grammar_braced_if_refused(lang, node.clone(), pattern, body_braced)
        && if_body_matches(lang, &node, body, body_braced)
    {
        // The else-tail alignment gates the emission — an else-carrying
        // pattern whose candidate lacks (or mismatches) the tail emits
        // nothing. The descent below continues either way so a refused OUTER
        // candidate still yields its nested if candidates.
        if_alternative_matches(lang, &node, source, pattern, alternative, &mut |captures| {
            emit_if_match(lang, &node, source, pattern, cond, captures, out);
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ifs(
            lang,
            child,
            source,
            pattern,
            cond,
            body,
            body_braced,
            alternative,
            out,
        );
    }
}

/// The else-tail alignment for one candidate if node. `None` (else-less
/// pattern) keeps the registered behavior — the `emit` closure runs
/// unconditionally and the callee returns true. A parsed tail demands a
/// matching candidate alternative:
/// * `Else` — the candidate's `alternative` field must exist and satisfy the
///   braced/count body grammar; a `$$$`/`$B` meta body binds its text.
/// * `ElseIf` — the candidate's alternative must itself be an if node whose
///   condition and body align, then the nested tail recurses.
/// The `emit` closure runs ONCE with the extended captures when (and only
/// when) the whole tail aligned, so the else-if binding lands in one
/// match row.
pub(crate) fn if_alternative_matches(
    lang: Language,
    node: &Node,
    source: &str,
    pattern: &str,
    alternative: Option<&IfAlternative>,
    emit: &mut dyn FnMut(&mut BTreeMap<String, String>),
) -> bool {
    let Some(spec) = alternative else {
        emit(&mut BTreeMap::new());
        return true;
    };
    let Some(cand_alt) = node.child_by_field_name("alternative") else {
        return false;
    };
    let mut captures = BTreeMap::new();
    if !if_alternative_aligns(lang, spec, &cand_alt, source, pattern, &mut captures) {
        return false;
    }
    emit(&mut captures);
    true
}

/// One else-link alignment: `spec` (pattern side) against `cand` (candidate
/// side, the `alternative` node — for `ElseIf` an if node, for `Else` the
/// fallback statement). Returns false on any structural mismatch; binds the
/// tail's captures into `captures` (name conflicts veto, reference unification).
pub(crate) fn if_alternative_aligns(
    lang: Language,
    spec: &IfAlternative,
    cand: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    // js/ts/c-style grammars wrap the else tail in an `else_clause` node
    // (field `alternative` never points at the inner if/block directly);
    // the alignment grammar below speaks in terms of the tail's BODY (block
    // or nested if node), so unwrap the wrapper first. python's `elif_clause`
    // skips this shape — it IS the nested if-like node (accepted below).
    let unwrapped = {
        let mut c = *cand;
        while c.kind() == "else_clause" {
            let Some(inner) = c.named_child(0) else {
                return false;
            };
            c = inner;
        }
        c
    };
    let cand = &unwrapped;
    match spec {
        IfAlternative::Else {
            body,
            body_braced,
            body_meta,
        } => {
            if *body_braced && !consequence_is_braced(cand) {
                return false;
            }
            if !if_body_template_matches(cand, *body) {
                return false;
            }
            if let Some(name) = body_meta {
                if let Some(text) = node_text(cand, source) {
                    if bind_capture(captures, name, strip_container(text)).is_none() {
                        return false;
                    }
                }
            }
            true
        }
        IfAlternative::ElseIf {
            cond,
            body,
            body_braced,
            cond_meta,
            body_meta,
            alternative,
        } => {
            // python spells the tail `elif` as its own node kind; its
            // condition/consequence/alternative fields line up with the
            // if-node grammar this arm speaks.
            if !(IF_KINDS.contains(&cand.kind()) || cand.kind() == "elif_clause")
                || !cand.is_named()
            {
                return false;
            }
            if let Some(cond_text) = cond {
                let Some(template) = cached_if_cond_template(lang, cond_text) else {
                    return false;
                };
                let Some(template_root) = general_template_root(&template) else {
                    return false;
                };
                let Some(cand_cond) = cand.child_by_field_name("condition") else {
                    return false;
                };
                let mut cand_cond = cand_cond;
                if template_root.kind() != "parenthesized_expression" {
                    while cand_cond.kind() == "parenthesized_expression" {
                        let Some(inner) = cand_cond.named_child(0) else {
                            break;
                        };
                        cand_cond = inner;
                    }
                }
                if general_eq(&template, template_root, cand_cond, source, captures).is_none() {
                    return false;
                }
            } else if let Some(name) = cond_meta {
                if let Some(cond_node) = cand.child_by_field_name("condition") {
                    if let Some(text) = node_text(&cond_node, source) {
                        if bind_capture(captures, name, strip_container(text)).is_none() {
                            return false;
                        }
                    }
                }
            }
            let Some(consequence) = if_consequence(cand) else {
                return false;
            };
            if *body_braced && !consequence_is_braced(&consequence) {
                return false;
            }
            if !if_body_template_matches(&consequence, *body) {
                return false;
            }
            if let Some(name) = body_meta {
                if let Some(text) = node_text(&consequence, source) {
                    if bind_capture(captures, name, strip_container(text)).is_none() {
                        return false;
                    }
                }
            }
            match alternative {
                Some(nested) => match cand.child_by_field_name("alternative") {
                    Some(nested_cand) => {
                        if_alternative_aligns(lang, nested, &nested_cand, source, pattern, captures)
                    }
                    None => false,
                },
                None => true,
            }
        }
    }
}

/// Body-template count/Any check for an else-tail branch node (the
/// meta-capture binding itself lives in the callers, which know which pattern
/// section the meta came from).
pub(crate) fn if_body_template_matches(node: &Node, body: Option<BodyTemplate>) -> bool {
    match body {
        None => true,
        Some(BodyTemplate::Any) => true,
        Some(BodyTemplate::Exactly(want)) => {
            if BLOCK_KINDS.contains(&node.kind()) {
                count_statements(*node) == want
            } else {
                want == 1
            }
        }
    }
}

/// The shared if-emit path: registered-capture base (MATCH + head cond/body
/// metas via [`captures_for_node`]) plus the else-tail captures, unified
/// through [`bind_capture`] (conflicts veto the candidate), then one match
/// row.
pub(crate) fn emit_if_match(
    lang: Language,
    node: &Node,
    source: &str,
    pattern: &str,
    cond: Option<&str>,
    tail_captures: &mut BTreeMap<String, String>,
    out: &mut Vec<PatternMatch>,
) {
    match cond {
        None => {
            let Some(mut captures) = captures_for_node(node, source, pattern, Some("if")) else {
                return;
            };
            if let Some(name) = if_head_body_meta(pattern) {
                if !bind_if_head_body(&mut captures, name, node, source) {
                    return;
                }
            }
            for (name, text) in tail_captures.iter() {
                if bind_capture(&mut captures, name, text).is_none() {
                    return;
                }
            }
            push_match_with_captures(node, source, pattern, captures, out);
        }
        Some(cond) => {
            // The concrete-cond path binds cond metas first; the tail
            // captures must unify with them (a same-name conflict vetoes).
            let mut staged: Vec<PatternMatch> = Vec::new();
            push_cond_match(lang, node, source, pattern, cond, &mut staged);
            for m in staged.iter_mut() {
                if let Some(name) = if_head_body_meta(pattern) {
                    if !bind_if_head_body(&mut m.captures, name, node, source) {
                        return;
                    }
                }
                for (name, text) in tail_captures.iter() {
                    if bind_capture(&mut m.captures, name, text).is_none() {
                        return;
                    }
                }
            }
            out.extend(staged);
        }
    }
}

/// The `$NAME` of the pattern's FIRST braced section when the pattern
/// carries an `else` tail — the head body meta. `body_capture`'s
/// first-`{`-to-last-`}` slice spans the tail for these patterns and can never
/// return the bare head meta, so the emit path binds it here (the else-less
/// spelling keeps `body_capture`, whose slice is the head itself).
pub(crate) fn if_head_body_meta(pattern: &str) -> Option<&str> {
    if !pattern.contains("else") {
        return None;
    }
    let open = pattern.find('{')?;
    let close = open + pattern[open..].find('}')?;
    capture_name(pattern.get(open + 1..close)?.trim())
}

/// Binds the head body meta to the head consequence's stripped text; false on
/// a same-name conflict (the unification veto, [`bind_capture`]).
pub(crate) fn bind_if_head_body(
    captures: &mut BTreeMap<String, String>,
    name: &str,
    node: &Node,
    source: &str,
) -> bool {
    let Some(consequence) = if_consequence(node) else {
        return true;
    };
    let Some(text) = node_text(&consequence, source) else {
        return true;
    };
    bind_capture(captures, name, strip_container(text)).is_some()
}

/// The concrete-condition candidate path. The candidate's condition node
/// unwraps its anonymous `parenthesized_expression` wrapper whenever the
/// template root is NOT itself a parenthesized expression (js/ts/c/php/java
/// wrap every condition; go/py/rust candidates carry the condition directly
/// or exactly as spelled — the go paren structural doctrine stays enforced
/// by [`cross_grammar_braced_if_refused`]). `general_eq` compares the cond
/// template against the condition and binds cond metas; a same-name
/// conflict with the body/MATCH captures vetoes the candidate (unification
/// semantics, [`bind_capture`]).
pub(crate) fn push_cond_match(
    lang: Language,
    node: &Node,
    source: &str,
    pattern: &str,
    cond: &str,
    out: &mut Vec<PatternMatch>,
) {
    let Some(template) = cached_if_cond_template(lang, cond) else {
        return;
    };
    let Some(template_root) = general_template_root(&template) else {
        return;
    };
    let Some(mut candidate) = node.child_by_field_name("condition") else {
        return;
    };
    if template_root.kind() != "parenthesized_expression" {
        while candidate.kind() == "parenthesized_expression" {
            let Some(inner) = candidate.named_child(0) else {
                break;
            };
            candidate = inner;
        }
    }
    let mut cond_captures = BTreeMap::new();
    if general_eq(
        &template,
        template_root,
        candidate,
        source,
        &mut cond_captures,
    )
    .is_none()
    {
        return;
    }
    let Some(mut captures) = captures_for_node(node, source, pattern, Some("if")) else {
        return;
    };
    for (name, text) in cond_captures {
        if bind_capture(&mut captures, &name, &text).is_none() {
            return;
        }
    }
    push_match_with_captures(node, source, pattern, captures, out);
}

/// Cross-grammar STRUCTURAL alignment of the braced if template — the two
/// file-language families where the brace-ness rule alone is not exact.
/// (a) python cannot align a `{...}` if body at all (suites are
/// `:`-indented; even the literal and parenthesized-condition faces are
/// semantic-empty), so a braced pattern is empty on EVERY py candidate.
/// (b) go binds the pattern's condition parens structurally: a `($X)`
/// spelling is a parenthesized_expression node the candidate condition
/// must repeat, while the paren-free `if $X { $B }` pattern answers BOTH
/// candidate shapes. js/ts/c/php/kt are unaffected: their parens/braces
/// are anonymous delimiters the brace-ness gate already covers.
pub(crate) fn cross_grammar_braced_if_refused(
    lang: Language,
    node: Node,
    pattern: &str,
    body_braced: bool,
) -> bool {
    if !body_braced {
        return false;
    }
    if lang == Language::Python {
        return true;
    }
    if lang == Language::Go {
        let pattern_cond_parenthesized = pattern
            .trim()
            .strip_prefix("if")
            .is_some_and(|after| after.trim_start().starts_with('('));
        if pattern_cond_parenthesized
            && node
                .child_by_field_name("condition")
                .is_some_and(|cond| cond.kind() != "parenthesized_expression")
        {
            return true;
        }
    }
    false
}
