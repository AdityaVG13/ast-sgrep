//! General structural lane: matching and walks.

use super::*;
use crate::extract::{is_inside_comment_or_string, is_member_expr_kind, node_text};
use crate::Language;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{OnceLock, RwLock};
use tree_sitter::Node;

/// Process-wide support cache for `needs_ast_grep_fallback` (language-free
/// question: does ANY grammar build a template?).
pub(crate) fn general_lane_supported(pattern: &str) -> bool {
    static SUPPORTED: OnceLock<RwLock<HashMap<String, bool>>> = OnceLock::new();
    let cache = SUPPORTED.get_or_init(|| RwLock::new(HashMap::new()));
    let cached = cache.read().ok().and_then(|map| map.get(pattern).copied());
    if let Some(supported) = cached {
        return supported;
    }
    let supported = general_lane_supported_uncached(pattern);
    if let Ok(mut writer) = cache.write() {
        writer.insert(pattern.to_string(), supported);
    }
    supported
}

pub(crate) fn general_lane_supported_uncached(pattern: &str) -> bool {
    // The conditional-directive family is natively answerable when the
    // condition parses under the shared [`CondBinding`] rule — lone
    // metavariable, metavar-free text, a single metavariable nested in the
    // condition (`#if defined($A)` ANSWERS like the reference; the preproc
    // lane binds the nested metavar), or a parseable structural condition
    // template over 2+ canonical metavariables. `#define` joins through
    // [`parse_define_tail`]; non-directive patterns fall through untouched.
    if let Some(supported) = preproc_directive_supported(pattern) {
        return supported;
    }
    // The csharp statement-head lane is a language-free "some lane serves
    // this" admission for the ingress gate — the general template cannot root
    // these heads, so the any-language build loop below would report
    // unsupported and `needs_ast_grep_fallback` would refuse faces the
    // reference answers. The lane parse is spelling-level only; the walk +
    // census keep per-file/per-language honesty.
    if matches!(
        pattern.split_whitespace().next(),
        Some("fixed" | "checked" | "unchecked" | "unsafe" | "lock" | "using")
    ) && csharp_statement_template(pattern).is_some()
    {
        return true;
    }
    // The java class member-count lane is a language-free "some lane serves
    // this" admission for the ingress gate — the general template cannot
    // substitute a bare-meta class body (the starvation that kept the face
    // census-loud), so the fallback gate refuses faces the reference
    // answers. The lane parse is spelling-level only; the walk + census
    // keep per-file/per-language honesty.
    if classify_java_class_member_count(pattern).is_some() {
        return true;
    }
    // The directive roots, the remaining statement roots, the java
    // synchronized NESTED/METHOD faces, the php literal-name/`$$`-body
    // namespace faces, the csharp checked/unchecked expression root, and
    // go's `$`-carrying `;`-ful spellings — the language-free "some lane
    // serves this" admissions. The lane parses are spelling-level only;
    // the walk + census keep per-file/per-language honesty.
    if directive_template(pattern).is_some()
        || cs_expression_template(pattern).is_some()
        || kt_typealias_template(pattern).is_some()
        || kt_for_template(pattern).is_some()
        || swift_for_template(pattern).is_some()
        || rs_let_else_template(pattern).is_some()
        || c_goto_template(pattern).is_some()
        || ja_sync_block_nested_template(pattern).is_some()
        || ja_sync_method_template(pattern).is_some()
        || php_namespace_block_template(pattern).is_some()
        || sg_goto_semi_pattern(pattern)
        || sg_go_import_semi_pattern(pattern)
        // The del lane's `$$$`-slot faces ride the dedicated delete template
        // — the general lane's build refuses the multi-meta spelling and
        // would keep the answering face ingress-loud. The census arm stays
        // py-scoped, so other languages keep their loud class.
        || py_delete_meta_pattern(pattern)
    {
        return true;
    }
    if !general_lane_text_eligible(pattern) {
        return false;
    }
    let Some((substituted, placeholders, multi_names)) = substitute_general_metavariables(pattern)
    else {
        return false;
    };
    // Identifier soup (`$A $B`, `RETURN $A`) is not a structural template.
    // Registered statement-head templates (`return $A`) carry no
    // structural punctuation but are still structural — the statement-root
    // template lane decides them; uppercase/unregistered heads never reach
    // here (the bare-keyword guard refuses them first). The head check no
    // longer demands a `$` — the probed `$`-less faces
    // (`break`/`continue`) are statement templates too.
    let statement_head_template = pattern
        .split_whitespace()
        .next()
        .is_some_and(|head| STATEMENT_HEAD_KEYWORDS.contains(&head));
    // A whole-content string meta (`"$A"` — the parsed string-content leaf
    // IS the metavariable) carries no punctuation, but is a structural
    // template whose build the any-language loop below verifies (and whose
    // per-language root-kind admission + whole-content placeholder rule keep
    // `"pre-$A"`/`"$A$B"`/escaped spellings refused).
    let bare_string_meta_template = substituted.len() >= 2
        && substituted.starts_with('"')
        && substituted.ends_with('"')
        && placeholders.len() == 1
        && placeholders.contains_key(&substituted[1..substituted.len() - 1]);
    // A KEYWORD-OPERATOR template (`$X and $Y`, `$X or $Y`, `$X as $Y`,
    // `not $X`, `typeof $X`) carries no punctuation from the structural
    // set — the operator is an alphabetic keyword — but is a structural
    // template whose per-language build the any-language loop below
    // verifies (the root kinds land on the existing
    // `_operator`/`binary`/`_expression` admissions; the `not`/`typeof`
    // heads ride the head admissions above). The unary KEYWORD faces
    // (`delete $X`, `void $X`, `del $X`) join the same lane; their root
    // kinds are admitted in `is_general_root_kind`.
    let keyword_operator_template = !bare_string_meta_template
        && !placeholders.is_empty()
        && pattern.split_whitespace().any(|token| {
            matches!(
                token,
                "and" | "or" | "as" | "not" | "typeof" | "delete" | "void" | "del"
            )
        });
    // The ruby MODIFIER statement template (`x if $C`) carries no
    // structural punctuation but is structural — the rb modifier root kinds
    // are admitted in `is_general_root_kind` and the any-language loop below
    // verifies the build.
    let rb_modifier_template = !placeholders.is_empty() && {
        let tokens: Vec<&str> = pattern.split_whitespace().collect();
        tokens.len() >= 3
            && matches!(
                tokens[tokens.len() - 2],
                "if" | "unless" | "while" | "until"
            )
    };
    if !substituted.contains(|c: char| "()[]{}<>=:,.!?+-*/%&|;~^@#".contains(c))
        && !statement_head_template
        && !bare_string_meta_template
        && !keyword_operator_template
        && !rb_modifier_template
    {
        return false;
    }
    // A `#` whose tail carries NO code structure is a comment-glue tail
    // (`foo($A) # note`, `$A + $A# note`, the python `# tail` ctrl faces)
    // — the registered contract keeps those fail-closed at the
    // language-free ingress even though some grammars happily parse the `#`
    // tail as syntax. The `#`-as-SYNTAX faces (`#[derive($A)]`,
    // `#include $X`, `this.#x = $V`, `tag(r#"$A"#)`) all carry code
    // punctuation after their first `#` and stay templatable.
    if contains_comment_glued_hash(pattern) {
        return false;
    }
    // `#` is comment syntax only in python/ruby/php — the any-language
    // support answer must not count a hash-comment language templating a
    // pattern whose `#` is real syntax elsewhere (and vice versa: rust
    // attribute faces stay supported while python hash-glued faces keep
    // failing closed).
    Language::all().iter().any(|&lang| {
        !lane_comment_refused(lang, pattern)
            && build_general_template(lang, pattern, &substituted, &placeholders, &multi_names)
                .is_some()
    })
}

thread_local! {
    /// Per-thread general-lane template cache (template construction parses the
    /// pattern up to twice; the search walk pays it once per thread/pattern).
    static GENERAL_TEMPLATES: RefCell<HashMap<(Language, String), Option<GeneralTemplate>>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn cached_general_template(lang: Language, pattern: &str) -> Option<GeneralTemplate> {
    let key = (lang, pattern.trim().to_string());
    GENERAL_TEMPLATES.with(|cell| {
        let mut map = cell.borrow_mut();
        if let Some(template) = map.get(&key) {
            return template.clone();
        }
        let built = if general_lane_text_eligible_for(lang, pattern)
            && !lane_comment_refused(lang, pattern)
        {
            let (substituted, placeholders, multi_names) =
                substitute_general_metavariables(pattern)?;
            build_general_template(lang, pattern, &substituted, &placeholders, &multi_names)
        } else {
            None
        };
        map.insert(key.clone(), built.clone());
        built
    })
}

pub(crate) fn match_structural_general(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Vec<PatternMatch> {
    let Some(template) = cached_general_template(lang, pattern) else {
        return Vec::new();
    };
    // The csharp meta-body lock/using faces are ACCEPTED bind-nothing faces
    // — the walk's empty is the agreement, never a census concern.
    if template.force_empty {
        return Vec::new();
    }
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let root = general_template_root(&template);
    let mut out = Vec::new();
    if let Some(root) = root {
        // The byte-range dedup is a hash set, not a linear scan — the
        // scan made lone-metavar walks quadratic in the match count.
        // Insertion into `out` keeps emission order byte-for-byte.
        let mut seen = std::collections::HashSet::new();
        walk_general(
            tree.root_node(),
            source,
            pattern,
            &template,
            root,
            lang,
            &mut seen,
            &mut out,
        );
    }
    // The general lane is the member path the kt/swift DOTTED `$`-less
    // spellings ride (property chains `a.b.c`, plain calls `a.b(1)`) — the
    // dotted faces over-answered receiver-link trivia here because the
    // general lane had NO link-structural consult. Apply the union doctrine
    // PER-CANDIDATE (the candidate's own subtree only, so the answered
    // trivia-free inner link of a longer chain keeps answering), scoped to
    // the two grammars whose trivia doctrine is probe-verified.
    retain_member_link_trivia_free(lang, &tree, &mut out);
    // Per-candidate general-lane gates (the structural alignment skips
    // trivia/unnamed children and over-served these refusals):
    // * a cs candidate whose subtree carries an ERROR node holding a
    //   gap-junk char OUTSIDE the cs trivia class (U+2028/U+2029/U+0085)
    //   drops; in-class spellings keep binding, and the junk-transparent
    //   directive/throw/return faces ride their dedicated lanes.
    // * a cs call candidate whose callee→`(` junction carries a
    //   comment/U+2028/U+2029 run drops; FEFF/NBSP junctions bind and
    //   comment-inside-args stays outside the junction.
    // * a ts member-link candidate with a trivia child directly before the
    //   `?.` connector drops; the after-connector position keeps binding.
    if matches!(lang, Language::CSharp | Language::TypeScript | Language::Go) {
        out.retain(|hit| {
            node_with_span(tree.root_node(), hit.byte_start, hit.byte_end).is_none_or(|node| {
                match lang {
                    // The outsider-junk gate is scoped to the using-var HEAD
                    // — the parse recovers junk-ERROR inside the initializer
                    // and still binds the outer meta, so the subtree-wide
                    // scan over-refused it. The head gate now also refuses
                    // COMMENT children in the keyword→name head while
                    // comments after the name and at the initializer keep
                    // binding.
                    Language::CSharp => {
                        (node.kind() != "local_declaration_statement"
                            || cs_using_var_head_clean(&node, source))
                            && (!is_call_kind(node.kind())
                                || cs_call_junction_trivia_free(&node, source))
                    }
                    Language::TypeScript => !ts_member_link_receiver_trivia(&node),
                    // The reference refuses comments AND the junk class
                    // (U+2028/FEFF/A0) at the `for`/`go`/`defer`
                    // keyword→body junctions while clean junctions bind,
                    // argument comments bind, and `func` is NOT gated (the
                    // keyword→name refusal is per-keyword — the scope
                    // discipline).
                    Language::Go => {
                        // node_with_span can resolve to the enclosing
                        // `statement_list` sharing the hit's byte span
                        // (single-statement bodies); descend to the gated
                        // statement node before judging the junction.
                        match go_gate_target(&node) {
                            Some(n) => go_keyword_body_junction_clean(&n, source),
                            None => true,
                        }
                    }
                    _ => true,
                }
            })
        });
    }
    out
}

/// True when the bytes between the leading `for`/`go`/`defer` keyword token
/// and the first non-trivia child are comment-free ASCII whitespace — the
/// law at these junctions, scoped to the three covered keywords.
/// The gated statement node for a general-lane go hit: the hit node itself
/// or a same-span descendant of the three gated kinds.
pub(crate) fn go_gate_target<'tree>(node: &Node<'tree>) -> Option<Node<'tree>> {
    if matches!(
        node.kind(),
        "for_statement" | "go_statement" | "defer_statement"
    ) {
        return Some(*node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() == node.start_byte() && child.end_byte() == node.end_byte() {
            if let Some(found) = go_gate_target(&child) {
                return Some(found);
            }
        }
    }
    None
}

pub(crate) fn go_keyword_body_junction_clean(node: &Node, source: &str) -> bool {
    // The reference refuses the head comment on the CLAUSE forms
    // (range/3-clause — first named child is a `for_clause`) while the
    // while-form's head-leading comment stays transparent
    // (`for /* h */ n > 0 {` binds, A=`n > 0`).
    if node.kind() == "for_statement" {
        let mut cursor = node.walk();
        let first_named = node
            .children(&mut cursor)
            .find(|c| c.is_named() && !is_trivia_kind(c.kind()) && !c.is_extra());
        if !matches!(
            first_named.map(|c| c.kind()),
            Some("for_clause") | Some("range_clause")
        ) {
            return true;
        }
    }
    let mut cursor = node.walk();
    let mut children = node.children(&mut cursor);
    let Some(kw) = children.next() else {
        return true;
    };
    if kw.is_named() || !matches!(kw.kind(), "for" | "go" | "defer") {
        return true;
    }
    let Some(first) = children.find(|child| !is_trivia_kind(child.kind()) && !child.is_extra())
    else {
        return true;
    };
    let gap = &source[kw.end_byte()..first.start_byte()];
    !gap.contains("//") && !gap.contains("/*") && gap.chars().all(|c| c.is_ascii_whitespace())
}

/// The using-var head gate — the head spans from the statement start to the
/// declarator NAME's first byte. The reference refuses comments and
/// outsider-junk ERROR runs there while binding everything after the name
/// and every junk position INSIDE the initializer (junk retained in the
/// capture).
pub(crate) fn cs_using_var_head_clean(node: &Node, source: &str) -> bool {
    // The declarator name is the first `identifier` named descendant
    // (`using var s` -> variable_declarator -> identifier; plain `int x`
    // decls share the shape).
    fn first_identifier<'tree>(node: &Node<'tree>) -> Option<Node<'tree>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() && !is_trivia_kind(child.kind()) && child.kind() == "identifier" {
                return Some(child);
            }
            if let Some(found) = first_identifier(&child) {
                return Some(found);
            }
        }
        None
    }
    let Some(name) = first_identifier(node) else {
        return true;
    };
    let head_end = name.start_byte();
    // The reference refuses comments and outsider-junk ERROR runs ANYWHERE
    // in the keyword->name head — the comment extras attach BELOW the
    // statement node, so the scan is recursive. After the name everything
    // binds, junk retained.
    fn head_scan(node: &Node, source: &str, head_end: usize) -> bool {
        if node.start_byte() >= head_end {
            return true; // pre-order: nothing later can overlap the head
        }
        if node.end_byte() > node.start_byte() && node.start_byte() < head_end {
            if node.is_extra() || node.kind() == "comment" {
                return false;
            }
            if node.kind() == "ERROR" && !cs_subtree_outsider_junk_free(*node, source) {
                return false;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !head_scan(&child, source, head_end) {
                return false;
            }
        }
        true
    }
    head_scan(node, source, head_end)
}

/// True when a trivia child sits directly inside a member link before the
/// `?.` connector — the anonymous token or the named `optional_chain`
/// wrapper — the ts-only refusal position (the junction doctrine's
/// general-lane twin).
pub(crate) fn ts_member_link_receiver_trivia(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (index, child) in children.iter().enumerate() {
        if is_trivia_kind(child.kind()) || child.is_extra() {
            let next = children[index + 1..]
                .iter()
                .find(|next| !is_trivia_kind(next.kind()) && !next.is_extra());
            if next.is_some_and(|next| {
                (next.is_named() && next.kind().contains("optional"))
                    || (!next.is_named() && next.kind() == "?.")
            }) {
                return true;
            }
        }
    }
    false
}

/// True when no ERROR node in the subtree carries a candidate gap-junk char
/// outside the pattern-side cs trivia class (see the [`is_sg_cs_trivia`] /
/// [`is_sg_cs_gap_junk`] seam distinction; the OUTSIDER set is the junk that
/// is neither reference-class trivia nor plain ASCII whitespace —
/// U+2028/U+2029/U+0085/U+1680/…, the spellings the cs parse refuses at token
/// gaps).
pub(crate) fn cs_subtree_outsider_junk_free(node: Node, source: &str) -> bool {
    if node.is_error() {
        if let Some(text) = node_text(&node, source) {
            if text
                .chars()
                .any(|c| is_sg_cs_gap_junk(c) && !is_sg_cs_trivia(c) && !c.is_ascii_whitespace())
            {
                return false;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !cs_subtree_outsider_junk_free(child, source) {
            return false;
        }
    }
    true
}

/// The general-lane arm for `?.`-chain PROPERTY faces outside
/// {TypeScript, JavaScript}, GATED with the ONE chain rules. The
/// byte-identical preservation arm over-answered kotlin/swift
/// junction-comment and kotlin mid-chain/receiver-trivia faces where the
/// reference refuses. Every candidate the general lane matched must be a
/// chain whose EVERY consumed call level has an exact callee→`(` junction
/// and whose member links carry no trivia child; statement-level trivia
/// BEFORE the chain attaches outside the matched node and keeps answering.
pub(crate) fn match_structural_chain_gated(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Vec<PatternMatch> {
    let mut out = match_structural_general(lang, source, pattern);
    if out.is_empty() {
        return out;
    }
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let root = tree.root_node();
    out.retain(|m| chain_candidate_exact(root, m.byte_start, m.byte_end, lang));
    out
}

/// The chain-exactness gate for one general-lane candidate — the candidate
/// node (matched at its exact byte span) descends through callee/receiver
/// hops; every call level must pass [`call_junction_exact`] and every
/// member link must be trivia-free. Each hop lands on a strictly smaller
/// child span, so the walk terminates at the chain root. A call hops to
/// its CALLEE first so the member branch trivia-checks the callee link
/// itself. Swift's WRAPPED links let the kt-shaped later-ANONYMOUS rule
/// pass vacuously on this lane (the connector lives inside a NAMED
/// navigation_suffix), so swift — and kt's dotted links — consult the
/// union [`member_link_trivia_structural`] doctrine here too.
pub(crate) fn chain_candidate_exact(root: Node, start: usize, end: usize, lang: Language) -> bool {
    let Some(node) = node_with_span(root, start, end) else {
        return false;
    };
    let mut cursor = node;
    loop {
        if is_call_kind(cursor.kind()) {
            if !call_junction_exact(&cursor) {
                return false;
            }
            match call_field_node(&cursor) {
                Some(next) => cursor = next,
                None => return true,
            }
        } else if is_member_expr_kind(cursor.kind()) {
            let link_structural = if matches!(lang, Language::Swift | Language::Kotlin) {
                member_link_trivia_structural(&cursor)
            } else {
                member_link_has_trivia(&cursor)
            };
            if link_structural {
                return false;
            }
            match member_receiver(&cursor) {
                Some(next) => cursor = next,
                None => return true,
            }
        } else {
            return true;
        }
    }
}

/// Whether a kotlin member link carries link-STRUCTURAL trivia: a trivia
/// child with ANY later anonymous sibling (the `.`/`?.` connector token on
/// every observed face — kotlin-ng flattens `navigation_suffix`, so a
/// post-`?.` comment is a direct link child). Trivia followed only by NAMED
/// siblings sits in the callee-internal position past the link's own
/// connector — transparent. The one shared gate for both kotlin optional
/// decomposers, so a future grammar edit cannot split their conditions.
pub(crate) fn kt_link_structural(lang: Language, node: &Node) -> bool {
    lang == Language::Kotlin && member_link_has_trivia(node)
}

pub(crate) fn member_link_has_trivia(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (index, child) in children.iter().enumerate() {
        if !is_trivia_kind(child.kind()) && !child.is_extra() {
            continue;
        }
        if children[index + 1..].iter().any(|later| !later.is_named()) {
            return true;
        }
    }
    false
}

/// The SWIFT member-link trivia doctrine, same position rule as the kotlin
/// consults but read against swift's WRAPPED links (`a /*c*/ .b(1)` hangs
/// the comment off the navigation_expression; `a. /*c*/ b(1)` puts it
/// INSIDE the navigation_suffix after its `.`). Refuse every candidate
/// whose comment sits BEFORE a later navigation_suffix — receiver-side,
/// mid-link, and doubled alike — while the callee-INTERNAL position
/// (inside the suffix, after the `.`) stays transparent. Swift never
/// enters the kotlin decomposers, so the plain-call and chain lanes consult
/// THIS predicate: any navigation node in the callee subtree carrying a
/// direct trivia/extra child with a LATER navigation_suffix sibling is
/// link-STRUCTURAL → refuse.
pub(crate) fn swift_member_link_structural(node: &Node) -> bool {
    let kind = node.kind();
    if is_navigation_suffix_kind(kind) || kind == "navigation_expression" {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for (index, child) in children.iter().enumerate() {
            if !is_trivia_kind(child.kind()) && !child.is_extra() {
                continue;
            }
            if children[index + 1..]
                .iter()
                .any(|later| is_navigation_suffix_kind(later.kind()))
            {
                return true;
            }
        }
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    children
        .iter()
        .any(|child| swift_member_link_structural(child))
}

/// The UNION member-link trivia doctrine, RECURSIVE over the candidate
/// subtree — at every node, a direct COMMENT-kind trivia child (extras
/// excluded — error-glue recovery rows are extras the reference answers
/// through) with a later ANONYMOUS sibling (flat `?.` links) or a later
/// `navigation_suffix` sibling (WRAPPED dotted links) is link-STRUCTURAL.
/// The two one-level predicates alone miss the nested shapes the
/// per-candidate retain sees (the trivia lives one link down inside the
/// candidate span). Positional scope is unchanged: callee-internal trivia
/// (inside the suffix, after the `.`) and tail trivia (outside the
/// candidate span) never fire.
pub(crate) fn member_link_trivia_structural(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (index, child) in children.iter().enumerate() {
        // COMMENT-kind trivia only: the registered doctrine is comment
        // trivia. Swift's error-glue EXTRA fragments are extras the
        // reference ANSWERS through — folding `is_extra` in here
        // over-refused them (swift faces, caught by the suite).
        if !is_trivia_kind(child.kind()) {
            continue;
        }
        if children[index + 1..]
            .iter()
            .any(|later| !later.is_named() || is_navigation_suffix_kind(later.kind()))
        {
            return true;
        }
    }
    children
        .iter()
        .any(|child| member_link_trivia_structural(child))
}

/// The node whose byte span is exactly `(start, end)`, descending through
/// the children that contain the span.
pub(crate) fn node_with_span<'a>(node: Node<'a>, start: usize, end: usize) -> Option<Node<'a>> {
    if node.start_byte() == start && node.end_byte() == end {
        return Some(node);
    }
    if node.child_count() == 0 || node.start_byte() > start || node.end_byte() < end {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() <= start && child.end_byte() >= end {
            if let Some(found) = node_with_span(child, start, end) {
                return Some(found);
            }
        }
    }
    None
}

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

// Matcher-lane shape: (node/source/pattern/template/...) is threaded
// deliberately; bundling would churn every lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_general<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    template: &GeneralTemplate,
    template_root: Node<'a>,
    lang: Language,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    // The guard is ANCESTOR-only — a string-rooted template must match the
    // string node itself, while everything nested inside a comment/string
    // ancestor stays skipped (calls inside `#{…}` for other templates,
    // string_content, …).
    if node.kind() == template.root_kind && !is_inside_comment_or_string(&node) {
        // The loop-head comment discipline — position-scoped refusals on
        // trivia-carrying loop candidates (junction comments, leading
        // init/while comments, go/rs clause zones). Clean and
        // transparent-position candidates are untouched, so the unwalled
        // cases keep their bindings.
        if loop_head_trivia_structural(lang, &node) {
            return walk_children_general(
                node,
                source,
                pattern,
                template,
                template_root,
                lang,
                seen,
                out,
            );
        }
        // The exact-children junction doctrine covers the after-`>` gap of
        // type-args-spelled calls — a trivia child fully inside the
        // typeargs→arguments gap refuses the candidate (descent-only),
        // whitespace-only gaps stay admitted.
        if ts_typeargs_junction_refused(lang, template_root, &node, source) {
            return walk_children_general(
                node,
                source,
                pattern,
                template,
                template_root,
                lang,
                seen,
                out,
            );
        }
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
        }
        if general_eq(template, template_root, node, source, &mut captures).is_some() {
            let byte_start = node.start_byte();
            let byte_end = node.end_byte();
            if seen.insert((byte_start, byte_end)) {
                out.push(hit_for_node(&node, source, pattern, captures));
            }
        }
    }
    walk_children_general(
        node,
        source,
        pattern,
        template,
        template_root,
        lang,
        seen,
        out,
    );
}

/// The descend-only tail of [`walk_general`], shared by the refused-candidate
/// path (a vetoed loop candidate still descends for nested candidates).
// Matcher-lane shape: (node/source/pattern/template/...) is threaded
// deliberately; bundling would churn every lane for no behavior gain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_children_general<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    template: &GeneralTemplate,
    template_root: Node<'a>,
    lang: Language,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_general(
            child,
            source,
            pattern,
            template,
            template_root,
            lang,
            seen,
            out,
        );
    }
}

/// First-order structural comparison of a pattern subtree against a candidate
/// subtree. A pattern node whose ENTIRE text is a substituted metavariable
/// binds the whole candidate node (reference semantics — this is how a TS
/// `required_parameter` spelled `$B` binds the full `param: type` text).
pub(crate) fn general_eq<'p>(
    template: &GeneralTemplate,
    p: Node<'p>,
    c: Node<'p>,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    if let Some(text) = node_text(&p, &template.doc) {
        if let Some(name) = template.placeholders.get(text.trim()) {
            let bound = node_text(&c, source)?;
            return bind_capture(captures, name, bound);
        }
    }
    if p.kind() != c.kind() {
        return None;
    }
    // Leaf tokens (literals, operators, identifiers, punctuation) must carry
    // the exact same text on both sides — `foo($X + 1)` must not match
    // `foo(x + 2)`.
    if p.child_count() == 0 {
        let expected = node_text(&p, &template.doc)?;
        let actual = node_text(&c, source)?;
        return (expected == actual).then_some(());
    }
    let mut p_children: Vec<Node> = {
        let mut cursor = p.walk();
        p.children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            .collect()
    };
    let mut c_children: Vec<Node> = {
        let mut cursor = c.walk();
        c.children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            // Tree-sitter marks in-file parse-error children (ERROR,
            // is_extra=true) `extra`, and Smart strictness skips candidate
            // extras during child alignment. The error-glued identifiers
            // recovered as `identifier + ERROR($A) + …` therefore align on
            // the NAMED children only and the meta binds the identifier
            // FRAGMENT. Keeping the extra here cost it an alignment slot
            // (count mismatch → silently unanswered). The pattern side is
            // NOT skipped — Smart never skips goal-side nodes either.
            .filter(|child| !child.is_extra())
            .collect()
    };
    // Statement-terminator lenience. Templates parse at doc roots where
    // ASI-tolerant grammars omit the `;`, or carry it via the context
    // suffix; candidate statements spell whichever the source used. A lone
    // anonymous `;` tail difference is a termination artifact, not a
    // structural difference. The lenience is ONE-DIRECTIONAL — only the
    // CANDIDATE's terminator is an ASI artifact; a PATTERN-trailing `;`
    // stays significant, so a pattern that spells it never pops it.
    if p_children.last().is_some_and(|n| n.kind() != ";")
        && c_children.last().is_some_and(|n| n.kind() == ";")
    {
        c_children.pop();
    }
    // A LONE metavariable argument binds the whole argument list when the
    // source command carries several (`raise $A` on
    // `raise ArgumentError, 'bad'`). Only argument containers take this —
    // call patterns are served by the call lane, whose arity contract is
    // untouched.
    if p_children.len() == 1 && c.kind().contains("argument") && c_children.len() > 1 {
        if let Some(text) = node_text(&p_children[0], &template.doc) {
            if let Some(name) = template.placeholders.get(text.trim()) {
                let bound = node_text(&c, source)?;
                return bind_capture(captures, name, bound);
            }
        }
    }
    // Sole-rest argument lists. A `$$$NAME` spelled as the SOLE argument of
    // its call (the only shape [`nested_call_rest_template`] admits into
    // this lane) matches any candidate arity of the aligned arguments
    // container — empty (`fetch()`), single (`fetch(a)`), or multi
    // (`fetch(a, b)`) — binding the container's inner text in the MULTI
    // namespace exactly like [`capture_arguments`]. Probes: `g(fetch($$$A))`
    // answers all three arities and answers BOTH the outer and the inner
    // call of `fetch(g(fetch($$$A)))` faces.
    if p_children.len() == 3
        && c.kind() == p.kind()
        && p.kind().contains("argument")
        && p_children[0].kind() == "("
        && p_children[2].kind() == ")"
        && node_text(&p_children[1], &template.doc)
            .and_then(|text| template.placeholders.get(text.trim()))
            .is_some_and(|name| template.multi_names.contains(name))
    {
        let middle_text = node_text(&p_children[1], &template.doc).unwrap_or_default();
        let name = template.placeholders[middle_text.trim()].clone();
        let bound = node_text(&c, source)
            .map(|text| strip_container(text).to_string())
            .unwrap_or_default();
        return bind_capture_kind(captures, &name, &bound, true);
    }
    if p_children.len() != c_children.len() {
        return None;
    }
    for (p_child, c_child) in p_children.drain(..).zip(c_children.drain(..)) {
        general_eq(template, p_child, c_child, source, captures)?;
    }
    Some(())
}

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

/// Grammar fields that carry an optional declared return type. Only optional
/// presence fields belong here: grammars whose return-type-ish child is
/// mandatory (java/c/cpp `type`) must stay matchable by keyword-agnostic
/// templates — the strictness is about presence agreement, not keyword
/// translation.
pub(crate) const RETURN_TYPE_FIELDS: &[&str] = &["return_type", "result", "returns"];

/// Return-type presence agreement between the template and the matched
/// declaration node.
pub(crate) fn function_return_type_absent(node: &Node) -> bool {
    RETURN_TYPE_FIELDS
        .iter()
        .all(|field| node.child_by_field_name(field).is_none())
}

pub(crate) fn function_body_matches(node: &Node, template: &BodyTemplate) -> bool {
    let Some(body) = function_body_node(node) else {
        return false;
    };
    match template {
        BodyTemplate::Any => true,
        BodyTemplate::Exactly(want) => count_statements(body) == *want,
    }
}

/// The body block of a function-like match node. Falls back to scanning named
/// children (and their `body` fields, for `const f = () => {...}` declarators)
/// when the grammar has no `body` field.
pub(crate) fn function_body_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    // The ts type_alias_declaration spells its member-count body as the
    // `value` field (the object_type) — the `type-alias` Class face counts
    // its members there.
    if node.kind() == "type_alias_declaration" {
        if let Some(value) = node.child_by_field_name("value") {
            return Some(value);
        }
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'a>> = node.named_children(&mut cursor).collect();
    children
        .iter()
        .find(|child| BLOCK_KINDS.contains(&child.kind()))
        .copied()
        .or_else(|| {
            children
                .iter()
                .find_map(|child| child.child_by_field_name("body"))
        })
}

/// Count named non-comment statements in a body, descending through
/// statement-free wrapper nodes (`function_body` → `statements` → …).
pub(crate) fn count_statements(body: Node) -> usize {
    let mut container = body;
    while STMT_WRAPPER_KINDS.contains(&container.kind()) {
        let mut cursor = container.walk();
        let named: Vec<Node> = container
            .named_children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            .collect();
        match named.as_slice() {
            [only] if BLOCK_KINDS.contains(&only.kind()) => container = *only,
            _ => break,
        }
    }
    let mut cursor = container.walk();
    container
        .named_children(&mut cursor)
        .filter(|child| !is_trivia_kind(child.kind()))
        .count()
}
