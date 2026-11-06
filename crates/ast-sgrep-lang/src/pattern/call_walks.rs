//! Plain/member call walks and chain matching.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

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
    seen: &mut std::collections::HashSet<(usize, usize)>,
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
            None => {
                if let Some(captures) =
                    captures_for_node(&node, source, pattern, Some(&callee.join(".")))
                {
                    if seen.insert((node.start_byte(), node.end_byte())) {
                        out.push(hit_for_node(&node, source, pattern, captures));
                    }
                }
            }
            Some(slots) => {
                // The slots arm constructs captures directly —
                // `capture_call_path` (and the junction gate
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
                    if match_arg_slots(slots, &nodes, source, &mut captures, ArgSlotMode::Positional).is_some()
                        && seen.insert((node.start_byte(), node.end_byte()))
                    {
                        out.push(hit_for_node(&node, source, pattern, captures));
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls(
            child, source, pattern, path, arguments, arg_slots, lang, out, seen,
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
    seen: &mut std::collections::HashSet<(usize, usize)>,
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
                            && match_arg_slots(slots, &nodes, source, &mut captures, ArgSlotMode::Backtracking).is_some()
                            && seen.insert((node.start_byte(), node.end_byte()))
                        {
                            out.push(hit_for_node(&node, source, pattern, captures));
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
            seen,
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
    // Connector kind has already been admitted by walk_member_calls.
    // Normalize nullsafe before plain arrows so `$O?` cannot lose its
    // capture name and bypass receiver/argument equality.
    let normalized = callee
        .replace("?->", ".")
        .replace("::", ".")
        .replace("->", ".");
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
            bind_capture(&mut absorption, capture_name(pattern_segments[0])?, &head)?;
            for (want, have) in pattern_segments[1..].iter().zip(actual[head_len..].iter()) {
                if let Some(variable) = capture_name(want) {
                    bind_capture(&mut absorption, variable, have)?;
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
