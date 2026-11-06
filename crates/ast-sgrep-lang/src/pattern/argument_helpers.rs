//! Call/declaration argument-list helpers (arity + container walks).

use super::*;
use tree_sitter::Node;

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
