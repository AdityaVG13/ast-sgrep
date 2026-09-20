//! C preprocessor directive lane.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// The C/C++ conditional-compilation directives answer kind-level on the
/// directive head — the reference ignores the branch body (`#ifdef $A`
/// answers both corpora with A = the name, `#if $A` binds the whole
/// condition, `#if defined($A)` binds the metavar INSIDE the condition
/// text (A = FEATURE)). `Some(matches)` = the lane engaged; `None` = not
/// a directive template.
///
/// The condition acceptance rule is ONE consistent three-form family shared
/// with [`preproc_directive_supported`] (see [`CondBinding`]).
pub(crate) fn match_preproc_directive(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let (directive, condition) = match pattern.split_once(char::is_whitespace) {
        Some((d, rest)) => (d, rest.trim()),
        None => (pattern, ""),
    };
    let kind = match (directive, lang) {
        ("#ifdef", _) => "preproc_ifdef",
        // tree-sitter-c AND tree-sitter-cpp fold BOTH spellings into
        // `preproc_ifdef` — the grammar rule is
        // `choice(preprocessor('ifdef'), preprocessor('ifndef'))` and there
        // is no dedicated `preproc_ifndef` kind in either vendored grammar.
        // The anonymous head-token check in the walk disambiguates them.
        // (The previous c-specific `preproc_ifndef` arm named a kind the c
        // tree never produces and silently answered 0 where the reference answers.)
        ("#ifndef", _) => "preproc_ifdef",
        ("#if", _) => "preproc_if",
        // The object-like macro define (function-like spellings are
        // `preproc_function_def` and refuse at the tail parse).
        ("#define", _) => "preproc_def",
        _ => return None,
    };
    // `#define` carries a name binding plus an optional value binding; every
    // other directive keeps the single condition binding.
    let (binding, value_binding) = if directive == "#define" {
        parse_define_tail(condition)?
    } else {
        (CondBinding::parse(condition)?, None)
    };
    let tree = parse_source(lang, source).ok()?;
    let mut out = Vec::new();
    walk_preproc_directive(
        tree.root_node(),
        source,
        pattern,
        directive,
        kind,
        &binding,
        value_binding.as_ref(),
        &mut out,
    );
    Some(out)
}

/// How a directive-head condition template unifies against a source
/// condition: a LONE metavariable binds the whole head text, a metavar-free
/// text compares exactly, and a single metavariable nested in literal text
/// (`defined($A)`) binds the in-between span (A = the argument identifier).
/// Conditions carrying 2+ canonical metavariables unify STRUCTURALLY
/// through a parsed condition template — the reference binds a metavariable
/// leaf to its whole candidate subtree (`#if $A && defined($B)` binds A to
/// the entire left operand `defined(FEATURE_A)`), which text slicing cannot
/// express. Any other `$` shape (`$$$`, non-canonical) refuses the lane —
/// the registered loud contract.
pub(crate) enum CondBinding {
    Whole(String),
    Exact(String),
    Inner(String, String, String),
    Structural(GeneralTemplate),
}

impl CondBinding {
    pub(crate) fn parse(condition: &str) -> Option<CondBinding> {
        if let Some(name) = capture_name(condition) {
            return Some(CondBinding::Whole(name.to_string()));
        }
        // 2+ canonical metavariables go through the structural condition
        // template (a `#if … #endif` document parse under the C grammar —
        // cpp extends the same preproc rules, and both engines share the
        // grammars, so acceptance mirrors the reference's by construction).
        // A template that does not parse clean refuses the lane (the loud
        // contract).
        if canonical_metavariable_count(condition) >= 2 {
            return structural_condition_template(condition).map(CondBinding::Structural);
        }
        if !condition.contains('$') {
            return (!condition.is_empty()).then(|| CondBinding::Exact(condition.to_string()));
        }
        // A single canonical `$NAME` nested inside otherwise-literal text.
        // The literal prefix/suffix may contain any punctuation (`defined(`
        // does); the BOUND span's cleanliness is checked at unify time.
        let dollar = condition.find('$')?;
        let rest = &condition[dollar + 1..];
        let name_len = rest
            .bytes()
            .take_while(|&b| b == b'_' || b.is_ascii_alphanumeric())
            .count();
        let name = &rest[..name_len];
        if !is_metavar_name(name) || rest[name_len..].contains('$') {
            return None;
        }
        Some(CondBinding::Inner(
            name.to_string(),
            condition[..dollar].to_string(),
            rest[name_len..].to_string(),
        ))
    }

    /// Unify the binding against the directive head's text (and node, for
    /// the structural arm).
    pub(crate) fn unify(
        &self,
        head_text: &str,
        head_node: Option<Node>,
        source: &str,
        captures: &mut BTreeMap<String, String>,
    ) -> bool {
        match self {
            CondBinding::Whole(name) => bind_capture(captures, name, head_text).is_some(),
            CondBinding::Exact(want) => head_text == want,
            CondBinding::Inner(name, prefix, suffix) => {
                let Some(middle) = head_text
                    .strip_prefix(prefix.as_str())
                    .and_then(|rest| rest.strip_suffix(suffix.as_str()))
                else {
                    return false;
                };
                let binds_clean = !middle.is_empty()
                    && !middle.contains(['(', ')'])
                    && !middle.chars().any(char::is_whitespace);
                binds_clean && bind_capture(captures, name, middle).is_some()
            }
            CondBinding::Structural(template) => {
                let Some(head) = head_node else {
                    return false;
                };
                // The condition template node sits at
                // program → preproc_if → first named child (the `#if` token
                // and the newline are anonymous).
                let Some(cond) = template
                    .tree
                    .root_node()
                    .named_child(0)
                    .and_then(|directive| directive.named_child(0))
                else {
                    return false;
                };
                general_eq(template, cond, head, source, captures).is_some()
            }
        }
    }
}

/// The number of canonical single-`$` metavariables in a condition text
/// (`$$$` runs and non-canonical names do not count).
pub(crate) fn canonical_metavariable_count(condition: &str) -> usize {
    let bytes = condition.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut dollars = 0usize;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        let name_start = i + dollars;
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
        {
            name_end += 1;
        }
        if dollars == 1 && is_metavar_name(&condition[name_start..name_end]) {
            count += 1;
        }
        i = name_end.max(i + 1);
    }
    count
}

/// Parse the substituted condition inside a COMPLETE `#if … #endif`
/// directive document (the condition must sit on its own line — the preproc
/// grammar demands the newline). The template root is the `preproc_if` node;
/// its first named child is the condition that [`general_eq`] unifies
/// against a candidate condition node.
pub(crate) fn structural_condition_template(condition: &str) -> Option<GeneralTemplate> {
    let (substituted, placeholders, multi_names) = substitute_general_metavariables(condition)?;
    let doc = format!("#if {substituted}\n#endif\n");
    let tree = parse_source(Language::C, &doc).ok()?;
    let root = tree.root_node();
    if root.has_error() || root.named_child(0).is_none() {
        return None;
    }
    Some(GeneralTemplate {
        doc,
        tree,
        placeholders,
        multi_names,
        span: None,
        had_semi: false,
        root_kind: String::new(),
        force_empty: false,
    })
}

/// The `#define` tail — a name binding plus an optional value binding
/// (`#define $A`, `#define $A $B`, `#define NAME`, `#define NAME $V`).
/// The name must be a plain identifier or metavariable (function-like
/// spellings are `preproc_function_def`, a different kind, and refuse
/// here); the value must be a lone metavariable or metavar-free text
/// (`#define $A $B` binds B = the value text and answers only
/// value-carrying defines; `#define $A` answers both shapes).
pub(crate) fn parse_define_tail(tail: &str) -> Option<(CondBinding, Option<CondBinding>)> {
    let mut parts = tail.split_whitespace();
    let name_tok = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    if rest.len() > 1 {
        return None;
    }
    if name_tok.contains('(') || name_tok.contains(')') {
        return None;
    }
    let name_binding = match capture_name(name_tok) {
        Some(name) => CondBinding::Whole(name.to_string()),
        None if is_pattern_ident(name_tok) => CondBinding::Exact(name_tok.to_string()),
        None => return None,
    };
    let value_binding = match rest.first().copied() {
        None => None,
        Some(tok) => match capture_name(tok) {
            Some(name) => Some(CondBinding::Whole(name.to_string())),
            None if !tok.contains('$') => Some(CondBinding::Exact(tok.to_string())),
            None => return None,
        },
    };
    Some((name_binding, value_binding))
}

/// A spelled `#define` value binding demands the value child (the second
/// named child of `preproc_def`) and unifies against it; with no value
/// spelled the directive answers value-carrying defines too (the head-level
/// rule).
pub(crate) fn define_value_matches(
    value: Option<&CondBinding>,
    node: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    let Some(binding) = value else {
        return true;
    };
    let Some(value_node) = node.named_child(1) else {
        return false;
    };
    node_text(&value_node, source)
        .is_some_and(|text| binding.unify(text, Some(value_node), source, captures))
}

// Matcher-lane shape: (node/source/pattern/...) is threaded
// deliberately; bundling would churn every lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_preproc_directive(
    node: Node,
    source: &str,
    pattern: &str,
    directive: &str,
    kind: &str,
    binding: &CondBinding,
    value_binding: Option<&CondBinding>,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == kind && !is_in_comment_or_string(&node) {
        // The directive keyword itself must agree — some grammars fold the
        // whole conditional family into shared region kinds, and only the
        // leading anonymous token (`#ifdef` vs `#ifndef`) tells them apart.
        let directive_token_ok = node
            .child(0)
            .is_some_and(|token| !token.is_named() && token.kind() == directive);
        // The directive head is the FIRST named child: the name identifier
        // for ifdef/ifndef, the condition expression for `#if`, the macro
        // name for `#define`. Branch-body children are ignored — exactly
        // the reference's directive-head semantics.
        if directive_token_ok {
            if let Some(head) = node.named_child(0) {
                let mut head_captures = BTreeMap::new();
                let unified = node_text(&head, source).is_some_and(|text| {
                    binding.unify(text, Some(head), source, &mut head_captures)
                });
                // A spelled `#define` value must exist and unify; an unspelled
                // value ignores the source value.
                if unified && define_value_matches(value_binding, &node, source, &mut head_captures)
                {
                    let mut captures = BTreeMap::new();
                    if let Some(text) = node_text(&node, source) {
                        captures.insert("MATCH".to_string(), text.to_string());
                    }
                    captures.extend(head_captures);
                    out.push(hit_for_node(&node, source, pattern, captures));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_preproc_directive(
            child,
            source,
            pattern,
            directive,
            kind,
            binding,
            value_binding,
            out,
        );
    }
}

/// Answerability of the conditional-directive family — `Some(true)` when
/// the dedicated lane answers the pattern (the shared [`CondBinding`]
/// rule: lone metavariable, metavar-free text, a single metavariable
/// nested in literal condition text, or a parseable structural condition
/// template over 2+ canonical metavariables), `Some(false)`/`None`
/// keeping the registered loud contracts. `#define` joins through
/// [`parse_define_tail`].
pub(crate) fn preproc_directive_supported(pattern: &str) -> Option<bool> {
    let (directive, condition) = pattern.trim().split_once(char::is_whitespace)?;
    match directive {
        "#ifdef" | "#ifndef" | "#if" => Some(CondBinding::parse(condition.trim()).is_some()),
        "#define" => Some(parse_define_tail(condition).is_some()),
        _ => None,
    }
}
