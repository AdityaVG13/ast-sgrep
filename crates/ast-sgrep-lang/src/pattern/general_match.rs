//! General structural lane: matching and walks.

use super::*;
use crate::extract::{is_inside_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Build the general-lane template for (`lang`, `pattern`) — a fresh build on
/// every call. The historical `cached_` name promised a memoization that was
/// never there; the rename keeps the contract honest (H3).
pub(crate) fn general_template_for(lang: Language, pattern: &str) -> Option<GeneralTemplate> {
    if general_lane_text_eligible_for(lang, pattern)
        && !comment_face_refused_for_lang(lang, &pattern_comment_content(pattern))
    {
        let (substituted, placeholders, multi_names) = substitute_general_metavariables(pattern)?;
        build_general_template(lang, pattern, &substituted, &placeholders, &multi_names)
    } else {
        None
    }
}

pub(crate) fn match_structural_general(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Vec<PatternMatch> {
    let Some(template) = general_template_for(lang, pattern) else {
        return Vec::new();
    };
    // The csharp meta-body lock/using faces are ACCEPTED bind-nothing faces
    // — the walk's empty is the agreement, never a census concern.
    if template.force_empty {
        return Vec::new();
    }
    let Some(tree) = parse_source_silent(lang, source) else {
        return Vec::new();
    };
    let root = general_template_root(&template);
    let mut out = Vec::new();
    if let Some(root) = root {
        // The byte-range dedup is a hash set, not a linear scan — the
        // scan made lone-metavar walks quadratic in the match count.
        // Insertion into `out` keeps emission order byte-for-byte.
        let mut seen = std::collections::HashSet::new();
        walk_general(
            tree.root_node(),
            source,
            pattern,
            &template,
            root,
            lang,
            &mut seen,
            &mut out,
        );
    }
    // The general lane is the member path the kt/swift DOTTED `$`-less
    // spellings ride (property chains `a.b.c`, plain calls `a.b(1)`) — the
    // dotted faces over-answered receiver-link trivia here because the
    // general lane had NO link-structural consult. Apply the union doctrine
    // PER-CANDIDATE (the candidate's own subtree only, so the answered
    // trivia-free inner link of a longer chain keeps answering), scoped to
    // the two grammars whose trivia doctrine is probe-verified.
    retain_member_link_trivia_free(lang, &tree, &mut out);
    // Per-candidate general-lane gates (the structural alignment skips
    // trivia/unnamed children and over-served these refusals):
    // * a cs candidate whose subtree carries an ERROR node holding a
    //   gap-junk char OUTSIDE the cs trivia class (U+2028/U+2029/U+0085)
    //   drops; in-class spellings keep binding, and the junk-transparent
    //   directive/throw/return faces ride their dedicated lanes.
    // * a cs call candidate whose callee→`(` junction carries a
    //   comment/U+2028/U+2029 run drops; FEFF/NBSP junctions bind and
    //   comment-inside-args stays outside the junction.
    // * a ts member-link candidate with a trivia child directly before the
    //   `?.` connector drops; the after-connector position keeps binding.
    if matches!(lang, Language::CSharp | Language::TypeScript | Language::Go) {
        out.retain(|hit| {
            node_with_span(tree.root_node(), hit.byte_start, hit.byte_end).is_none_or(|node| {
                match lang {
                    // The outsider-junk gate is scoped to the using-var HEAD
                    // — the parse recovers junk-ERROR inside the initializer
                    // and still binds the outer meta, so the subtree-wide
                    // scan over-refused it. The head gate now also refuses
                    // COMMENT children in the keyword→name head while
                    // comments after the name and at the initializer keep
                    // binding.
                    Language::CSharp => {
                        (node.kind() != "local_declaration_statement"
                            || cs_using_var_head_clean(&node, source))
                            && (!is_call_kind(node.kind())
                                || cs_call_junction_trivia_free(&node, source))
                    }
                    Language::TypeScript => !ts_member_link_receiver_trivia(&node),
                    // The reference refuses comments AND the junk class
                    // (U+2028/FEFF/A0) at the `for`/`go`/`defer`
                    // keyword→body junctions while clean junctions bind,
                    // argument comments bind, and `func` is NOT gated (the
                    // keyword→name refusal is per-keyword — the scope
                    // discipline).
                    Language::Go => {
                        // node_with_span can resolve to the enclosing
                        // `statement_list` sharing the hit's byte span
                        // (single-statement bodies); descend to the gated
                        // statement node before judging the junction.
                        match go_gate_target(&node) {
                            Some(n) => go_keyword_body_junction_clean(&n, source),
                            None => true,
                        }
                    }
                    _ => true,
                }
            })
        });
    }
    out
}

// Matcher-lane shape: (node/source/pattern/template/...) is threaded
// deliberately; bundling would churn every lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_general<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    template: &GeneralTemplate,
    template_root: Node<'a>,
    lang: Language,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    // The guard is ANCESTOR-only — a string-rooted template must match the
    // string node itself, while everything nested inside a comment/string
    // ancestor stays skipped (calls inside `#{…}` for other templates,
    // string_content, …).
    if node.kind() == template.root_kind && !is_inside_comment_or_string(&node) {
        // The loop-head comment discipline — position-scoped refusals on
        // trivia-carrying loop candidates (junction comments, leading
        // init/while comments, go/rs clause zones). Clean and
        // transparent-position candidates are untouched, so the unwalled
        // cases keep their bindings.
        if loop_head_trivia_structural(lang, &node) {
            return walk_children_general(
                node,
                source,
                pattern,
                template,
                template_root,
                lang,
                seen,
                out,
            );
        }
        // The exact-children junction doctrine covers the after-`>` gap of
        // type-args-spelled calls — a trivia child fully inside the
        // typeargs→arguments gap refuses the candidate (descent-only),
        // whitespace-only gaps stay admitted.
        if ts_typeargs_junction_refused(lang, template_root, &node, source) {
            return walk_children_general(
                node,
                source,
                pattern,
                template,
                template_root,
                lang,
                seen,
                out,
            );
        }
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
        }
        if general_eq(template, template_root, node, source, &mut captures).is_some() {
            let byte_start = node.start_byte();
            let byte_end = node.end_byte();
            if seen.insert((byte_start, byte_end)) {
                out.push(hit_for_node(&node, source, pattern, captures));
            }
        }
    }
    walk_children_general(
        node,
        source,
        pattern,
        template,
        template_root,
        lang,
        seen,
        out,
    );
}

/// The descend-only tail of [`walk_general`], shared by the refused-candidate
/// path (a vetoed loop candidate still descends for nested candidates).
// Matcher-lane shape: (node/source/pattern/template/...) is threaded
// deliberately; bundling would churn every lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_children_general<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    template: &GeneralTemplate,
    template_root: Node<'a>,
    lang: Language,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_general(
            child,
            source,
            pattern,
            template,
            template_root,
            lang,
            seen,
            out,
        );
    }
}

/// First-order structural comparison of a pattern subtree against a candidate
/// subtree. A pattern node whose ENTIRE text is a substituted metavariable
/// binds the whole candidate node (reference semantics — this is how a TS
/// `required_parameter` spelled `$B` binds the full `param: type` text).
pub(crate) fn general_eq<'p>(
    template: &GeneralTemplate,
    p: Node<'p>,
    c: Node<'p>,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    if let Some(text) = node_text(&p, &template.doc) {
        if let Some(name) = template.placeholders.get(text.trim()) {
            let bound = node_text(&c, source)?;
            return bind_capture(captures, name, bound);
        }
    }
    if p.kind() != c.kind() {
        return None;
    }
    // Leaf tokens (literals, operators, identifiers, punctuation) must carry
    // the exact same text on both sides — `foo($X + 1)` must not match
    // `foo(x + 2)`.
    if p.child_count() == 0 {
        let expected = node_text(&p, &template.doc)?;
        let actual = node_text(&c, source)?;
        return (expected == actual).then_some(());
    }
    let mut p_children: Vec<Node> = {
        let mut cursor = p.walk();
        p.children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            .collect()
    };
    let mut c_children: Vec<Node> = {
        let mut cursor = c.walk();
        c.children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            // Tree-sitter marks in-file parse-error children (ERROR,
            // is_extra=true) `extra`, and Smart strictness skips candidate
            // extras during child alignment. The error-glued identifiers
            // recovered as `identifier + ERROR($A) + …` therefore align on
            // the NAMED children only and the meta binds the identifier
            // FRAGMENT. Keeping the extra here cost it an alignment slot
            // (count mismatch → silently unanswered). The pattern side is
            // NOT skipped — Smart never skips goal-side nodes either.
            .filter(|child| !child.is_extra())
            .collect()
    };
    // Statement-terminator lenience. Templates parse at doc roots where
    // ASI-tolerant grammars omit the `;`, or carry it via the context
    // suffix; candidate statements spell whichever the source used. A lone
    // anonymous `;` tail difference is a termination artifact, not a
    // structural difference. The lenience is ONE-DIRECTIONAL — only the
    // CANDIDATE's terminator is an ASI artifact; a PATTERN-trailing `;`
    // stays significant, so a pattern that spells it never pops it.
    if p_children.last().is_some_and(|n| n.kind() != ";")
        && c_children.last().is_some_and(|n| n.kind() == ";")
    {
        c_children.pop();
    }
    // A LONE metavariable argument binds the whole argument list when the
    // source command carries several (`raise $A` on
    // `raise ArgumentError, 'bad'`). Only argument containers take this —
    // call patterns are served by the call lane, whose arity contract is
    // untouched.
    if p_children.len() == 1 && c.kind().contains("argument") && c_children.len() > 1 {
        if let Some(text) = node_text(&p_children[0], &template.doc) {
            if let Some(name) = template.placeholders.get(text.trim()) {
                let bound = node_text(&c, source)?;
                return bind_capture(captures, name, bound);
            }
        }
    }
    // Sole-rest argument lists. A `$$$NAME` spelled as the SOLE argument of
    // its call (the only shape [`nested_call_rest_template`] admits into
    // this lane) matches any candidate arity of the aligned arguments
    // container — empty (`fetch()`), single (`fetch(a)`), or multi
    // (`fetch(a, b)`) — binding the container's inner text in the MULTI
    // namespace exactly like [`capture_arguments`]. Probes: `g(fetch($$$A))`
    // answers all three arities and answers BOTH the outer and the inner
    // call of `fetch(g(fetch($$$A)))` faces.
    if p_children.len() == 3
        && c.kind() == p.kind()
        && p.kind().contains("argument")
        && p_children[0].kind() == "("
        && p_children[2].kind() == ")"
        && node_text(&p_children[1], &template.doc)
            .and_then(|text| template.placeholders.get(text.trim()))
            .is_some_and(|name| template.multi_names.contains(name))
    {
        let middle_text = node_text(&p_children[1], &template.doc).unwrap_or_default();
        let name = template.placeholders[middle_text.trim()].clone();
        let bound = node_text(&c, source)
            .map(|text| strip_container(text).to_string())
            .unwrap_or_default();
        return bind_capture_kind(captures, &name, &bound, true);
    }
    if p_children.len() != c_children.len() {
        return None;
    }
    for (p_child, c_child) in p_children.drain(..).zip(c_children.drain(..)) {
        general_eq(template, p_child, c_child, source, captures)?;
    }
    Some(())
}
