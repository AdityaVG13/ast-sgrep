//! Python `del` statement lane.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// One PATTERN operand element of the python `del` lane, which walks
/// `delete_statement` candidates directly and binds the `expression_list`
/// child's text. Binding rules: a lone `$V` binds the whole operand-list
/// text; multi-element lists bind positionally (candidate extras absorb,
/// short candidates refuse); literals byte-match; postfix chains unify
/// level by level over the left-nested tree with kinds agreeing at every
/// level; parens are structural (inner text binds, each direction enforced);
/// `$$$V` slots bind in the MULTI namespace. Unlisted spellings keep their
/// registered route (fail-closed).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PyDelOperand {
    /// `$V` — binds the candidate operand text (the whole list text when a
    /// single-element template faces a multi-operand candidate).
    Whole(String),
    /// `$$$V` — binds the whole candidate operand text in the MULTI namespace.
    WholeMulti(String),
    /// A bare meta under the top-level paren spelling (`del ($X)`) — binds
    /// the candidate's inner text.
    Paren(String),
    /// A fully literal element under the paren spelling (`del (x)`) — binds
    /// the paren candidate byte-exactly, refuses the paren-free spelling.
    ParenLiteral(String),
    /// A STRUCTURAL element under the paren spelling (`del ($O[$K])`) —
    /// accepted but binds nothing: the paren group roots as a
    /// parenthesized expression no delete operand aligns with.
    ParenStructural,
    /// `del ($$$X)` — binds the candidate's single inner operand in the
    /// multi namespace, refuses 0 or 2+ elements.
    ParenMulti(String),
    /// A postfix chain `head[i1][i2].a…` — the head and every bracket/dot
    /// atom are a canonical meta or a literal identifier, unified level by
    /// level over the left-nested tree (`d[k1][k2]` is
    /// `subscript(subscript(d,k1),k2)`): metas bind their level's node
    /// text, literals byte-match. Pure-attribute chains keep multi-element
    /// width; any bracket group demands a single-element list.
    Postfix {
        head: PyDelAtom,
        ops: Vec<PyDelPostfix>,
    },
    /// `head(arg)` — exactly ONE argument; exact-count structural. The head
    /// is a meta or literal identifier, as is the argument.
    Call { head: PyDelAtom, arg: PyDelAtom },
    /// `($O[$K])` — a postfix chain under a PER-ELEMENT paren; unifies the
    /// chain against the parenthesized candidate's inner node.
    ParenPostfix {
        head: PyDelAtom,
        ops: Vec<PyDelPostfix>,
    },
    /// A literal identifier element (`del x, $B`) — byte-match.
    Literal(String),
}

/// One atom of a postfix chain: a canonical meta (`$K`), a multi meta
/// (`$$$K`), or a literal identifier (`k`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PyDelAtom {
    Meta(String),
    MetaMulti(String),
    Literal(String),
    /// A dotted call head `$A.b` — the FIRST segment is a canonical meta
    /// binding the receiver base, every later segment a literal identifier
    /// byte-matched innermost-first.
    ChainBase {
        base: String,
        attrs: Vec<String>,
    },
}

/// One postfix operator: a `[atom]` bracket group, a `.atom` attribute
/// access, or a `[lo:hi]` slice — a step-slot spelling demands a candidate
/// step field; a step-less pattern absorbs one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PyDelPostfix {
    Index(PyDelAtom),
    Attr(PyDelAtom),
    Slice {
        lo: PyDelAtom,
        hi: PyDelAtom,
        step: Option<PyDelAtom>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PyDelTemplate {
    operands: Vec<PyDelOperand>,
    /// The whole operand section was paren-wrapped (`del ($X, $Y)`).
    paren_wrapped: bool,
}

pub(crate) fn py_delete_meta_pattern(pattern: &str) -> bool {
    py_delete_template(pattern).is_some()
}

/// Parse the delete-lane pattern shapes (see the lane doc). Every element
/// must be a listed spelling; anything else (trailing commas, meta indexes,
/// deep chains) keeps its registered route (fail-closed). `$$$`-prefixed
/// slots bind the MULTI namespace.
pub(crate) fn py_delete_template(pattern: &str) -> Option<PyDelTemplate> {
    let trimmed = pattern.trim();
    let after_del = trimmed.strip_prefix("del")?;
    // `del $X` demands the whitespace; the tight paren spelling `del($X)`
    // may glue the paren.
    if !(after_del.starts_with(char::is_whitespace) || after_del.starts_with('(')) {
        return None;
    }
    let section = after_del.trim();
    if section.is_empty() {
        return None;
    }
    let (inner, paren_wrapped) = if section.starts_with('(') {
        let after_paren = section.strip_prefix('(')?;
        let close = balanced_paren_close(after_paren)?;
        if after_paren[close + 1..].trim().is_empty() {
            (after_paren[..close].trim(), true)
        } else {
            // `del ($X), $Y` — the paren wraps only the FIRST element; the
            // list keeps its elements and each element parses its own
            // per-element shape (paren_wrapped stays FALSE so the
            // expression_list extraction branch stays open and the
            // per-element kinds decide).
            (section, false)
        }
    } else {
        (section, false)
    };
    let elements = split_top_level_commas(inner);
    if elements.is_empty() || elements.iter().any(|e| e.trim().is_empty()) {
        // A trailing comma (`del $A, $B,`) keeps the refusal.
        return None;
    }
    let mut operands = Vec::new();
    for element in &elements {
        let text = element.trim();
        if let Some(name) = capture_name(text) {
            if text.starts_with("$$$") {
                // `del $$$X` binds the whole candidate operand in the MULTI
                // namespace.
                operands.push(PyDelOperand::WholeMulti(name.to_string()));
            } else {
                operands.push(PyDelOperand::Whole(name.to_string()));
            }
            continue;
        }
        if is_pattern_ident(text) {
            operands.push(PyDelOperand::Literal(text.to_string()));
            continue;
        }
        // Structural shapes are covered UNWRAPPED only (`del ($X)`/`del
        // ($X, $Y)` bind bare metas). A SINGLE structural element under the
        // paren spelling is the ACCEPTED-binds-nothing class (`del ($O[$K])`
        // — the reference's pattern parse roots the paren group as a
        // parenthesized expression no delete operand aligns with); anything
        // else structural under parens (incl. mixed lists) stays refused.
        if paren_wrapped {
            if elements.len() == 1
                && parse_py_del_postfix(text).is_some_and(|(_, ops)| !ops.is_empty())
            {
                operands.push(PyDelOperand::ParenStructural);
                continue;
            }
            return None;
        }
        // A PER-ELEMENT paren (`($X)` / `($O[$K])` as one list element among
        // several) — a bare meta inside re-tags to the Paren slot, a postfix
        // chain to the ParenPostfix slot; anything else inside refuses.
        if let Some(element_inner) = py_del_element_paren_section(text) {
            if let Some(name) = capture_name(element_inner) {
                operands.push(PyDelOperand::Paren(name.to_string()));
                continue;
            }
            if let Some((head, ops)) = parse_py_del_postfix(element_inner) {
                if !ops.is_empty() {
                    operands.push(PyDelOperand::ParenPostfix { head, ops });
                    continue;
                }
            }
            return None;
        }
        // The call element `head(arg)` — exactly one meta/literal argument.
        if let Some((head, arg)) = parse_py_del_call(text) {
            operands.push(PyDelOperand::Call { head, arg });
            continue;
        }
        // Postfix chains — `head[i1][i2]`, `a.b.c`, `$O[$K].$A`, `d.b[k]`…
        // The head is a meta or literal identifier; every bracket/dot atom
        // likewise. Index-bearing chains are admitted in MULTI-element
        // lists too (`del $O[$K], $Y`; `del $A, $O[$K]`; `del f($G), $Y`)
        // — the old "bracket groups keep the single-element width" refusal
        // was an unregistered over-refusal (the post-walk backstop kept
        // the face loud where the reference answers).
        if let Some((head, ops)) = parse_py_del_postfix(text) {
            operands.push(PyDelOperand::Postfix { head, ops });
            continue;
        }
        return None;
    }
    // A single Paren element is the `del ($X)` shape (the dedicated arm
    // binds the parenthesized candidate's INNER text). Multi-element
    // paren-wrapped lists (`del ($X, $Y)`) keep the inner flavors — the
    // wrap is LIST-level (one paren pair around the whole list), so the
    // elementwise path's list unwrap already consumed it and each operand
    // unifies against the bare element.
    // A single fully-literal element re-tags to ParenLiteral — the reference
    // binds the paren candidate byte-exactly (`del (x)` × `del (x)`) and
    // refuses the paren-free spelling (`del (x)` × `del x`); the old code
    // left it a plain Literal whose paren-candidate element extraction found
    // no named children and answered silent-empty.
    if paren_wrapped && operands.len() == 1 {
        for operand in &mut operands {
            match operand {
                PyDelOperand::Whole(name) => {
                    *operand = PyDelOperand::Paren(name.clone());
                }
                PyDelOperand::Literal(lit) => {
                    *operand = PyDelOperand::ParenLiteral(lit.clone());
                }
                // `del ($$$X)` BINDS the single-element candidate (multi X=["x"])
                // and refuses 0/≥2 inner operands — re-tag to the dedicated
                // ParenMulti slot, not ParenStructural.
                PyDelOperand::WholeMulti(name) => {
                    *operand = PyDelOperand::ParenMulti(name.clone());
                }
                _ => {}
            }
        }
    }
    Some(PyDelTemplate {
        operands,
        paren_wrapped,
    })
}

/// A WHOLE element that is one balanced paren group — `($X)`, `($O[$K])`.
/// Returns the trimmed inner text.
pub(crate) fn py_del_element_paren_section(text: &str) -> Option<&str> {
    let after = text.strip_prefix('(')?;
    let close = balanced_paren_close(after)?;
    if !after[close + 1..].trim().is_empty() {
        return None;
    }
    Some(after[..close].trim())
}

/// `head(arg)` — the head is a meta or literal identifier; the paren group
/// holds EXACTLY ONE meta/literal argument; nothing after the group.
pub(crate) fn parse_py_del_call(text: &str) -> Option<(PyDelAtom, PyDelAtom)> {
    let open = text.find('(')?;
    let head_text = text[..open].trim();
    let head = py_del_call_atom(head_text).or_else(|| py_del_chain_head(head_text))?;
    let after = &text[open + 1..];
    let close = balanced_paren_close(after)?;
    if !after[close + 1..].trim().is_empty() {
        return None;
    }
    let arg_text = after[..close].trim();
    if arg_text.is_empty() {
        return None;
    }
    let arg = py_del_call_atom(arg_text)?;
    Some((head, arg))
}

/// A DOTTED call head `$A.b` — the FIRST segment is a canonical meta
/// (binding the receiver base; a `$$$` base refuses) and every later segment
/// a literal identifier; the last segment is the call's literal tail. A
/// chain-free head (`f`) never reaches here.
pub(crate) fn py_del_chain_head(text: &str) -> Option<PyDelAtom> {
    if !text.contains('.') {
        return None;
    }
    let mut segments = text.split('.');
    let base = segments.next()?.trim();
    if base.starts_with("$$$") {
        return None;
    }
    let base_name = capture_name(base)?;
    let attrs: Vec<String> = segments.map(|segment| segment.trim().to_string()).collect();
    if attrs.is_empty() || attrs.iter().any(|segment| !is_pattern_ident(segment)) {
        return None;
    }
    Some(PyDelAtom::ChainBase {
        base: base_name.to_string(),
        attrs,
    })
}

pub(crate) fn py_del_call_atom(text: &str) -> Option<PyDelAtom> {
    if let Some(name) = capture_name(text) {
        if text.starts_with("$$$") {
            return Some(PyDelAtom::MetaMulti(name.to_string()));
        }
        return Some(PyDelAtom::Meta(name.to_string()));
    }
    is_pattern_ident(text).then(|| PyDelAtom::Literal(text.to_string()))
}

/// Parse a postfix chain operand — `head`, then one or more `[atom]` /
/// `.atom` operators; every atom is a canonical meta or a literal identifier;
/// the whole text must be consumed; no commas inside a bracket group
/// (`d[k, j]` stays refused).
pub(crate) fn parse_py_del_postfix(text: &str) -> Option<(PyDelAtom, Vec<PyDelPostfix>)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let atom_at = |segment: &str| -> Option<PyDelAtom> {
        if let Some(name) = capture_name(segment) {
            if segment.starts_with("$$$") {
                Some(PyDelAtom::MetaMulti(name.to_string()))
            } else {
                Some(PyDelAtom::Meta(name.to_string()))
            }
        } else if is_pattern_ident(segment) {
            Some(PyDelAtom::Literal(segment.to_string()))
        } else {
            None
        }
    };
    // Head: the identifier/meta run before the first `[` or `.`.
    let head_end = text.find(['[', '.']).unwrap_or(text.len());
    let head = atom_at(text[..head_end].trim())?;
    let mut ops = Vec::new();
    let mut rest = &text[head_end..];
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['[', '.']).unwrap_or(after.len());
            let atom = atom_at(after[..end].trim())?;
            ops.push(PyDelPostfix::Attr(atom));
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix('[') {
            let close = after.find(']')?;
            // One atom per bracket group; a comma (or nested brackets)
            // refuses (the 137 `d[k, j]` discipline).
            let inner = after[..close].trim();
            if inner.contains([',', '[', ']']) {
                return None;
            }
            // The slice spellings — one or two `:` separators, every populated
            // slot an atom (a pattern-side OPEN slot keeps the loud
            // fail-closed route).
            if inner.contains(':') {
                let parts: Vec<&str> = inner.split(':').collect();
                if parts.len() > 3 {
                    return None;
                }
                let slot = |segment: &str| -> Option<PyDelAtom> {
                    let segment = segment.trim();
                    if let Some(name) = capture_name(segment) {
                        if segment.starts_with("$$$") {
                            Some(PyDelAtom::MetaMulti(name.to_string()))
                        } else {
                            Some(PyDelAtom::Meta(name.to_string()))
                        }
                    } else if is_pattern_ident(segment)
                        || !segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit())
                    {
                        Some(PyDelAtom::Literal(segment.to_string()))
                    } else {
                        None
                    }
                };
                let (lo, hi, step) = match parts.len() {
                    2 => (slot(parts[0])?, slot(parts[1])?, None),
                    3 => (slot(parts[0])?, slot(parts[1])?, Some(slot(parts[2])?)),
                    _ => return None,
                };
                ops.push(PyDelPostfix::Slice { lo, hi, step });
                rest = &after[close + 1..];
                continue;
            }
            let atom = atom_at(inner)?;
            ops.push(PyDelPostfix::Index(atom));
            rest = &after[close + 1..];
        } else {
            return None;
        }
    }
    Some((head, ops))
}

/// Top-level comma split — nesting over `()[]{}"` keeps structural elements
/// (`d[k, j]` stays one element... though the lane refuses it later) whole.
pub(crate) fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

pub(crate) fn match_py_delete_meta(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    if !py_delete_meta_pattern(pattern) {
        return None;
    }
    let template = py_delete_template(pattern)?;
    let tree = parse_source(Language::Python, source).ok()?;
    let mut out = Vec::new();
    walk_py_delete_meta(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

pub(crate) fn walk_py_delete_meta(
    node: Node,
    source: &str,
    pattern: &str,
    template: &PyDelTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "delete_statement" && !is_in_comment_or_string(&node) {
        if let Some(hits) = py_delete_statement_match(&node, source, pattern, template) {
            out.extend(hits);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_py_delete_meta(child, source, pattern, template, out);
    }
}

/// The delete_statement unification. `operand_node` is the statement's
/// single named child: the `expression_list` for 2+ operands, the bare
/// operand expression for the inlined single (tree-sitter-python inlines the
/// one-operand list).
pub(crate) fn py_delete_statement_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &PyDelTemplate,
) -> Option<Vec<PatternMatch>> {
    let mut cursor = node.walk();
    let operand_node = node.children(&mut cursor).find(|child| child.is_named())?;
    let operand_kind = operand_node.kind();
    // Whole-bind: the single `$V` template keeps its registered semantics —
    // the whole operand-list text (multi-operand candidates) or the bare
    // operand text (inlined single, parens included: X=`(x)`).
    if template.operands.len() == 1 {
        match template.operands.first() {
            Some(PyDelOperand::Whole(name)) => {
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture(&mut captures, name, node_text(&operand_node, source)?).is_none() {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            // `del $$$X` — the same whole-bind in the MULTI namespace.
            Some(PyDelOperand::WholeMulti(name)) => {
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture_kind(&mut captures, name, node_text(&operand_node, source)?, true)
                    .is_none()
                {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            Some(PyDelOperand::Paren(name)) => {
                if operand_kind != "parenthesized_expression" {
                    return Some(Vec::new());
                }
                let Some(inner) = operand_node.named_child(0) else {
                    return Some(Vec::new());
                };
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture(&mut captures, name, node_text(&inner, source)?).is_none() {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            // `del (x)` × `del (x)` — the inner text byte-matches; the
            // paren-free spelling refuses (`del (x)` × `del x` answers
            // empty).
            Some(PyDelOperand::ParenLiteral(lit)) => {
                if operand_kind != "parenthesized_expression" {
                    return Some(Vec::new());
                }
                let Some(inner) = operand_node.named_child(0) else {
                    return Some(Vec::new());
                };
                if node_text(&inner, source).is_some_and(|text| text.trim() != lit) {
                    return Some(Vec::new());
                }
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            // The paren-structural spelling binds NOTHING — the walk's empty
            // IS the agreement.
            Some(PyDelOperand::ParenStructural) => {
                return Some(Vec::new());
            }
            // `del ($$$X)` — the multi slot demands the parenthesized
            // single-element candidate; the inner operand binds under the
            // `$$$NAME` key. 0/≥2 inner operands and paren-free candidates
            // refuse (`del ()`, `del (a, b)`, `del x`).
            Some(PyDelOperand::ParenMulti(name)) => {
                if operand_kind != "parenthesized_expression" {
                    return Some(Vec::new());
                }
                let Some(inner) = operand_node.named_child(0) else {
                    return Some(Vec::new());
                };
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture_kind(&mut captures, name, node_text(&inner, source)?, true)
                    .is_none()
                {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            _ => {}
        }
    }
    // Elementwise paths need the candidate ELEMENTS (operand expressions).
    let paren_required = template.paren_wrapped
        || template
            .operands
            .iter()
            .any(|operand| matches!(operand, PyDelOperand::Paren(_)));
    let elements: Vec<Node> =
        if paren_required && matches!(operand_kind, "parenthesized_expression") {
            let Some(inner) = operand_node.named_child(0) else {
                return Some(Vec::new());
            };
            let mut inner_cursor = inner.walk();
            let inner_children: Vec<Node> = inner
                .children(&mut inner_cursor)
                .filter(|c| c.is_named())
                .collect();
            match inner_children.len() {
                0 => return Some(Vec::new()),
                1 => {
                    let only = &inner_children[0];
                    if matches!(only.kind(), "tuple" | "expression_list" | "list") {
                        let mut only_cursor = only.walk();
                        let nested: Vec<Node> = only
                            .children(&mut only_cursor)
                            .filter(|c| c.is_named())
                            .collect();
                        if nested.is_empty() {
                            vec![*only]
                        } else {
                            nested
                        }
                    } else {
                        inner_children
                    }
                }
                _ => inner_children,
            }
        } else if paren_required && operand_kind == "tuple" {
            // `del (x, y)` — python parses the paren-wrapped list as a bare
            // `tuple` (the parens belong to the tuple itself), so the wrap is
            // already consumed and the tuple's named children ARE the elements
            // (X=`x` Y=`y`).
            let mut tuple_cursor = operand_node.walk();
            operand_node
                .children(&mut tuple_cursor)
                .filter(|c| c.is_named())
                .collect()
        } else if operand_kind == "expression_list" && !template.paren_wrapped {
            // A LIST-level wrap (`del ($X, $Y)`, paren_wrapped=true) REFUSES the
            // paren-free candidate spellings (the paren wrap is structurally
            // significant on the pattern side). A PER-ELEMENT paren
            // (`del ($X), $Y`) does NOT refuse the free candidate — the
            // elementwise unify's per-slot kind checks decide (the Paren slot
            // demands a parenthesized_expression element; the postfix/meta
            // slots bind).
            let mut list_cursor = operand_node.walk();
            operand_node
                .children(&mut list_cursor)
                .filter(|c| c.is_named())
                .collect()
        } else {
            vec![operand_node]
        };
    // Operand-count alignment — a SINGLE-operand template demands the exact
    // candidate count (`del $O[$K]` × `del d[k], y` refuses), while a
    // MULTI-operand template aligns the operands as an in-order PREFIX and
    // absorbs TRAILING candidate extras (`del $O[$K], $Y` × `del d[k], y, z`
    // binds Y=`y`) — leading extras and short candidates refuse either way
    // (`del z, d[k], y` / `del d[k]`). The zip below performs the in-order
    // prefix unification, so absorbing is implicit.
    if template.operands.len() == 1 {
        if elements.len() != 1 {
            return Some(Vec::new());
        }
    } else if elements.len() < template.operands.len() {
        return Some(Vec::new());
    }
    let mut captures = BTreeMap::new();
    if let Some(whole) = node_text(node, source) {
        captures.insert("MATCH".to_string(), whole.to_string());
    }
    for (operand, element) in template.operands.iter().zip(elements.iter()) {
        if !py_del_operand_unify(operand, element, source, &mut captures) {
            return Some(Vec::new());
        }
    }
    let mut out = Vec::new();
    push_match_with_captures(node, source, pattern, captures, &mut out);
    Some(out)
}

/// One candidate operand element against one PATTERN operand: metas bind the
/// element text, literals byte-match, the subscript shape byte-matches its
/// head and binds the index meta, the attribute shape binds the
/// all-but-last-attribute receiver and the last identifier.
pub(crate) fn py_del_operand_unify(
    operand: &PyDelOperand,
    element: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    match operand {
        PyDelOperand::Whole(name) => bind_capture(
            captures,
            name,
            &node_text(element, source).unwrap_or_default(),
        )
        .is_some(),
        // `$$$X` binds in the MULTI namespace.
        PyDelOperand::WholeMulti(name) => bind_capture_kind(
            captures,
            name,
            &node_text(element, source).unwrap_or_default(),
            true,
        )
        .is_some(),
        PyDelOperand::Paren(name) => {
            element.kind() == "parenthesized_expression"
                && element
                    .named_child(0)
                    .and_then(|inner| node_text(&inner, source))
                    .is_some_and(|text| bind_capture(captures, name, &text).is_some())
        }
        PyDelOperand::Literal(lit) => {
            node_text(element, source).is_some_and(|text| text.trim() == lit)
        }
        PyDelOperand::Postfix { head, ops } => {
            py_del_postfix_unify(head, ops, element, source, captures)
        }
        // The candidate element must be a call with EXACTLY ONE argument
        // node; head/arg unify against the function child and the argument
        // node respectively (the argument meta binds the whole node text:
        // A=`x.y`).
        PyDelOperand::Call { head, arg } => {
            if element.kind() != "call" {
                return false;
            }
            let Some(function) = element.child_by_field_name("function") else {
                return false;
            };
            let Some(arguments) = element.child_by_field_name("arguments") else {
                return false;
            };
            let mut arg_cursor = arguments.walk();
            let args: Vec<Node> = arguments
                .children(&mut arg_cursor)
                .filter(|c| c.is_named())
                .collect();
            let [only] = args.as_slice() else {
                return false;
            };
            py_del_atom_unify(head, &function, source, captures)
                && py_del_atom_unify(arg, only, source, captures)
        }
        // The chain unifies against the parenthesized candidate's INNER node
        // (X=`d[k]`).
        PyDelOperand::ParenPostfix { head, ops } => {
            if element.kind() != "parenthesized_expression" {
                return false;
            }
            let Some(inner) = element.named_child(0) else {
                return false;
            };
            py_del_postfix_unify(head, ops, &inner, source, captures)
        }
        // ParenLiteral/ParenStructural never reach the elementwise unifier:
        // both exist only as single-element templates, which take their
        // dedicated arms before any elementwise work.
        PyDelOperand::ParenLiteral(_) | PyDelOperand::ParenStructural => true,
        // ParenMulti also never reaches the elementwise unifier: its
        // dedicated arm returns before elementwise work.
        PyDelOperand::ParenMulti(_) => true,
    }
}

/// One postfix-chain operand against one candidate element — structural
/// alignment LEVEL BY LEVEL over the left-nested tree (`d[k1][k2]` is
/// `subscript(subscript(d,k1),k2)`, `x.y.z` is
/// `attribute(attribute(x,y),z)`). The LAST operator aligns the element's
/// own kind; the prefix chain recurses into the object child. Metas bind
/// their level's node text, literals byte-match (`del $O[$K][j]` ×
/// `del d[k1][k2]` refuses — `j`≠`k2`), and the candidate kind must
/// agree at every level (`del $O[$K]` × `del x.y` refuses — attribute vs
/// subscript).
pub(crate) fn py_del_postfix_unify(
    head: &PyDelAtom,
    ops: &[PyDelPostfix],
    element: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    let Some((last, prefix)) = ops.split_last() else {
        return py_del_atom_unify(head, element, source, captures);
    };
    match last {
        PyDelPostfix::Index(atom) => {
            if element.kind() != "subscript" {
                return false;
            }
            let object = element
                .child_by_field_name("object")
                .or_else(|| element.named_child(0));
            let subscripts = element
                .child_by_field_name("subscripts")
                .or_else(|| element.named_child(1));
            let (Some(object), Some(subscripts)) = (object, subscripts) else {
                return false;
            };
            py_del_atom_unify(atom, &subscripts, source, captures)
                && py_del_prefix_unify(head, prefix, &object, source, captures)
        }
        PyDelPostfix::Slice { lo, hi, step } => {
            // The slice element law. The tree-sitter-python `slice` node has NO
            // field names — its bounds are POSITIONAL named children (absent
            // bounds produce no child) and the step marker is a SECOND
            // anonymous `:`. A 2-named slice is [start, stop]; a 3-named
            // slice (two `:`s) is [start, stop, step]; 0/1-named slices (open
            // bounds) and partial-step shapes refuse.
            if element.kind() != "subscript" {
                return false;
            }
            let object = element
                .child_by_field_name("object")
                .or_else(|| element.named_child(0));
            let subscripts = element
                .child_by_field_name("subscripts")
                .or_else(|| element.named_child(1));
            let (Some(object), Some(subscripts)) = (object, subscripts) else {
                return false;
            };
            if subscripts.kind() != "slice" {
                return false;
            }
            // Comments are trivia BETWEEN the slice bounds/colons but refuse in
            // the START-bound position (the comment occupies the slot the
            // exact-children match can not skip there) or between the object
            // and the slice node. Filter trivia children from the positional
            // bounds and veto the leading position (both extra-attachment
            // shapes).
            let mut sub_cursor = element.walk();
            if element.children(&mut sub_cursor).any(|c| {
                c.is_named()
                    && is_trivia_kind(c.kind())
                    && c.start_byte() >= object.end_byte()
                    && c.end_byte() <= subscripts.start_byte()
            }) {
                return false;
            }
            let mut slice_cursor = subscripts.walk();
            let slice_children: Vec<Node> = subscripts.children(&mut slice_cursor).collect();
            if slice_children
                .iter()
                .find(|c| c.is_named())
                .is_some_and(|first| is_trivia_kind(first.kind()))
            {
                return false;
            }
            let bounds: Vec<Node> = slice_children
                .iter()
                .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
                .copied()
                .collect();
            let mut colons_cursor = subscripts.walk();
            let colons = subscripts
                .children(&mut colons_cursor)
                .filter(|c| !c.is_named() && node_text(&c, source).is_some_and(|t| t == ":"))
                .count();
            let mut bind_atom = |atom: &PyDelAtom, bound: &Node| -> bool {
                node_text(bound, source)
                    .is_some_and(|t| py_del_atom_bind_text(atom, t.trim(), captures))
            };
            let (lo_node, hi_node, step_node) = match (bounds.len(), colons) {
                (2, 1) => (Some(&bounds[0]), Some(&bounds[1]), None),
                (3, 2) => (Some(&bounds[0]), Some(&bounds[1]), Some(&bounds[2])),
                _ => return false,
            };
            let (Some(lo_node), Some(hi_node)) = (lo_node, hi_node) else {
                return false;
            };
            if !bind_atom(lo, lo_node) || !bind_atom(hi, hi_node) {
                return false;
            }
            match step {
                Some(step_atom) => {
                    let Some(step_bound) = step_node else {
                        return false;
                    };
                    if !bind_atom(step_atom, step_bound) {
                        return false;
                    }
                }
                None => {}
            }
            py_del_prefix_unify(head, prefix, &object, source, captures)
        }
        PyDelPostfix::Attr(atom) => {
            if element.kind() != "attribute" {
                return false;
            }
            let object = element
                .child_by_field_name("object")
                .or_else(|| element.named_child(0));
            let attribute = element
                .child_by_field_name("attr")
                .or_else(|| element.named_child(1));
            let (Some(object), Some(attribute)) = (object, attribute) else {
                return false;
            };
            py_del_atom_unify(atom, &attribute, source, captures)
                && py_del_prefix_unify(head, prefix, &object, source, captures)
        }
    }
}

/// The prefix recursion: an empty operator list means the HEAD atom
/// compares against the object node itself (the 137 receiver law — the
/// head meta absorbs the WHOLE all-but-last chain: `del $O.$A` ×
/// `del o.a.b` binds O=`o.a`); otherwise the prefix chain re-unifies.
pub(crate) fn py_del_prefix_unify(
    head: &PyDelAtom,
    prefix: &[PyDelPostfix],
    object: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    if prefix.is_empty() {
        py_del_atom_unify(head, object, source, captures)
    } else {
        py_del_postfix_unify(head, prefix, object, source, captures)
    }
}

/// Bind one slice-bound atom against its bound TEXT (the slice arms
/// compare field texts, not nodes).
pub(crate) fn py_del_atom_bind_text(
    atom: &PyDelAtom,
    text: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    match atom {
        PyDelAtom::Meta(name) => bind_capture(captures, name, text).is_some(),
        PyDelAtom::MetaMulti(name) => bind_capture_kind(captures, name, text, true).is_some(),
        PyDelAtom::Literal(literal) => text == literal,
        PyDelAtom::ChainBase { .. } => false,
    }
}

pub(crate) fn py_del_atom_unify(
    atom: &PyDelAtom,
    node: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    let Some(text) = node_text(node, source) else {
        return false;
    };
    match atom {
        PyDelAtom::Meta(name) => bind_capture(captures, name, text.trim()).is_some(),
        // `$$$K` binds the MULTI namespace.
        PyDelAtom::MetaMulti(name) => {
            bind_capture_kind(captures, name, text.trim(), true).is_some()
        }
        PyDelAtom::Literal(lit) => text.trim() == lit,
        // Peel the candidate attribute chain innermost-first; every pattern
        // segment byte-matches the attr field and the receiver BASE binds
        // the remaining object text (`del $A.b($C)` × `del a.b(c)` → A=`a`;
        // a non-attribute function refuses).
        PyDelAtom::ChainBase { base, attrs } => {
            let mut current = node.clone();
            for want in attrs.iter().rev() {
                if current.kind() != "attribute" {
                    return false;
                }
                let Some(attr) = current.child_by_field_name("attribute") else {
                    return false;
                };
                let Some(attr_text) = node_text(&attr, source) else {
                    return false;
                };
                if attr_text.trim() != want {
                    return false;
                }
                let Some(object) = current.child_by_field_name("object") else {
                    return false;
                };
                current = object;
            }
            node_text(&current, source)
                .is_some_and(|t| bind_capture(captures, base, t.trim()).is_some())
        }
    }
}
