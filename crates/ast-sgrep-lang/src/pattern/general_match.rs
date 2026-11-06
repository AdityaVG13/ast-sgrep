//! General structural lane: matching and walks.

use super::*;
use crate::extract::{is_inside_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Language-free: does ANY grammar build a general-lane template?
pub(crate) fn general_lane_supported(pattern: &str) -> bool {
    // The conditional-directive family is natively answerable when the
    // condition parses under the shared [`CondBinding`] rule — lone
    // metavariable, metavar-free text, a single metavariable nested in the
    // condition (`#if defined($A)` ANSWERS like the reference; the preproc
    // lane binds the nested metavar), or a parseable structural condition
    // template over 2+ canonical metavariables. `#define` joins through
    // [`parse_define_tail`]; non-directive patterns fall through untouched.
    if let Some(supported) = preproc_directive_supported(pattern) {
        return supported;
    }
    // The csharp statement-head lane is a language-free "some lane serves
    // this" admission for the ingress gate — the general template cannot root
    // these heads, so the any-language build loop below would report
    // unsupported and `needs_ast_grep_fallback` would refuse faces the
    // reference answers. The lane parse is spelling-level only; the walk +
    // census keep per-file/per-language honesty.
    if matches!(
        pattern.split_whitespace().next(),
        Some("fixed" | "checked" | "unchecked" | "unsafe" | "lock" | "using")
    ) && csharp_statement_template(pattern).is_some()
    {
        return true;
    }
    // The java class member-count lane is a language-free "some lane serves
    // this" admission for the ingress gate — the general template cannot
    // substitute a bare-meta class body (the starvation that kept the face
    // census-loud), so the fallback gate refuses faces the reference
    // answers. The lane parse is spelling-level only; the walk + census
    // keep per-file/per-language honesty.
    if classify_java_class_member_count(pattern).is_some() {
        return true;
    }
    // The directive roots, the remaining statement roots, the java
    // synchronized NESTED/METHOD faces, the php literal-name/`$$`-body
    // namespace faces, the csharp checked/unchecked expression root, and
    // go's `$`-carrying `;`-ful spellings — the language-free "some lane
    // serves this" admissions. The lane parses are spelling-level only;
    // the walk + census keep per-file/per-language honesty.
    if directive_template(pattern).is_some()
        || cs_expression_template(pattern).is_some()
        || kt_typealias_template(pattern).is_some()
        || kt_for_template(pattern).is_some()
        || swift_for_template(pattern).is_some()
        || rs_let_else_template(pattern).is_some()
        || c_goto_template(pattern).is_some()
        || ja_sync_block_nested_template(pattern).is_some()
        || ja_sync_method_template(pattern).is_some()
        || php_namespace_block_template(pattern).is_some()
        || sg_goto_semi_pattern(pattern)
        || sg_go_import_semi_pattern(pattern)
        // The del lane's `$$$`-slot faces ride the dedicated delete template
        // — the general lane's build refuses the multi-meta spelling and
        // would keep the answering face ingress-loud. The census arm stays
        // py-scoped, so other languages keep their loud class.
        || py_delete_meta_pattern(pattern)
    {
        return true;
    }
    if !general_lane_text_eligible(pattern) {
        return false;
    }
    let Some((substituted, placeholders, multi_names)) = substitute_general_metavariables(pattern)
    else {
        return false;
    };
    // Identifier soup (`$A $B`, `RETURN $A`) is not a structural template.
    // Registered statement-head templates (`return $A`) carry no
    // structural punctuation but are still structural — the statement-root
    // template lane decides them; uppercase/unregistered heads never reach
    // here (the bare-keyword guard refuses them first). The head check no
    // longer demands a `$` — the probed `$`-less faces
    // (`break`/`continue`) are statement templates too.
    let statement_head_template = pattern
        .split_whitespace()
        .next()
        .is_some_and(|head| STATEMENT_HEAD_KEYWORDS.contains(&head));
    // A whole-content string meta (`"$A"` — the parsed string-content leaf
    // IS the metavariable) carries no punctuation, but is a structural
    // template whose build the any-language loop below verifies (and whose
    // per-language root-kind admission + whole-content placeholder rule keep
    // `"pre-$A"`/`"$A$B"`/escaped spellings refused).
    let bare_string_meta_template = substituted.len() >= 2
        && substituted.starts_with('"')
        && substituted.ends_with('"')
        && placeholders.len() == 1
        && placeholders.contains_key(&substituted[1..substituted.len() - 1]);
    // A KEYWORD-OPERATOR template (`$X and $Y`, `$X or $Y`, `$X as $Y`,
    // `not $X`, `typeof $X`) carries no punctuation from the structural
    // set — the operator is an alphabetic keyword — but is a structural
    // template whose per-language build the any-language loop below
    // verifies (the root kinds land on the existing
    // `_operator`/`binary`/`_expression` admissions; the `not`/`typeof`
    // heads ride the head admissions above). The unary KEYWORD faces
    // (`delete $X`, `void $X`, `del $X`) join the same lane; their root
    // kinds are admitted in `is_general_root_kind`.
    let keyword_operator_template = !bare_string_meta_template
        && !placeholders.is_empty()
        && pattern.split_whitespace().any(|token| {
            matches!(
                token,
                "and" | "or" | "as" | "not" | "typeof" | "delete" | "void" | "del"
            )
        });
    // The ruby MODIFIER statement template (`x if $C`) carries no
    // structural punctuation but is structural — the rb modifier root kinds
    // are admitted in `is_general_root_kind` and the any-language loop below
    // verifies the build.
    let rb_modifier_template = !placeholders.is_empty() && {
        let tokens: Vec<&str> = pattern.split_whitespace().collect();
        tokens.len() >= 3
            && matches!(
                tokens[tokens.len() - 2],
                "if" | "unless" | "while" | "until"
            )
    };
    if !substituted.contains(|c: char| "()[]{}<>=:,.!?+-*/%&|;~^@#".contains(c))
        && !statement_head_template
        && !bare_string_meta_template
        && !keyword_operator_template
        && !rb_modifier_template
    {
        return false;
    }
    // A `#` whose tail carries NO code structure is a comment-glue tail
    // (`foo($A) # note`, `$A + $A# note`, the python `# tail` ctrl faces)
    // — the registered contract keeps those fail-closed at the
    // language-free ingress even though some grammars happily parse the `#`
    // tail as syntax. The `#`-as-SYNTAX faces (`#[derive($A)]`,
    // `#include $X`, `this.#x = $V`, `tag(r#"$A"#)`) all carry code
    // punctuation after their first `#` and stay templatable.
    if contains_comment_glued_hash(pattern) {
        return false;
    }
    // `#` is comment syntax only in python/ruby/php — the any-language
    // support answer must not count a hash-comment language templating a
    // pattern whose `#` is real syntax elsewhere (and vice versa: rust
    // attribute faces stay supported while python hash-glued faces keep
    // failing closed).
    Language::all().iter().any(|&lang| {
        !lane_comment_refused(lang, pattern)
            && build_general_template(lang, pattern, &substituted, &placeholders, &multi_names)
                .is_some()
    })
}

/// Build the general-lane template for (`lang`, `pattern`) — a fresh build on
/// every call. The historical `cached_` name promised a memoization that was
/// never there; the rename keeps the contract honest (H3).
pub(crate) fn general_template_for(lang: Language, pattern: &str) -> Option<GeneralTemplate> {
    if general_lane_text_eligible_for(lang, pattern) && !lane_comment_refused(lang, pattern) {
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
