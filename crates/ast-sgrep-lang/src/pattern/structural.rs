//! Query-driven structural matching and call walks.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::pattern_queries::{
    bodyless_function_queries_for, class_queries_for, name_only_function_queries_for, queries_for,
    return_function_queries_for, FUNCTION_QUERY_TABLE,
};
use crate::Language;
use tree_sitter::{Node, QueryCursor, StreamingIterator};

pub(crate) fn match_structural(
    lang: Language,
    source: &str,
    pattern: &str,
    kind: &NativeKind,
) -> anyhow::Result<Vec<PatternMatch>> {
    let language = tree_sitter_language(lang);
    let tree = parse_source(lang, source)?;
    let mut out = Vec::new();
    let arguments = argument_template(pattern);
    let (_, declaration_modifiers) = strip_declaration_modifiers(pattern);
    match kind {
        NativeKind::Function {
            name,
            body,
            return_spec,
            semicolon,
        } => {
            // br-hs1: spelled return sections and `;`-completed forms are
            // receipted to rust/go/ts/php/py (probed 2026-10-27, ast-grep
            // 0.45.2). Unreceipted grammars stay silent here — the
            // answerability gate keeps their census loud (same defense as
            // the interface member-count lane).
            let return_receipted = matches!(
                lang,
                Language::Rust
                    | Language::Go
                    | Language::TypeScript
                    | Language::Php
                    | Language::Python
            );
            if !return_receipted && (return_spec.is_some() || *semicolon) {
                return Ok(out);
            }
            // `;`-completed forms: rust keeps grammar-specific strictness
            // for the NO-RETURN form (return must be ABSENT; probed); with
            // a spelled return the `;` is a no-op terminator (re-probed on
            // the main fixture 2026-10-27: `-> u32;` answers the `-> u32`
            // sigs — an earlier fixture-specific empty was a misread); go,
            // ts and php answer empty outright.
            if *semicolon && lang != Language::Rust {
                return Ok(out);
            }
            // br-osk: NO-PARENS forms (argument list absent). A pure name
            // (`fn $A`) answers every fn-shaped node of the grammar —
            // rust: sigs ∪ items, generics wildcarded; any spelled
            // section OR body template on a no-parens form answers EMPTY
            // (concrete, metavar, `;`, `{...}`/`:` bodies), except go's
            // bare metavar result which the reference reads as no
            // section AND BINDS it to the parameter-list text (probed
            // 2026-10-27, 0.45.2, 4 stable rounds/cell).
            if arguments.is_none() {
                let go_bare_metavar = matches!(return_spec, Some(ReturnSpec::Metavar(_)))
                    && lang == Language::Go
                    && body.is_none();
                if (return_spec.is_some() || *semicolon || body.is_some()) && !go_bare_metavar {
                    return Ok(out);
                }
                if body.is_none() {
                    run_queries(
                        &language,
                        lang,
                        tree.root_node(),
                        source,
                        name_only_function_queries_for(lang),
                        declaration_modifiers,
                        name.as_deref(),
                        None,
                        None,
                        None,
                        pattern,
                        /* bodyless_lane: */ false,
                        &ReturnLane::Wildcard,
                        &mut out,
                    )?;
                    // The lane runs several queries; hits collect in query
                    // order. Restore document order (byte_start) — the
                    // reference answers in document order.
                    out.sort_by_key(|m| m.byte_start);
                    // go bare metavar: the reference binds the metavar to
                    // the parameter-list text, parens included (`(a int)`,
                    // `()` for the empty list).
                    if let Some(ReturnSpec::Metavar(mv)) = return_spec.as_ref() {
                        let root = tree.root_node();
                        for hit in out.iter_mut() {
                            if hit.captures.contains_key(mv) {
                                continue;
                            }
                            let params = root
                                .descendant_for_byte_range(hit.byte_start, hit.byte_end)
                                .and_then(|n| n.child_by_field_name("parameters"))
                                .and_then(|p| node_text(&p, source));
                            if let Some(text) = params {
                                hit.captures.insert(mv.clone(), text.to_string());
                            }
                        }
                    }
                    return Ok(out);
                }
            }
            let return_lane = return_lane_of(return_spec.as_ref(), *semicolon);
            // br-f4w reference-parity lane: a BODY-LESS pattern (argument
            // list present, no `{...}` section) diverts to the reference's
            // per-grammar recovery kinds for the receipted grammars, with
            // the return-type gate OFF — the reference wildcards a
            // pattern-absent return on those candidate kinds (ast-grep
            // 0.45.2 + 0.51.0 probes, 2026-10-27). Patterns without an
            // argument list, body-having patterns, and every other
            // language keep the legacy path unchanged.
            let bodyless = bodyless_function_queries_for(lang, return_spec.is_some())
                .filter(|_| body.is_none() && arguments.is_some());
            if let Some(queries) = bodyless {
                run_queries(
                    &language,
                    lang,
                    tree.root_node(),
                    source,
                    queries,
                    declaration_modifiers,
                    name.as_deref(),
                    None,
                    None,
                    arguments.as_ref(),
                    pattern,
                    /* bodyless_lane: */ true,
                    &return_lane,
                    &mut out,
                )?;
            } else {
                // ts/php BODY templates with a spelled return answer free
                // functions only — the legacy table's method rows never
                // answer (probed on class-method fixtures, br-hs1).
                let queries = if return_spec.is_some()
                    && matches!(lang, Language::TypeScript | Language::Php)
                {
                    return_function_queries_for(lang)
                } else {
                    queries_for(FUNCTION_QUERY_TABLE, lang)
                };
                run_queries(
                    &language,
                    lang,
                    tree.root_node(),
                    source,
                    queries,
                    declaration_modifiers,
                    name.as_deref(),
                    None,
                    body.as_ref(),
                    arguments.as_ref(),
                    pattern,
                    /* bodyless_lane: */ false,
                    &return_lane,
                    &mut out,
                )?;
            }
        }
        NativeKind::Class {
            keyword,
            name,
            body,
        } => {
            // The interface member-count template rides the existing
            // body-filter machinery — the ts/java `body` field of an
            // interface_declaration IS the interface_body, whose named
            // non-trivia children are the members. Scoped to the receipted
            // grammars AND the interface keyword; the walk stays
            // silent-empty for other languages as a defense. The refusal
            // doctrine on this lane is CANDIDATE-scoped — extends/base
            // clauses, non-empty modifier lists, and trivia in the name→body
            // gap refuse — consulted per candidate inside [`run_queries`].
            // The ts `type-alias` object face rides the same machinery with
            // the type_alias_declaration's `value` object-type as the body.
            let body_filter = if matches!(*keyword, "interface" | "type-alias" | "class")
                && match (*keyword, lang) {
                    ("interface", Language::TypeScript | Language::Java | Language::CSharp) => true,
                    ("type-alias", Language::TypeScript) => true,
                    // The java class member-count face — binds single-member
                    // java classes, refuses empty/multi-member/extends.
                    ("class", Language::Java) => true,
                    _ => false,
                } {
                body.as_ref()
            } else {
                None
            };
            run_queries(
                &language,
                lang,
                tree.root_node(),
                source,
                class_queries_for(lang, keyword),
                declaration_modifiers,
                name.as_deref(),
                Some((lang, *keyword)),
                body_filter,
                None,
                pattern,
                /* bodyless_lane: */ false,
                &ReturnLane::unspecified(),
                &mut out,
            )?;
        }
        NativeKind::Call { path, arg_slots } => {
            // This lane owns a fresh output; index exact admitted ranges
            // without changing traversal order or the first captured winner.
            let mut seen = std::collections::HashSet::new();
            walk_calls(
                tree.root_node(),
                source,
                pattern,
                path,
                arguments.as_ref(),
                arg_slots.as_deref(),
                lang,
                &mut out,
                &mut seen,
            );
        }
        NativeKind::MemberCall {
            path,
            arg_slots,
            require_continuation,
            nullsafe,
        } => {
            // The slots option is fixed for this entire fresh-output walk;
            // only the slots emitter uses the range index. None stays unchanged.
            let mut seen = std::collections::HashSet::new();
            walk_member_calls(
                tree.root_node(),
                source,
                pattern,
                path,
                arguments.as_ref(),
                arg_slots.as_deref(),
                *require_continuation,
                *nullsafe,
                &mut out,
                &mut seen,
            );
        }
        NativeKind::MemberCallChain {
            segments,
            nullsafe_flags,
        } => {
            walk_member_call_chains(
                tree.root_node(),
                source,
                pattern,
                segments,
                nullsafe_flags,
                &mut out,
            );
        }
        NativeKind::CallChain { segments } => {
            walk_call_chains(lang, tree.root_node(), source, pattern, segments, &mut out);
        }
        NativeKind::OptionalCallChain {
            segments,
            optional_flags,
        } => {
            // A property-segment `?.` chain outside the optional machinery's
            // registered grammar family keeps the general structural lane
            // (byte-identical walk) — the leaf-decomposition contract refuses
            // kotlin/swift/rust connector shapes, and silently emptying their
            // answering clean faces is not an option. Inside the family
            // (ts/js) the gated `walk_optional_call_chains` is the whole fix.
            // The preserved general arm is no longer UNGATED — kotlin/swift
            // junction-comment and mid-chain/receiver trivia faces
            // over-answered where the reference refuses. The gated wrapper
            // below keeps every clean face's answer and refuses exactly the
            // commented faces, with the ONE junction rule (every consumed
            // call level) plus the member-link trivia veto applied to the
            // CANDIDATE nodes the general lane matched.
            let property_face = segments
                .iter()
                .skip(1)
                .any(|segment| segment.args.is_none() && segment.arg_slots.is_none());
            if property_face && !matches!(lang, Language::TypeScript | Language::JavaScript) {
                return Ok(match_structural_chain_gated(lang, source, pattern));
            }
            walk_optional_call_chains(
                lang,
                tree.root_node(),
                source,
                pattern,
                segments,
                optional_flags,
                &mut out,
            );
        }
        NativeKind::OptionalCall {
            head_capture,
            head_literal,
            tail_capture,
            tail_literal,
        } => {
            walk_optional_calls(
                lang,
                tree.root_node(),
                source,
                pattern,
                head_capture.as_deref(),
                head_literal.as_deref(),
                tail_capture.as_deref(),
                tail_literal.as_deref(),
                arguments.as_ref(),
                &mut out,
            );
        }
        NativeKind::If {
            cond,
            body,
            body_braced,
            alternative,
        } => {
            walk_ifs(
                lang,
                tree.root_node(),
                source,
                pattern,
                cond.as_deref(),
                body.as_ref(),
                *body_braced,
                alternative.as_ref(),
                &mut out,
            );
        }
        NativeKind::Assignment {
            target,
            op,
            op_class,
            rhs_expr,
            value,
            value_multi,
        } => {
            // The general RHS pattern tree must outlive the walk (classified
            // faces always parse — `validate_php_rhs_expr` — so a None here
            // only means the bare-meta path). The rhs PATTERN text rides
            // along: pattern node offsets index the pattern source, not the
            // candidate.
            let rhs_tree = rhs_expr.as_ref().and_then(|rhs| parse_php_rhs_tree(rhs));
            // The pattern source is the template DOC (`<?php $V + 1;`) —
            // pattern node offsets are doc-relative.
            let rhs_root = rhs_tree
                .as_ref()
                .map(|parsed| (parsed.tree.root_node(), parsed.doc.as_str()));
            // A META assignment target (`$X`, `$o->$A`) parses its own php doc
            // so the walker binds it structurally (the whole-meta spelling
            // binds the candidate LHS text; a meta link binds the link node
            // text) instead of the verbatim LHS text equality literal targets
            // use. So does a STATIC target carrying a single-canonical-meta
            // subscript index — and so does EVERY other static target, since
            // matching is layout-insensitive: the byte compare is replaced
            // by structural LHS unification for the whole literal-static
            // family. Dynamic-class heads stay on the walker's dedicated
            // scope-text branch below (their binding is the whole candidate
            // scope TEXT, not a node unification).
            let lhs_tree = if php_assignment_target_is_meta(target)
                || php_static_target_has_meta_index(target)
                || (is_php_static_scope_target(target)
                    && php_static_target_dynamic_head(target).is_none())
            {
                parse_php_rhs_tree(target)
            } else {
                None
            };
            let lhs_root = lhs_tree
                .as_ref()
                .map(|parsed| (parsed.tree.root_node(), parsed.doc.as_str()));
            // The pattern's RHS-head comment sequence, derived once
            // for the whole walk.
            let head_comments = php_assignment_head_comments(pattern, target, op);
            walk_php_assignments(
                tree.root_node(),
                source,
                pattern,
                target,
                op,
                *op_class,
                rhs_root,
                lhs_root,
                value,
                *value_multi,
                &head_comments,
                &mut out,
            );
        }
        // NeverMatches is valid ingress with zero candidates — ok:true
        // empty, exactly the reference's accepted-empty faces.
        NativeKind::NeverMatches => return Ok(Vec::new()),
        NativeKind::Universal { name } => {
            walk_universal(tree.root_node(), source, pattern, name.as_deref(), &mut out);
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_queries(
    language: &tree_sitter::Language,
    lang: Language,
    root: Node,
    source: &str,
    queries: &'static [&'static str],
    declaration_modifiers: Option<&str>,
    name_filter: Option<&str>,
    class_filter: Option<(Language, &str)>,
    body_filter: Option<&BodyTemplate>,
    argument_filter: Option<&ArgumentTemplate>,
    pattern: &str,
    // True only for the br-f4w body-less reference-parity lane: the return
    // type is wildcarded (the reference ignores a pattern-absent return on
    // recovery candidate kinds — rust signature items, go declarations) and
    // generic candidates (type_parameters present, rust/go) are refused
    // because the native subset cannot spell them.
    bodyless_lane: bool,
    return_lane: &ReturnLane,
    out: &mut Vec<PatternMatch>,
) -> anyhow::Result<()> {
    for qsrc in queries {
        let Some(query) = compiled_query(language, lang, qsrc) else {
            continue;
        };
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let name_idx = query.capture_index_for_name("name");
        let match_idx = query.capture_index_for_name("match");
        while let Some(m) = matches.next() {
            let mut name_text: Option<&str> = None;
            let mut match_node: Option<Node> = None;
            for cap in m.captures() {
                if name_idx == Some(cap.index) {
                    name_text = node_text(&cap.node, source);
                }
                if match_idx == Some(cap.index) {
                    match_node = Some(cap.node);
                }
            }
            let node = match_node.or_else(|| m.captures().first().map(|c| c.node));
            let Some(node) = node else {
                continue;
            };
            if is_in_comment_or_string(&node) {
                continue;
            }
            // For JAVA INTERFACE candidates the prefix demand is delegated to
            // [`interface_candidate_refused`]'s subsequence+annotation arm —
            // non-prefix candidates bind, which this leading-text gate would
            // refuse before the comparative arm ran. JAVA CLASS member-count
            // candidates join too — pattern keywords hop over candidate
            // modifier keywords on classes as well. The class extension is
            // scoped to the member-count faces (body_filter.is_some()), so
            // every pre-existing class face keeps its exact gate.
            // Every other lane keeps the strict prefix gate.
            let java_interface_comparative =
                matches!(class_filter, Some((Language::Java, "interface")))
                    && declaration_modifiers.is_some()
                    || matches!(class_filter, Some((Language::Java, "class")))
                        && declaration_modifiers.is_some()
                        && body_filter.is_some();
            if !java_interface_comparative
                && !declaration_modifiers_match(&node, source, declaration_modifiers)
            {
                continue;
            }
            if let Some((lang, keyword)) = class_filter {
                if !class_keyword_matches(lang, &node, source, keyword) {
                    continue;
                }
                // Per-candidate refusals on the interface member-count lane:
                // extends/base heritage clauses, non-empty modifier lists,
                // and trivia inside the name→body gap all refuse; trivia
                // BEFORE the name keeps answering. The modifier refusal is
                // PATTERN-COMPARATIVE, not blanket — a modifier-MATCHING
                // candidate binds. [`interface_candidate_refused`] receives
                // the pattern-side modifier text and adjudicates per
                // grammar. Java class candidates consult the same scan on
                // the member-count faces.
                if (matches!(keyword, "interface")
                    || (matches!(keyword, "class")
                        && lang == Language::Java
                        && body_filter.is_some()))
                    && interface_candidate_refused(lang, &node, source, declaration_modifiers)
                {
                    continue;
                }
            }
            if let Some(want) = name_filter {
                if name_text != Some(want) {
                    continue;
                }
            }
            if let Some(template) = body_filter {
                if !function_body_matches(&node, template) {
                    continue;
                }
            }
            // Return agreement between the template's return section and
            // the candidate (br-hs1): a section-less pattern requires an
            // absent return on the body lane (reference strictness), a
            // spelled metavariable requires presence (and binds), a
            // spelled concrete text requires equality. The body-less lane
            // keeps the br-f4w wildcard for section-less patterns and the
            // go metavar wildcard. Class templates are shape-agnostic (no
            // return fields on class nodes — vacuously matches).
            if class_filter.is_none()
                && !return_section_matches(lang, &node, source, return_lane, bodyless_lane)
            {
                continue;
            }
            // br-f4w: the reference refuses generic candidates for
            // body-less patterns — the native subset cannot spell type
            // parameters, so a spelled-out generic never aligns (probed:
            // warp body.rs decode sig, go `[T any]`, 0.45.2 + 0.51.0).
            if bodyless_lane
                && matches!(lang, Language::Rust | Language::Go)
                && node.child_by_field_name("type_parameters").is_some()
            {
                continue;
            }
            if !arguments_match(&node, argument_filter, &["parameters"]) {
                continue;
            }
            push_match(&node, source, pattern, name_text, out);
        }
    }
    Ok(())
}

pub(crate) fn declaration_modifiers_match(
    node: &Node,
    source: &str,
    modifiers: Option<&str>,
) -> bool {
    let Some(modifiers) = modifiers else {
        return true;
    };
    [Some(*node), node.parent()]
        .into_iter()
        .flatten()
        .filter_map(|candidate| node_text(&candidate, source))
        .any(|text| {
            text.trim_start()
                .strip_prefix(modifiers)
                .and_then(|rest| rest.chars().next())
                .is_some_and(char::is_whitespace)
        })
}

pub(crate) fn class_keyword_matches(
    lang: Language,
    node: &Node,
    source: &str,
    keyword: &str,
) -> bool {
    if lang != Language::Kotlin {
        return true;
    }
    let kind = kotlin_class_keyword(node, source);
    match keyword {
        "class" => kind == "class",
        "interface" => kind == "interface",
        "type" => true,
        _ => false,
    }
}

/// Whether a member-count-lane interface/class candidate refuses. A
/// heritage clause or a comment fully inside the name→body gap always
/// refuses; trivia before the name keeps answering. Modifier matching is
/// sequence-comparative: a modifier-free pattern refuses any modified
/// candidate; C# requires the exact modifier sequence; Java matches
/// keywords as an order-preserving subsequence (trivia-insensitive) while
/// annotations are positional, never skippable, and block keyword passage.
/// Callers outside TS/Java/C# take the conservative posture: any candidate
/// modifier refuses.
pub(crate) fn interface_candidate_refused(
    lang: Language,
    node: &Node,
    source: &str,
    pattern_modifiers: Option<&str>,
) -> bool {
    let receipted = matches!(
        lang,
        Language::TypeScript | Language::Java | Language::CSharp
    );
    let name = node.child_by_field_name("name");
    let body = node.child_by_field_name("body");
    let mut cursor = node.walk();
    let mut candidate_modifiers: Vec<&str> = Vec::new();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Heritage kinds refuse: `superclass`/`interfaces` appear only
            // on java class declarations (java interfaces carry
            // `extends_interfaces`).
            "extends_type_clause"
            | "extends_interfaces"
            | "base_list"
            | "superclass"
            | "interfaces" => return true,
            // java wraps its modifier list in ONE `modifiers` node; csharp
            // spells each modifier as its own leaf `modifier` child
            // (tree-sitter-c-sharp has no wrapper — node-types:
            // interface_declaration children carry `modifier` singles).
            "modifiers" => {
                let mut modifier_cursor = child.walk();
                for modifier in child.children(&mut modifier_cursor) {
                    if let Some(text) = node_text(&modifier, source) {
                        candidate_modifiers.push(text.trim());
                    }
                }
            }
            "modifier" => {
                if let Some(text) = node_text(&child, source) {
                    candidate_modifiers.push(text.trim());
                }
            }
            kind if is_trivia_kind(kind) => {
                if let (Some(name), Some(body)) = (name, body) {
                    if child.start_byte() >= name.end_byte()
                        && child.end_byte() <= body.start_byte()
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    let modifier_sequence_refuses = |wanted: &str| -> bool {
        let want: Vec<&str> = wanted.split_whitespace().collect();
        // Uncovered languages (reachable through the library `match_pattern`
        // entry, which has no answerability gate) keep the blanket posture.
        if !receipted {
            return !candidate_modifiers.is_empty();
        }
        match lang {
            Language::Java => {
                // The pattern's modifier+annotation tokens match the
                // candidate's combined token list under a greedy scan where
                // KEYWORDS may hop over non-matching candidate KEYWORDS
                // (unanchored subsequence: `public` binds `abstract public`;
                // `public abstract` refuses `abstract public` by order)
                // while ANNOTATIONS are POSITIONAL and never skippable:
                // they must sit exactly at the scan cursor. Keyword passage
                // is BLOCKED by a sitting annotation. An exhausted
                // candidate list refuses.
                if want.is_empty() {
                    return !candidate_modifiers.is_empty();
                }
                let is_anno = |token: &str| token.starts_with('@');
                let mut cursor = 0usize;
                for token in &want {
                    if is_anno(token) {
                        match candidate_modifiers.get(cursor) {
                            Some(cand) if *cand == *token => cursor += 1,
                            _ => return true,
                        }
                    } else {
                        loop {
                            match candidate_modifiers.get(cursor) {
                                None => return true,
                                Some(cand) if is_anno(cand) => return true,
                                Some(cand) if *cand == *token => {
                                    cursor += 1;
                                    break;
                                }
                                Some(_) => cursor += 1,
                            }
                        }
                    }
                }
                false
            }
            Language::CSharp =>
            // csharp leaf modifiers must byte-equal the pattern sequence
            // exactly (`public` refuses `public sealed`).
            {
                candidate_modifiers != want
            }
            // TypeScript keeps the blanket posture byte-for-byte: refuse
            // any candidate carrying modifier children, regardless of the
            // pattern side — the ts `export interface` face binds through
            // declaration_modifiers_match's wrapper-parent arm, never
            // through this scan (the exact-sequence arm must not leak
            // onto ts).
            _ => !candidate_modifiers.is_empty(),
        }
    };
    match pattern_modifiers {
        None => !candidate_modifiers.is_empty(),
        Some(wanted) => modifier_sequence_refuses(wanted),
    }
}

/// The kotlin enum/interface/class head sniffer: the declaration keyword is
/// read from the candidate's modifier wrapper (`modifiers`/`class_modifier`
/// children recurse — kotlin folds the `enum`/`interface` head tokens in
/// there, so a plain first-child byte-read of the declaration head misses
/// them); anything else defaults to `"class"`. The JAVA interface/class
/// modifier-matching doctrine this function was historically mislabeled
/// with (the 137A-F5 subsequence note) lives on
/// [`interface_candidate_refused`]'s Java arm.
pub(crate) fn kotlin_class_keyword(node: &Node, source: &str) -> &'static str {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "modifiers" | "class_modifier")
            && kotlin_class_keyword(&child, source) == "enum"
        {
            return "enum";
        }
        if let Some(text) = node_text(&child, source) {
            match text.trim() {
                "enum" => return "enum",
                "interface" => return "interface",
                _ => {}
            }
        }
    }
    "class"
}

pub(crate) fn arguments_match(
    node: &Node,
    template: Option<&ArgumentTemplate>,
    fields: &[&str],
) -> bool {
    let Some(template) = template else {
        return true;
    };
    let count = argument_nodes(node, fields).map_or(0, |arguments| arguments.len());
    match template {
        ArgumentTemplate::Any => true,
        ArgumentTemplate::Exactly(expected) => count == *expected,
    }
}

pub(crate) fn argument_container<'a>(node: &Node<'a>, fields: &[&str]) -> Option<Node<'a>> {
    if let Some(container) = fields
        .iter()
        .find_map(|field| node.child_by_field_name(field))
    {
        return Some(container);
    }
    // Fieldless grammars (MoonBit apply/dot-apply calls) expose the container
    // as a direct named child whose kind equals the field name.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if fields.contains(&child.kind()) {
            return Some(child);
        }
    }
    let mut cursor = node.walk();
    let named: Vec<Node<'a>> = node.named_children(&mut cursor).collect();
    // Swift/Kotlin declarations carry no `parameters` field; their
    // parameter list is a plain named child. Accept it directly so arity
    // counting stays honest instead of descending into a leaf parameter
    // (which silently made every arity look satisfiable).
    if let Some(list) = named.iter().find(|child| {
        let kind = child.kind();
        kind.contains("parameter") || kind.contains("argument")
    }) {
        return Some(*list);
    }
    for child in &named {
        if BLOCK_KINDS.contains(&child.kind()) {
            continue;
        }
        if let Some(container) = argument_container(child, fields) {
            return Some(container);
        }
    }
    None
}

pub(crate) fn argument_nodes<'a>(node: &Node<'a>, fields: &[&str]) -> Option<Vec<Node<'a>>> {
    let container = argument_container(node, fields)?;
    let mut cursor = container.walk();
    Some(
        container
            .named_children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            // Tree-sitter recovers an error-glued argument as
            // `identifier` + an ERROR child carrying the tail, the same
            // extra-marked recovery the alignment filter models. Smart
            // strictness skips candidate extras in ARGUMENT position too,
            // so the row presents ONE argument — the identifier fragment
            // the meta binds. Counting the extra as an arity slot made
            // `q(µA_)` a 2-against-1 mismatch and the row went silently
            // unanswered.
            .filter(|child| !child.is_extra())
            .collect(),
    )
}

/// True when the candidate call presents its argument list INSIDE a `(`/`)`
/// token pair. Call-lane patterns always spell the parens ([`classify_native`]
/// requires them) and the child alignment matches those anonymous tokens
/// one-for-one — ruby's paren-less command call (`q x`) carries a bare
/// argument list with no paren tokens, so the `(`-vs-argument_list kind
/// mismatch answers [] there. The tokens may live inside the argument
/// container (py/go/rust/swift/kotlin/ruby grammars nest them) or as direct
/// call children (grammars that split them); a container with neither shape
/// is a command call.
pub(crate) fn candidate_call_parenthesized(node: &Node) -> bool {
    let Some(container) = argument_container(node, &["arguments"]) else {
        return false;
    };
    let mut cursor = container.walk();
    let container_children: Vec<Node> = container.children(&mut cursor).collect();
    if container_children
        .first()
        .is_some_and(|child| child.kind() == "(")
        && container_children
            .last()
            .is_some_and(|child| child.kind() == ")")
    {
        return true;
    }
    let mut cursor = node.walk();
    let node_children: Vec<Node> = node.children(&mut cursor).collect();
    node_children.iter().any(|child| child.kind() == "(")
        && node_children.iter().any(|child| child.kind() == ")")
}
