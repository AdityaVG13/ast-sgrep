//! PHP RHS expression structural matcher (isomorphic split of php.rs).

use super::*;
use crate::extract::node_text;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Structural match of a general RHS expression pattern against the
/// candidate's right-hand node. A `variable_name` pattern leaf is either a
/// canonical meta (`$V`/`$$V` single namespace, `$$$V` multi — binds the
/// candidate node text, same-name veto via `bind_capture_kind`) or literal
/// variable text (byte-exact). A `$$V` dynamic-variable pattern token
/// wildcards plain AND dynamic candidate variables. Any other node requires
/// the SAME kind with aligned children: childless leaves (integers, names,
/// string content) compare text-exactly, anonymous tokens (operators,
/// punctuation) compare text-exactly, named children recurse pairwise.
/// `pat`'s text is read from `pat_source` (pattern node offsets index the
/// pattern text, not the candidate source).
pub(crate) fn php_rhs_expr_matches(
    pat: Node,
    cand: Node,
    source: &str,
    pat_source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    if pat.kind() == "variable_name" {
        if let Some(text) = node_text(&pat, pat_source) {
            if let Some(name) = capture_name(text) {
                let multi = text.starts_with("$$$");
                if let Some(cand_text) = node_text(&cand, source) {
                    return bind_capture_kind(captures, name, cand_text, multi).is_some();
                }
                return false;
            }
        }
        return match (node_text(&pat, pat_source), node_text(&cand, source)) {
            (Some(want), Some(have)) => want == have,
            _ => false,
        };
    }
    // A `$$V` dynamic-variable pattern token is a canonical wildcard over
    // BOTH variable spellings (plain and dynamic); candidate nodes of any
    // other kind refuse (fail-closed — only the probed faces admit).
    if pat.kind() == "dynamic_variable_name" {
        if let Some(text) = node_text(&pat, pat_source) {
            if let Some(name) = text.strip_prefix("$$").filter(|n| is_metavar_name(n)) {
                if matches!(cand.kind(), "variable_name" | "dynamic_variable_name") {
                    if let Some(cand_text) = node_text(&cand, source) {
                        return bind_capture_kind(captures, name, cand_text, false).is_some();
                    }
                }
                return false;
            }
        }
    }
    // An argument list carrying a `$$$NAME` rest slot is the
    // rest-metavariable contract — bind the remaining arguments' source
    // bytes (whole-list incl. empty, leading, mid, or trailing) instead
    // of the strict child zip, which cannot decompose the raw `$$$` text
    // (probed: `$b = f($v, $$$A);` answers f($v,1)/f($v,1,2)/f($v,$u)/
    // f($v,1,2,3) and refuses the zero-argument trailing face;
    // `$b = f($$$A);` answers every arity incl. empty).
    // Lists without a rest keep the strict structural path byte-compatible.
    if pat.kind() == "arguments" {
        if let Some(text) = node_text(&pat, pat_source) {
            if text.contains("$$$") {
                let (Some(open), Some(close)) = (text.find('('), text.rfind(')')) else {
                    return false;
                };
                if close <= open {
                    return false;
                }
                let inner = &text[open + 1..close];
                let Some(slots) = parse_arg_slots(inner, ArgSlotGrammar::Member) else {
                    return false;
                };
                if cand.kind() != "arguments" {
                    return false;
                }
                let mut cand_cursor = cand.walk();
                let nodes: Vec<_> = cand
                    .named_children(&mut cand_cursor)
                    .filter(|child| !is_trivia_kind(child.kind()))
                    .collect();
                return match_arg_slots(&slots, &nodes, source, captures, ArgSlotMode::Backtracking).is_some();
            }
        }
    }
    // Childless leaves are literal tokens — byte-exact text compare (the
    // zero-children recursion below would otherwise admit ANY same-kind
    // leaf, e.g. `1` matching `2`).
    if pat.child_count() == 0 {
        return match (node_text(&pat, pat_source), node_text(&cand, source)) {
            (Some(want), Some(have)) => want == have,
            _ => false,
        };
    }
    if pat.kind() != cand.kind() {
        return false;
    }
    // The reference aligns expression children with CANDIDATE comment
    // children transparent (probed: `$a = $V + 1;` answers
    // `$a = $v /* = */ + 1;` and the paren spellings), while a PATTERN-side
    // comment must find its text-exact counterpart (probed: the commented
    // paren pattern answers only the self line — a different comment text
    // does not match). Walk the pattern children against the candidate
    // stream with that asymmetry.
    let pat_children: Vec<_> = {
        let mut cursor = pat.walk();
        pat.children(&mut cursor).collect()
    };
    let cand_children: Vec<_> = {
        let mut cursor = cand.walk();
        cand.children(&mut cursor).collect()
    };
    let mut cand_index = 0usize;
    for pat_child in &pat_children {
        if is_trivia_kind(pat_child.kind()) {
            let Some(want) = node_text(pat_child, pat_source) else {
                return false;
            };
            let mut matched = false;
            while cand_index < cand_children.len() {
                let cand_child = &cand_children[cand_index];
                cand_index += 1;
                if is_trivia_kind(cand_child.kind())
                    && node_text(cand_child, source).is_some_and(|text| text == want)
                {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return false;
            }
            continue;
        }
        while cand_index < cand_children.len()
            && is_trivia_kind(cand_children[cand_index].kind())
        {
            cand_index += 1;
        }
        let Some(cand_child) = cand_children.get(cand_index) else {
            return false;
        };
        cand_index += 1;
        if pat_child.is_named() {
            if !cand_child.is_named()
                || !php_rhs_expr_matches(*pat_child, *cand_child, source, pat_source, captures)
            {
                return false;
            }
        } else if cand_child.is_named() {
            return false;
        } else {
            match (
                node_text(pat_child, pat_source),
                node_text(cand_child, source),
            ) {
                (Some(want), Some(have)) if want == have => {}
                _ => return false,
            }
        }
    }
    // Trailing candidate comments are transparent; any other leftover
    // candidate child breaks the alignment.
    while cand_index < cand_children.len() && is_trivia_kind(cand_children[cand_index].kind()) {
        cand_index += 1;
    }
    cand_index == cand_children.len()
}
