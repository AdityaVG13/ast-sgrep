//! Loop-head trivia discipline for the general lane.

use super::*;
use crate::Language;
use tree_sitter::Node;

/// Loop-head comment discipline: a loop-kind candidate is STRUCTURAL (refuse)
/// when its direct comment children sit in a refused zone:
/// - js/ts `for`/`for_in`: a comment refuses unless fully BEFORE the `(`
///   or TRAILING a named header element with the next non-comment sibling
///   a `;`/`)` terminator. Refused: after-`(`, after-`;` gaps, the `)`→`{`
///   junction, an init-declaration comment preceding a later sibling.
/// - js/ts `while`: transparent ONLY trailing the condition before `)`.
/// - js/ts `do`: comments BEFORE the body refuse; post-body stay transparent.
/// - go `for`: comments after the first clause element's start refuse;
///   head-leading trivia stays transparent.
/// - rs `for`/`while`: comments at/after the last header child refuse;
///   leading/mid-header trivia stays transparent.
/// - rs `loop`: any root-level comment refuses.
///
/// Non-loop kinds and comment-free candidates never fire.
pub(crate) fn loop_head_trivia_structural(lang: Language, node: &Node) -> bool {
    let kind = node.kind();
    let js_ts = matches!(lang, Language::JavaScript | Language::TypeScript);
    let loop_kind = match kind {
        "for_statement" | "for_in_statement" => js_ts || lang == Language::Go,
        "while_statement" | "do_statement" => js_ts,
        "for_expression" | "while_expression" | "loop_expression" => lang == Language::Rust,
        _ => false,
    };
    if !loop_kind {
        return false;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    // Comments are NAMED nodes in tree-sitter — exclude trivia here so the
    // per-arm first/last-element anchors are real code elements.
    let named: Vec<&Node> = children
        .iter()
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let comments: Vec<&Node> = children
        .iter()
        .filter(|child| is_trivia_kind(child.kind()))
        .collect();
    match kind {
        "for_statement" | "for_in_statement" => {
            if js_ts {
                // The discipline is PER-COMMENT-POSITION, not a whole-header
                // zone — tree-sitter surfaces trailing header comments at the
                // for ROOT (`i < n /* t */ ;`, `i++ /* u */ )`) and the
                // reference ANSWERS those (the for-of `xs /* of */ )`
                // junction included). A root comment is transparent iff (a)
                // it sits fully before the `(` (`for /* f */ (` — the
                // protected lead) or (b) it TRAILS a named header element
                // whose next non-comment sibling is the `;`/`)` terminator.
                // Comments after `(`, in the after-`;` gap, or between `)`
                // and the body refuse.
                if let Some(init) = named.first() {
                    if init.kind().contains("declaration") {
                        let mut init_cursor = init.walk();
                        let init_children: Vec<Node> = init.children(&mut init_cursor).collect();
                        for (index, child) in init_children.iter().enumerate() {
                            if is_trivia_kind(child.kind())
                                && init_children[index + 1..].iter().any(|later| {
                                    // The ASI/auto-semicolon tail is a
                                    // terminator, not a real sibling: a
                                    // TRAILING comment (`i = 0 /* s */ ;`)
                                    // stays transparent, a LEADING one
                                    // (`let /* c0 */ i`) refuses.
                                    !is_trivia_kind(later.kind()) && later.kind() != ";"
                                })
                            {
                                // `let /* c0 */ i` — the leading init
                                // declaration comment refuses.
                                return true;
                            }
                        }
                    }
                }
                let open = children
                    .iter()
                    .find(|child| !child.is_named() && child.kind() == "(");
                for c in &comments {
                    let lead = open.is_some_and(|open| c.end_byte() <= open.start_byte());
                    let trail = {
                        let idx = children
                            .iter()
                            .position(|x| x.id() == c.id())
                            .unwrap_or(children.len());
                        let prev = children[..idx]
                            .iter()
                            .rev()
                            .find(|x| !is_trivia_kind(x.kind()));
                        let next = children[idx + 1..]
                            .iter()
                            .find(|x| !is_trivia_kind(x.kind()));
                        prev.is_some_and(|p| p.is_named())
                            && next.is_some_and(|n| n.kind() == ";" || n.kind() == ")")
                    };
                    if !lead && !trail {
                        return true;
                    }
                }
                false
            } else {
                // go: (1) any root-level comment AT OR AFTER the first named
                // element refuses (junction + trailing); (2) a root comment
                // fully BEFORE a `for_clause` head refuses too — the
                // 3-clause form's `for /* h */ i := 0` is reference-refused
                // (the single-condition `for /* h */ n > 0` has no clause
                // and stays transparent); (3) for_clause comments are judged
                // unconditionally (they never surface on the for root):
                // transparent ONLY trailing a named element before its `;`
                // terminator (`i := 0 /* c */ ;`); the after-`;` gap
                // (`; /* g */ i < n`) and the post-update junction
                // (`i++ /* j */ {`) refuse.
                let clause_head = named.first().filter(|first| first.kind() == "for_clause");
                let lead_bad = comments
                    .iter()
                    .any(|c| clause_head.is_some_and(|cl| c.end_byte() <= cl.start_byte()));
                let after_bad = comments.iter().any(|c| {
                    named
                        .first()
                        .is_some_and(|first| c.end_byte() > first.start_byte())
                });
                after_bad
                    || lead_bad
                    || go_for_clause_comments_refused(named.first().copied().copied())
            }
        }
        "while_statement" => {
            // js/ts while: a root-level comment is transparent ONLY trailing
            // the condition before `)` (`n > 0 /* t */ )`, A=`n > 0`
            // clean); leading (`while /* w */ (`) and the `)`→`{`
            // junction refuse. (The mid-condition comment answers
            // with the comment INSIDE the capture text — inside-expression
            // alignment, untouched by this veto.)
            comments.iter().any(|c| {
                let idx = children
                    .iter()
                    .position(|x| x.id() == c.id())
                    .unwrap_or(children.len());
                let prev = children[..idx]
                    .iter()
                    .rev()
                    .find(|x| !is_trivia_kind(x.kind()));
                let next = children[idx + 1..]
                    .iter()
                    .find(|x| !is_trivia_kind(x.kind()));
                !(prev.is_some_and(|p| p.is_named()) && next.is_some_and(|n| n.kind() == ")"))
            })
        }
        "do_statement" => {
            // js/ts do: comments before the body refuse; post-body comments
            // (`} /* z */ while`) stay transparent. The body is the FIRST
            // named child (it directly follows `do`).
            comments.iter().any(|c| {
                named
                    .first()
                    .is_some_and(|b| c.end_byte() <= b.start_byte())
            })
        }
        "for_expression" | "while_expression" => {
            // rs: refuse comments at/after the last HEADER child (the range
            // expression / condition); leading and mid-header trivia stay
            // transparent.
            match named.len() {
                0 | 1 => !comments.is_empty(),
                _ => {
                    let header = named[named.len() - 2];
                    comments.iter().any(|c| c.end_byte() > header.end_byte())
                }
            }
        }
        "loop_expression" => {
            // rs loop: no header zone — any root-level comment refuses;
            // comment-free candidates never fire.
            !comments.is_empty()
        }
        _ => false,
    }
}

/// Go's `for a := 0; cond; update` clause wraps its header elements in a
/// named `for_clause` child, so clause-level comments never surface on the
/// for root — this predicate is consulted UNCONDITIONALLY for every go for
/// candidate. A clause comment is transparent ONLY trailing a named element
/// before its `;` terminator (`i := 0 /* c */ ;`); the after-`;` gap
/// (`; /* g */ i < n`), the head-leading 3-clause position (`for /* h */ i
/// := 0`), and the post-update junction (`i++ /* j */ {`) all refuse.
pub(crate) fn go_for_clause_comments_refused(clause: Option<Node>) -> bool {
    let Some(clause) = clause else {
        return false;
    };
    if clause.kind() != "for_clause" {
        return false;
    }
    let mut cursor = clause.walk();
    let children: Vec<Node> = clause.children(&mut cursor).collect();
    // A clause comment is transparent ONLY trailing a named element whose
    // next non-comment sibling is the `;` terminator (`i := 0 /* c */ ;`).
    // Everything else refuses: the after-`;` gap (`; /* g */ i < n`), the
    // head-leading 3-clause position (`for /* h */ i := 0`), and the
    // post-update junction (`i++ /* j */ {`), per the probed reference.
    children.iter().any(|c| {
        if !is_trivia_kind(c.kind()) {
            return false;
        }
        let idx = children
            .iter()
            .position(|x| x.id() == c.id())
            .unwrap_or(children.len());
        let prev = children[..idx]
            .iter()
            .rev()
            .find(|x| !is_trivia_kind(x.kind()));
        let next = children[idx + 1..]
            .iter()
            .find(|x| !is_trivia_kind(x.kind()));
        !(prev.is_some_and(|p| p.is_named()) && next.is_some_and(|n| n.kind() == ";"))
    })
}
