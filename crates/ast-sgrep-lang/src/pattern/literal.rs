//! Literal R3 structural-comparison lane.

use super::*;
use crate::extract::{is_ident_kind, is_inside_comment_or_string, node_text};
use crate::Language;
use std::cell::RefCell;
use std::collections::HashMap;
use tree_sitter::Node;

pub(crate) fn walk_literal(
    node: Node,
    source: &str,
    pattern: &str,
    template: Option<&LiteralTemplate>,
    out: &mut Vec<PatternMatch>,
) {
    // ANCESTOR-only trivia judgment. The old self-inclusive check excluded
    // the string node ITSELF, so a string-rooted literal pattern (`'q'`)
    // silently answered [] at every position where the reference answers
    // the string node. Nodes nested UNDER a comment/string ancestor stay
    // skipped, so string/comment content never becomes matchable.
    if !is_inside_comment_or_string(&node) {
        if identifier_matches(&node, source, pattern) {
            push_match(&node, source, pattern, Some(pattern), out);
        } else if literal_content_matches(&node, source, pattern) {
            push_match(&node, source, pattern, Some(pattern), out);
        } else if let Some(template) = template {
            // The structural arm — only ever ADDS matches after the
            // exact-text fast paths above declined.
            if let Some(pat_root) = literal_template_root(template) {
                // Anonymous preproc directive tokens (`#endif`) are not
                // structural roots — the reference answers nothing.
                if node.kind() == pat_root.kind()
                    && !pat_root.kind().starts_with('#')
                    && literal_structural_eq(pat_root, &template.doc, node, source, true)
                {
                    push_match(&node, source, pattern, Some(pattern), out);
                }
            }
        }
        // A `$`-less pattern names the IDENTIFIER itself — emit the `name`
        // node's span, never the enclosing item. Pushing `&node` here made a
        // bare-ident pattern also match the whole `fn old_name() { … }`
        // declaration: search over-answered (parent + child rows for one site)
        // and codemod planned a destructive whole-item rewrite that only the
        // overlap validator caught.
        if let Some(name_node) = node.child_by_field_name("name") {
            if identifier_matches(&name_node, source, pattern) {
                push_match(&name_node, source, pattern, Some(pattern), out);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_literal(child, source, pattern, template, out);
    }
}

pub(crate) fn identifier_matches(node: &Node, source: &str, pattern: &str) -> bool {
    is_ident_kind(node.kind()) && node_text(node, source).is_some_and(|t| t == pattern)
}

/// Full-node-text arm of the literal lane. A `$`-less pattern matches any
/// non-trivia node outside comments/strings whose COMPLETE text equals the
/// pattern — number literals, literal-argument calls, zero-arg member calls,
/// whole statements (the reference's literal-content semantics). Previously
/// only identifier-kind nodes could match, so these faces silently answered
/// `ok:true` empty — a fail-open. The arm only ADDS matches: the
/// loud-fallback guard (`needs_ast_grep_fallback`) keeps exempting `$`-less
/// patterns, and valid-but-empty results stay `ok:true`. The
/// whitespace/trivia-variant residual is closed by the structural arm in
/// `walk_literal` — this exact-text arm stays as the byte-stable fast path
/// in front of it.
pub(crate) fn literal_content_matches(node: &Node, source: &str, pattern: &str) -> bool {
    if is_trivia_kind(node.kind()) {
        return false;
    }
    // Bare preproc END/else directives are anonymous tokens, not matchable
    // nodes — the reference cannot shape `#endif` into a pattern tree. The
    // exact-text arm must not answer them from the token bytes.
    if node.kind().starts_with('#') {
        return false;
    }
    if !node_text(node, source).is_some_and(|text| text.trim() == pattern) {
        return false;
    }
    // Innermost span wins. When a DIRECT non-trivia CHILD's trimmed text
    // also equals the pattern (the co-extensive wrapper chain program >
    // expression_statement > call on a single-row file), this node is an
    // outer shell — the reference reports the innermost node's span (hit
    // text `q(µAble) // c`, never the file bytes with the trailing
    // newline), and emitting every shell duplicated each exact-text row at
    // the search surface. The deepest co-text node has no trim-equal child
    // and is the one that pushes.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_trivia_kind(child.kind()) {
            continue;
        }
        if node_text(&child, source).is_some_and(|text| text.trim() == pattern) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Rule R3 — a `$`-less literal pattern matches a candidate node iff their
// trees are isomorphic. Whitespace of every kind (spaces, tabs, newlines,
// blank lines, operator and callee-paren gaps) and code-side trailing commas
// are invisible; comment nodes are invisible iff they are attached INSIDE an
// argument/parameters container (a comment child of the call node itself
// blocks, either side); a PATTERN-side trailing comma is significant (it
// matches only sites whose source carries one); any other AST delta (arg
// count, concatenated_string vs string, kind changes) blocks. The comparator
// integrates AFTER the exact-text arms, so it only ever ADDS matches —
// fail-open cannot be introduced, and valid-but-empty results stay ok:true.
// Patterns no grammar parses keep today's exact-text behavior
// (`parse_pattern_tree` yields None).

/// A parsed literal-pattern template: the pattern document, its tree, and —
/// when the language required an expression context to parse it — the byte
/// span of the pattern inside that document.
pub(crate) struct LiteralTemplate {
    pub(crate) doc: String,
    pub(crate) tree: tree_sitter::Tree,
    pub(crate) span: Option<(usize, usize)>,
}

thread_local! {
    /// Per-thread literal-pattern template cache (one pattern parse per
    /// (language, pattern); `Tree` clone is reference-counted).
    static LITERAL_TEMPLATES:
        RefCell<HashMap<(Language, String), Option<std::sync::Arc<LiteralTemplate>>>> =
        RefCell::new(HashMap::new());
}

/// Parse a `$`-less pattern into a template tree for R3 structural matching.
/// Wraps the shared thread-local parsers with a per-thread (language,
/// pattern) cache. `None` — the pattern is not valid code for this language —
/// disables the structural arm so reference-rejected pattern text
/// (`text# note` py) keeps today's exact-text behavior with no regression.
pub(crate) fn parse_pattern_tree(
    lang: Language,
    pattern: &str,
) -> Option<std::sync::Arc<LiteralTemplate>> {
    let key = (lang, pattern.to_string());
    LITERAL_TEMPLATES.with(|cell| {
        let mut map = cell.borrow_mut();
        if let Some(template) = map.get(&key) {
            return template.clone();
        }
        let built = build_literal_template(lang, pattern).map(std::sync::Arc::new);
        map.insert(key, built.clone());
        built
    })
}

pub(crate) fn build_literal_template(lang: Language, pattern: &str) -> Option<LiteralTemplate> {
    // Php literal faces wrap behind the `<?php ` tag — the tag-less parse
    // folds the document into a bare `text` node (or an ERROR), which left
    // every php literal pattern without a structural comparator: `$x = $v
    // + 1;` answered only through the exact-text arm and missed the `$x = $v
    // /* mid */ + 1;` candidate the reference answers. The span covers only
    // the pattern bytes, so the statement root and comparison are tag-free;
    // the exact-text arms run in front of the structural comparator, so this
    // only ever ADDS matches.
    if lang == Language::Php {
        let doc = format!("<?php {pattern}");
        if let Ok(tree) = parse_source(lang, &doc) {
            if !tree.root_node().has_error() {
                return Some(LiteralTemplate {
                    doc,
                    tree,
                    span: Some((6, 6 + pattern.len())),
                });
            }
        }
    }
    if let Ok(tree) = parse_source(lang, pattern) {
        if !tree.root_node().has_error() {
            return Some(LiteralTemplate {
                doc: pattern.to_string(),
                tree,
                span: None,
            });
        }
    }
    let (prefix, suffix) = general_expression_context(lang)?;
    let doc = format!("{prefix}{pattern}{suffix}");
    let tree = parse_source(lang, &doc).ok()?;
    if tree.root_node().has_error() {
        return None;
    }
    Some(LiteralTemplate {
        doc,
        tree,
        span: Some((prefix.len(), prefix.len() + pattern.len())),
    })
}

/// The pattern's comparison root: collapse same-span context wrappers, then
/// descend through single-named-child statement wrappers (module /
/// `expression_statement`). A multi-child wrapper (multi-statement pattern)
/// yields `None` — the reference reports each statement separately and
/// the subject does not approximate that here.
pub(crate) fn literal_template_root<'a>(template: &'a LiteralTemplate) -> Option<Node<'a>> {
    let mut node = match template.span {
        Some((start, end)) => {
            let mut node = template
                .tree
                .root_node()
                .descendant_for_byte_range(start, end)?;
            while let Some(parent) = node.parent() {
                if parent.start_byte() == node.start_byte() && parent.end_byte() == node.end_byte()
                {
                    node = parent;
                } else {
                    break;
                }
            }
            node
        }
        None => template.tree.root_node(),
    };
    loop {
        // A pattern-trailing `;` is significant — descending through an
        // expression_statement that CARRIES the terminator would let the R3
        // structural arm answer the bare sub-expression node (a second,
        // `;`-less span where the reference reports the statement span).
        // Statement roots keep the `;` child in the comparison; `;`-less
        // patterns keep descending.
        if node.kind() == "expression_statement"
            && node.child_count() > 0
            && node
                .child((node.child_count() - 1) as u32)
                .is_some_and(|c| c.kind() == ";")
        {
            return Some(node);
        }
        if !GENERAL_WRAPPER_KINDS.contains(&node.kind()) {
            return Some(node);
        }
        let mut cursor = node.walk();
        let named: Vec<Node> = node.named_children(&mut cursor).collect();
        match named.as_slice() {
            [only] => node = *only,
            // A wrapper carrying ONE code child plus a TRAILING comment run
            // is the comment-carrying literal face the reference answers
            // (`q(µAble) // c` — the comment is part of the reported hit
            // text; js/ts attach the comment to the statement). Root at the
            // DEEPEST wrapper holding the comment (the statement), so the
            // comment children stay in the R3 comparison; leading-comment
            // and multi-statement shapes keep the `None` refusal (the
            // registered multi-root posture).
            [code, tail @ ..]
                if !tail.is_empty()
                    && tail.iter().all(|child| is_trivia_kind(child.kind()))
                    && !is_trivia_kind(code.kind()) =>
            {
                if GENERAL_WRAPPER_KINDS.contains(&code.kind()) {
                    node = *code;
                } else {
                    return Some(node);
                }
            }
            _ => return None,
        }
    }
}

/// Admission of the meta-free WHOLE-TOKEN LITERAL route's trailing
/// line-comment faces (`q(µAble) // c`). The trailing comment parses as a
/// real child of the pattern root and the comment-carrying rows answer. The
/// lane admits a face only when every leg holds: (a) the pattern carries no
/// `$` (meta capability would reopen the placement refusals), (b) the parse
/// gate accepts the spelling in THIS language, (c) the pattern parses
/// cleanly so the comparator exists (operator-soup faces stay loud), and
/// (d) the root's tail is a line-comment run behind at least one code
/// child. Consumers: core's placement gate and the per-language
/// answerability consult.
pub fn literal_trailing_comment_lane(lang: Language, pattern: &str) -> bool {
    if pattern.contains('$') {
        return false;
    }
    if !sg_pattern_gate_accepts(lang, pattern) {
        return false;
    }
    let Some(template) = parse_pattern_tree(lang, pattern.trim()) else {
        return false;
    };
    let Some(root) = literal_template_root(&template) else {
        return false;
    };
    let mut cursor = root.walk();
    let children: Vec<Node> = root.children(&mut cursor).collect();
    let comments = children
        .iter()
        .rev()
        .take_while(|child| is_trivia_kind(child.kind()))
        .count();
    // Need at least one code child AND a trailing comment run; interior or
    // leading pattern comments keep the registered block.
    if comments == 0 || comments == children.len() {
        return false;
    }
    children[children.len() - comments..]
        .iter()
        .all(|child| node_text(child, &template.doc).is_some_and(|text| text.starts_with("//")))
        && !children[..children.len() - comments]
            .iter()
            .any(|child| is_trivia_kind(child.kind()))
}

/// Root-level alignment for a pattern-trailing line-comment run. The code
/// head aligns 1:1 (recursive R3, non-root), then the candidate must carry
/// the SAME number of trailing comments with EQUAL node text (`// c` ==
/// `// c` — the whitespace between tokens is trivia, which is why the
/// two-space variant answers). Leading/interior pattern comments never
/// reach here (the lane admission and the guard in [`literal_structural_eq`]
/// block them).
pub(crate) fn root_trailing_line_comments_eq(
    p: Node,
    pattern_doc: &str,
    c: Node,
    source: &str,
) -> bool {
    fn children_with_fields<'a>(node: Node<'a>) -> Vec<(Option<&'a str>, Node<'a>)> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .enumerate()
            .map(|(index, child)| (node.field_name_for_child(index as u32), child))
            .collect()
    }
    let p_children = children_with_fields(p);
    let c_children = children_with_fields(c);
    let comments = p_children
        .iter()
        .rev()
        .take_while(|(_, child)| is_trivia_kind(child.kind()))
        .count();
    if comments == 0 || comments == p_children.len() || c_children.len() < p_children.len() {
        return false;
    }
    let (p_code, p_tail) = p_children.split_at(p_children.len() - comments);
    // Interior pattern comments keep the registered block.
    if p_code.iter().any(|(_, child)| is_trivia_kind(child.kind())) {
        return false;
    }
    // Candidate code head aligns 1:1.
    for ((p_field, p_child), (c_field, c_child)) in p_code.iter().zip(c_children.iter()) {
        if p_field != c_field {
            return false;
        }
        if !literal_structural_eq(*p_child, pattern_doc, *c_child, source, false) {
            return false;
        }
    }
    // Candidate comment tail: same count, comment kind, equal text.
    let c_tail = &c_children[p_code.len()..];
    if c_tail.len() != comments {
        return false;
    }
    for ((p_field, p_comment), (c_field, c_comment)) in p_tail.iter().zip(c_tail.iter()) {
        if !is_trivia_kind(c_comment.kind()) || p_field != c_field {
            return false;
        }
        if node_text(p_comment, pattern_doc) != node_text(c_comment, source) {
            return false;
        }
    }
    true
}

/// Recursive R3 isomorphism check between a pattern subtree and a candidate
/// subtree: kind equality; exact-text leaves; the container-scoped comment
/// guard; comma significance; field-name alignment. `is_root` marks the
/// pattern's matched root node — reference probes show candidate
/// comments are transparent BELOW the matched root (`$x = $v /* mid */ + 1;`
/// answers: the comment sits inside the binary operand) but still block as
/// a DIRECT child of the matched root itself (a comment on the root call
/// node refuses).
pub(crate) fn literal_structural_eq(
    p: Node,
    pattern_doc: &str,
    c: Node,
    source: &str,
    is_root: bool,
) -> bool {
    if p.kind() != c.kind() {
        return false;
    }
    if p.child_count() == 0 {
        return node_text(&p, pattern_doc).is_some_and(|text| node_text(&c, source) == Some(text));
    }
    let container = is_argument_container_kind(p.kind());
    // R3 comment guard: a PATTERN-side comment outside an arguments /
    // parameters container is a real AST child and blocks (unchanged). A
    // CANDIDATE-side comment blocks only as a DIRECT child of the MATCHED
    // ROOT (the root-call comment). Below the root, candidate comments are
    // transparent: the reference answers `$x = $v /* mid */ + 1;` for the
    // comment-free pattern, and the binary operand level is not a container.
    // Each half of the root guard alone is redundant defense — the
    // "comments invisible everywhere" mutant (this guard AND the
    // container-only skip in `comparable_children` disabled together) flips
    // the root-comment case to a match; only the combined mutant is a valid
    // discrimination probe.
    if !container && has_comment_child(&p) {
        // At the matched ROOT, a pattern-trailing line-comment run is a
        // REQUIRED text-exact slot — the alignment matches the comment
        // child one-for-one. Interior/leading pattern comments and
        // non-root placements keep the block. A successful trailing-run
        // alignment IS the full root contract (it already aligned the code
        // head 1:1 and pinned the candidate comment tail), so it returns
        // true here rather than falling through to the candidate-side
        // guard below, which would re-block the demanded comment child.
        if is_root && root_trailing_line_comments_eq(p, pattern_doc, c, source) {
            return true;
        }
        return false;
    }
    if is_root && !container && has_comment_child(&c) {
        return false;
    }
    // R3 comma clause: a PATTERN-side trailing comma is significant; a
    // code-side one is invisible unless the pattern demands it.
    if container && has_trailing_comma(&p) && !has_trailing_comma(&c) {
        return false;
    }
    // A PATTERN-side comment inside an argument container is a REQUIRED
    // SLOT (pattern `add(1, /* n */ 2)` answers only sites carrying a
    // comment in that argument position — text-free, so `/* mid */`
    // matches `/* note */` — and does NOT reach the comment-less
    // `add(1, 2)` site). Source-side comments remain invisible where no
    // slot demands them, and trailing source comments after the last
    // matched child stay invisible.
    if container && has_comment_child(&p) {
        return container_comment_slot_eq(p, pattern_doc, c, source);
    }
    // Below the matched root, CANDIDATE-side comment children are invisible
    // at every node level (php mid-comment binaries and go comment-carrying
    // argument lists answer); the PATTERN side keeps the
    // container-only visibility (skip_trivia == container) so a pattern
    // comment stays the significant child it is registered to be.
    let mut p_children = comparable_children(p, container);
    let mut c_children = comparable_children(c, container || !is_root);
    // A source-side statement terminator is invisible when the pattern lacks
    // it — `const $x = 1` answers `const $x = 1;` (js/ts declaration roots
    // own the `;` byte, so the child lists mismatch without this clause). A
    // PATTERN-side `;` stays significant (the terminator pin — this clause
    // only ever drops one trailing source child).
    if c_children.len() == p_children.len() + 1
        && c_children
            .last()
            .is_some_and(|child| child.node.kind() == ";")
    {
        c_children.pop();
    }
    if p_children.len() != c_children.len() {
        return false;
    }
    for (p_child, c_child) in p_children.drain(..).zip(c_children.drain(..)) {
        if p_child.field != c_child.field {
            return false;
        }
        if !literal_structural_eq(p_child.node, pattern_doc, c_child.node, source, false) {
            return false;
        }
    }
    true
}

/// Positional alignment for containers whose PATTERN carries comment
/// children (required slots). Pattern children keep comments and drop
/// commas; source children keep comments and drop commas; trailing source
/// comments after the last consumed child are invisible. A pattern comment
/// consumes exactly one source comment (any kind, text-free); a pattern
/// code child consumes the next source CODE child (leading source comments
/// are skipped only while a code child is being matched and no slot
/// precedes it positionally — once a slot has been demanded, source
/// comments are consumed by slots or block).
pub(crate) fn container_comment_slot_eq(p: Node, pattern_doc: &str, c: Node, source: &str) -> bool {
    let mut p_children: Vec<ComparableChild> = {
        let mut cursor = p.walk();
        p.children(&mut cursor)
            .enumerate()
            .filter(|(_, child)| child.kind() != ",")
            .map(|(index, child)| ComparableChild {
                field: p.field_name_for_child(index as u32),
                node: child,
            })
            .collect()
    };
    let mut c_children: Vec<ComparableChild> = {
        let mut cursor = c.walk();
        c.children(&mut cursor)
            .enumerate()
            .filter(|(_, child)| child.kind() != ",")
            .map(|(index, child)| ComparableChild {
                field: c.field_name_for_child(index as u32),
                node: child,
            })
            .collect()
    };
    // Drop TRAILING source comments (trivia attached after the last real
    // argument is invisible); interior comments stay as slot candidates.
    while c_children
        .last()
        .is_some_and(|child| is_trivia_kind(child.node.kind()))
    {
        c_children.pop();
    }
    // Comment slots align by RAW comma-adjacency, not by text and not
    // across separator positions. The reference keeps the separator
    // significant when attaching comments (`calc(1, /* n */ 2)` answers
    // only the source whose comment also sits AFTER the comma — the
    // pre-comma `calc(1 /* mid */, 2)` is rejected — while a DIFFERENT
    // comment text in the same slot still answers). The comma-stripped
    // child lists above erase exactly that distinction, so both sides get
    // a slot class from their RAW siblings: PostComma (previous raw
    // non-trivia sibling is `,`), Leading (container opener or first
    // child), Glued (after a code argument).
    let p_slot_classes = slot_classes_by_id(p);
    let c_slot_classes = slot_classes_by_id(c);
    let mut c_iter = c_children.into_iter();
    for p_child in p_children.drain(..) {
        if is_trivia_kind(p_child.node.kind()) {
            // Slot: the next source child must be a comment sitting in the
            // SAME slot class the pattern's comment occupies.
            let p_class = p_slot_classes
                .get(&p_child.node.id())
                .copied()
                .unwrap_or(SlotClass::Glued);
            match c_iter.next() {
                Some(c_child) if is_trivia_kind(c_child.node.kind()) => {
                    let c_class = c_slot_classes
                        .get(&c_child.node.id())
                        .copied()
                        .unwrap_or(SlotClass::Glued);
                    if c_class != p_class {
                        return false;
                    }
                    continue;
                }
                _ => return false,
            }
        }
        // Code child: source comments NOT demanded by a pending slot stay
        // invisible (a comment-free pattern still matches comment-carrying
        // sources), so skip them here; a slot above never skips, which is
        // what keeps the slotted alignment positional.
        let mut c_child = match c_iter.next() {
            Some(c_child) => c_child,
            None => return false,
        };
        while is_trivia_kind(c_child.node.kind()) {
            c_child = match c_iter.next() {
                Some(c_child) => c_child,
                None => return false,
            };
        }
        if p_child.field != c_child.field {
            return false;
        }
        if !literal_structural_eq(p_child.node, pattern_doc, c_child.node, source, false) {
            return false;
        }
    }
    // Source children left over (after trailing-comment strip) block.
    c_iter.next().is_none()
}

/// Raw-sibling slot class of every comment child of a container.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotClass {
    Leading,
    PostComma,
    Glued,
}

pub(crate) fn slot_classes_by_id(container: Node) -> std::collections::HashMap<usize, SlotClass> {
    let mut classes = std::collections::HashMap::new();
    let mut cursor = container.walk();
    let mut prev_non_trivia: Option<String> = None;
    for child in container.children(&mut cursor) {
        if is_trivia_kind(child.kind()) {
            let class = match prev_non_trivia.as_deref() {
                Some(",") => SlotClass::PostComma,
                None | Some("(") => SlotClass::Leading,
                _ => SlotClass::Glued,
            };
            classes.insert(child.id(), class);
        } else {
            prev_non_trivia = Some(child.kind().to_string());
        }
    }
    classes
}

/// R3 comment-guard scope: the grammar containers whose comment children are
/// invisible trivia. Kind-string check; exotic containers could mis-guard,
/// which is exactly what the root-comment probe pins. Known
/// grammar-attachment variance (scoped residual, documented in the ledger):
/// the reference's pinned python fork attaches a pre-first-argument comment
/// at the CALL node (so the reference blocks that face) while the vendored
/// tree-sitter-python attaches it inside `argument_list` (so this guard
/// skips it and the face matches). The attachment rule itself is faithful;
/// blocking that face positionally would break the reference's rust face
/// (`calc(/* lead */ 1, 2)` matches) which our tree also attaches inside
/// the container.
pub(crate) fn is_argument_container_kind(kind: &str) -> bool {
    kind.contains("argument") || kind.contains("parameters")
}

pub(crate) fn has_comment_child(node: &Node) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| is_trivia_kind(child.kind()));
    found
}

pub(crate) struct ComparableChild<'a> {
    field: Option<&'a str>,
    node: Node<'a>,
}

/// Children aligned for comparison: inside containers, comment trivia and
/// separator/trailing commas are dropped from BOTH sides (comma significance
/// is enforced separately by `has_trailing_comma`); grammar field names are
/// captured at their raw child index so alignment survives the filtering.
pub(crate) fn comparable_children<'a>(
    node: Node<'a>,
    skip_trivia: bool,
) -> Vec<ComparableChild<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .enumerate()
        .filter(|(_, child)| !skip_trivia || (!is_trivia_kind(child.kind()) && child.kind() != ","))
        .map(|(index, child)| ComparableChild {
            field: node.field_name_for_child(index as u32),
            node: child,
        })
        .collect()
}

/// True when the container's source carries a trailing comma: the last
/// significant child before any closing paren token (comments are
/// transparent) is `,`.
pub(crate) fn has_trailing_comma(node: &Node) -> bool {
    let mut cursor = node.walk();
    let mut kinds: Vec<&str> = node
        .children(&mut cursor)
        .map(|child| child.kind())
        .collect();
    while kinds.last().is_some_and(|kind| is_trivia_kind(kind)) {
        kinds.pop();
    }
    if kinds.last() == Some(&")") {
        kinds.pop();
        while kinds.last().is_some_and(|kind| is_trivia_kind(kind)) {
            kinds.pop();
        }
    }
    kinds.last() == Some(&",")
}
