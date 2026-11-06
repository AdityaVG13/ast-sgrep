//! If-body helpers shared by the general and if lanes.

use super::*;
use crate::Language;
use tree_sitter::Node;

pub(crate) fn if_body_matches(
    lang: Language,
    node: &Node,
    template: Option<&BodyTemplate>,
    body_braced: bool,
) -> bool {
    let Some(template) = template else {
        return true;
    };
    let Some(consequence) = if_consequence(node) else {
        return false;
    };
    // A BRACED pattern body (`{ $B }` / `{ $$$B }`) is STRUCTURAL — the
    // candidate consequence must itself be a braced block, or the
    // reference refuses the candidate (brace-less `if (c) d();` sites are
    // `[]` under braced patterns: js/ts/c/php all refuse). kt's
    // `control_structure_body` wrapper counts as braced exactly when its
    // first non-trivia child is a block or the `{}` tokens.
    if body_braced && !consequence_is_braced(&consequence) {
        return false;
    }
    match template {
        BodyTemplate::Any => true,
        BodyTemplate::Exactly(want) => {
            // The py colon-suite rule — `if $X: $B` answers EVERY python if
            // and binds $B to the WHOLE suite text regardless of statement
            // count (two/three-statement indented suites and the one-line
            // `a(); b()` suite all answer with B = suite text; the old
            // Exactly(1) statement count refused each). The bare-colon
            // EMPTY-suite pin is untouched: Exactly(0) keeps the count
            // comparison and refuses.
            if lang == Language::Python && *want >= 1 {
                return true;
            }
            if BLOCK_KINDS.contains(&consequence.kind()) {
                count_statements(consequence) == *want
            } else {
                // Braceless consequence (`if (x) foo();`) is one statement.
                *want == 1
            }
        }
    }
}

/// True when a candidate consequence node is a BRACED block. Direct block
/// kinds are braced by construction; a non-block wrapper (kt
/// `control_structure_body`) is braced exactly when its first non-trivia
/// child is `{` or a block kind.
pub(crate) fn consequence_is_braced(consequence: &Node) -> bool {
    if BLOCK_KINDS.contains(&consequence.kind()) {
        return true;
    }
    let mut cursor = consequence.walk();
    let children: Vec<Node> = consequence
        .children(&mut cursor)
        .filter(|child| !is_trivia_kind(child.kind()) && !child.is_extra())
        .collect();
    children
        .first()
        .is_some_and(|first| first.kind() == "{" || BLOCK_KINDS.contains(&first.kind()))
}

/// The then-branch of an if node: `consequence`/`body` field, else the first
/// block-like named child (first, not last, so an else block is never picked).
pub(crate) fn if_consequence<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("consequence")
        .or_else(|| node.child_by_field_name("body"))
        .or_else(|| {
            let mut cursor = node.walk();
            let found = node
                .named_children(&mut cursor)
                .find(|child| BLOCK_KINDS.contains(&child.kind()));
            found
        })
}
