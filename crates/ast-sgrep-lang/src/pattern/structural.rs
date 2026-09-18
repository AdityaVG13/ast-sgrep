//! Query-driven structural matching and call walks.

use super::*;
use crate::extract::{byte_to_line, is_in_comment_or_string, node_text};
use crate::pattern_queries::{class_queries_for, queries_for, FUNCTION_QUERY_TABLE};
use crate::Language;
use std::collections::BTreeMap;
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
        NativeKind::Function { name, body } => {
            run_queries(
                &language,
                lang,
                tree.root_node(),
                source,
                queries_for(FUNCTION_QUERY_TABLE, lang),
                declaration_modifiers,
                name.as_deref(),
                None,
                body.as_ref(),
                arguments.as_ref(),
                pattern,
                &mut out,
            )?;
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
                &mut out,
            )?;
        }
        NativeKind::Call { path, arg_slots } => {
            walk_calls(
                tree.root_node(),
                source,
                pattern,
                path,
                arguments.as_ref(),
                arg_slots.as_deref(),
                lang,
                &mut out,
            );
        }
        NativeKind::MemberCall {
            path,
            arg_slots,
            require_continuation,
            nullsafe,
        } => {
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
                &optional_flags,
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

/// `$$NAME` / `$$_` universal matching — visit EVERY node (anonymous
/// tokens included, no comment/string skip: pinned comment, docstring, and
/// string rows) and push each via the shared byte-range dedup.
/// A named metavariable binds the node text through `bind_capture`, so the
/// reserved `MATCH` key keeps its overwrite semantics.
pub(crate) fn walk_universal(
    node: Node,
    source: &str,
    pattern: &str,
    name: Option<&str>,
    out: &mut Vec<PatternMatch>,
) {
    // Hash-set dedup — the previous linear scan over `out` made the
    // universal walk quadratic. The set only gates the push; emission order
    // is unchanged.
    let mut seen = std::collections::HashSet::new();
    // The dedup alone left the walk quadratic — `node_lines` re-counts `\n`
    // from byte 0 twice per node, and the excerpt fallback re-scanned the
    // whole source per long node. Pre-order start bytes are strictly
    // increasing, so a forward (byte,line) cursor amortizes the
    // start-line computation to O(source) across the whole walk; the end
    // line is a local scan over the node's own span. Both produce exactly
    // the values `node_lines` would.
    let mut cursor = (0usize, 1u32);
    walk_universal_inner(node, source, pattern, name, &mut seen, &mut cursor, out);
}

/// Forward line cursor: `line_at` is the 1-based line number of `byte_at`.
/// Only valid while bytes are visited in non-decreasing order (pre-order).
pub(crate) type LineCursor = (usize, u32);

pub(crate) fn walk_universal_inner(
    node: Node,
    source: &str,
    pattern: &str,
    name: Option<&str>,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    line_cursor: &mut LineCursor,
    out: &mut Vec<PatternMatch>,
) {
    let byte_start = node.start_byte();
    let byte_end = node.end_byte();
    // Advance the monotonic cursor to this node's start (pre-order guarantees
    // byte_start >= cursor byte).
    let from = line_cursor.0.min(byte_start);
    let newlines = source.as_bytes()[from..byte_start]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32;
    let line_start = if byte_start >= line_cursor.0 {
        line_cursor.1 + newlines
    } else {
        byte_to_line(source, byte_start)
    };
    line_cursor.0 = byte_start;
    line_cursor.1 = line_start;
    // End line: newlines inside the node's own span (local, O(span)).
    let line_end = line_start
        + source.as_bytes()[byte_start..byte_end.min(source.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32;
    if seen.insert((byte_start, byte_end)) {
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
            if let Some(name) = name {
                bind_capture(&mut captures, name, text);
            }
        }
        out.push(PatternMatch {
            line_start,
            line_end,
            byte_start,
            byte_end,
            excerpt: excerpt_for_node(&node, source, pattern),
            captures,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_universal_inner(child, source, pattern, name, seen, line_cursor, out);
    }
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
            for cap in m.captures {
                if name_idx == Some(cap.index) {
                    name_text = node_text(&cap.node, source);
                }
                if match_idx == Some(cap.index) {
                    match_node = Some(cap.node);
                }
            }
            let node = match_node.or_else(|| m.captures.first().map(|c| c.node));
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
                if matches!(keyword, "interface")
                    || (matches!(keyword, "class")
                        && lang == Language::Java
                        && body_filter.is_some())
                {
                    if interface_candidate_refused(lang, &node, source, declaration_modifiers) {
                        continue;
                    }
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
            // A native function template never carries a return-type
            // section (those shapes are beyond the native subset and fail
            // closed), so per reference strictness a matched declaration
            // must not declare one either. Class templates are
            // shape-agnostic.
            if class_filter.is_none() && !function_return_type_absent(&node) {
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
            kind if kind.contains("comment") => {
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_calls(
    node: Node,
    source: &str,
    pattern: &str,
    path: &[Option<String>],
    arguments: Option<&ArgumentTemplate>,
    arg_slots: Option<&[ArgSlot]>,
    lang: Language,
    out: &mut Vec<PatternMatch>,
) {
    // The php member-call kinds are the dedicated MemberCall/OptionalCall
    // lanes' candidates — a plain path (dot- or name-spelled) never
    // answers a `->`/`?->` call site (connector token-exactness). When a
    // rest slot owns the argument list, the slots decide the arity — the
    // text-derived Exactly(n) vetoed rest-expanded candidates (3-arg
    // sites) in the pre-filter BEFORE the slots ran.
    let arguments = match arg_slots {
        Some(slots) if slots.iter().any(|slot| matches!(slot, ArgSlot::Rest(_))) => None,
        _ => arguments,
    };
    let callee = if matches!(
        node.kind(),
        "member_call_expression" | "nullsafe_member_call_expression"
    ) {
        None
    } else {
        call_match_path(&node, source, path)
            // The pattern's paren tokens must exist on the candidate — a
            // paren-less command call is a different node shape the reference
            // never matches with a paren-spelled pattern.
            .filter(|_| candidate_call_parenthesized(&node))
            .filter(|_| arguments_match(&node, arguments, &["arguments"]))
            // Swift's member-link trivia rule is PER-CANDIDATE — a callee
            // link whose OWN text carries a comment before a later
            // `navigation_suffix` is link-STRUCTURAL and the reference
            // refuses that candidate (`a /*c*/ .b(1)` × `a.b($X)`). A
            // trivia-free INNER link of a longer chain still answers
            // (`a.b(1) /*c*/ .c(2)` × `a.b($X)`) — the earlier
            // ancestor-chain form over-refused those and was removed.
            // Callee-internal comments keep the transparency contract.
            .filter(|_| {
                !(lang == Language::Swift
                    && call_field_node(&node)
                        .is_some_and(|callee| swift_member_link_structural(&callee)))
            })
            // The kt DOTTED call spelling (`a.b($X)`, `$A.b($X)`, `$`-less
            // `a.b(1)`) rides this plain-call lane, which had NO
            // link-structural consult — receiver-link trivia (`a /*c*/ .b(1)`)
            // over-answered where the reference refuses. Consult the union
            // doctrine (kt `?.` faces keep their decomposer/optional-lane
            // consults; dotted links need the wrapped rule).
            .filter(|_| {
                !(lang == Language::Kotlin
                    && call_field_node(&node)
                        .is_some_and(|callee| member_link_trivia_structural(&callee)))
            })
            // The cs junction gate — the reference refuses a comment/U+2028/
            // U+2029 run in the callee→`(` gap while the FEFF/NBSP junctions
            // bind and comment-inside-args stays outside the junction.
            .filter(|_| !(lang == Language::CSharp && !cs_call_junction_trivia_free(&node, source)))
            // Swift's arithmetic-binary rows never present a matchable call.
            .filter(|_| !swift_compound_callee_call(lang, &node))
    };
    if let Some(callee) = callee {
        match arg_slots {
            None => push_match(&node, source, pattern, Some(&callee.join(".")), out),
            Some(slots) => {
                // The slots arm answers through `push_match_with_captures`
                // DIRECTLY — `capture_call_path` (and the junction gate
                // inside it) never runs, so consult the same rule here
                // before answering. Mixed rest-slot patterns classify native
                // and the arity admits junction-extra candidates
                // (`a?.(1, 2)` under `a($A, $$$B)`; `a?.(1)` / `a /*c*/ (1)`
                // under `a($$$A, $B)`; ts `a<number>(...)` twins — all
                // reference-[]). The arity semantics themselves are
                // untouched: this only vetoes the junction-extra candidates.
                if let Some(nodes) =
                    argument_nodes(&node, &["arguments"]).filter(|_| call_junction_exact(&node))
                {
                    let mut captures = BTreeMap::new();
                    if let Some(text) = node_text(&node, source) {
                        captures.insert("MATCH".to_string(), text.to_string());
                    }
                    if call_arg_slots_match(slots, &nodes, source, &mut captures).is_some() {
                        push_match_with_captures(&node, source, pattern, captures, out);
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls(
            child, source, pattern, path, arguments, arg_slots, lang, out,
        );
    }
}

/// True when a swift call candidate's CALLEE is a folded compound. This
/// grammar folds `LHS <binop> q` into the compound and hangs the
/// `call_suffix` off the WHOLE call — there is NO standalone inner call
/// node. The gate keys on the folded shape (a direct compound-callee child
/// kind), never on operator text. `try q(1)` wraps a real `call_expression`
/// and keeps answering; a call on the LEFT operand is the matchable call.
/// A compound callee can never be spelled by a plain identifier path, so
/// the candidate is refused outright.
pub(crate) fn swift_compound_callee_call(lang: Language, node: &Node) -> bool {
    if lang != Language::Swift || node.kind() != "call_expression" {
        return false;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    children.iter().any(|child| {
        matches!(
            child.kind(),
            "additive_expression" | "multiplicative_expression" | "prefix_expression"
        )
    })
}

/// Match the php plain `->` member-call spelling against
/// `member_call_expression` candidates only. A pattern that SPELLS the
/// nullsafe `?->` connector switches the candidates to
/// `nullsafe_member_call_expression` — token-exact in both directions (a
/// plain spelling never answers a nullsafe site and vice versa); shapes
/// the carve refuses keep the dedicated optional lane. The candidate callee
/// decomposes into the full object->name chain; the empty synthetic shape
/// vetoes unresolvable receivers like the reference's empty answers. When
/// the pattern's argument list mixes literal tokens with metas, the
/// per-position slots decide the argument contract and the captures (the
/// generic pattern-text capture path would mis-bind a meta across a literal
/// position).
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_member_calls(
    node: Node,
    source: &str,
    pattern: &str,
    path: &[Option<String>],
    arguments: Option<&ArgumentTemplate>,
    arg_slots: Option<&[ArgSlot]>,
    require_continuation: bool,
    nullsafe: bool,
    out: &mut Vec<PatternMatch>,
) {
    // When a rest slot owns the argument list, the slots decide the
    // arity — the text-derived Exactly(n) vetoed rest-expanded
    // candidates (3-arg sites) in the pre-filter BEFORE the slots ran,
    // while the chained spelling (segment.args = Any) answered the same
    // sources.
    let arguments = match arg_slots {
        Some(slots) if slots.iter().any(|slot| matches!(slot, ArgSlot::Rest(_))) => None,
        _ => arguments,
    };
    // A `;`-terminated flat pattern is the reference's statement-level
    // spelling — it binds statement-rooted member calls only (probed:
    // `$w->q9(1, $$$A);` answers {2,3,8} on the flat/chain fixture,
    // excluding the chained lines' inner q9 subnodes), while the `;`-less
    // spelling answers embedded faces too ({2,3,5,8}). The same statement
    // discipline the assignment lane registered.
    let pattern_is_semi = pattern.trim().ends_with(';');
    // The dangling-arrow repair answers only chain-PREFIX sites — the
    // repaired pattern never answers the standalone statement spelling, so
    // the candidate member-call node must be continued by a
    // member-access/call link.
    let continuation_ok = !require_continuation
        || node.parent().is_some_and(|parent| {
            matches!(
                parent.kind(),
                "member_access_expression"
                    | "member_call_expression"
                    | "nullsafe_member_access_expression"
                    | "nullsafe_member_call_expression"
            )
        });
    let statement_ok = !pattern_is_semi
        || node
            .parent()
            .is_some_and(|parent| parent.kind() == "expression_statement");
    // The candidate kind carries the PATTERN's connector spelling —
    // token-exact in both directions.
    let kind_ok = if nullsafe {
        node.kind() == "nullsafe_member_call_expression"
    } else {
        node.kind() == "member_call_expression"
    };
    if kind_ok && continuation_ok && statement_ok && !is_in_comment_or_string(&node) {
        let matched = call_callee(&node, source)
            .filter(|(segments, _)| !segments.is_empty())
            .filter(|(segments, _)| path_matches(segments, path))
            .filter(|_| arguments_match(&node, arguments, &["arguments"]));
        if matched.is_some() {
            match arg_slots {
                None => push_match(&node, source, pattern, None, out),
                Some(slots) => {
                    if let Some(nodes) = argument_nodes(&node, &["arguments"]) {
                        let mut captures = BTreeMap::new();
                        if let Some(text) = node_text(&node, source) {
                            captures.insert("MATCH".to_string(), text.to_string());
                        }
                        // The slot faces build captures HERE (the generic push
                        // path never runs for them), so a callee-path
                        // metavariable stayed unbound where the reference
                        // binds it per site (`C::$s->$M();` binds M=`m`).
                        // Bind the EQUAL-LENGTH callee path segment-wise; a
                        // conflict is the unification veto.
                        if bind_slot_face_callee_metas(&node, source, pattern, &mut captures)
                            .is_some()
                            && arg_slots_match(slots, &nodes, source, &mut captures).is_some()
                        {
                            push_match_with_captures(&node, source, pattern, captures, out);
                        }
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_member_calls(
            child,
            source,
            pattern,
            path,
            arguments,
            arg_slots,
            require_continuation,
            nullsafe,
            out,
        );
    }
}

/// Callee-path metavariable binding for the SLOT faces of the flat
/// member-call lane. The slot arms build their capture map in-walk (the
/// generic capture path never runs for them), so meta segments bind here —
/// the same unification [`capture_call_path`] applies elsewhere. The
/// nullsafe `?->` spelling normalizes FIRST so a meta receiver binds
/// instead of being skipped. On length mismatch with a leading pattern
/// meta, bind the absorbed head the way the absorption arm does, then align
/// tails segment-wise; a `bind_capture` conflict vetoes. Mismatch without
/// a leading meta never reaches here; the arm keeps the no-binding return
/// for that unreachable shape.
pub(crate) fn bind_slot_face_callee_metas(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let open = pattern.find('(')?;
    let callee = pattern[..open].trim();
    // Normalization: `::` and `->` both spell one dotted segment chain,
    // mirroring `parse_call_path`'s decomposition. The nullsafe `?->` is
    // deliberately NOT normalized (both cross directions refuse — the
    // token-exact doctrine); a `?` here would glue onto the receiver
    // segment and refuse, which IS the reference's behavior for the cross
    // spellings.
    let normalized = callee.replace("::", ".").replace("->", ".");
    if normalized.is_empty() {
        return Some(());
    }
    let pattern_segments: Vec<&str> = normalized.split('.').collect();
    let (actual, _) = call_callee(node, source)?;
    if actual.len() != pattern_segments.len() {
        // Leading-meta absorption — bind the pattern's first segment to the
        // whole candidate head, then align the tails. This is the php flat
        // member-call slot lane, so the head text carries the `->`
        // connector the capture reports (`$A->b($X);` × `$x->y->b(1);` →
        // A=`$x->y`; the dot-joined `capture_call_path` arm serves the
        // DOT-connector grammars where the dedicated php lane is never
        // consulted).
        if actual.len() > pattern_segments.len() && capture_name(pattern_segments[0]).is_some() {
            let head_len = actual.len() - (pattern_segments.len() - 1);
            let head = actual[..head_len].join("->");
            let mut absorption = captures.clone();
            if bind_capture(&mut absorption, capture_name(pattern_segments[0])?, &head).is_none() {
                return None;
            }
            for (want, have) in pattern_segments[1..].iter().zip(actual[head_len..].iter()) {
                if let Some(variable) = capture_name(want) {
                    if bind_capture(&mut absorption, variable, have).is_none() {
                        return None;
                    }
                }
            }
            *captures = absorption;
        }
        return Some(());
    }
    for (want, have) in pattern_segments.iter().zip(actual.iter()) {
        if let Some(variable) = capture_name(want) {
            bind_capture(captures, variable, have)?;
        }
    }
    Some(())
}

/// Match php `->` member-call CHAIN templates against php member-call
/// candidates. Every chain node in the tree is visited, so a pattern answers
/// the outermost chain whose per-segment shape matches exactly AND the inner
/// prefix subnodes whose own depth equals the pattern's. Both member-call
/// spellings are candidates — the candidate node kind carries the LAST
/// link's connector and the decomposition carries every link's, so the
/// per-link flag comparison keeps `->` and `?->` token-exact in both
/// directions. When the PATTERN ends in a property link, the member-access
/// spellings are candidates too (a call-tail pattern still never matches
/// one — the exact-depth unification vetoes the args mismatch).
pub(crate) fn walk_member_call_chains(
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    nullsafe_flags: &[bool],
    out: &mut Vec<PatternMatch>,
) {
    let pattern_ends_property = segments.len() >= 2
        && segments
            .last()
            .is_some_and(|segment| segment.args.is_none());
    // A `;`-terminated FLAT PROPERTY pattern is the statement-rooted
    // spelling — it answers the property-access statement, never an
    // embedded access node (`$this->$P;` answers the `$this->prop;`
    // statement but not the `$this->prop = 1;` assignment's inner
    // `$this->prop`). Registered chain faces keep the embedded answering
    // contract: property-TAIL faces with a call segment and every
    // semi-less spelling.
    let pattern_is_semi = pattern.trim().ends_with(';');
    let flat_property = segments.len() == 2 && segments.iter().all(|s| s.args.is_none());
    let statement_ok = !(pattern_is_semi && flat_property)
        || node
            .parent()
            .is_some_and(|parent| parent.kind() == "expression_statement");
    let candidate_kind = node.kind();
    let is_candidate = if pattern_ends_property {
        matches!(
            candidate_kind,
            "member_access_expression"
                | "nullsafe_member_access_expression"
                | "member_call_expression"
                | "nullsafe_member_call_expression"
        )
    } else {
        matches!(
            candidate_kind,
            "member_call_expression" | "nullsafe_member_call_expression"
        )
    };
    if is_candidate && statement_ok && !is_in_comment_or_string(&node) {
        if let Some(captures) = member_chain_matches(&node, source, segments, nullsafe_flags) {
            let mut captures = captures;
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            out.push(hit_for_node(&node, source, pattern, captures));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_member_call_chains(child, source, pattern, segments, nullsafe_flags, out);
    }
}
