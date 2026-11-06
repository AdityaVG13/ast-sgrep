//! `->` member-call chain decomposition and matching.

use super::*;
use crate::extract::{is_ident_kind, node_text};
use std::collections::BTreeMap;
use tree_sitter::Node;

/// One decomposed candidate chain segment: its name text, the argument NODES
/// when the segment is a real call (`None` = receiver/property link — no
/// call happened, so a pattern call segment can never sit here), and the
/// argument-list content text for whole-list captures.
pub(crate) struct MemberChainSegment<'a> {
    text: String,
    args: Option<Vec<Node<'a>>>,
    args_content: Option<String>,
}

/// Exact-depth chain unification: the candidate's member-call decomposition
/// must have EXACTLY the pattern's segment count (inner prefix subnodes are
/// separate candidates the walk visits, never an absorb here), every link's
/// nullsafe flag must equal the pattern's (token exactness),
/// every segment name unifies through `bind_capture` (the same-name veto),
/// a pattern property segment must land on a property link and every
/// pattern call segment on a REAL call link with matching arity (a property
/// access in the receiver chain is not a call: `$a->b->c($u)` has no `b()`
/// link, so `$a->b()->c($A)` must not answer it, and `$a->b()->c($A)` has
/// no `->b` property link, so `$a->b->c($A)` must not answer it either),
/// and argument metavars bind like the flat lane (`$A`/`$$A` → single
/// namespace key `A`; `$$$A` → multi). A receiver (argument-free) pattern
/// segment carries no argument contract, mirroring [`chain_matches`].
pub(crate) fn member_chain_matches(
    node: &Node,
    source: &str,
    segments: &[CallChainSegment],
    nullsafe_flags: &[bool],
) -> Option<BTreeMap<String, String>> {
    let mut actual = Vec::new();
    let mut actual_flags = Vec::new();
    member_chain_segments(node, source, &mut actual, &mut actual_flags)?;
    if actual.len() != segments.len() {
        return None;
    }
    let mut captures = BTreeMap::new();
    for (index, (segment, candidate)) in segments.iter().zip(actual.iter()).enumerate() {
        if actual_flags[index] != nullsafe_flags[index] {
            return None;
        }
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want != &candidate.text => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, &candidate.text)?,
            (None, None) => return None,
        }
        if segment.args.is_none() && index > 0 && candidate.args.is_some() {
            // A pattern property link never folds onto a candidate call
            // position (the reference keeps the faces disjoint).
            return None;
        }
        if let Some(template) = &segment.args {
            // A pattern call segment requires a real call link: a property
            // access in the receiver chain is not a call (`$a->b->c($u)` has
            // no `b()` link, so `$a->b()->c($A)` must not answer it).
            let nodes = candidate.args.as_ref()?;
            let arity_ok = match template {
                ArgumentTemplate::Any => true,
                ArgumentTemplate::Exactly(expected) => nodes.len() == *expected,
            };
            if !arity_ok {
                return None;
            }
            if let Some((name, multi)) = &segment.args_capture {
                if let Some(text) = &candidate.args_content {
                    bind_capture_kind(&mut captures, name, text, *multi)?;
                }
            }
            // A `$A, $B` list binds every name to its positional candidate
            // argument — the flat lane's `capture_arguments` contract,
            // without which the codemod rewrite falsely bails "unbound
            // metavariable" on names the reference substitutes (arity above
            // already pinned the equal lengths). Mixed literal/meta lists
            // match through per-position slots instead (literals byte-exact,
            // metas bind, rest binds the remaining source bytes).
            if let Some(slots) = &segment.arg_slots {
                match_arg_slots(slots, nodes, source, &mut captures, ArgSlotMode::Backtracking)?;
            } else if let Some(names) = &segment.arg_metas {
                for (name, argument) in names.iter().zip(nodes.iter()) {
                    if let Some(text) = node_text(argument, source) {
                        bind_capture(&mut captures, name, text)?;
                    }
                }
            }
        }
    }
    Some(captures)
}

/// Recursive `->`-chain decomposition of a candidate member-call node:
/// `$repo->find($id)->hydrate($row)` is [receiver $repo, call find, call
/// hydrate]; a `member_access_expression` object contributes its property as
/// an argument-free link; a `scoped_call_expression` object contributes its
/// `scope::name` texts (the call keeping its argument list). The parallel
/// `flags` vector records, per pushed segment, whether the connector INTO it
/// is the nullsafe `?->` (the nullsafe node kinds carry the named connector
/// kind; plain links push false). Unresolvable receivers (exotic nodes) veto
/// the whole decomposition — such faces stay empty, never an over-match.
pub(crate) fn member_chain_segments<'a>(
    node: &Node<'a>,
    source: &'a str,
    segs: &mut Vec<MemberChainSegment<'a>>,
    flags: &mut Vec<bool>,
) -> Option<()> {
    match node.kind() {
        kind @ ("member_call_expression" | "nullsafe_member_call_expression") => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs, flags)?;
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            flags.push(kind == "nullsafe_member_call_expression");
            Some(())
        }
        kind @ ("member_access_expression" | "nullsafe_member_access_expression") => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs, flags)?;
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args: None,
                args_content: None,
            });
            flags.push(kind == "nullsafe_member_access_expression");
            Some(())
        }
        "scoped_call_expression" => {
            let scope = node.child_by_field_name("scope")?;
            let name = node.child_by_field_name("name")?;
            segs.push(MemberChainSegment {
                text: node_text(&scope, source)?.to_string(),
                args: None,
                args_content: None,
            });
            flags.push(false);
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            flags.push(false);
            Some(())
        }
        _ => {
            // A dynamic-variable link node (`->$$dyn`) pushes its FULL text
            // including the dollars — the classifier's property literal
            // (`$$dyn`) compares it byte-exactly.
            if is_ident_kind(node.kind())
                || KEYWORD_RECEIVER_KINDS.contains(&node.kind())
                || node.kind() == "variable_name"
                || node.kind() == "dynamic_variable_name"
            {
                segs.push(MemberChainSegment {
                    text: node_text(node, source)?.to_string(),
                    args: None,
                    args_content: None,
                });
                flags.push(false);
                Some(())
            } else {
                None
            }
        }
    }
}
