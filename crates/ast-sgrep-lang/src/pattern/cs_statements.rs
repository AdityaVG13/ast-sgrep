//! C# statement-head lanes.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::Node;

// ===========================================================================
// The csharp statement-head lane: `fixed (R) { B }`, `checked { B }`,
// `unchecked { B }`, `unsafe { B }`. The heads are reserved words, so bare
// templates cannot parse at top level and the general lane cannot align
// them (its expression-statement shape never matches statement bodies).
//
//   * paren heads (fixed) with a META resource bind the whole resource text.
//   * a META BODY binds only under checked/unchecked/unsafe with a
//     ONE-statement candidate body; under `fixed` every candidate answers
//     valid-empty.
//   * concrete bodies align statement-wise with meta unification.
//   * NESTED head compositions BIND one match at the OUTERMOST statement,
//     each slot binding its own level; the one-statement law holds per
//     pattern body slot at every level.
//   * lock/using META bodies are accepted bind-nothing faces (walk-empty);
//     their concrete bodies stay on the general lane.
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CsStatementHead {
    Fixed,
    Checked,
    Unchecked,
    Unsafe,
    Lock,
    Using,
}

impl CsStatementHead {
    pub(crate) fn root_kind(self) -> &'static str {
        match self {
            CsStatementHead::Fixed => "fixed_statement",
            // tree-sitter-c-sharp has NO unchecked_statement — both spellings
            // root as `checked_statement` (the grammar's single
            // checked/unchecked kind); the anonymous HEAD KEYWORD token is
            // the discriminator and the reference's structural match
            // compares it, so csharp_statement_match verifies the keyword
            // text.
            CsStatementHead::Checked | CsStatementHead::Unchecked => "checked_statement",
            CsStatementHead::Unsafe => "unsafe_statement",
            // tree-sitter-c-sharp spells the lock head `lock_statement` (node-types).
            CsStatementHead::Lock => "lock_statement",
            CsStatementHead::Using => "using_statement",
        }
    }
    pub(crate) fn spell(self) -> &'static str {
        match self {
            CsStatementHead::Fixed => "fixed",
            CsStatementHead::Checked => "checked",
            CsStatementHead::Unchecked => "unchecked",
            CsStatementHead::Unsafe => "unsafe",
            CsStatementHead::Lock => "lock",
            CsStatementHead::Using => "using",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CsResource {
    Meta(String),
    Literal(String),
}

#[derive(Debug, Clone)]
pub(crate) enum CsBody {
    /// `{ $B }` — binds the candidate's single body statement (checked/
    /// unchecked/unsafe); under `fixed`/`lock`/`using` the bare-meta face
    /// answers valid-empty at ANY nesting depth.
    Meta(String),
    /// The body section is itself a head-statement spelling
    /// (`fixed ($D) { checked { $B } }`) — the reference binds one match
    /// at the OUTERMOST statement, every level's resource/meta binding its
    /// own capture (`$B` = the INNERMOST body's single statement). The
    /// accepted-empty law of `fixed`/`lock`/`using` holds only for a
    /// BARE-meta body at that level; a nested-head body under those heads
    /// BINDS. Served by recursing [`csharp_statement_bind`] into the
    /// candidate's single body statement — captures merge upward, the match
    /// emits at the root.
    Nested {
        inner: Box<CsStatementTemplate>,
        /// The PATTERN-side seam — bytes between the enclosing `{` and the
        /// nested head in the QUERY must be reference-class trivia; an
        /// outsider char refuses where the reference answers valid-empty.
        /// Symmetric with the candidate-side byte check in
        /// [`csharp_statement_bind`].
        seam_clean: bool,
    },
    /// Concrete statement text with optional canonical metas — substituted,
    /// parsed in a method-body context, aligned statement-wise.
    Template {
        substituted: String,
        placeholders: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct CsStatementTemplate {
    head: CsStatementHead,
    resource: Option<CsResource>,
    body: CsBody,
}

/// The csharp statement-template parser. Spelling-level admission only: head
/// keyword, balanced resource parens (fixed), balanced braces, and the body
/// grammar (bare meta / meta-free-of-noncanonical-substitution statement
/// text). The per-language truth is the walk.
pub(crate) fn csharp_statement_template(pattern: &str) -> Option<CsStatementTemplate> {
    let p = pattern.trim();
    let head = match p.split_whitespace().next()? {
        "fixed" => CsStatementHead::Fixed,
        "checked" => CsStatementHead::Checked,
        "unchecked" => CsStatementHead::Unchecked,
        "unsafe" => CsStatementHead::Unsafe,
        "lock" => CsStatementHead::Lock,
        "using" => CsStatementHead::Using,
        _ => return None,
    };
    let mut rest = p[head.spell().len()..].trim_start();
    let resource = if matches!(
        head,
        CsStatementHead::Fixed | CsStatementHead::Lock | CsStatementHead::Using
    ) {
        let inner = rest.strip_prefix('(')?;
        let close = balanced_paren_close(inner)?;
        let section = inner[..close].trim();
        rest = inner[close + 1..].trim_start();
        if let Some(name) = capture_name(section) {
            Some(CsResource::Meta(name.to_string()))
        } else if !section.is_empty() {
            Some(CsResource::Literal(section.to_string()))
        } else {
            return None;
        }
    } else {
        None
    };
    let inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(inner)?;
    // Nothing but trivia may follow the body braces.
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    let section = inner[..close].trim();
    let body = if let Some(name) = capture_name(section) {
        CsBody::Meta(name.to_string())
    } else if let Some(nested) = csharp_statement_template(section) {
        // A nested statement-head composition binds at the nested level too,
        // so the body recurses into its own template instead of refusing.
        // This recursion is PATTERN-controlled — one frame per nested
        // head-brace group in the QUERY string. No depth bound is enforced:
        // any bound is a refusal claim that would need its own conformance
        // evidence first (a wrong bound trades a remote-DoS surface for
        // silent under-serve). Hostile query strings remain a documented
        // exposure of this lane.
        // Capture the pattern-side seam class — the lead bytes before the
        // section are trivia-clean or the bind refuses.
        let section_raw = &inner[..close];
        let lead = &section_raw[..section_raw.len() - section_raw.trim_start().len()];
        let seam_clean = lead.chars().all(is_sg_cs_trivia);
        CsBody::Nested {
            inner: Box::new(nested),
            seam_clean,
        }
    } else {
        // A concrete Template body parses for fixed/checked/unchecked/unsafe
        // heads and binds exactly in the dedicated lane. lock/using keep
        // their contract: a placeholders-EMPTY (fully concrete) body stays
        // refused at parse — those spellings keep the general-lane route
        // (the dispatch above serves lock/using via the general template);
        // a lock/using template WITH placeholders is new-lane territory and
        // binds.
        let (substituted, placeholders, _) = substitute_general_metavariables(section)?;
        if placeholders.is_empty() && matches!(head, CsStatementHead::Lock | CsStatementHead::Using)
        {
            return None;
        }
        CsBody::Template {
            substituted,
            placeholders,
        }
    };
    Some(CsStatementTemplate {
        head,
        resource,
        body,
    })
}

/// The lane dispatch: parse the template once, walk, emit.
pub(crate) fn match_csharp_statement(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = csharp_statement_template(pattern)?;
    // The registered meta-body law for `fixed`/`lock`/`using` — the law
    // holds at EVERY nesting level (`unchecked { fixed ($D) { $B } }`,
    // `unsafe { lock ($L) { $B } }`, `fixed ($D1) { fixed ($D2) { $B } }`
    // all answer `[]`), and ONLY for the bare-meta body — a nested-head
    // body under those heads BINDS. The walk's empty IS the agreement for
    // the binds-nothing family.
    if cs_template_binds_nothing(&template) {
        return Some(Vec::new());
    }
    let tree = parse_source(Language::CSharp, source).ok()?;
    let mut out = Vec::new();
    walk_csharp_statement(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

/// True when any level of the head composition is a `fixed`/`lock`/`using`
/// head with a BARE-meta (`$B`) body — the accepted-and-binds-nothing class.
/// Nested-head bodies do NOT trigger it (they bind).
pub(crate) fn cs_template_binds_nothing(template: &CsStatementTemplate) -> bool {
    if matches!(template.body, CsBody::Meta(_))
        && matches!(
            template.head,
            CsStatementHead::Fixed | CsStatementHead::Lock | CsStatementHead::Using
        )
    {
        return true;
    }
    match &template.body {
        CsBody::Nested { inner: nested, .. } => cs_template_binds_nothing(nested),
        _ => false,
    }
}

pub(crate) fn walk_csharp_statement(
    node: Node,
    source: &str,
    pattern: &str,
    template: &CsStatementTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == template.head.root_kind()
        && node.is_named()
        && !is_in_comment_or_string(&node)
    {
        if let Some(hits) = csharp_statement_match(&node, source, pattern, template) {
            out.extend(hits);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_csharp_statement(child, source, pattern, template, out);
    }
}

pub(crate) fn csharp_statement_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &CsStatementTemplate,
) -> Option<Vec<PatternMatch>> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    csharp_statement_bind(node, source, template, &mut captures)?;
    let mut out = Vec::new();
    push_match_with_captures(node, source, pattern, captures, &mut out);
    Some(out)
}

/// The per-candidate BIND half — head-keyword token check, resource bind,
/// body bind — mutating one shared capture map so the Nested arm can recurse
/// into the candidate's single body statement and merge every level's
/// captures (the reference emits ONE match at the OUTERMOST statement:
/// `$D`=outer resource, `$B`=innermost body statement).
pub(crate) fn csharp_statement_bind(
    node: &Node,
    source: &str,
    template: &CsStatementTemplate,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let mut cursor = node.walk();
    let named: Vec<Node> = node
        .children(&mut cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    // The head keyword must agree token-exactly (unchecked/checked share one
    // root kind; the reference's structural match compares the anonymous
    // keyword).
    {
        let mut kw_cursor = node.walk();
        let head_text = node
            .children(&mut kw_cursor)
            .find(|child| !is_trivia_kind(child.kind()) && !child.is_extra())
            .and_then(|child| node_text(&child, source));
        if head_text.map(str::trim) != Some(template.head.spell()) {
            return None;
        }
    }
    // tree-sitter-c-sharp shapes the head as keyword + parens (anonymous)
    // around the resource declaration and the body block, so the named
    // children are exactly [resource?, block].
    let body = named
        .iter()
        .copied()
        .find(|c| BLOCK_KINDS.contains(&c.kind()))?;
    let resource = match template.resource.as_ref() {
        None => None,
        Some(res) => Some((
            res,
            named
                .iter()
                .copied()
                .find(|c| !BLOCK_KINDS.contains(&c.kind()))?,
        )),
    };
    if let Some((res, res_node)) = resource.as_ref() {
        let Some(text) = node_text(res_node, source) else {
            return None;
        };
        let ok = match res {
            CsResource::Meta(name) => bind_capture(captures, name, text.trim()).is_some(),
            CsResource::Literal(lit) => text.trim() == lit,
        };
        if !ok {
            return None;
        }
    }
    let mut body_cursor = body.walk();
    let stmts: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    match template.body.clone() {
        CsBody::Meta(name) => {
            // Reference law: `{ $B }` binds a ONE-statement body's
            // statement text (`checked { $B }` → B=`int v = a + b;`).
            let [only] = stmts.as_slice() else {
                return None;
            };
            let text = node_text(only, source)?;
            if bind_capture(captures, &name, text).is_none() {
                return None;
            }
        }
        CsBody::Nested {
            inner: nested,
            seam_clean,
        } => {
            // The seam gate is symmetrical — a pattern-side outsider char
            // refuses (valid-empty) exactly as the candidate-side check below
            // refuses.
            if !seam_clean {
                return None;
            }
            // The pattern body is itself a head statement — the candidate
            // body must be exactly ONE statement of the nested head's root
            // kind; the bind recurses (head token check, resource bind, body
            // bind) and captures merge upward. The nested-head seam is
            // TRIVIA-TIGHT — a comment between the enclosing body's `{` and
            // the nested-head statement refuses the match, while comments
            // before the outer head, inside the innermost body, and after
            // the inner close keep binding. Byte-tight check: only
            // whitespace may sit between the body's open brace and the
            // nested statement.
            let [only] = stmts.as_slice() else {
                return None;
            };
            // The seam trivia class of record — the cs grammar extras are
            // /[\s\u00A0\uFEFF\u3000]+/ where that `\s` is ASCII-scoped
            // ([\t\n\v\f\r ]), so the class is ASCII whitespace (incl.
            // vertical tab U+000B, which Rust's `is_ascii_whitespace`
            // excludes) PLUS the explicit members U+00A0 (NBSP), U+FEFF,
            // U+3000, while U+0085/U+2028/U+202F REFUSE. Comments still
            // refuse (any comment byte is outside the class); CRLF/formfeed
            // controls keep binding.
            let seam = &source[body.start_byte() + 1..only.start_byte()];
            if seam.chars().any(|c| {
                !(matches!(
                    c,
                    '\t' | '\n'
                        | '\u{000B}'
                        | '\u{000C}'
                        | '\r'
                        | ' '
                        | '\u{00A0}'
                        | '\u{FEFF}'
                        | '\u{3000}'
                ))
            }) {
                return None;
            }
            if only.kind() != nested.head.root_kind()
                || !only.is_named()
                || is_in_comment_or_string(only)
            {
                return None;
            }
            csharp_statement_bind(only, source, &nested, captures)?;
        }
        CsBody::Template {
            substituted,
            placeholders,
        } => {
            let doc = format!("class __AsgrepCtx {{ void M() {{ {substituted} }} }}");
            let prefix_len = "class __AsgrepCtx { void M() { ".len();
            let tpl_tree = parse_source(Language::CSharp, &doc).ok()?;
            if tpl_tree.root_node().has_error() {
                return None;
            }
            // The tree moves into the template first; every node borrows
            // from the template afterwards (Node lifetimes are tied to the
            // owned Tree).
            let tpl = GeneralTemplate {
                doc,
                tree: tpl_tree,
                placeholders,
                multi_names: BTreeSet::new(),
                span: None,
                had_semi: false,
                root_kind: String::new(),
                force_empty: false,
            };
            let mut span_node = tpl
                .tree
                .root_node()
                .descendant_for_byte_range(prefix_len, prefix_len + substituted.len())?;
            while span_node.kind() != "block" {
                span_node = span_node.parent()?;
            }
            let tpl_stmts: Vec<Node> = {
                let mut tpl_cursor = span_node.walk();
                span_node
                    .children(&mut tpl_cursor)
                    .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
                    .collect()
            };
            if tpl_stmts.len() != stmts.len() {
                return None;
            }
            for (p_stmt, c_stmt) in tpl_stmts.iter().zip(stmts.iter()) {
                if general_eq(&tpl, *p_stmt, *c_stmt, source, captures).is_none() {
                    return None;
                }
            }
        }
    }
    Some(())
}

/// The `return (META)` family lane (js/ts). The reference answers
/// `return ($X)` and `return($X)` on `return (1);` binding X=`1` — the
/// parenthesized operand's inner text; the operand-less `return;` and
/// paren-free operand candidates never align (structural). The Call
/// classification of the spelling (callee `return`) never fires — a source
/// can never spell a callee `return` — so this lane serves the family
/// exactly and every other return face keeps its routes.
pub(crate) fn match_return_paren_meta(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let mut p = pattern.trim();
    if let Some(stripped) = p.strip_suffix(';') {
        p = stripped.trim_end();
    }
    let rest = p.strip_prefix("return")?.trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    let name = capture_name(inner[..close].trim())?;
    let tree = parse_source(lang, source).ok()?;
    let mut out = Vec::new();
    walk_return_paren_meta(tree.root_node(), source, pattern, name, &mut out);
    Some(out)
}

pub(crate) fn walk_return_paren_meta(
    node: Node,
    source: &str,
    pattern: &str,
    name: &str,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "return_statement" && !is_in_comment_or_string(&node) {
        let mut cursor = node.walk();
        let named: Vec<Node> = node
            .children(&mut cursor)
            .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
            .collect();
        if let [operand] = named.as_slice() {
            if operand.kind() == "parenthesized_expression" {
                if let Some(inner_expr) = operand.named_child(0) {
                    let mut captures = BTreeMap::new();
                    if let Some(whole) = node_text(&node, source) {
                        captures.insert("MATCH".to_string(), whole.to_string());
                    }
                    if let Some(text) = node_text(&inner_expr, source) {
                        if bind_capture(&mut captures, name, text).is_some() {
                            push_match_with_captures(&node, source, pattern, captures, out);
                        }
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_return_paren_meta(child, source, pattern, name, out);
    }
}

/// Bare statement heads (`break`, `continue`, `throw`, `yield`, `raise`,
/// with or without `;`) are KIND templates: answers every statement of the
/// head's family regardless of arguments or semicolon.
/// `Some` = the lane engaged; `None` = not a bare-head pattern.
pub(crate) fn match_bare_statement_kind(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let trimmed = pattern.trim();
    if trimmed.contains(char::is_whitespace) || trimmed.contains('$') {
        return None;
    }
    let keyword = trimmed.trim_end_matches(';').trim();
    // Ruby has no `raise_statement` kind — raises are `call` nodes whose
    // method identifier the bare pattern matches at IDENTIFIER level. A
    // kind-level arm here can only silence or overmatch, and the full
    // literal lane would over-answer whitespace-led wrapper nodes whose
    // trimmed text equals the pattern. Identifiers only.
    if keyword == "raise" && lang == Language::Ruby {
        return Some(collect_identifier_matches(lang, source, pattern));
    }
    // Escaped statement-head spellings OUTSIDE their keyword grammars keep
    // the plain-identifier doctrine — the identifier faces answer, NEVER
    // the general lane's childless template (which silently answers nothing
    // for them). `go`/`defer` ride the full literal lane's keyword-token-leaf
    // route; `await` takes the identifier-faces-ONLY route (the token inside
    // an await_expression is never answered), and python's bare `await` is
    // the refusal (the literal lane would over-answer the `await g()`
    // token). TypeScript keeps its await_expression arm below.
    if matches!(keyword, "go" | "defer") && lang != Language::Go {
        return Some(match_literal_pattern(lang, source, pattern).unwrap_or_default());
    }
    if keyword == "await" {
        // Python's bare `await` binds only the bare keyword subtree: a bare
        // `await` token error-recovers to a plain `identifier`, while
        // `await <expr>` yields an `await`-kind node the identifier-shaped
        // pattern cannot match. Serve through the literal-lane keyword-token
        // route, refusing every hit whose span sits inside an
        // operand-bearing `await` node. The `;`-ful PATTERN spelling is
        // valid-empty on every fixture.
        if lang == Language::Python {
            if trimmed.ends_with(';') {
                return Some(Vec::new());
            }
            let mut hits = match_literal_pattern(lang, source, pattern).unwrap_or_default();
            retain_py_await_operand_free(source, &mut hits);
            return Some(hits);
        }
        if lang != Language::TypeScript {
            return Some(collect_identifier_matches(lang, source, pattern));
        }
    }
    let kinds: &[&str] = match keyword {
        // Ruby names the bare/control node `break` (both the bare and
        // command spellings share the name) — the statement/expression
        // kinds below do not exist in tree-sitter-ruby, so the arm
        // SILENCED the answered face instead of falling through to the
        // literal lane like arm-less `next` did. `break` as a kind name
        // exists in no other indexed grammar.
        "break" => &["break_statement", "break_expression", "break"],
        "continue" => &["continue_statement", "continue_expression"],
        "throw" => &["throw_statement"],
        "yield" => &["yield", "yield_statement"],
        "raise" => &["raise_statement"],
        // The js/ts debugger statement root — the reference answers the
        // debugger_statement for both the `;`-ful and bare spellings. SCOPED
        // to the two grammars where `debugger` is a keyword: elsewhere the
        // token is a plain identifier the literal lane must keep serving.
        "debugger" if matches!(lang, Language::JavaScript | Language::TypeScript) => {
            &["debugger_statement"]
        }
        // The bare `return` family needs explicit kinds arms: the general
        // lane's childless-template face would otherwise answer only the
        // operand-less form. Scoped per grammar (rust has no
        // return_statement — its rows sit at return_expression spans; ruby
        // names the node `return`; an arm whose kinds a grammar lacks
        // SILENCES the face). Other bare keywords fall through to the
        // literal lane, which answers the keyword-token leaf identically.
        // The `;`-ful js/ts `return;` answers ONLY operand-less statements,
        // while rust's `;` is not an operand marker; go's `defer`/`go` and
        // ts `await` join the statement arm (js `await` stays arm-less —
        // both engines answer nothing).
        "return"
            if matches!(
                lang,
                Language::JavaScript | Language::TypeScript | Language::Go
            ) =>
        {
            &["return_statement"]
        }
        "return" if lang == Language::Rust => &["return_expression"],
        "return" if lang == Language::Ruby => &["return"],
        "return" if lang == Language::Python => &["return_statement"],
        "defer" if lang == Language::Go => &["defer_statement"],
        "go" if lang == Language::Go => &["go_statement"],
        "await" if lang == Language::TypeScript => &["await_expression"],
        _ => return None,
    };
    // The `;`-ful js/ts return spelling demands an operand-less candidate
    // (the reference's empty-operand discipline). A trivia-carrying empty
    // return (`return /* c */;`) stays operand-less — comments are trivia.
    let require_operand_less = keyword == "return"
        && matches!(lang, Language::JavaScript | Language::TypeScript)
        && trimmed.ends_with(';');
    Some(collect_kind_matches_filtered(
        lang,
        source,
        pattern,
        kinds,
        require_operand_less,
    ))
}
