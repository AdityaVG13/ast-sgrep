//! PHP bare-operand template lane (isomorphic split of php.rs).

use super::*;
use crate::extract::node_text;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// A classified bare php binary-expression META template — the parsed
/// `<?php {core};` doc plus whether the raw pattern spelled a trailing `;`
/// (the reference roots that spelling at the statement).
pub(crate) struct PhpOperandTemplate {
    parsed: std::sync::Arc<LiteralTemplate>,
    had_semi: bool,
}

impl PhpOperandTemplate {
    /// The comparison root inside the doc: `program` → `expression_statement`
    /// → the binary/paren expression (no-semi), or the statement itself
    /// (semi — the reference's statement-rooted `;` discipline).
    pub(crate) fn comparison_root(&self) -> Option<Node<'_>> {
        php_operand_comparison_root(&self.parsed, self.had_semi)
    }
}

/// Admission of a bare php binary-expression META template — no `=`
/// assignment root, no literal LHS: `$A && $B`, `($A && $B)`, `$A && $B;`.
/// Every `$`-token must be canonical; the `;`-stripped doc's comparison
/// root must be a `binary_expression` or `parenthesized_expression`
/// (no-semi: kind-exact bind) or an `expression_statement` wrapping a binary
/// (semi: only bare `expr;` statements answer). Rootless NON-binary faces
/// (`$A = $B`, `$A;`) are NOT admitted — they keep their loud riders. The
/// sibling assignment lane's [`validate_no_meta_target`] veto runs over the
/// parsed doc here too: an operand template carrying an embedded
/// assignment/augmented-assignment with a META target is refused the lane.
pub(crate) fn classify_php_operand_template(pattern: &str) -> Option<PhpOperandTemplate> {
    let p = pattern.trim();
    let had_semi = p.ends_with(';');
    let core = p.strip_suffix(';').unwrap_or(p).trim();
    if !core.contains('$')
        || pattern_has_noncanonical_metavar(core)
        || php_operand_has_two_dollar_token(core)
        || php_operand_has_literal_operand(core)
    {
        return None;
    }
    let parsed = parse_php_rhs_tree(core)?;
    php_operand_comparison_root(&parsed, had_semi)?;
    validate_no_meta_target(parsed.tree.root_node(), &parsed.doc)?;
    Some(PhpOperandTemplate { parsed, had_semi })
}

/// Scope guard: a LITERAL leaf operand anywhere in the pattern (an
/// identifier/number/quoted byte outside a `$`-token). The operand family is
/// all-meta; literal-operand faces keep their registered classes.
pub(crate) fn php_operand_has_literal_operand(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'$' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'(' | b')' | b';' | b'+' | b'-' | b'*' | b'/'
            | b'%' | b'=' | b'<' | b'>' | b'!' | b'?' | b'.' | b'|' | b'&' | b':' => i += 1,
            _ => return true,
        }
    }
    false
}

/// Scope guard: a `$$NAME` dynamic-variable token. The registered `$$`
/// wildcard binds plain AND dynamic VARIABLE candidates only
/// ([`php_rhs_expr_matches`]), while the reference answers the universal
/// spelling over every operand kind (probed `$$A && $B` == the `$A && $B`
/// set, integers included) — the lane must not serve it with narrower
/// semantics, so the face keeps its registered loud rider.
pub(crate) fn php_operand_has_two_dollar_token(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let mut run = 0usize;
            while i < bytes.len() && bytes[i] == b'$' {
                run += 1;
                i += 1;
            }
            let name_len = bytes[i..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            if run == 2 && name_len > 0 {
                return true;
            }
            i += name_len;
        } else {
            i += 1;
        }
    }
    false
}

/// Named children for the operand-template descent: the php tag and
/// comment trivia never shape the single-child chain (a pattern-side
/// comment stays INSIDE the comparison root, where
/// [`php_rhs_expr_matches`] text-matches it).
pub(crate) fn php_operand_children<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() != "php_tag" && !is_trivia_kind(child.kind()))
        .collect()
}

pub(crate) fn php_operand_comparison_root<'a>(
    parsed: &'a LiteralTemplate,
    had_semi: bool,
) -> Option<Node<'a>> {
    let mut node = parsed.tree.root_node();
    // `program`: descend through the single named child once the php tag
    // and trivia are filtered (the `unwrap_pattern_expression` discipline).
    while node.kind() == "program" {
        let named = php_operand_children(node);
        match named.as_slice() {
            [only] => node = *only,
            _ => return None,
        }
    }
    if node.kind() != "expression_statement" {
        return None;
    }
    let named = php_operand_children(node);
    match named.as_slice() {
        [only] => {
            if had_semi {
                // The reference roots the `;`-terminated pattern at the
                // statement; the statement's single child must still be the
                // binary shape.
                (only.kind() == "binary_expression").then_some(node)
            } else if matches!(
                only.kind(),
                "binary_expression" | "parenthesized_expression"
            ) {
                Some(*only)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Walk a classified bare php binary-expression META template
/// ([`classify_php_operand_template`]). Candidates are nodes of the
/// pattern's OWN comparison root kind — the binary/paren expression
/// (no-semi: the reference binds at expression level, nested binaries
/// included) or the wrapping `expression_statement` (semi: the
/// reference's statement-rooted `;` discipline, so only bare `expr;`
/// statements answer) — unified through [`php_rhs_expr_matches`] (the
/// expression machinery: metavariable binds with the same-name veto,
/// text-exact leaves/tokens, transparent
/// candidate comments, text-exact pattern comments).
pub(crate) fn walk_php_operand_template(
    tree: &tree_sitter::Tree,
    source: &str,
    pattern: &str,
    tpl: &PhpOperandTemplate,
    out: &mut Vec<PatternMatch>,
) {
    let Some(pat_root) = tpl.comparison_root() else {
        return;
    };
    let doc = tpl.parsed.doc.as_str();
    // P7-style hash-set dedup: nested self-similar binaries can emit the
    // same node twice through distinct binds; insertion order is preserved.
    // The set lives inside the shared walk (one set per top-level call).
    walk_php_operand_node(tree.root_node(), source, pattern, pat_root, doc, out);
}

pub(crate) fn walk_php_operand_node<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    pat_root: Node<'_>,
    doc: &str,
    out: &mut Vec<PatternMatch>,
) {
    walk_kind_seen(node, pat_root.kind(), false, out, |n| {
        php_operand_node_match(n, source, pattern, pat_root, doc)
    });
}

/// Single-candidate check for [`walk_php_operand_node`].
fn php_operand_node_match(
    node: &Node,
    source: &str,
    pattern: &str,
    pat_root: Node<'_>,
    doc: &str,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if !php_rhs_expr_matches(pat_root, *node, source, doc, &mut captures) {
        return None;
    }
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    Some(hit_for_node(node, source, pattern, captures))
}
