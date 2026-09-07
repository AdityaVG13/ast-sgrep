//! Structural and literal pattern matching over tree-sitter ASTs.
//!
//! **Why this exists (vs shelling out to ast-grep):**
//! - Indexed hybrid search needs a fast, in-process structural channel.
//! - External `ast-grep` is excellent for full metavariable rules, but process
//!   spawn + JSON parse is too heavy for tight loops and offline agents.
//! - We implement the common ~80% of patterns natively (function/method/class
//!   decls and calls with `$NAME` / `$$$` holes). Exotic shapes are match-none
//!   or fail-closed in search; they are **not** silently shelled out to
//!   ast-grep (`DISC-pattern-native-subset`). Bench spawn is opt-in only.

use crate::extract::{
    byte_to_line, is_ident_kind, is_in_comment_or_string, is_inside_comment_or_string,
    is_member_expr_kind, last_identifier_in_chain, node_lines, node_text,
};
use crate::pattern_queries::{class_queries_for, queries_for, FUNCTION_QUERY_TABLE};
use crate::{Language, PatternNode};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock, RwLock};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
    pub excerpt: String,
    /// Metavariable bindings without the leading `$`. `MATCH` is always the
    /// complete matched node and is available to rewrite templates.
    pub captures: BTreeMap<String, String>,
}

/// Declaration / type keyword prefixes used by native classification and prefilters.
///
/// `true` means class-like (`Class`); `false` means function-like (`Function`).
pub const DECL_PATTERN_PREFIXES: &[(&str, bool)] = &[
    ("fn ", false),
    ("def ", false),
    ("function ", false),
    ("func ", false),
    ("class ", true),
    ("struct ", true),
    ("interface ", true),
    ("type ", true),
];

/// True when the pattern needs external ast-grep (we cannot handle it natively).
///
/// Patterns without `$` always run in-process. Patterns with `$`/`$$$` use the
/// native structural matcher when they fit a known shape; anything the
/// classifier rejects needs the (bench-only, never-delegated) external engine.
///
/// H-CONF-023 (pass 30): the previous no-structural-syntax exemption (bare
/// `$$$word<<<`, `RETURN $A`) silently answered `ok:true` empty for patterns
/// the reference fails to parse (sg pattern-parse error, exit 8) — a
/// fail-open. Classification rejection is now always loud, matching codemod's
/// long-standing ingress rule (`plan_codemod` bails on any rejected
/// `$`-pattern), so every `$`-pattern the native classifier rejects fails
/// closed instead of degrading to a silent empty result.
///
/// PASS 51 (r9): a `$`-pattern the classifier rejects is served natively when
/// the *general structural lane* below can build a template for it (the
/// dup-meta family faces from the pass-48 probe: operator chains, nested-call
/// arguments, typed params, return/assignment bodies). Everything else —
/// `$$$` templates, multi-line tails, binding keywords outside the native
/// declaration prefixes (`let`, `fun`, `int`), comment tails, bare
/// metavariable soup, patterns no grammar parses — keeps the loud fail-closed
/// contract (cases.jsonl `subject_expect=fail_closed` rows).
pub fn needs_ast_grep_fallback(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() || !p.contains('$') {
        return false;
    }
    if classify_native(p).is_some() {
        return false;
    }
    !general_lane_supported(p)
}

/// PASS 60 (H-CONF-029): true when the native engine can answer `pattern`
/// for files of `lang` at all — classifier-accepted shapes, `$`-less shapes,
/// or a general-lane template buildable IN THIS LANGUAGE. The language-free
/// `needs_ast_grep_fallback` must stay permissive for ingress (the pass-51
/// dup-meta faces are served through any-language support), so the per-file
/// unanswerable signal comes from here: core's walk uses it to separate
/// "pattern unanswerable for this file's language" — loud when the whole
/// query answers empty (sg exits 8 on the same inputs) — from per-file
/// source-parse robustness (silent skip).
///
/// PASS 63 (H-CONF-031, P0): the classifier-accepted arm is now
/// language-aware. `classify_native` is language-free, so a decl template
/// spelled in one language's syntax (`fn $A($B) { $$$C }`) classified as
/// answerable in EVERY language and the census never fired — cross-language
/// probes answered silent `ok:true`-0 where sg rejects the pattern (exit 8;
/// the 3 pass-62b fail-opens + hand probes). The answerability decision for
/// classifier-accepted Function/Class/Call shapes now replicates sg's own
/// pattern-acceptance gate (see [`sg_pattern_parses`]): the substituted
/// pattern must parse under THIS language's grammar the way sg's
/// `Pattern::try_new` requires. If/Universal/NeverMatches kinds keep their
/// registered cross-language contracts, and shapes whose metavariables the
/// general substitution refuses (bare `$$$`) keep today's answerable class.
///
/// PASS 63 (F62-1): `#` counts as comment syntax here only in the languages
/// where it IS comment syntax (python/ruby/php); rust attributes, C
/// preprocessor lines, swift `#selector`, and js private fields are real
/// syntax and stay answerable. The registered python/ruby comment-glued
/// faces keep failing closed through this same guard.
pub fn native_pattern_answerable(lang: Language, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() || !pattern.contains('$') {
        return true;
    }
    if lane_comment_refused(lang, pattern) {
        return false;
    }
    match classify_native(pattern) {
        Some(kind) => native_kind_language_answerable(lang, pattern, &kind),
        None => {
            // PASS 65 (F64-3): the conditional-directive family answers
            // natively in C/C++ through the dedicated preproc lane.
            if matches!(lang, Language::C | Language::Cpp)
                && preproc_directive_supported(pattern) == Some(true)
            {
                return true;
            }
            cached_general_template(lang, pattern).is_some()
        }
    }
}

/// PASS 63 (H-CONF-031): language-aware answerability of a
/// classifier-accepted shape. Decl/call shapes must parse under `lang`'s
/// grammar through sg's pattern-acceptance gate; the registered
/// cross-language kinds (If normalization, `$$A` universal, NeverMatches
/// accepted-empty) stay answerable everywhere, and shapes whose
/// substitution refuses (bare `$$$` rest args — the registered rest-arg
/// faces) are not probed.
fn native_kind_language_answerable(lang: Language, pattern: &str, kind: &NativeKind) -> bool {
    match kind {
        NativeKind::If { .. }
        | NativeKind::Universal { .. }
        | NativeKind::NeverMatches => true,
        NativeKind::Function { .. } | NativeKind::Class { .. } | NativeKind::Call { .. } => {
            sg_pattern_parses(lang, pattern)
        }
        // PASS 75a (F74a-2): the `->` member-call spelling adopts the same
        // sg pattern-acceptance gate (php parses `µsvc->run(µA)` as a member
        // call; grammars without `->` member syntax refuse it).
        NativeKind::MemberCall { .. } => sg_pattern_parses(lang, pattern),
        // PASS 77b (F76-1): the `->` member-call CHAIN adopts the same gate —
        // php parses the expando-substituted chain; grammars without `->`
        // member syntax refuse (sg's own parse refuses there too).
        NativeKind::MemberCallChain { .. } => sg_pattern_parses(lang, pattern),
        // PASS 65 (F64-2): the chain lane adopts the same sg pattern-acceptance
        // gate — the expando-substituted document must parse to a single node
        // under this language's grammar.
        NativeKind::CallChain { .. } => sg_pattern_parses(lang, pattern),
        // PASS 67a (F64-7/F66a-3): the optional chain additionally requires
        // sg's parse to accept the spelling, and the language to be in the
        // registered optional-connector registry — the grammars where sg's
        // parse of a depth-0 `?.` IS a member connector and the matcher's
        // leaf decomposition reproduces it (probes 0.45.2: ts/js answer the
        // head- and mid-chain optional spellings; php's `?->` chains carry no
        // dots and never classify here). kotlin/swift wrap the connector in
        // navigation-suffix shapes the leaf checks refuse, csharp's
        // `invocation_expression` callee never presents a member-expression
        // node, rust's `?` folds into a try-expression head, and python has
        // no `?.` spelling (sg's own parse refuses) — those keep the
        // fail-closed refusal.
        NativeKind::OptionalCallChain { .. } => {
            sg_pattern_parses(lang, pattern)
                && matches!(lang, Language::TypeScript | Language::JavaScript)
        }
        // PASS 65f (F64-7): the optional lane additionally requires sg's parse
        // of the template to carry the `?.` as the callee's CONNECTOR token —
        // the exact shape the matcher decomposes. Grammars where `?` means
        // something else (rust folds it into a try-expression head) or that
        // cannot parse the spelling (python) keep the fail-closed refusal;
        // both engines share the grammars, so this answers identically to
        // sg's per-language behavior by construction.
        NativeKind::OptionalCall { .. } => {
            sg_pattern_parses(lang, pattern) && sg_optional_connector_spelling(lang, pattern)
        }
    }
}

/// PASS 65f (F64-7): true when sg's parse of the expando-substituted
/// optional template puts the `?.` as an ANONYMOUS TOKEN CHILD of the
/// callee member node — the connector position the optional lane matches
/// on. The per-language truth (0.45.2 probes, PASS 67a adjudication):
/// ts/js spell it so and pass, and kotlin's `navigation_suffix` carries
/// the same clean receiver+`?.`+identifier triple so it passes too (the
/// PASS 66a flip 1: kotlin `$O?.$M($$$A)` answers sg-exactly). rust's `?`
/// lands inside a try-expression head (no optional link), python has no
/// `?.` spelling, swift wraps the tail in navigation-suffix shapes the
/// leaf check refuses, and csharp's `invocation_expression` callee never
/// presents a member-expression node here — those all stay refused (the
/// swift/csharp refusals are REGISTERED sg-answers divergences). php
/// passes through the split-field `nullsafe_member_call_expression` kind
/// arm below (F66a-6 — the kind IS the `?->` token-exact discriminator);
/// 3+-segment optional chains go through the per-connector-flag
/// [`NativeKind::OptionalCallChain`] gate instead.
fn sg_optional_connector_spelling(lang: Language, pattern: &str) -> bool {
    let doc: String = pattern
        .chars()
        .map(|c| if c == '$' { SG_EXPANDO_CHAR } else { c })
        .collect();
    // PASS 67a (F66a-6): sg pre-processes php patterns behind a `<?php ` tag
    // (Language::pre_process_pattern) — without it the whole doc parses to a
    // bare `text` node and no call shape is reachable. The leading `php_tag`
    // node is pattern-irrelevant, so the descent starts at the first child
    // after it (oracle 0.45.2: `$O?->$M($$$A)` answers on php).
    let (doc, lang_php) = if lang == Language::Php {
        (format!("<?php {doc}"), true)
    } else {
        (doc, false)
    };
    let Ok(tree) = parse_source(lang, &doc) else {
        return false;
    };
    let mut node = tree.root_node();
    if lang_php {
        let mut cursor = node.walk();
        let Some(first) = node
            .children(&mut cursor)
            .find(|child| child.is_named() && child.kind() != "php_tag")
        else {
            return false;
        };
        node = first;
    }
    while is_sg_single_node(node) {
        node = node.child(0).expect("single node has a child");
    }
    if !is_call_kind(node.kind()) {
        return false;
    }
    // PASS 67a (F66a-6): php's split-field member call carries the `?->`
    // connector as its own named kind — `nullsafe_member_call_expression`
    // (plain `->` stays `member_call_expression`), so the kind is the
    // token-exact discriminator. The name leaf must be a plain identifier —
    // the matcher's decompose contract.
    if matches!(
        node.kind(),
        "member_call_expression" | "nullsafe_member_call_expression"
    ) {
        return node.kind() == "nullsafe_member_call_expression"
            && node
                .child_by_field_name("name")
                .is_some_and(|name| is_ident_kind(name.kind()));
    }
    let Some(field) = call_field_node(&node) else {
        return false;
    };
    if !is_member_expr_kind(field.kind()) {
        return false;
    }
    // The tail segment must be a plain identifier leaf (the matcher's
    // decompose contract); a wrapped suffix keeps the refusal.
    let Some(link) = member_link_parts(&field) else {
        return false;
    };
    link.optional && is_ident_kind(link.leaf.kind())
}

/// One member-link's parts classified from a member-expression node's
/// children: a receiver base, ONE connector, and an identifier leaf, in
/// that child order. PASS 65f (F64-7): the `?.` connector appears either
/// as an anonymous `?.` token or wrapped in a named `optional_chain` node
/// (tree-sitter-typescript, whose inner anonymous token must be `?.`) —
/// NOTE (PASS 67a): csharp grammars never reach this decomposition (the
/// `invocation_expression` callee lookup refuses first, keeping csharp
/// fail-closed); `.` is always an anonymous token. Any other anonymous
/// token or extra segment vetoes the link.
struct MemberLink<'a> {
    base: Node<'a>,
    optional: bool,
    leaf: Node<'a>,
}

fn member_link_parts<'a>(node: &Node<'a>) -> Option<MemberLink<'a>> {
    let mut cursor = node.walk();
    let children: Vec<Node<'a>> = node.children(&mut cursor).collect();
    let mut base: Option<Node<'a>> = None;
    let mut leaf: Option<Node<'a>> = None;
    let mut optional: Option<bool> = None;
    for child in children {
        if child.is_named() && child.kind().contains("optional") {
            // Named `?.` wrapper: the inner anonymous token must be `?.`.
            let mut inner = child.walk();
            let has_q = child
                .children(&mut inner)
                .any(|token| !token.is_named() && token.kind() == "?.");
            if !has_q || optional.replace(true).is_some() {
                return None;
            }
        } else if child.is_named() {
            let slot = if optional.is_none() { &mut base } else { &mut leaf };
            if slot.replace(child).is_some() {
                return None;
            }
        } else {
            let flag = match child.kind() {
                "." => false,
                "?." => true,
                _ => return None,
            };
            if optional.replace(flag).is_some() {
                return None;
            }
        }
    }
    Some(MemberLink {
        base: base?,
        optional: optional?,
        leaf: leaf?,
    })
}

/// PASS 63 (H-CONF-031): replication of sg's pattern-acceptance gate
/// (ast_grep_core `PatternBuilder::single`, 0.39.9 / CLI 0.45.2): every `$`
/// is rewritten to the expando char (ast-grep's own preprocessing — its
/// MultipleNode error echoes `func µA(µB) { µµµC }`), the document is parsed
/// under `lang`'s grammar, and the root must be a "single node" — exactly one
/// child, or two where the second is a missing/empty token (the golang
/// trailing-node quirk) — descended through the single-child chain. NO
/// final-node ERROR rejection: sg 0.45.2 accepts single-child ERROR-wrapped
/// parses with a warning (`def $A($B): $$$C` --lang rust probes exit 0 with
/// "Pattern contains an ERROR node"), so the only rejections are
/// PatternError::NoContent and ::MultipleNode — exactly the inputs sg's CLI
/// refuses with exit 8 ("Cannot parse query as a valid pattern"). Both
/// engines share the same tree-sitter grammars, so this gate answers
/// identically to sg's per-language pattern acceptance by construction
/// (matrix-validated pass 63 against ast-grep 0.45.2).
fn sg_pattern_parses(lang: Language, pattern: &str) -> bool {
    let doc: String = pattern
        .chars()
        .map(|c| if c == '$' { SG_EXPANDO_CHAR } else { c })
        .collect();
    let Ok(tree) = parse_source(lang, &doc) else {
        return false;
    };
    let mut node = tree.root_node();
    if node.child_count() == 0 {
        // sg PatternError::NoContent.
        return false;
    }
    if !is_sg_single_node(node) {
        // sg PatternError::MultipleNode ("Multiple AST nodes are detected").
        return false;
    }
    while is_sg_single_node(node) {
        node = node.child(0).expect("single node has a child");
    }
    true
}

/// ast-grep's metavariable expando char (`$` is rewritten to it before the
/// pattern is parsed; it is a valid identifier char in every indexed
/// grammar, which is why ast-grep picked it).
const SG_EXPANDO_CHAR: char = 'µ';

/// sg `is_single_node`: one child, or two where the second is a
/// missing/empty token (some grammars emit a spurious empty node at the end).
fn is_sg_single_node(node: Node) -> bool {
    match node.child_count() {
        1 => true,
        2 => node
            .child(1)
            .is_some_and(|child| child.is_missing() || child.kind().is_empty()),
        _ => false,
    }
}

pub fn tree_sitter_language(lang: Language) -> tree_sitter::Language {
    match lang {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        // e2hc/difu.5: C# patterns were parsed with the Java grammar, causing
        // misparses of C#-specific syntax. Use the real C# grammar so the
        // pattern channel agrees with the extraction channel.
        Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Language::Swift => tree_sitter_swift::LANGUAGE.into(),
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
    }
}

/// Unified entry: literal identifier match, or native structural match for `$` patterns.
pub fn match_pattern(
    lang: Language,
    source: &str,
    pattern: &str,
) -> anyhow::Result<Vec<PatternMatch>> {
    // PASS 60 (H-CONF-030 iv): a BOM-led pattern is stripped like sg strips
    // it; the raw U+FEFF otherwise degrades every lane to a silent empty.
    let pattern = pattern.trim().trim_start_matches('\u{feff}').trim();
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    // PASS 65 (F64-3): the C/C++ conditional-compilation directives answer
    // kind-level on the directive head (sg probes) — a dedicated lane ahead
    // of the general/literal routing, which cannot spell compound-region
    // roots. Non-directive patterns fall through untouched.
    if matches!(lang, Language::C | Language::Cpp) {
        if let Some(hits) = match_preproc_directive(lang, source, pattern) {
            return Ok(hits);
        }
    }
    if !pattern.contains('$') {
        // PASS 65 (F64-6): a bare statement head (`break`, `break;`) is a
        // KIND template — sg answers every statement of the head's family
        // regardless of arguments or the trailing semicolon (probed
        // java/csharp/rust/python 0.45.2). The family predates the general
        // lane's childless-template face, which only ever matched the exact
        // childless form.
        if let Some(hits) = match_bare_statement_kind(lang, source, pattern) {
            return Ok(hits);
        }
        // PASS 63 (F62-2): a bare statement keyword (`break`, `continue`) is
        // a statement TEMPLATE, not an identifier literal — sg matches the
        // statement node (probe: `break` -l javascript answers the
        // break_statement; the literal lane only ever matches identifier /
        // literal text nodes and answered silent 0). Registered statement
        // heads route to the structural general lane; every other `$`-less
        // pattern keeps the literal lane.
        let statement_head = pattern
            .split_whitespace()
            .next()
            .is_some_and(|head| STATEMENT_HEAD_KEYWORDS.contains(&head));
        if statement_head {
            return Ok(match_structural_general(lang, source, pattern));
        }
        return match_literal_pattern(lang, source, pattern);
    }
    // PASS 69a (F68a-1): in languages where `$` is name syntax, sg parses
    // non-canonical `$`-tokens as literal code and answers the literal
    // faces; the honest-ladder rung (a) — answer sg-exactly through the
    // existing literal lane (exact-text + R3 structural arms) instead of
    // the silent NeverMatches empty.
    if dollar_literal_lane(lang, pattern) {
        return match_literal_pattern(lang, source, pattern);
    }
    match classify_native(pattern) {
        Some(kind) => match_structural(lang, source, pattern, &kind),
        // PASS 51 (r9): classifier-rejected shapes get the general structural
        // lane (single-expression / function-declaration templates with
        // metavariable leaves, `bind_capture` unification). A pattern the lane
        // cannot template for this language matches nothing here; the search
        // ingress (`needs_ast_grep_fallback`) already failed those closed.
        None => Ok(match_structural_general(lang, source, pattern)),
    }
}

/// Matches identifier text exactly, including case.
///
/// This syntax-level policy intentionally differs from relevance ranking, where symbol
/// comparisons are case-folded. A pattern for `Foo` does not match an identifier `foo`.
pub fn match_literal_pattern(
    lang: Language,
    source: &str,
    pattern: &str,
) -> anyhow::Result<Vec<PatternMatch>> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let tree = parse_source(lang, source)?;
    let template = parse_pattern_tree(lang, pattern.trim());
    let mut matches = Vec::new();
    walk_literal(tree.root_node(), source, pattern, template.as_deref(), &mut matches);
    Ok(matches)
}

/// Statement-count template inside a nested `{ ... }` (or `:` suite) section.
///
/// ast-grep semantics: a single metavariable statement (`{ $STMT }`) matches a
/// body with **exactly one** statement; `$$$` matches any body; `{}` matches an
/// empty body. Comments are not counted as statements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyTemplate {
    /// `{ $$$ }` / `{ $$$BODY }` — any statements, but a body must exist.
    Any,
    /// `{}` → 0 statements, `{ $STMT }` → exactly 1 statement.
    Exactly(usize),
}

/// Argument-list shape for call templates (same contract as
/// [`BodyTemplate`], for argument lists).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentTemplate {
    Any,
    Exactly(usize),
}

/// One dotted segment of a [`NativeKind::CallChain`] pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallChainSegment {
    /// Literal segment text (`None` when the segment is a `$METAVAR`).
    pub literal: Option<String>,
    /// Capture name when the segment is a metavariable.
    pub capture: Option<String>,
    /// Argument template for call segments (`None` = plain receiver).
    pub args: Option<ArgumentTemplate>,
    /// `(name, multi)` when the segment's WHOLE argument list is one
    /// metavariable (`$A` / `$$$A`).
    pub args_capture: Option<(String, bool)>,
}

/// PASS 65 (F64-2): classify a dotted member-call chain with per-segment
/// argument lists. `None` unless MORE THAN ONE segment carries an argument
/// list — every single-argument-list face keeps the simple
/// [`NativeKind::Call`] lane unchanged.
fn classify_call_chain(p: &str) -> Option<NativeKind> {
    if !p.contains('.') || !p.contains('(') {
        return None;
    }
    // Split on depth-0 dots; parentheses and subscripts nest.
    let mut raw_segments: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in p.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            '.' if depth == 0 => raw_segments.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    raw_segments.push(current);
    if raw_segments.len() < 2 {
        return None;
    }
    // PASS 67a (F66a-3): a trailing `?` on a segment spells the OPTIONAL
    // connector INTO the NEXT segment — the head link (`$O?.first().second()`)
    // and the mid-chain links (`$O.first()?.second()`, probed 0.45.2: sg
    // parses and answers both spellings token-exactly). A trailing `?` on
    // the LAST segment has no next link and no classified contract
    // (rust-try spellings), and any other `?` (ternaries, conditional
    // arguments) keeps the loud refusal.
    let last = raw_segments.len() - 1;
    let mut optional_flags = vec![false; raw_segments.len()];
    for (index, raw) in raw_segments.iter().enumerate() {
        if raw.trim_end().ends_with('?') {
            if index == last {
                return None;
            }
            optional_flags[index + 1] = true;
        }
    }
    let mut segments = Vec::with_capacity(raw_segments.len());
    let mut call_segments = 0usize;
    for (index, raw) in raw_segments.iter().enumerate() {
        let raw = raw.trim();
        let raw = if index < last && raw.ends_with('?') {
            raw[..raw.len() - 1].trim_end()
        } else {
            raw
        };
        if raw.is_empty() {
            return None;
        }
        if raw.contains('?') {
            return None;
        }
        let (name_part, args_text) = match raw.find('(') {
            Some(open) => {
                // A call segment must end at its argument list.
                if !raw.ends_with(')') {
                    return None;
                }
                if !(is_pure_metavariable(name_head(raw, open)) || is_pattern_ident(name_head(raw, open)))
                {
                    return None;
                }
                call_segments += 1;
                (name_head(raw, open), Some(raw[open + 1..raw.len() - 1].trim()))
            }
            None => {
                // Only the leading receiver segment may be argument-free.
                if index != 0 {
                    return None;
                }
                (raw, None)
            }
        };
        let (literal, capture) = if is_pure_metavariable(name_part) {
            (None, capture_name(name_part).map(str::to_string))
        } else {
            (Some(name_part.to_string()), None)
        };
        let args = match args_text {
            None => None,
            Some(args) if args.is_empty() => Some(ArgumentTemplate::Exactly(0)),
            // PASS 65a (F64-2 RED-fix): `$$$` / `$$$NAME` is sg's ANY-arity
            // rest argument (probed 0.45.2: the chains bind EMPTY argument
            // lists, multi A=[]). The previous all-pure-metavar arm spelled
            // `$$$A` as Exactly(1) and made every `$$$` chain face silent
            // where sg answers.
            Some(args) if args == "$$$" => Some(ArgumentTemplate::Any),
            Some(args) if args.starts_with("$$$") && is_metavar_name(&args[3..]) => {
                Some(ArgumentTemplate::Any)
            }
            // Plain single/metavar lists keep the exact-arity contract; a
            // rest metavariable mixed with singles has no sg evidence and
            // never templates (the single-call classifier refuses the same
            // mix through validate_argument_pattern).
            Some(args) if !args.contains("$$$") && args.split(',').all(is_pure_metavariable) => {
                Some(ArgumentTemplate::Exactly(args.split(',').count()))
            }
            Some(_) => return None,
        };
        let args_capture = match args_text {
            Some(args) if args == "$$$" => None,
            Some(args) => {
                if let Some(name) = args.strip_prefix("$$$") {
                    is_metavar_name(name).then(|| (name.to_string(), true))
                } else {
                    capture_name(args).map(|name| (name.to_string(), false))
                }
            }
            None => None,
        };
        segments.push(CallChainSegment {
            literal,
            capture,
            args,
            args_capture,
        });
    }
    if call_segments < 2 {
        return None;
    }
    // PASS 67a (F66a-3): the 3+-segment optional chain is its own kind —
    // the two-segment spelling keeps the registered [`NativeKind::OptionalCall`]
    // lane (call_segments == 1 falls through to it via the single-call arm).
    // The per-connector flags ride along: `optional_flags[j]` is true when
    // the connector INTO pattern segment j is the spelled `?.`.
    if optional_flags.iter().any(|flag| *flag) {
        return Some(NativeKind::OptionalCallChain {
            segments,
            optional_flags,
        });
    }
    Some(NativeKind::CallChain { segments })
}

/// PASS 77b (F76-1): classify a php `->` member-call chain with per-segment
/// argument lists. `None` unless MORE THAN ONE segment carries an argument
/// list — single-call-segment member faces keep the simple
/// [`NativeKind::MemberCall`] lane, and a refused shape falls through to the
/// historical arms unchanged (the pre-77b posture is preserved verbatim).
/// The segment/argument grammar mirrors [`classify_call_chain`] with the
/// depth-0 connector spelled `->`; a leftover `?` (the nullsafe `?->`
/// connector) refuses like the `.`-lane's `?.` refusal so the optional lane
/// keeps its token-exact contract.
fn classify_member_call_chain(p: &str) -> Option<NativeKind> {
    if !p.contains("->") || !p.contains('(') {
        return None;
    }
    // Split on depth-0 `->`; parentheses and subscripts nest.
    let bytes = p.as_bytes();
    let mut raw_segments: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut segment_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'-' if depth == 0 && bytes.get(i + 1) == Some(&b'>') => {
                raw_segments.push(p[segment_start..i].to_string());
                i += 2;
                segment_start = i;
            }
            _ => i += 1,
        }
    }
    raw_segments.push(p[segment_start..].to_string());
    if raw_segments.len() < 2 {
        return None;
    }
    let mut segments = Vec::with_capacity(raw_segments.len());
    let mut call_segments = 0usize;
    for (index, raw) in raw_segments.iter().enumerate() {
        let raw = raw.trim();
        // A trailing `?` is the nullsafe `?->` connector INTO the next
        // segment — no probed chain contract here, fail-closed.
        if raw.contains('?') {
            return None;
        }
        if raw.is_empty() {
            return None;
        }
        let (name_part, args_text) = match raw.find('(') {
            Some(open) => {
                // A call segment must end at its argument list.
                if !raw.ends_with(')') {
                    return None;
                }
                if !(is_pure_metavariable(name_head(raw, open)) || is_pattern_ident(name_head(raw, open)))
                {
                    return None;
                }
                call_segments += 1;
                (name_head(raw, open), Some(raw[open + 1..raw.len() - 1].trim()))
            }
            None => {
                // Only the leading receiver segment may be argument-free.
                // Receiver class discipline (PASS 77b): a canonical metavar
                // (`$A`/`$$A`), a plain identifier, or a lowercase-led
                // literal variable (`$obj` — sg's literal-source reading,
                // matched byte-exactly) classify; a MixedCase or garbage-led
                // `$`-token refuses (php poisons the whole pattern — the
                // NeverMatches gate keeps those faces).
                if index != 0 {
                    return None;
                }
                let receiver_ok = is_pure_metavariable(raw)
                    || is_pattern_ident(raw)
                    || dollar_name_class(raw.strip_prefix('$').unwrap_or(""))
                        == Some(DollarTokenClass::LowercaseLed);
                if !receiver_ok {
                    return None;
                }
                (raw, None)
            }
        };
        let (literal, capture) = if is_pure_metavariable(name_part) {
            (None, capture_name(name_part).map(str::to_string))
        } else {
            (Some(name_part.to_string()), None)
        };
        // Same argument table as [`classify_call_chain`] — `is_pure_metavariable`
        // already carries the F74a-3 canonical `$$A` arm, so 2-dollar chain
        // arguments classify exactly like `$A` (F76-1 × F76-2 faces).
        let args = match args_text {
            None => None,
            Some(args) if args.is_empty() => Some(ArgumentTemplate::Exactly(0)),
            Some(args) if args == "$$$" => Some(ArgumentTemplate::Any),
            Some(args) if args.starts_with("$$$") && is_metavar_name(&args[3..]) => {
                Some(ArgumentTemplate::Any)
            }
            Some(args) if !args.contains("$$$") && args.split(',').all(is_pure_metavariable) => {
                Some(ArgumentTemplate::Exactly(args.split(',').count()))
            }
            Some(_) => return None,
        };
        let args_capture = match args_text {
            Some(args) if args == "$$$" => None,
            Some(args) => {
                if let Some(name) = args.strip_prefix("$$$") {
                    is_metavar_name(name).then(|| (name.to_string(), true))
                } else {
                    capture_name(args).map(|name| (name.to_string(), false))
                }
            }
            None => None,
        };
        segments.push(CallChainSegment {
            literal,
            capture,
            args,
            args_capture,
        });
    }
    // Single-call-segment faces keep the [`NativeKind::MemberCall`] lane.
    if call_segments < 2 {
        return None;
    }
    Some(NativeKind::MemberCallChain { segments })
}

/// The callee text before the argument list of a call segment.
fn name_head(raw: &str, open: usize) -> &str {
    raw[..open].trim()
}

/// Native structural pattern shapes handled in-process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeKind {
    /// Function-like declaration; `name == None` means any name (`$NAME`).
    /// `body` constrains the statement count of the body (`fn $N($$$) { $STMT }`).
    Function {
        name: Option<String>,
        body: Option<BodyTemplate>,
    },
    /// Class/struct/type declaration. `keyword` is the pattern prefix
    /// (`class`, `struct`, `interface`, or `type`) so queries stay kind-specific.
    Class {
        keyword: &'static str,
        name: Option<String>,
    },
    /// Free or method call; method path segments may be `$` wildcards.
    Call {
        /// Exact path like `foo.bar` or single name; segments that were `$X` are None.
        path: Vec<Option<String>>,
    },
    /// PASS 75a (F74a-2): the php plain `->` member-call spelling
    /// (`$svc->run($A)`). sg is connector token-exact — a `->`-spelled callee
    /// never answers `.`/`::`/`?->` call sites and vice versa — so the
    /// spelling gets its own lane over `member_call_expression` candidates
    /// with the full object->name segment chain (raw object text, meta
    /// segments wildcard). Nullsafe candidates stay on the
    /// [`NativeKind::OptionalCall`] lane.
    MemberCall {
        /// Exact path; segments that were `$X` are None.
        path: Vec<Option<String>>,
    },
    /// PASS 77b (F76-1): the php plain `->` member-call CHAIN spelling with
    /// MORE THAN ONE argument list (`$obj->m1()->m2($A)`). Pass-75's
    /// [`NativeKind::MemberCall`] lane serves single-call-segment faces only;
    /// every deeper chain fell through the classifier into the silent general
    /// lane and answered `ok:true []` where sg 0.45.2 answers the chain node
    /// (and its inner prefix subnodes, which the walk visits as their own
    /// member-call nodes). Per-segment contract mirrors
    /// [`NativeKind::CallChain`], but candidates are `member_call_expression`
    /// nodes only — connector token-exact, never a `.`/`?->` site — and each
    /// call segment's argument list must sit on a REAL call link (a property
    /// access in the receiver chain is not a call). Canonical `$$A` argument
    /// slots ride the F74a-3 family binding (single namespace, key `A`).
    MemberCallChain {
        /// Outermost-first segments. Only the leading segment may be a
        /// plain receiver; every later segment carries an argument list.
        segments: Vec<CallChainSegment>,
    },
    /// PASS 65 (F64-2): dotted member-call chains with MORE THAN ONE
    /// argument list (`$O.$M1($$$A).$M2($$$B)`). sg answers the outermost
    /// chain whose callee path has exactly the pattern's segment count,
    /// unifying per-segment names through metavariable equality (the
    /// same-name veto) and checking every call segment's argument list.
    CallChain {
        /// Outermost-first segments. Only the leading segment may be a
        /// plain receiver; every later segment carries an argument list.
        segments: Vec<CallChainSegment>,
    },
    /// PASS 67a (F66a-3): the 3+-segment optional chain
    /// `$O?.$M1($$$A).$M2($$$B)` — sg 0.45.2 answers the shape (correcting
    /// 65f's registered sg-EMPTY claim). The spelled connectors are
    /// token-exact per position: `optional_flags[j]` is true when the
    /// connector INTO pattern segment j is the spelled `?.` (the head link
    /// `$O?.…`, the mid-chain spellings `$O.$M1()?.$M2($$$B)`, or both) and
    /// a candidate must match every ALIGNED flag exactly; a wildcard (or
    /// single-token literal) head folds the whole receiver prefix before
    /// the first aligned segment. The two-segment spelling stays on
    /// [`NativeKind::OptionalCall`].
    OptionalCallChain {
        /// `segments[0]` is the folded head; every later segment carries an
        /// argument list (outermost-last).
        segments: Vec<CallChainSegment>,
        /// `optional_flags[j]` = the connector INTO pattern segment j is
        /// `?.` (`flags[0]` is always false — the head has no connector).
        optional_flags: Vec<bool>,
    },
    /// PASS 65f (F64-7 scope-back): the two-segment optional-chain call
    /// spelling `HEAD?.$TAIL(...)`. sg 0.45.2 treats `?.` as a required
    /// anonymous connector token: this template answers exactly the
    /// optional-chain call faces (incl. wildcard-head folding,
    /// `$O` = `user?.profile` on `user?.profile?.load()`) and never the
    /// plain `.` receivers — the mirror of the F64-7 plain-template veto.
    /// Per-language acceptance is sg's own pattern parse (see
    /// [`native_kind_language_answerable`]): grammars where `?` is not a
    /// member connector (rust try, python: none) keep the refusal.
    OptionalCall {
        /// `Some(name)` when the head is the wildcard `$name` (folds any
        /// receiver expression); else the head is a literal identifier.
        head_capture: Option<String>,
        /// Literal head identifier (exact match against the receiver text).
        head_literal: Option<String>,
        /// `Some(name)` when the tail (method) segment is `$name`.
        tail_capture: Option<String>,
        /// Literal tail identifier.
        tail_literal: Option<String>,
    },
    /// `if` statement/expression template: `if ($COND) { $BODY }`,
    /// `if $COND { $BODY }`, or `if $COND: $BODY`. The condition must be a
    /// metavariable; paren, brace, and colon forms are normalized so one
    /// pattern matches if-nodes across all indexed languages.
    If { body: Option<BodyTemplate> },
    /// H-CONF-022 v2 (pass 54), scope extended by F26-0182 (pass 69a): a
    /// pattern carrying a NON-canonical `$`-token — lowercase-LED (`$a`,
    /// `$$$a`) or uppercase/underscore-LED with a lowercase tail (`$ABc`,
    /// `$_a`; sg's meta-name grammar is `[A-Z_][A-Z0-9_]*`). In `$`-name
    /// languages (js/ts/php-lowercase) such tokens are literal code instead
    /// and route to the literal lane (F68a-1); here they are sg pattern-tree
    /// ERROR nodes that match nothing (accepted-empty, or exit 8 where
    /// per-language error recovery fails). Valid ingress (ok:true), zero
    /// candidates, never an overmatch; the sg-exit-8 faces stay a registered
    /// empty-vs-error divergence.
    NeverMatches,
    /// `$$NAME` / `$$_` (H-CONF-022 v2, pass 54): sg's universal node
    /// metavariable — matches EVERY node including comments, docstrings,
    /// strings, and anonymous tokens; a named metavariable binds the node
    /// text (`MATCH` reserved-key overwrite semantics preserved).
    Universal { name: Option<String> },
}

fn strip_declaration_modifiers(pattern: &str) -> (&str, Option<&str>) {
    const MODIFIERS: &[&str] = &[
        "export default ",
        "export ",
        "public ",
        "private ",
        "protected ",
        "internal ",
        "abstract ",
        "static ",
        "final ",
        "async ",
        "unsafe ",
        "pub ",
    ];

    let original = pattern.trim();
    let mut rest = original;
    loop {
        if let Some(after_pub) = rest.strip_prefix("pub(") {
            if let Some(close) = after_pub.find(") ") {
                let Some(next) = after_pub.get(close + 2..) else {
                    break;
                };
                rest = next;
                continue;
            }
        }
        let Some(next) = MODIFIERS
            .iter()
            .find_map(|modifier| rest.strip_prefix(*modifier))
        else {
            break;
        };
        rest = next;
    }
    let modifiers = original.strip_suffix(rest).unwrap_or_default().trim();
    (rest, (!modifiers.is_empty()).then_some(modifiers))
}

/// Classify a metavariable / structural pattern into the native subset.
pub fn classify_native(pattern: &str) -> Option<NativeKind> {
    let p = pattern.trim();
    // `if` templates first: `if ($COND)` must never classify as a call to `if`.
    if is_if_prefixed(p) {
        return classify_if_template(p);
    }
    // PASS 77b (F76-1): a strict php `->` member-call CHAIN (≥2 call
    // segments, canonical-meta or identifier call heads, pure-metavar
    // argument lists — see [`classify_member_call_chain`]) classifies BEFORE
    // the non-canonical gate: sg 0.45.2 answers `$obj->m1()->m2($A)` on the
    // lowercase-led receiver's literal-source reading (the F74a-1 semantics),
    // while the carve below deliberately refuses multi-call shapes (its
    // single-call discipline) — without this pre-gate arm every chain face
    // with a `$variable` receiver landed in NeverMatches and answered silent
    // `ok:true []`. MixedCase/garbage `$`-receivers refuse inside the
    // classifier (php poisons the whole pattern) and keep the gate's
    // NeverMatches class; every other refusal falls through to the gate and
    // the historical arms unchanged.
    if let Some(chain) = classify_member_call_chain(p) {
        return Some(chain);
    }
    // H-CONF-022 v2 (pass 54), extended by F26-0182 (pass 69a): a
    // non-canonical `$`-token anywhere in the pattern (lowercase-LED, or an
    // uppercase/underscore-LED name with a lowercase tail) is not a
    // canonical metavariable — sg's pattern tree carries an ERROR node that
    // matches nothing. Route the WHOLE pattern to NeverMatches before any
    // shape-specific arm can wildcard it.
    // PASS 75a (F74a-1/F74a-2): a 1-dollar LOWERCASE-led token is php
    // variable / js-ts identifier SYNTAX, not a poisoned meta. When every
    // non-canonical token in the pattern is such a token AND each sits as a
    // complete callee-head segment (`::` / `->` / `.`-delimited), the pattern
    // keeps the structural call lanes with those segments matched as literal
    // text (sg 0.45.2: `$obj::bar($A)` answers only the `$obj` scope line,
    // `$anything::bar($A)` answers []; the all-lowercase faces keep the
    // registered literal lane via `dollar_literal_lane`, and mixed-case or
    // 2/3-dollar classes keep the NeverMatches class here).
    // the dedicated optional lane classifies it.
    if pattern_has_noncanonical_metavar(p) && !literal_variable_callee_admitted(p) {
        return Some(NativeKind::NeverMatches);
    }
    // `$$NAME` / `$$_` — sg's universal node metavariable. `$$`-led shapes
    // that are not a canonical name (`$$`, `$$a`, `$$3`) keep today's
    // loud-reject class (registered residuals, matrix rows 3/4).
    if let Some(rest) = p.strip_prefix("$$") {
        return is_metavar_name(rest).then(|| NativeKind::Universal {
            name: (rest != "_").then(|| rest.to_string()),
        });
    }
    let (declaration, _) = strip_declaration_modifiers(p);
    for &(prefix, is_class) in DECL_PATTERN_PREFIXES {
        if let Some(rest) = declaration.strip_prefix(prefix) {
            let head = rest
                .split(|c: char| c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim();
            if head.is_empty() {
                return None;
            }
            // H-CONF-022 v2 (pass 54) declaration-head three-way: a canonical
            // metavariable head stays a wildcard name; a garbage-led head
            // (`$3.14`, `$0x1F`) is a loud reject — never the wildcard
            // empty-body acceptance hole. Lowercase-led heads never reach
            // here (the NeverMatches gate above already routed them).
            let name = match head.strip_prefix('$').filter(|rest| !rest.starts_with('$')) {
                Some(metavar) if is_metavar_name(metavar) => None,
                Some(_) => return None,
                None if is_pattern_ident(head) => Some(head.to_string()),
                None => return None,
            };
            let tail = rest[head.len()..].trim();
            let body = if is_class {
                parse_body_template(tail)?
            } else {
                parse_function_tail(tail)?
            };
            return Some(if is_class {
                // Statement-count templates on type bodies are language-specific
                // (fields vs methods); only `{ $$$ }` / no body are supported.
                if matches!(body, Some(BodyTemplate::Exactly(_))) {
                    return None;
                }
                NativeKind::Class {
                    keyword: prefix.trim(),
                    name,
                }
            } else {
                NativeKind::Function { name, body }
            });
        }
    }

    // Calls: $F($$$), foo($$$), $O.$M($$$), a.b.$$$c($$$) — and (F64-2)
    // dotted chains carrying MORE THAN ONE argument list
    // (`$O.$M1($$$A).$M2($$$B)`), which the single-argument-list call shape
    // below cannot spell (its `(`..`)` slice spans both lists).
    if let Some(chain) = classify_call_chain(p) {
        return Some(chain);
    }
    let open = p.find('(')?;
    let close = p.rfind(')')?;
    if close <= open {
        return None;
    }
    // Allow trailing whitespace only after the closing paren.
    if close + 1 != p.len() && !p[close + 1..].trim().is_empty() {
        return None;
    }
    let args = p[open + 1..close].trim();
    // Args must be empty, $$$, or pure metavars separated by commas.
    validate_argument_pattern(args)?;
    let callee = p[..open].trim();
    if callee.is_empty() {
        return None;
    }
    if let Some(path) = parse_call_path(callee) {
        // PASS 75a (F74a-2): a `->`-spelled callee is the php member-call
        // lane — connector token-exact, never the plain Call lane.
        if callee.contains("->") {
            return Some(NativeKind::MemberCall { path });
        }
        return Some(NativeKind::Call { path });
    }
    // PASS 65f (F64-7 scope-back): the optional-chain spelling `HEAD?.$TAIL`
    // (exactly one `?.` connector, two segments). Deeper optional chains and
    // `?.` inside multi-call chains have no sg-probed contract and stay
    // fail-closed here.
    let (head_capture, head_literal, tail_capture, tail_literal) =
        parse_optional_call_path(callee)?;
    Some(NativeKind::OptionalCall {
        head_capture,
        head_literal,
        tail_capture,
        tail_literal,
    })
}

/// PASS 65f (F64-7): parse the two-segment optional-chain callee spelling
/// `HEAD?.$TAIL` — the trailing `?` on the head marks the `?.` connector.
/// PASS 67a (F66a-6): php spells the nullsafe connector `?->` — the marker
/// rides BETWEEN head and tail (`$O?->$M`), so it splits there instead.
/// sg 0.45.2 accepts arbitrary expression heads (`maybe()?.$M($$$A)`
/// answers) but those have no structural contract here and stay
/// fail-closed; only metavariable and plain-identifier heads classify.
fn parse_optional_call_path(
    callee: &str,
) -> Option<(Option<String>, Option<String>, Option<String>, Option<String>)> {
    let (head, tail) = match callee.rsplit_once('.') {
        Some((head, tail)) => (head.strip_suffix('?')?, tail),
        None => callee.rsplit_once("?->")?,
    };
    if head.is_empty() || tail.is_empty() || head.contains('?') || head.contains('.') {
        return None;
    }
    let head = match capture_name(head) {
        Some(name) => (Some(name.to_string()), None),
        None if is_pattern_ident(head) => (None, Some(head.to_string())),
        // PASS 75a (F74a-2): a lowercase-led `$name` head is php variable
        // TEXT — the literal head compares byte-exactly against the object
        // node (sg 0.45.2: `$o?->m($A)` answers only the `$o` object line).
        None
            if dollar_name_class(head.strip_prefix('$').unwrap_or(""))
                == Some(DollarTokenClass::LowercaseLed) =>
        {
            (None, Some(head.to_string()))
        }
        None => return None,
    };
    let tail = match capture_name(tail) {
        Some(name) => (Some(name.to_string()), None),
        None if is_pattern_ident(tail) => (None, Some(tail.to_string())),
        None => return None,
    };
    Some((head.0, head.1, tail.0, tail.1))
}

/// Parse an optional metavariable argument list followed by an optional body.
/// Declaration heads without either are valid (`def $NAME`); every other byte
/// must belong to one of these supported sections.
fn parse_function_tail(tail: &str) -> Option<Option<BodyTemplate>> {
    let mut after = tail.trim();
    if let Some(args) = after.strip_prefix('(') {
        let close = args.find(')')?;
        let inner = args[..close].trim();
        if !inner.is_empty() && inner != "$$$" && !inner.split(',').all(is_pure_metavariable) {
            return None;
        }
        after = args[close + 1..].trim();
    }
    parse_body_template(after)
}

/// True when the pattern starts an `if` template (`if `, `if(`).
/// Identifiers like `iffy(...)` are not if-prefixed.
fn is_if_prefixed(p: &str) -> bool {
    p.strip_prefix("if")
        .is_some_and(|rest| rest.starts_with([' ', '(']))
}

/// Parse `if ($COND) { $BODY }` / `if $COND { $BODY }` / `if $COND: $BODY`.
///
/// The condition must be a single metavariable (`$COND`); concrete condition
/// expressions are out of the native subset and fail closed. `None` here means
/// unsupported — never fall through to call classification.
fn classify_if_template(p: &str) -> Option<NativeKind> {
    let rest = p.strip_prefix("if")?.trim_start();
    let (condition, after) = if let Some(inner) = rest.strip_prefix('(') {
        let close = inner.find(')')?;
        (inner[..close].trim(), inner[close + 1..].trim_start())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '{' || c == ':')
            .unwrap_or(rest.len());
        (rest[..end].trim(), rest[end..].trim_start())
    };
    if !is_single_metavariable(condition) {
        return None;
    }
    let body = parse_body_template(after)?;
    Some(NativeKind::If { body })
}

/// Parse the nested body section of a template.
///
/// `after` is either empty (no body constraint), a `{ ... }` section, or a
/// `: ...` suite (Python form). Outer `None` = unsupported inner shape.
fn parse_body_template(after: &str) -> Option<Option<BodyTemplate>> {
    let after = after.trim();
    if after.is_empty() {
        return Some(None);
    }
    let (inner, braced) = if let Some(rest) = after.strip_prefix('{') {
        (rest.strip_suffix('}')?, true)
    } else {
        (after.strip_prefix(':')?, false)
    };
    let inner = inner.trim();
    if inner.is_empty() {
        // `{}` matches an empty body; a bare `:` adds no constraint.
        return Some(braced.then_some(BodyTemplate::Exactly(0)));
    }
    if inner
        .strip_prefix("$$$")
        .is_some_and(|rest| rest.is_empty() || is_metavar_name(rest))
    {
        return Some(Some(BodyTemplate::Any));
    }
    if is_single_metavariable(inner) {
        return Some(Some(BodyTemplate::Exactly(1)));
    }
    None
}

/// `$NAME` — exactly one metavariable, not `$$$`.
fn is_single_metavariable(s: &str) -> bool {
    s.strip_prefix('$')
        .is_some_and(|rest| !rest.starts_with('$') && is_metavar_name(rest))
}

/// Identifier token check shared with index signature builders.
#[inline]
pub(crate) fn is_pattern_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// H-CONF-022 v2 (pass 54), tail grammar CORRECTED by F26-0182 (pass 69a):
/// canonical metavariable NAME := ASCII `[A-Z_][A-Z0-9_]*` — underscore-led
/// OK, digit tail OK, but a lowercase byte ANYWHERE in the tail (`$ABc`,
/// `$A1b`, `$A_b`, `$_a`) is NOT canonical. sg 0.45.2 probed 2026-09-04:
/// every mixed-case tail token parses empty / poisons the pattern, while
/// `$A`, `$A1`, `$A_B`, `$A_`, `$_`, `$_A` wildcard-match. Code identifiers
/// keep [`is_pattern_ident`]; only `$`-token consumers switch to this
/// grammar, so lowercase code identifiers (`a(1)` patterns, `def a` names)
/// stay literal.
fn is_metavar_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    match bytes.first() {
        Some(&first) if first == b'_' || first.is_ascii_uppercase() => {}
        _ => return false,
    }
    bytes[1..]
        .iter()
        .all(|&b| b == b'_' || b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// PASS 69a (F26-0182): classification of one `$`-token NAME against sg
/// 0.45.2's meta grammar (probed 2026-09-04, every cell). `None` — empty or
/// garbage-led names (`$3.14`, `$0x1F`, `$Ü`) — keeps the token's existing
/// registered class (loud residuals); those never route here.
///
/// PASS 71a (70c-F1/F3, probed 2026-09-06 with the literal faces in every
/// fixture): the class is DOLLARS-COUNT-AWARE. sg answers `$$`/`$$$`-led
/// NON-canonical tokens as literal code in js/ts (identifiers: `$$x`,
/// `$$ABc`, `$$_x`, `$$$x`, `$$$ABc`) and `$$`-led lowercase tokens as php
/// variable-variables (`$$tot`), but REFUSES the pattern for php
/// `$$ABc`/`$$$/lowercase`, python (rc=8) and rust (ERROR-node). Classifying
/// a 3-dollar name by the 1-dollar table made php `$$$u + 2` literal-answer
/// (and the codemod dry-run PLAN an edit) where sg refuses it — the F1
/// destructive fail-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DollarTokenClass {
    /// `$A`, `$_` — and the canonical 2/3-dollar runs (`$$A` since PASS 77b /
    /// F76-2, `$$$ARGS`): sg metavariables that every literal-lane consumer
    /// must exclude.
    Canonical,
    /// `$x` — lowercase-LED. NOT a meta: literal code where `$` is name
    /// syntax (js/ts identifiers, php variables), parse error elsewhere.
    LowercaseLed,
    /// `$ABc`, `$_a` — uppercase/underscore-LED with a lowercase byte in the
    /// tail. sg's tokenizer rejects them; php poisons the WHOLE pattern
    /// (ERROR node, answers nothing — probed `echo $ABc;` empty).
    MixedCase,
    /// `$$x`, `$$_x` — 2-dollar lowercase-led: js/ts identifier literal AND
    /// php variable-variable literal (sg answers both, face present).
    Dollar2Lowercase,
    /// `$$ABc` — 2-dollar MixedCase: js/ts identifier literal only; php/py/
    /// rust refuse the pattern.
    Dollar2Mixed,
    /// `$$$x` — 3-dollar lowercase-led: js/ts identifier literal ONLY
    /// (php/py/rust refuse — the F1 cell).
    Dollar3Lowercase,
    /// `$$$ABc` — 3-dollar MixedCase: js/ts identifier literal only.
    Dollar3Mixed,
}

fn dollar_name_class(name: &str) -> Option<DollarTokenClass> {
    if name.is_empty() {
        return None;
    }
    if is_metavar_name(name) {
        return Some(DollarTokenClass::Canonical);
    }
    let bytes = name.as_bytes();
    if !bytes
        .iter()
        .all(|&b| b == b'_' || b.is_ascii_alphanumeric())
    {
        return None;
    }
    match bytes[0] {
        b'a'..=b'z' => Some(DollarTokenClass::LowercaseLed),
        b'A'..=b'Z' | b'_' => Some(DollarTokenClass::MixedCase),
        _ => None,
    }
}

/// True when the pattern carries a `$`-token (1, 2, or 3 dollars) of ANY
/// non-canonical class. H-CONF-022 v2 (pass 54) covered only lowercase-LED
/// tokens; F26-0182 (pass 69a) extends the gate to MixedCase tails, which
/// the wide `[A-Za-z0-9_]` tail grammar let through the structural wildcard
/// lanes where sg answers nothing. PASS 71a (70c-F1/F3) extends it to the
/// 2/3-dollar non-canonical classes: the sg-refusing `$$`/`$$$` faces
/// (php MixedCase / 3-dollar, py, rust) keep the NeverMatches accepted-empty
/// class, while the sg-literal faces are intercepted ahead of this gate by
/// [`dollar_literal_lane`]. Canonical `$$A`/`$$_` runs stay with the caller
/// (universal-or-loud), and garbage-led tokens keep their existing classes.
fn pattern_has_noncanonical_metavar(p: &str) -> bool {
    dollar_token_classes(p).iter().any(|&class| {
        !matches!(class, DollarTokenClass::Canonical)
    })
}

/// PASS 75a (F74a-1/F74a-2): true when `p` is a call shape whose ONLY
/// non-canonical `$`-tokens are 1-dollar lowercase-led tokens sitting as
/// COMPLETE callee-head segments (before the first `(`, `::`/`->`/`.`
/// delimited). Those tokens are source variable text in the `$`-name
/// languages, and sg matches them literally; the structural call lanes then
/// compare the segment bytes exactly (`call_path_segment`). Any
/// mixed-case/2/3-dollar class, or any non-canonical token OUTSIDE the head
/// (arguments, operator shapes), keeps the registered NeverMatches class.
fn literal_variable_callee_admitted(p: &str) -> bool {
    let classes = dollar_token_classes(p);
    if !classes
        .iter()
        .all(|&class| matches!(class, DollarTokenClass::Canonical | DollarTokenClass::LowercaseLed))
        || !classes.contains(&DollarTokenClass::LowercaseLed)
    {
        return false;
    }
    let Some(open) = p.find('(') else {
        return false;
    };
    // Single-call discipline (the same grammar the call arm enforces): the
    // carve only routes patterns whose ONE argument list closes the pattern —
    // a multi-call tail (`$y->a()->b()`) keeps its registered lanes.
    let Some(close) = p.rfind(')') else {
        return false;
    };
    if close < open
        || !p[close + 1..].trim().is_empty()
        || p[open + 1..close].contains(['(', ')'])
        || validate_argument_pattern(p[open + 1..close].trim()).is_none()
    {
        return false;
    }
    let head = p[..open].trim();
    let normalized = head.replace("::", "\u{1}").replace("->", "\u{1}");
    let mut admitted = false;
    for segment in normalized.split(['\u{1}', '.']) {
        let segment = segment.trim();
        if segment.is_empty() || is_pure_metavariable(segment) || is_pattern_ident(segment) {
            continue;
        }
        match segment.strip_prefix('$') {
            Some(name) => {
                // A trailing `?` is the php nullsafe marker riding the left
                // segment (`$o?->m`) — strip it before the literal check;
                // `parse_call_path` still refuses the spelling so the
                // dedicated optional lane classifies it.
                let name = name.strip_suffix('?').unwrap_or(name);
                if dollar_name_class(name) == Some(DollarTokenClass::LowercaseLed) {
                    admitted = true;
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }
    admitted
}

/// PASS 75a (F74a-4): a namespace-qualified callee segment (`\Foo`,
/// `App\Models\User`) — backslash-led or interior `\`-separated plain
/// identifiers. The segment text is matched raw (the scope node's exact
/// source bytes), mirroring the re-keyed `call:` rows (74c-F3).
pub(crate) fn namespace_qualified_segment(segment: &str) -> bool {
    let led = segment.starts_with('\\');
    let rest = segment.strip_prefix('\\').unwrap_or(segment);
    let parts: Vec<&str> = rest.split('\\').collect();
    if parts.iter().any(|part| part.is_empty() || !is_pattern_ident(part)) {
        return false;
    }
    led || parts.len() > 1
}

/// Classify every `$`/`$$`/`$$$` token in the pattern (deduplicated).
/// PASS 71a (70c-F1/F3): the dollars COUNT participates — a 2-dollar run
/// never rode the 1-dollar table (that skip is what let php `$$$u` classify
/// LowercaseLed and literal-answer where sg refuses). PASS 77b (F76-2): a
/// canonical 2-dollar run classes `Canonical` like the 1/3-dollar canonical
/// cells — the pass-71a skip made `$$A` INVISIBLE to this table, so a php
/// member pattern like `$svc->run($$A)` looked all-lowercase-literal to
/// [`dollar_literal_lane`], was hijacked out of the structural member lane,
/// and answered silent `[]` where sg 0.45.2 answers. Canonical entries keep
/// every consumer contract: `pattern_has_noncanonical_metavar` and
/// [`literal_variable_callee_admitted`] ignore them, and
/// [`dollar_literal_lane`]'s arms all exclude `Canonical`, so only the
/// literal hijack changes (bare `$$A` keeps the universal lane through
/// `classify_native`'s own `$$` arm, which never consults this table).
fn dollar_token_classes(p: &str) -> Vec<DollarTokenClass> {
    let bytes = p.as_bytes();
    let mut classes = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut dollars = 0;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        let name_start = i + dollars;
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
        {
            name_end += 1;
        }
        if dollars >= 1 && dollars <= 3 {
            let name = p.get(name_start..name_end).unwrap_or("");
            let class = match (dollars, dollar_name_class(name)) {
                (_, None) => None,
                // PASS 77b (F76-2): a 2-dollar canonical name (`$$A`, `$$_`)
                // classes `Canonical` like the 1/3-dollar canonical cells —
                // the pass-71a skip (=> None) hid it from
                // [`dollar_literal_lane`] and hijacked php member patterns
                // into the literal lane (the F76-2 silent []).
                (2, Some(DollarTokenClass::LowercaseLed)) => {
                    Some(DollarTokenClass::Dollar2Lowercase)
                }
                (2, Some(DollarTokenClass::MixedCase)) => Some(DollarTokenClass::Dollar2Mixed),
                (3, Some(DollarTokenClass::LowercaseLed)) => {
                    Some(DollarTokenClass::Dollar3Lowercase)
                }
                (3, Some(DollarTokenClass::MixedCase)) => Some(DollarTokenClass::Dollar3Mixed),
                // 1-dollar runs keep the name class verbatim; the 3-dollar
                // CANONICAL run keeps the 1-dollar cell so canonical
                // mixes (`$$$A + $x`) keep their registered routing.
                (_, other) => other,
            };
            if let Some(class) = class {
                if !classes.contains(&class) {
                    classes.push(class);
                }
            }
        }
        i = name_end.max(i + 1);
    }
    classes
}

/// PASS 69a (F68a-1): true when sg answers the pattern's faces through
/// LITERAL code semantics — every `$`-token is a non-canonical NAME-class
/// token and the language gives `$` name syntax. sg 0.45.2 probed
/// 2026-09-04: js/ts parse such tokens as identifiers and answer the
/// literal faces (`const $x = 1` → its own line; `$ABc + 2` → its own
/// line); php answers lowercase-LED variable faces the same way, while a
/// php MixedCase token POISONS the whole pattern (ERROR node, answers
/// nothing) and canonical tokens keep the structural lanes. Mixed
/// canonical+non-canonical patterns (e.g. js `$A + $x`) keep today's
/// NeverMatches class — a registered sg-answers residual (r18 rider).
///
/// PASS 71a (70c-F1/F3, probed 2026-09-06): the rule is dollars-aware.
/// js/ts answer EVERY non-canonical run literally — 1-, 2-, and 3-dollar
/// (`$$x + 2`, `$$ABc + 2`, `$$_x + 2`, `$$$x + 2`, `$$$ABc + 2` — own line
/// only). php answers 1- and 2-dollar LOWERCASE-led runs (`$$tot + 1` is a
/// variable-variable) but REFUSES 3-dollar runs of any name — admitting
/// `$$$u + 2` planned a codemod edit where sg exits 1 (70c-F1, destructive).
/// Canonical `$$A`/`$$_` runs never land here (the universal lane keeps
/// them); py/rust never enter the lane (their non-canonical faces stay
/// NeverMatches, matching sg's rc=8 / ERROR-node refusals).
fn dollar_literal_lane(lang: Language, pattern: &str) -> bool {
    if !pattern.contains('$') {
        return false;
    }
    let classes = dollar_token_classes(pattern);
    if classes.is_empty() {
        return false;
    }
    match lang {
        Language::JavaScript | Language::TypeScript => classes
            .iter()
            .all(|&class| class != DollarTokenClass::Canonical),
        Language::Php => classes
            .iter()
            .all(|&class| {
                matches!(
                    class,
                    DollarTokenClass::LowercaseLed | DollarTokenClass::Dollar2Lowercase
                )
            }),
        _ => false,
    }
}

fn is_pure_metavariable(arg: &str) -> bool {
    let arg = arg.trim();
    arg.strip_prefix("$$$")
        .or_else(|| arg.strip_prefix('$'))
        .is_some_and(is_metavar_name)
        // PASS 75a (F74a-3): a canonical 2-dollar name in an argument slot
        // behaves EXACTLY like `$NAME` — sg 0.45.2 answers `g($$A)` with the
        // same line set and the same capture key `A` as `g($A)`.
        || arg.strip_prefix("$$").is_some_and(is_metavar_name)
}

fn validate_argument_pattern(arguments: &str) -> Option<()> {
    let arguments = arguments.trim();
    if arguments.is_empty() || arguments == "$$$" {
        return Some(());
    }
    let parts = arguments.split(',').map(str::trim).collect::<Vec<_>>();
    if !parts.iter().all(|part| is_pure_metavariable(part))
        || (parts.len() > 1 && parts.iter().any(|part| part.starts_with("$$$")))
    {
        return None;
    }
    Some(())
}

fn pattern_argument_text(pattern: &str) -> Option<&str> {
    let (pattern, _) = strip_declaration_modifiers(pattern);
    let open = pattern.find('(')?;
    let tail = pattern.get(open + 1..)?;
    let close = tail.find(')')?;
    Some(tail[..close].trim())
}

fn argument_template(pattern: &str) -> Option<ArgumentTemplate> {
    let arguments = pattern_argument_text(pattern)?;
    if arguments.starts_with("$$$") {
        Some(ArgumentTemplate::Any)
    } else if arguments.is_empty() {
        Some(ArgumentTemplate::Exactly(0))
    } else {
        Some(ArgumentTemplate::Exactly(arguments.split(',').count()))
    }
}

fn parse_call_path(callee: &str) -> Option<Vec<Option<String>>> {
    let callee = callee.strip_prefix("::").unwrap_or(callee);
    // PASS 75a (F74a-2): php spells the member connector `->` (the nullsafe
    // `?->` keeps its dedicated lane — the leftover `?` fails the segment
    // check below, exactly like the `.`-lane refusal of `?.`). `::` and `->`
    // both normalize to the dotted separator.
    let normalized = callee.replace("::", ".").replace("->", ".");
    if normalized.is_empty()
        || normalized.starts_with('.')
        || normalized.ends_with('.')
        || normalized.contains("..")
    {
        return None;
    }
    let mut segments = Vec::new();
    for part in normalized.split('.') {
        segments.push(call_path_segment(part.trim())?);
    }
    (!segments.is_empty()).then_some(segments)
}

/// One callee-path segment: `$META`, a plain identifier, a literal
/// lowercase-led `$variable` token (PASS 75a F74a-1 — source variable text in
/// the `$`-name languages, matched byte-exactly like sg), or a
/// namespace-qualified scope (PASS 75a F74a-4 / 74c-F3).
fn call_path_segment(part: &str) -> Option<Option<String>> {
    if is_pure_metavariable(part) {
        return Some(None);
    }
    if is_pattern_ident(part) {
        return Some(Some(part.to_string()));
    }
    if let Some(name) = part.strip_prefix('$') {
        if dollar_name_class(name) == Some(DollarTokenClass::LowercaseLed) {
            return Some(Some(part.to_string()));
        }
    }
    if namespace_qualified_segment(part) {
        return Some(Some(part.to_string()));
    }
    None
}

thread_local! {
    /// Per-thread reusable parsers (Amdahl: `Parser::new` + `set_language` were
    /// paid once per file in the rayon span; now once per thread per language).
    static PARSERS: RefCell<HashMap<Language, Parser>> = RefCell::new(HashMap::new());
}

fn parse_source(lang: Language, source: &str) -> anyhow::Result<tree_sitter::Tree> {
    PARSERS.with(|cell| {
        let mut parsers = cell.borrow_mut();
        let parser = match parsers.entry(lang) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut parser = Parser::new();
                parser
                    .set_language(&tree_sitter_language(lang))
                    .map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
                entry.insert(parser)
            }
        };
        parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("failed to parse source"))
    })
}

/// Process-wide compiled query cache (Amdahl: `Query::new` compiled every table
/// query per file in the rayon span; queries are `'static` table entries, so
/// key by pointer and compile once per process).
type QueryCache = RwLock<HashMap<(Language, usize), Option<Arc<Query>>>>;

fn compiled_query(
    language: &tree_sitter::Language,
    lang: Language,
    source: &'static str,
) -> Option<Arc<Query>> {
    static CACHE: OnceLock<QueryCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let key = (lang, source.as_ptr() as usize);
    if let Some(cached) = cache.read().ok()?.get(&key) {
        return cached.clone();
    }
    let compiled = Query::new(language, source).ok().map(Arc::new);
    if let Ok(mut writer) = cache.write() {
        writer.insert(key, compiled.clone());
    }
    compiled
}

/// PASS 65 (F64-6): bare statement heads (`break`, `continue`, `throw`,
/// `yield`, `raise` — with or without a trailing `;`) are KIND templates:
/// sg answers every statement of the head's family regardless of arguments
/// or semicolon (probed java/csharp/rust/python 0.45.2: rust `break`
/// answers `break 9;` and `break;`, csharp `throw` answers every
/// throw_statement, python `yield`/`raise` answer the arg-ful forms too).
/// `Some(matches)` = the lane engaged; `None` = not a bare-head pattern.
fn match_bare_statement_kind(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let trimmed = pattern.trim();
    if trimmed.contains(char::is_whitespace) || trimmed.contains('$') {
        return None;
    }
    let keyword = trimmed.trim_end_matches(';').trim();
    // PASS 67a (F66a-8): ruby has no `raise_statement` kind — raises are
    // `call` nodes whose method identifier the sg bare pattern matches at
    // IDENTIFIER level (probed 0.45.2: the raise family {2,4,6,12,16} on the
    // h_rb corpus, including the pure bare statement). A kind-level arm here
    // can only silence (the dead `raise_statement` lookup) or overmatch
    // (bare `call` would answer every ruby call), and the full literal lane
    // would over-answer: its whole-node-text arm matches whitespace-led
    // wrapper nodes (a `rescue`-clause body) whose trimmed text equals the
    // pattern — rows sg never emits. Identifiers only.
    if keyword == "raise" && lang == Language::Ruby {
        return Some(collect_identifier_matches(lang, source, pattern));
    }
    let kinds: &[&str] = match keyword {
        "break" => &["break_statement", "break_expression"],
        "continue" => &["continue_statement", "continue_expression"],
        "throw" => &["throw_statement"],
        "yield" => &["yield", "yield_statement"],
        "raise" => &["raise_statement"],
        _ => return None,
    };
    Some(collect_kind_matches(lang, source, pattern, kinds))
}

/// PASS 67a (F66a-8): sg's bare ruby pattern matches at IDENTIFIER level —
/// every identifier node whose text equals the pattern (the method slot of
/// every raise call, the bare statement included), outside comments and
/// strings. One match per node, byte-range deduped.
fn collect_identifier_matches(lang: Language, source: &str, pattern: &str) -> Vec<PatternMatch> {
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk_identifier_matches(tree.root_node(), source, pattern, &mut seen, &mut out);
    out
}

fn walk_identifier_matches(
    node: Node,
    source: &str,
    pattern: &str,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if is_in_comment_or_string(&node) {
        return;
    }
    if identifier_matches(&node, source, pattern) {
        let (byte_start, byte_end) = (node.start_byte(), node.end_byte());
        if seen.insert((byte_start, byte_end)) {
            let mut captures = BTreeMap::new();
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            let (line_start, line_end) = node_lines(&node, source);
            out.push(PatternMatch {
                line_start,
                line_end,
                byte_start,
                byte_end,
                excerpt: excerpt_for_node(&node, source, pattern),
                captures,
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_identifier_matches(child, source, pattern, seen, out);
    }
}

/// Walk every node of `kinds` (childless or not — the kind IS the
/// constraint) and push one match per node, byte-range deduped.
fn collect_kind_matches(    lang: Language,
    source: &str,
    pattern: &str,
    kinds: &[&str],
) -> Vec<PatternMatch> {
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk_kind_list(tree.root_node(), source, pattern, kinds, &mut seen, &mut out);
    out
}

fn walk_kind_list(
    node: Node,
    source: &str,
    pattern: &str,
    kinds: &[&str],
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if kinds.contains(&node.kind()) && !is_in_comment_or_string(&node) {
        let (byte_start, byte_end) = (node.start_byte(), node.end_byte());
        if seen.insert((byte_start, byte_end)) {
            let mut captures = BTreeMap::new();
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            let (line_start, line_end) = node_lines(&node, source);
            out.push(PatternMatch {
                line_start,
                line_end,
                byte_start,
                byte_end,
                excerpt: excerpt_for_node(&node, source, pattern),
                captures,
            });
        }
        // sg reports ONE hit per statement of the family: grammars that
        // wrap the statement kind inside another kind of the same set
        // (python `yield_statement` wrapping `yield`) must not answer
        // twice for one site — the outermost node is the statement.
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kind_list(child, source, pattern, kinds, seen, out);
    }
}

/// PASS 65 (F64-3): the C/C++ conditional-compilation directives answer
/// kind-level on the directive head — sg ignores the branch body (probes:
/// `#ifdef $A` answers both corpora with A = the name, `#if $A` binds the
/// whole condition, `#if defined($A)` binds the metavar INSIDE the condition
/// text (A = FEATURE, probed 0.45.2)). `Some(matches)` = the lane engaged;
/// `None` = not a directive template.
///
/// The condition acceptance rule is ONE consistent three-form family shared
/// with [`preproc_directive_supported`] (see [`CondBinding`]).
fn match_preproc_directive(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let (directive, condition) = match pattern.split_once(char::is_whitespace) {
        Some((d, rest)) => (d, rest.trim()),
        None => (pattern, ""),
    };
    let kind = match (directive, lang) {
        ("#ifdef", _) => "preproc_ifdef",
        // PASS 67a (F66a-2): tree-sitter-c AND tree-sitter-cpp fold BOTH
        // spellings into `preproc_ifdef` — the grammar rule is
        // `choice(preprocessor('ifdef'), preprocessor('ifndef'))` and there
        // is no dedicated `preproc_ifndef` kind in either vendored grammar.
        // The anonymous head-token check in the walk disambiguates them.
        // (The previous c-specific `preproc_ifndef` arm named a kind the c
        // tree never produces and silently answered 0 where sg answers.)
        ("#ifndef", _) => "preproc_ifdef",
        ("#if", _) => "preproc_if",
        // PASS 67a (F66a-5): the object-like macro define (function-like
        // spellings are `preproc_function_def` and refuse at the tail parse).
        ("#define", _) => "preproc_def",
        _ => return None,
    };
    // PASS 67a (F66a-5): `#define` carries a name binding plus an optional
    // value binding; every other directive keeps the single condition
    // binding.
    let (binding, value_binding) = if directive == "#define" {
        parse_define_tail(condition)?
    } else {
        (CondBinding::parse(condition)?, None)
    };
    let tree = parse_source(lang, source).ok()?;
    let mut out = Vec::new();
    walk_preproc_directive(
        tree.root_node(),
        source,
        pattern,
        directive,
        kind,
        &binding,
        value_binding.as_ref(),
        &mut out,
    );
    Some(out)
}

/// How a directive-head condition template unifies against a source
/// condition (F64-3): a LONE metavariable binds the whole head text, a
/// metavar-free text compares exactly, and a single metavariable nested in
/// literal text (`defined($A)`) binds the in-between span (sg probes 0.45.2:
/// A = the argument identifier). PASS 67a (F66a-4): conditions carrying 2+
/// canonical metavariables unify STRUCTURALLY through a parsed condition
/// template — sg binds a metavariable leaf to its whole candidate subtree
/// (`#if $A && defined($B)` binds A to the entire left operand
/// `defined(FEATURE_A)`, probed 0.45.2), which text slicing cannot express.
/// Any other `$` shape (`$$$`, non-canonical) refuses the lane — the
/// registered loud contract.
enum CondBinding {
    Whole(String),
    Exact(String),
    Inner(String, String, String),
    Structural(GeneralTemplate),
}

impl CondBinding {
    fn parse(condition: &str) -> Option<CondBinding> {
        if let Some(name) = capture_name(condition) {
            return Some(CondBinding::Whole(name.to_string()));
        }
        // PASS 67a (F66a-4): 2+ canonical metavariables go through the
        // structural condition template (a `#if … #endif` document parse
        // under the C grammar — cpp extends the same preproc rules, and both
        // engines share the grammars, so acceptance mirrors sg's by
        // construction). A template that does not parse clean refuses the
        // lane (the loud contract).
        if canonical_metavariable_count(condition) >= 2 {
            return structural_condition_template(condition).map(CondBinding::Structural);
        }
        if !condition.contains('$') {
            return (!condition.is_empty()).then(|| CondBinding::Exact(condition.to_string()));
        }
        // A single canonical `$NAME` nested inside otherwise-literal text.
        // The literal prefix/suffix may contain any punctuation (`defined(`
        // does); the BOUND span's cleanliness is checked at unify time.
        let dollar = condition.find('$')?;
        let rest = &condition[dollar + 1..];
        let name_len = rest
            .bytes()
            .take_while(|&b| b == b'_' || b.is_ascii_alphanumeric())
            .count();
        let name = &rest[..name_len];
        if !is_metavar_name(name) || rest[name_len..].contains('$') {
            return None;
        }
        Some(CondBinding::Inner(
            name.to_string(),
            condition[..dollar].to_string(),
            rest[name_len..].to_string(),
        ))
    }

    /// Unify the binding against the directive head's text (and node, for
    /// the structural arm).
    fn unify(
        &self,
        head_text: &str,
        head_node: Option<Node>,
        source: &str,
        captures: &mut BTreeMap<String, String>,
    ) -> bool {
        match self {
            CondBinding::Whole(name) => bind_capture(captures, name, head_text).is_some(),
            CondBinding::Exact(want) => head_text == want,
            CondBinding::Inner(name, prefix, suffix) => {
                let Some(middle) = head_text
                    .strip_prefix(prefix.as_str())
                    .and_then(|rest| rest.strip_suffix(suffix.as_str()))
                else {
                    return false;
                };
                let binds_clean = !middle.is_empty()
                    && !middle.contains(['(', ')'])
                    && !middle.chars().any(char::is_whitespace);
                binds_clean && bind_capture(captures, name, middle).is_some()
            }
            CondBinding::Structural(template) => {
                let Some(head) = head_node else {
                    return false;
                };
                // The condition template node sits at
                // program → preproc_if → first named child (the `#if` token
                // and the newline are anonymous).
                let Some(cond) = template
                    .tree
                    .root_node()
                    .named_child(0)
                    .and_then(|directive| directive.named_child(0))
                else {
                    return false;
                };
                general_eq(template, cond, head, source, captures).is_some()
            }
        }
    }
}

/// PASS 67a (F66a-4): the number of canonical single-`$` metavariables in a
/// condition text (`$$$` runs and non-canonical names do not count).
fn canonical_metavariable_count(condition: &str) -> usize {
    let bytes = condition.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut dollars = 0usize;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        let name_start = i + dollars;
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
        {
            name_end += 1;
        }
        if dollars == 1 && is_metavar_name(&condition[name_start..name_end]) {
            count += 1;
        }
        i = name_end.max(i + 1);
    }
    count
}

/// PASS 67a (F66a-4): parse the substituted condition inside a COMPLETE
/// `#if … #endif` directive document (the condition must sit on its own
/// line — the preproc grammar demands the newline). The template root is
/// the `preproc_if` node; its first named child is the condition that
/// [`general_eq`] unifies against a candidate condition node.
fn structural_condition_template(condition: &str) -> Option<GeneralTemplate> {
    let (substituted, placeholders, multi_names) = substitute_general_metavariables(condition)?;
    let doc = format!("#if {substituted}\n#endif\n");
    let tree = parse_source(Language::C, &doc).ok()?;
    let root = tree.root_node();
    if root.has_error() || root.named_child(0).is_none() {
        return None;
    }
    Some(GeneralTemplate {
        doc,
        tree,
        placeholders,
        multi_names,
        span: None,
        root_kind: String::new(),
    })
}

/// PASS 67a (F66a-5): the `#define` tail — a name binding plus an optional
/// value binding (`#define $A`, `#define $A $B`, `#define NAME`,
/// `#define NAME $V`). The name must be a plain identifier or metavariable
/// (function-like spellings are `preproc_function_def`, a different kind,
/// and refuse here); the value must be a lone metavariable or
/// metavar-free text (sg: `#define $A $B` binds B = the value text and
/// answers only value-carrying defines; `#define $A` answers both shapes).
fn parse_define_tail(tail: &str) -> Option<(CondBinding, Option<CondBinding>)> {
    let mut parts = tail.split_whitespace();
    let name_tok = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    if rest.len() > 1 {
        return None;
    }
    if name_tok.contains('(') || name_tok.contains(')') {
        return None;
    }
    let name_binding = match capture_name(name_tok) {
        Some(name) => CondBinding::Whole(name.to_string()),
        None if is_pattern_ident(name_tok) => CondBinding::Exact(name_tok.to_string()),
        None => return None,
    };
    let value_binding = match rest.first().copied() {
        None => None,
        Some(tok) => match capture_name(tok) {
            Some(name) => Some(CondBinding::Whole(name.to_string())),
            None if !tok.contains('$') => Some(CondBinding::Exact(tok.to_string())),
            None => return None,
        },
    };
    Some((name_binding, value_binding))
}

/// PASS 67a (F66a-5): a spelled `#define` value binding demands the value
/// child (the second named child of `preproc_def`) and unifies against it;
/// with no value spelled the directive answers value-carrying defines too
/// (sg's head-level rule).
fn define_value_matches(
    value: Option<&CondBinding>,
    node: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    let Some(binding) = value else {
        return true;
    };
    let Some(value_node) = node.named_child(1) else {
        return false;
    };
    node_text(&value_node, source)
        .is_some_and(|text| binding.unify(text, Some(value_node), source, captures))
}

fn walk_preproc_directive(
    node: Node,
    source: &str,
    pattern: &str,
    directive: &str,
    kind: &str,
    binding: &CondBinding,
    value_binding: Option<&CondBinding>,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == kind && !is_in_comment_or_string(&node) {
        // The directive keyword itself must agree — some grammars fold the
        // whole conditional family into shared region kinds, and only the
        // leading anonymous token (`#ifdef` vs `#ifndef`) tells them apart.
        let directive_token_ok = node
            .child(0)
            .is_some_and(|token| !token.is_named() && token.kind() == directive);
        // The directive head is the FIRST named child: the name identifier
        // for ifdef/ifndef, the condition expression for `#if`, the macro
        // name for `#define`. Branch-body children are ignored — exactly
        // sg's directive-head semantics.
        if directive_token_ok {
            if let Some(head) = node.named_child(0) {
                let mut head_captures = BTreeMap::new();
                let unified = node_text(&head, source).is_some_and(|text| {
                    binding.unify(text, Some(head), source, &mut head_captures)
                });
                // PASS 67a (F66a-5): a spelled `#define` value must exist
                // and unify; an unspelled value ignores the source value.
                if unified
                    && define_value_matches(value_binding, &node, source, &mut head_captures)
                {
                    let mut captures = BTreeMap::new();
                    if let Some(text) = node_text(&node, source) {
                        captures.insert("MATCH".to_string(), text.to_string());
                    }
                    captures.extend(head_captures);
                    let (line_start, line_end) = node_lines(&node, source);
                    out.push(PatternMatch {
                        line_start,
                        line_end,
                        byte_start: node.start_byte(),
                        byte_end: node.end_byte(),
                        excerpt: excerpt_for_node(&node, source, pattern),
                        captures,
                    });
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_preproc_directive(
            child,
            source,
            pattern,
            directive,
            kind,
            binding,
            value_binding,
            out,
        );
    }
}

/// PASS 65 (F64-3): answerability of the conditional-directive family —
/// `Some(true)` when the dedicated lane answers the pattern (the shared
/// [`CondBinding`] rule: lone metavariable, metavar-free text, a single
/// metavariable nested in literal condition text, or — PASS 67a (F66a-4) —
/// a parseable structural condition template over 2+ canonical
/// metavariables), `Some(false)`/`None` keeping the registered loud
/// contracts. PASS 67a (F66a-5): `#define` joins through
/// [`parse_define_tail`].
fn preproc_directive_supported(pattern: &str) -> Option<bool> {
    let (directive, condition) = pattern.trim().split_once(char::is_whitespace)?;
    match directive {
        "#ifdef" | "#ifndef" | "#if" => Some(CondBinding::parse(condition.trim()).is_some()),
        "#define" => Some(parse_define_tail(condition).is_some()),
        _ => None,
    }
}

fn match_structural(
    lang: Language,
    source: &str,
    pattern: &str,
    kind: &NativeKind,
) -> anyhow::Result<Vec<PatternMatch>> {
    let language = tree_sitter_language(lang);
    let tree = parse_source(lang, source)?;
    let mut out = Vec::new();
    let arguments = argument_template(pattern);
    let (_, declaration_modifiers) = strip_declaration_modifiers(pattern);
    match kind {
        NativeKind::Function { name, body } => {
            run_queries(
                &language,
                lang,
                tree.root_node(),
                source,
                queries_for(FUNCTION_QUERY_TABLE, lang),
                declaration_modifiers,
                name.as_deref(),
                None,
                body.as_ref(),
                arguments.as_ref(),
                pattern,
                &mut out,
            )?;
        }
        NativeKind::Class { keyword, name } => {
            run_queries(
                &language,
                lang,
                tree.root_node(),
                source,
                class_queries_for(lang, keyword),
                declaration_modifiers,
                name.as_deref(),
                Some((lang, *keyword)),
                None,
                None,
                pattern,
                &mut out,
            )?;
        }
        NativeKind::Call { path } => {
            walk_calls(
                tree.root_node(),
                source,
                pattern,
                path,
                arguments.as_ref(),
                &mut out,
            );
        }
        NativeKind::MemberCall { path } => {
            walk_member_calls(
                tree.root_node(),
                source,
                pattern,
                path,
                arguments.as_ref(),
                &mut out,
            );
        }
        NativeKind::MemberCallChain { segments } => {
            walk_member_call_chains(
                tree.root_node(),
                source,
                pattern,
                segments,
                &mut out,
            );
        }
        NativeKind::CallChain { segments } => {
            walk_call_chains(lang, tree.root_node(), source, pattern, segments, &mut out);
        }
        NativeKind::OptionalCallChain {
            segments,
            optional_flags,
        } => {
            walk_optional_call_chains(
                tree.root_node(),
                source,
                pattern,
                segments,
                &optional_flags,
                &mut out,
            );
        }
        NativeKind::OptionalCall {
            head_capture,
            head_literal,
            tail_capture,
            tail_literal,
        } => {
            walk_optional_calls(
                tree.root_node(),
                source,
                pattern,
                head_capture.as_deref(),
                head_literal.as_deref(),
                tail_capture.as_deref(),
                tail_literal.as_deref(),
                arguments.as_ref(),
                &mut out,
            );
        }
        NativeKind::If { body } => {
            walk_ifs(tree.root_node(), source, pattern, body.as_ref(), &mut out);
        }
        // H-CONF-022 v2 (pass 54): NeverMatches is valid ingress with zero
        // candidates — ok:true empty, exactly sg's accepted-empty faces.
        NativeKind::NeverMatches => return Ok(Vec::new()),
        NativeKind::Universal { name } => {
            walk_universal(tree.root_node(), source, pattern, name.as_deref(), &mut out);
        }
    }
    Ok(out)
}

/// H-CONF-022 v2 (pass 54): `$$NAME` / `$$_` universal matching — visit EVERY
/// node (anonymous tokens included, no comment/string skip: sg pins comment,
/// docstring, and string rows) and push each via the shared byte-range dedup.
/// A named metavariable binds the node text through `bind_capture`, so the
/// reserved `MATCH` key keeps its overwrite semantics.
fn walk_universal(
    node: Node,
    source: &str,
    pattern: &str,
    name: Option<&str>,
    out: &mut Vec<PatternMatch>,
) {
    // P7 (pass 65): hash-set dedup — the previous linear scan over `out`
    // made the universal walk quadratic (the pass-64 perf finding). The set
    // only gates the push; emission order is unchanged.
    let mut seen = std::collections::HashSet::new();
    // Pass 65a keep-gate: the dedup alone left the walk quadratic —
    // `node_lines` re-counts `\n` from byte 0 twice per node, and the
    // excerpt fallback re-scanned the whole source per long node. Pre-order
    // start bytes are strictly increasing, so a forward (byte,line) cursor
    // amortizes the start-line computation to O(source) across the whole
    // walk; the end line is a local scan over the node's own span. Both
    // produce exactly the values `node_lines` would.
    let mut cursor = (0usize, 1u32);
    walk_universal_inner(node, source, pattern, name, &mut seen, &mut cursor, out);
}

/// Forward line cursor: `line_at` is the 1-based line number of `byte_at`.
/// Only valid while bytes are visited in non-decreasing order (pre-order).
type LineCursor = (usize, u32);

fn walk_universal_inner(
    node: Node,
    source: &str,
    pattern: &str,
    name: Option<&str>,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    line_cursor: &mut LineCursor,
    out: &mut Vec<PatternMatch>,
) {
    let byte_start = node.start_byte();
    let byte_end = node.end_byte();
    // Advance the monotonic cursor to this node's start (pre-order guarantees
    // byte_start >= cursor byte).
    let from = line_cursor.0.min(byte_start);
    let newlines = source.as_bytes()[from..byte_start]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32;
    let line_start = if byte_start >= line_cursor.0 {
        line_cursor.1 + newlines
    } else {
        byte_to_line(source, byte_start)
    };
    line_cursor.0 = byte_start;
    line_cursor.1 = line_start;
    // End line: newlines inside the node's own span (local, O(span)).
    let line_end = line_start
        + source.as_bytes()[byte_start..byte_end.min(source.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32;
    if seen.insert((byte_start, byte_end)) {
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
            if let Some(name) = name {
                bind_capture(&mut captures, name, text);
            }
        }
        out.push(PatternMatch {
            line_start,
            line_end,
            byte_start,
            byte_end,
            excerpt: excerpt_for_node(&node, source, pattern),
            captures,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_universal_inner(child, source, pattern, name, seen, line_cursor, out);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_queries(
    language: &tree_sitter::Language,
    lang: Language,
    root: Node,
    source: &str,
    queries: &'static [&'static str],
    declaration_modifiers: Option<&str>,
    name_filter: Option<&str>,
    class_filter: Option<(Language, &str)>,
    body_filter: Option<&BodyTemplate>,
    argument_filter: Option<&ArgumentTemplate>,
    pattern: &str,
    out: &mut Vec<PatternMatch>,
) -> anyhow::Result<()> {
    for qsrc in queries {
        let Some(query) = compiled_query(language, lang, qsrc) else {
            continue;
        };
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let name_idx = query.capture_index_for_name("name");
        let match_idx = query.capture_index_for_name("match");
        while let Some(m) = matches.next() {
            let mut name_text: Option<&str> = None;
            let mut match_node: Option<Node> = None;
            for cap in m.captures {
                if name_idx == Some(cap.index) {
                    name_text = node_text(&cap.node, source);
                }
                if match_idx == Some(cap.index) {
                    match_node = Some(cap.node);
                }
            }
            let node = match_node.or_else(|| m.captures.first().map(|c| c.node));
            let Some(node) = node else {
                continue;
            };
            if is_in_comment_or_string(&node) {
                continue;
            }
            if !declaration_modifiers_match(&node, source, declaration_modifiers) {
                continue;
            }
            if let Some((lang, keyword)) = class_filter {
                if !class_keyword_matches(lang, &node, source, keyword) {
                    continue;
                }
            }
            if let Some(want) = name_filter {
                if name_text != Some(want) {
                    continue;
                }
            }
            if let Some(template) = body_filter {
                if !function_body_matches(&node, template) {
                    continue;
                }
            }
            // EXP-005 (H-CONF-005): a native function template never carries a
            // return-type section (those shapes are beyond the native subset
            // and fail closed), so per sg strictness a matched declaration
            // must not declare one either. Class templates are shape-agnostic.
            if class_filter.is_none() && !function_return_type_absent(&node) {
                continue;
            }
            if !arguments_match(&node, argument_filter, &["parameters"]) {
                continue;
            }
            push_match(&node, source, pattern, name_text, out);
        }
    }
    Ok(())
}

fn declaration_modifiers_match(node: &Node, source: &str, modifiers: Option<&str>) -> bool {
    let Some(modifiers) = modifiers else {
        return true;
    };
    [Some(*node), node.parent()]
        .into_iter()
        .flatten()
        .filter_map(|candidate| node_text(&candidate, source))
        .any(|text| {
            text.trim_start()
                .strip_prefix(modifiers)
                .and_then(|rest| rest.chars().next())
                .is_some_and(char::is_whitespace)
        })
}

fn class_keyword_matches(lang: Language, node: &Node, source: &str, keyword: &str) -> bool {
    if lang != Language::Kotlin {
        return true;
    }
    let kind = kotlin_class_keyword(node, source);
    match keyword {
        "class" => kind == "class",
        "interface" => kind == "interface",
        "type" => true,
        _ => false,
    }
}

fn kotlin_class_keyword(node: &Node, source: &str) -> &'static str {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "modifiers" | "class_modifier")
            && kotlin_class_keyword(&child, source) == "enum"
        {
            return "enum";
        }
        if let Some(text) = node_text(&child, source) {
            match text.trim() {
                "enum" => return "enum",
                "interface" => return "interface",
                _ => {}
            }
        }
    }
    "class"
}

fn arguments_match(node: &Node, template: Option<&ArgumentTemplate>, fields: &[&str]) -> bool {
    let Some(template) = template else {
        return true;
    };
    let count = argument_nodes(node, fields).map_or(0, |arguments| arguments.len());
    match template {
        ArgumentTemplate::Any => true,
        ArgumentTemplate::Exactly(expected) => count == *expected,
    }
}

fn argument_container<'a>(node: &Node<'a>, fields: &[&str]) -> Option<Node<'a>> {
    if let Some(container) = fields
        .iter()
        .find_map(|field| node.child_by_field_name(field))
    {
        return Some(container);
    }
    let mut cursor = node.walk();
    let named: Vec<Node<'a>> = node.named_children(&mut cursor).collect();
    // EXP-005 (H-CONF-010): Swift/Kotlin declarations carry no `parameters`
    // field; their parameter list is a plain named child. Accept it directly
    // so arity counting stays honest instead of descending into a leaf
    // parameter (which silently made every arity look satisfiable).
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

fn argument_nodes<'a>(node: &Node<'a>, fields: &[&str]) -> Option<Vec<Node<'a>>> {
    let container = argument_container(node, fields)?;
    let mut cursor = container.walk();
    Some(
        container
            .named_children(&mut cursor)
            .filter(|child| !is_trivia_kind(child.kind()))
            .collect(),
    )
}

fn walk_calls(
    node: Node,
    source: &str,
    pattern: &str,
    path: &[Option<String>],
    arguments: Option<&ArgumentTemplate>,
    out: &mut Vec<PatternMatch>,
) {
    // PASS 75a (F74a-2): the php member-call kinds are the dedicated
    // MemberCall/OptionalCall lanes' candidates — a plain path (dot- or
    // name-spelled) never answers a `->`/`?->` call site (sg connector
    // token-exactness, the registered pass-22/64-7 semantics).
    let callee = if matches!(
        node.kind(),
        "member_call_expression" | "nullsafe_member_call_expression"
    ) {
        None
    } else {
        call_match_path(&node, source, path).filter(|_| arguments_match(&node, arguments, &["arguments"]))
    };
    if let Some(callee) = callee {
        push_match(&node, source, pattern, Some(&callee.join(".")), out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls(child, source, pattern, path, arguments, out);
    }
}

/// PASS 75a (F74a-2): match the php plain `->` member-call spelling against
/// `member_call_expression` candidates only (nullsafe `?->` sites stay on the
/// token-exact optional lane). The candidate callee decomposes through
/// `call_callee`'s member arm into the full object->name chain; the empty
/// synthetic shape vetoes unresolvable receivers like sg's empty answers.
fn walk_member_calls(
    node: Node,
    source: &str,
    pattern: &str,
    path: &[Option<String>],
    arguments: Option<&ArgumentTemplate>,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "member_call_expression" && !is_in_comment_or_string(&node) {
        let matched = call_callee(&node, source)
            .filter(|(segments, _)| !segments.is_empty())
            .filter(|(segments, _)| path_matches(segments, path))
            .filter(|_| arguments_match(&node, arguments, &["arguments"]));
        if matched.is_some() {
            push_match(&node, source, pattern, None, out);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_member_calls(child, source, pattern, path, arguments, out);
    }
}

/// PASS 77b (F76-1): match php `->` member-call CHAIN templates against
/// `member_call_expression` candidates (the [`NativeKind::MemberCallChain`]
/// lane). Every chain node in the tree is visited, so a pattern answers the
/// outermost chain whose per-segment shape matches exactly AND the inner
/// prefix subnodes whose own depth equals the pattern's — the same
/// prefix-subnode contract the pass-75 flat lane already shows on chains
/// (`$obj->m($A)` answers the head of `$obj->m($w)->n($u)`). Nullsafe links
/// are never visited here (token-exact optional lane).
fn walk_member_call_chains(
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "member_call_expression" && !is_in_comment_or_string(&node) {
        if let Some(captures) = member_chain_matches(&node, source, segments) {
            let (line_start, line_end) = node_lines(&node, source);
            let excerpt = excerpt_for_node(&node, source, pattern);
            let mut captures = captures;
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            out.push(PatternMatch {
                line_start,
                line_end,
                byte_start: node.start_byte(),
                byte_end: node.end_byte(),
                excerpt,
                captures,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_member_call_chains(child, source, pattern, segments, out);
    }
}

/// One decomposed candidate chain segment: its name text, the argument NODES
/// when the segment is a real call (`None` = receiver/property link — no
/// call happened, so a pattern call segment can never sit here), and the
/// argument-list content text for whole-list captures.
struct MemberChainSegment<'a> {
    text: String,
    args: Option<Vec<Node<'a>>>,
    args_content: Option<String>,
}

/// Exact-depth chain unification: the candidate's member-call decomposition
/// must have EXACTLY the pattern's segment count (inner prefix subnodes are
/// separate candidates the walk visits, never an absorb here), every segment
/// name unifies through `bind_capture` (the same-name veto), every pattern
/// call segment must land on a REAL call link with matching arity, and
/// argument metavars bind like the flat lane (`$A`/`$$A` → single namespace
/// key `A`; `$$$A` → multi). A receiver (argument-free) pattern segment
/// carries no argument contract, mirroring [`chain_matches`].
fn member_chain_matches(
    node: &Node,
    source: &str,
    segments: &[CallChainSegment],
) -> Option<BTreeMap<String, String>> {
    let mut actual = Vec::new();
    member_chain_segments(node, source, &mut actual)?;
    if actual.len() != segments.len() {
        return None;
    }
    let mut captures = BTreeMap::new();
    for (segment, candidate) in segments.iter().zip(actual.iter()) {
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want != &candidate.text => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, &candidate.text)?,
            (None, None) => return None,
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
        }
    }
    Some(captures)
}

/// Recursive `->`-chain decomposition of a candidate member-call node:
/// `$repo->find($id)->hydrate($row)` is [receiver $repo, call find, call
/// hydrate]; a `member_access_expression` object contributes its property as
/// an argument-free link; a `scoped_call_expression` object contributes its
/// `scope::name` texts (the call keeping its argument list). Unresolvable
/// receivers (nullsafe links, exotic nodes) veto the whole decomposition —
/// sg keeps such faces empty, never an over-match.
fn member_chain_segments<'a>(
    node: &Node<'a>,
    source: &'a str,
    segs: &mut Vec<MemberChainSegment<'a>>,
) -> Option<()> {
    match node.kind() {
        "member_call_expression" => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs)?;
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(&text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            Some(())
        }
        "member_access_expression" => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs)?;
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args: None,
                args_content: None,
            });
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
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(&text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            Some(())
        }
        _ => {
            if is_ident_kind(node.kind())
                || KEYWORD_RECEIVER_KINDS.contains(&node.kind())
                || node.kind() == "variable_name"
            {
                segs.push(MemberChainSegment {
                    text: node_text(node, source)?.to_string(),
                    args: None,
                    args_content: None,
                });
                Some(())
            } else {
                None
            }
        }
    }
}

/// PASS 65 (F64-2): match dotted member-call chains with per-segment
/// argument lists. A candidate call answers when its callee path decomposes
/// into EXACTLY the pattern's segment count (sg reports the outermost
/// chain, plus inner-prefix subnodes on deeper chains — both are call nodes
/// this walk visits), every segment name unifies through `bind_capture`
/// (the same-name veto), and every call segment's argument list matches its
/// template: the LAST segment's list is the candidate's own arguments, the
/// inner segments' lists come from the nested receiver calls. Captures are
/// built directly (MATCH, segment names, per-call argument metavars) — the
/// shared `captures_for_node` path would re-parse the chain pattern as a
/// single call and mis-bind the truncated callee.
fn walk_call_chains(
    lang: Language,
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    out: &mut Vec<PatternMatch>,
) {
    if is_call_kind(node.kind()) && !is_in_comment_or_string(&node) {
        if let Some(captures) = chain_matches(lang, &node, source, segments) {
            let (line_start, line_end) = node_lines(&node, source);
            let excerpt = excerpt_for_node(&node, source, pattern);
            let mut captures = captures;
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            out.push(PatternMatch {
                line_start,
                line_end,
                byte_start: node.start_byte(),
                byte_end: node.end_byte(),
                excerpt,
                captures,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_call_chains(lang, child, source, pattern, segments, out);
    }
}

/// PASS 65f (F64-7 scope-back): match the two-segment optional-chain
/// template `HEAD?.$TAIL(...)` against candidate calls whose callee member
/// chain links into its tail segment through an anonymous `?.` token —
/// sg's token-exact rule (0.45.2 probes: the optional template answers
/// exactly the optional faces, zero plain faces; the plain template's
/// opposite contract lives in the F64-7 path veto). A wildcard head folds
/// the WHOLE receiver expression like sg's member-object metavariable:
/// `$O` = `user?.profile` on `user?.profile?.load()`, `maybe()` on
/// `maybe()?.load()`, `a.b` on `a.b?.c()`. Nested chains emit inner+outer
/// rows like sg (`conn?.open()?.send(1)` answers both calls).
fn walk_optional_calls(
    node: Node,
    source: &str,
    pattern: &str,
    head_capture: Option<&str>,
    head_literal: Option<&str>,
    tail_capture: Option<&str>,
    tail_literal: Option<&str>,
    arguments: Option<&ArgumentTemplate>,
    out: &mut Vec<PatternMatch>,
) {
    if let Some(captures) = optional_call_matches(
        &node,
        source,
        pattern,
        head_capture,
        head_literal,
        tail_capture,
        tail_literal,
        arguments,
    ) {
        let (line_start, line_end) = node_lines(&node, source);
        let excerpt = excerpt_for_node(&node, source, pattern);
        let mut captures = captures;
        if !captures.contains_key("MATCH") {
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
        }
        out.push(PatternMatch {
            line_start,
            line_end,
            byte_start: node.start_byte(),
            byte_end: node.end_byte(),
            excerpt,
            captures,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_optional_calls(
            child,
            source,
            pattern,
            head_capture,
            head_literal,
            tail_capture,
            tail_literal,
            arguments,
            out,
        );
    }
}

/// Segment unification + argument-template check for one optional-chain
/// candidate. `None` when the candidate is not a call, its chain does not
/// link into the tail through `?.`, or a head/tail spelling (literal or
/// same-name unification) fails.
fn optional_call_matches(
    node: &Node,
    source: &str,
    pattern: &str,
    head_capture: Option<&str>,
    head_literal: Option<&str>,
    tail_capture: Option<&str>,
    tail_literal: Option<&str>,
    arguments: Option<&ArgumentTemplate>,
) -> Option<BTreeMap<String, String>> {
    if !is_call_kind(node.kind()) || is_in_comment_or_string(node) {
        return None;
    }
    if !arguments_match(node, arguments, &["arguments"]) {
        return None;
    }
    // PASS 67a (F66a-6): php's member calls split the chain across fields —
    // the callee is a `name` node, not a member expression, and tree-sitter-php
    // 0.24 gives the nullsafe connector its own named kind
    // `nullsafe_member_call_expression` (plain `->` stays
    // `member_call_expression`). The kind IS the token-exact discriminator:
    // the plain spelling never answers here, and the wildcard head folds the
    // whole object expression like sg (O= `g?->h` on `g?->h?->i()`).
    if matches!(
        node.kind(),
        "member_call_expression" | "nullsafe_member_call_expression"
    ) {
        let object = node.child_by_field_name("object")?;
        let name = node.child_by_field_name("name")?;
        let nullsafe = node.kind() == "nullsafe_member_call_expression";
        if !nullsafe {
            return None;
        }
        let head_text = node_text(&object, source)?;
        let tail_text = node_text(&name, source)?;
        if tail_literal.is_some_and(|literal| literal != tail_text) {
            return None;
        }
        if head_literal.is_some_and(|literal| literal != head_text) {
            return None;
        }
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
        }
        capture_arguments(node, source, pattern, &mut captures)?;
        if let Some(name) = head_capture {
            bind_capture(&mut captures, name, head_text)?;
        }
        if let Some(name) = tail_capture {
            bind_capture(&mut captures, name, tail_text)?;
        }
        return Some(captures);
    }
    let field = call_field_node(node)?;
    let (segments, connectors) = optional_chain_decompose(&field)?;
    // A two-segment template needs a member chain: head + `?.` + tail.
    if segments.len() < 2 || !*connectors.last()? {
        return None;
    }
    let tail_node = segments.last()?;
    let tail_text = node_text(tail_node, source)?;
    if tail_literal.is_some_and(|literal| literal != tail_text) {
        return None;
    }
    // Wildcard-head folding: the receiver span covers segments 0..last,
    // ending at the LAST FOLDED segment's end — the trailing `?.` connector
    // bytes stay out of the capture (sg binds `user?.profile`, never
    // `user?.profile?`; the proven codemod corruption class).
    let head_node = &segments[0];
    let head_end = segments[segments.len() - 2].end_byte();
    let head_text = source.get(head_node.start_byte()..head_end)?;
    if head_literal.is_some_and(|literal| literal != head_text) {
        return None;
    }
    // MATCH + argument captures through the shared pattern-text contract;
    // `capture_call_path` no-ops for optional spellings (its simple_call
    // guard refuses `?`-bearing segments), so head/tail bind here.
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    capture_arguments(node, source, pattern, &mut captures)?;
    if let Some(name) = head_capture {
        bind_capture(&mut captures, name, head_text)?;
    }
    if let Some(name) = tail_capture {
        bind_capture(&mut captures, name, tail_text)?;
    }
    Some(captures)
}

/// Decompose a candidate callee into (segment nodes, connector flags):
/// `segments[0]` may be ANY expression (the folded head); every later
/// segment is a plain identifier leaf; `connectors[i]` is true when the
/// link INTO segment i+1 is `?.` (anonymous token or the named
/// `optional_chain` wrapper), false for `.`. A member link that is not a
/// clean base+connector+leaf triple, or a non-identifier tail (kotlin/
/// swift navigation suffixes), vetoes the chain — those grammars keep
/// their fail-closed contract.
fn optional_chain_decompose<'a>(node: &Node<'a>) -> Option<(Vec<Node<'a>>, Vec<bool>)> {
    if !is_member_expr_kind(node.kind()) {
        return Some((vec![*node], Vec::new()));
    }
    let link = member_link_parts(node)?;
    if !is_ident_kind(link.leaf.kind()) && !KEYWORD_RECEIVER_KINDS.contains(&link.leaf.kind()) {
        return None;
    }
    let (mut segments, mut connectors) = optional_chain_decompose(&link.base)?;
    segments.push(link.leaf);
    connectors.push(link.optional);
    Some((segments, connectors))
}

/// PASS 67a (F66a-3): chain-lane decomposition — like
/// [`optional_chain_decompose`] but the recursion CONTINUES through call
/// receivers (a method-call link contributes its callee leaf as the
/// segment, so `$M1` binds the method NAME and the connector INTO the call
/// is the link into its callee). The two-segment lane keeps the
/// stop-at-call folding in [`optional_chain_decompose`]: its registered
/// folded-head captures (O=`conn?.open()`, O=`maybe()`) depend on it.
/// Returns parallel (segment leaves, connector flags, EXPRESSION ends):
/// `ends[k]` is the end byte of the whole expression contributing segment
/// k — a call segment ends at its argument list's closing paren (sg binds
/// folded heads like `a.b()`), a plain member leaf at its member node.
fn optional_call_chain_decompose<'a>(
    node: &Node<'a>,
) -> Option<(Vec<Node<'a>>, Vec<bool>, Vec<usize>)> {
    if is_call_kind(node.kind()) {
        let field = call_field_node(node)?;
        let mut decomp = optional_call_chain_decompose(&field)?;
        if let Some(end) = decomp.2.last_mut() {
            *end = node.end_byte();
        }
        return Some(decomp);
    }
    if !is_member_expr_kind(node.kind()) {
        return Some((vec![*node], Vec::new(), vec![node.end_byte()]));
    }
    let link = member_link_parts(node)?;
    if !is_ident_kind(link.leaf.kind()) && !KEYWORD_RECEIVER_KINDS.contains(&link.leaf.kind()) {
        return None;
    }
    let (mut segments, mut connectors, mut ends) = optional_call_chain_decompose(&link.base)?;
    segments.push(link.leaf);
    connectors.push(link.optional);
    ends.push(node.end_byte());
    Some((segments, connectors, ends))
}

/// PASS 67a (F66a-3): match the N-segment optional chain template
/// `HEAD?.M1(...).M2(...)` against candidate calls whose callee decomposes
/// with EVERY ALIGNED connector equal to the template's per-position flag —
/// the head link `$O?.…`, the mid-chain spellings `$O.$M1()?.$M2($$$B)`,
/// and their combinations all reduce to flag equality (sg 0.45.2's
/// token-exact rule; the optional-mid sources
/// `user?.profile?.load()` / `conn?.open()?.send(1)` never answer the
/// all-plain template). The wildcard head folds the whole receiver prefix,
/// ending at the last absorbed segment's EXPRESSION end so no connector
/// bytes glue into the capture. Segment names unify through `bind_capture`
/// (the same-name veto) and every call segment's argument list is checked
/// along the receiver walk, exactly like [`chain_matches`].
fn optional_chain_matches(
    node: &Node,
    source: &str,
    _pattern: &str,
    segments: &[CallChainSegment],
    optional_flags: &[bool],
) -> Option<BTreeMap<String, String>> {
    if !is_call_kind(node.kind()) || is_in_comment_or_string(node) {
        return None;
    }
    let field = call_field_node(node)?;
    let (segs, connectors, ends) = optional_call_chain_decompose(&field)?;
    let head = &segments[0];
    let calls = &segments[1..];
    if segs.len() < calls.len() + 1 || connectors.len() + 1 != segs.len() {
        return None;
    }
    // Alignment: the pattern's call segments pair with the candidate's LAST
    // segments; the head folds everything before the first aligned segment.
    let absorb = segs.len() - (calls.len() + 1);
    // Token-exact per aligned connector: the link into candidate segment
    // absorb+j must EQUAL the template's flag for pattern segment j.
    for j in 1..segments.len() {
        if connectors[absorb + j - 1] != optional_flags[j] {
            return None;
        }
    }
    // The folded head span ends at the last absorbed segment's EXPRESSION
    // end — trailing connector bytes stay out of the capture (the F64-7
    // no-glued-? rule) while a folded call keeps its argument list (sg
    // binds `a.b()`, never `a.b`).
    let head_text = source.get(segs[0].start_byte()..ends[absorb])?;
    // PASS 69a (F-68c-1): a LITERAL head anchors the pattern chain at the
    // head segment's LEAF text — the segment NAME (`fetch`), not the folded
    // EXPRESSION span (`fetch()` carries the argument list). Comparing the
    // name against the span vetoed every literal-call-head candidate where
    // sg answers (`fetch()?.$M($$$A).$M2($$$B)` on `fetch()?.g().h()`,
    // probed 0.45.2). And a literal head admits no absorbed receiver
    // prefix: the pattern chain roots at the bare head, so a deeper
    // candidate chain is a different node (the plain lane's exact-length
    // rule; `x.fetch()?…` never answers).
    if head.literal.is_some() {
        if absorb > 0 {
            return None;
        }
        let head_leaf = node_text(&segs[absorb], source)?;
        if head.literal.as_deref() != Some(head_leaf) {
            return None;
        }
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    // PASS 71a (F70a-1b): NO pattern-level argument capture here. The legacy
    // `capture_arguments(node, …, pattern, …)` call extracted the FIRST call
    // segment's `$$$A` (its `(`..`)` slice starts at the first parenthesis of
    // the chain) and bound it against the OUTER node's argument list — the
    // LAST segment's — so on `fetch()?.g(1).h()` the multi key `$$$A` was
    // pre-bound to "" and the receiver walk's correct A=[1] binding collided
    // into a veto: the whole face silently missed. Every segment's argument
    // template and capture is enforced below by the segment-specific
    // machinery (head-args block, receiver walk, last-segment bind), which
    // binds each name exactly once against its own list.
    if let Some(name) = head.capture.as_deref() {
        bind_capture(&mut captures, name, head_text)?;
    }
    for (segment, seg_node) in calls.iter().zip(segs[absorb + 1..].iter()) {
        let text = node_text(seg_node, source)?;
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want != text => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, text)?,
            (None, None) => return None,
        }
    }
    // Arguments: the candidate node carries the LAST call segment's list;
    // every earlier call segment consumes one receiver link (must be a
    // call) along the receiver walk.
    let last = calls.last()?;
    let mut receiver = *node;
    for segment in calls[..calls.len() - 1].iter().rev() {
        receiver = chain_receiver(&receiver)?;
        if !is_call_kind(receiver.kind()) {
            return None;
        }
        if !arguments_match(&receiver, segment.args.as_ref(), &["arguments"]) {
            return None;
        }
        if let Some((name, multi)) = &segment.args_capture {
            if let Some(text) = capture_arguments_text(&receiver, source) {
                bind_capture_kind(&mut captures, name, strip_container(&text), *multi)?;
            }
        }
    }
    // PASS 69a (F-68c-1): a CALL head carries its own argument contract —
    // reachable as one more receiver link when nothing was absorbed (the
    // head segment IS the chain root then). Without this block the head's
    // arity/args-capture template was never consulted (`fetch(1)?…`
    // over-answered the arity-mismatched faces).
    if absorb == 0 && (head.args.is_some() || head.args_capture.is_some()) {
        receiver = chain_receiver(&receiver)?;
        if !is_call_kind(receiver.kind()) {
            return None;
        }
        if !arguments_match(&receiver, head.args.as_ref(), &["arguments"]) {
            return None;
        }
        if let Some((name, multi)) = &head.args_capture {
            if let Some(text) = capture_arguments_text(&receiver, source) {
                bind_capture_kind(&mut captures, name, strip_container(&text), *multi)?;
            }
        }
    }
    if !arguments_match(node, last.args.as_ref(), &["arguments"]) {
        return None;
    }
    if let Some((name, multi)) = &last.args_capture {
        if let Some(text) = capture_arguments_text(node, source) {
            bind_capture_kind(&mut captures, name, strip_container(&text), *multi)?;
        }
    }
    Some(captures)
}

/// PASS 67a (F66a-3): walker for the N-segment optional chain — one row per
/// matching call node (nested chains emit inner+outer rows like sg), the
/// same emission shape as [`walk_optional_calls`].
fn walk_optional_call_chains(
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    optional_flags: &[bool],
    out: &mut Vec<PatternMatch>,
) {
    if let Some(captures) = optional_chain_matches(&node, source, pattern, segments, optional_flags)
    {
        let (line_start, line_end) = node_lines(&node, source);
        let excerpt = excerpt_for_node(&node, source, pattern);
        let mut captures = captures;
        if !captures.contains_key("MATCH") {
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
        }
        out.push(PatternMatch {
            line_start,
            line_end,
            byte_start: node.start_byte(),
            byte_end: node.end_byte(),
            excerpt,
            captures,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_optional_call_chains(child, source, pattern, segments, optional_flags, out);
    }
}

/// Segment unification + argument-template check for one candidate call.
/// PASS 67a (F66a-1): a leading METAVARIABLE head absorbs the whole
/// property-access receiver prefix — the pattern's tail aligns with the
/// candidate path's LAST segments and the head binds the absorbed
/// EXPRESSION span (sg 0.45.2: `$O.$M1($$$A).$M2($$$B)` answers
/// `deep.x.y().z()` with O=`deep.x`, `s.a().b().c()` answers the absorbed
/// outer rows with O=`s.a()`, and `a?.b().c().d()` folds the whole
/// optional-call head with O=`a?.b()`). A LITERAL head pins the chain to
/// the exact segment count (sg answers `alpha.$M1($$$A).$M2($$$B)` on
/// `alpha.beta().gamma().delta()` only through the exact-length inner
/// node). The `?.` veto is position-scoped in the registered grammars: an
/// optional connector INTO any ALIGNED call segment refuses, while
/// head-internal optionals fold (sg probes 0.45.2); every other grammar
/// keeps the wholesale F64-7 refusal. The receiver walk pairs every
/// pattern CALL segment except the LAST with one receiver link (which must
/// itself be a call), so per-segment argument templates fire on
/// receiver-headed chains — and a receiver head is absorbed into the head
/// text above, consuming no link.
fn chain_matches(
    lang: Language,
    node: &Node,
    source: &str,
    segments: &[CallChainSegment],
) -> Option<BTreeMap<String, String>> {
    let path = chain_callee_segments(node, source)?;
    if path.len() < segments.len() {
        return None;
    }
    let absorb = path.len() - segments.len();
    if absorb > 0 && segments[0].literal.is_some() {
        return None;
    }
    // Token-exact veto at the aligned positions; head-INTERNAL connectors
    // fold only where sg's folding is probed (ts/js — the registered
    // optional-connector grammars); everywhere else any `?.` in the
    // candidate keeps the wholesale refusal.
    if path[absorb + 1..].iter().any(|seg| seg.optional)
        || (absorb > 0
            && path[1..=absorb].iter().any(|seg| seg.optional)
            && !matches!(lang, Language::TypeScript | Language::JavaScript))
    {
        return None;
    }
    let mut captures = BTreeMap::new();
    // Head binding: a metavariable head binds the WHOLE absorbed prefix —
    // the expression span when the walk recorded spans (call segments keep
    // their argument list: sg binds `s.a()`, connector bytes excluded), or
    // the joined ruby path text; `bind_capture`'s same-name veto then
    // rejects any repeated name against the tail bindings.
    let head_text = if path[..=absorb].iter().all(|seg| seg.end.is_some()) {
        source.get(path[0].start..path[absorb].end.unwrap())?.to_string()
    } else {
        path[..=absorb]
            .iter()
            .map(|seg| seg.text.as_str())
            .collect::<Vec<_>>()
            .join(".")
    };
    match (&segments[0].literal, &segments[0].capture) {
        (Some(want), _) if want != &path[0].text => return None,
        (Some(_), _) => {}
        (None, Some(name)) => bind_capture(&mut captures, name, &head_text)?,
        (None, None) => return None,
    }
    for (segment, actual) in segments[1..]
        .iter()
        .zip(path[absorb + 1..].iter().map(|seg| seg.text.as_str()))
    {
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want.as_str() != actual => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, actual)?,
            (None, None) => return None,
        }
    }
    // Argument lists: the candidate's own arguments are the LAST call
    // segment's; walking the receiver side yields the earlier call segments'
    // lists — each pattern call segment consumes exactly one receiver link
    // (innermost pattern segment first), and every link must be a call.
    let last = segments.last()?;
    let mut receiver = *node;
    for segment in segments[..segments.len() - 1].iter().rev() {
        if segment.args.is_none() {
            // Receiver head: no argument contract, no link consumed.
            continue;
        }
        receiver = chain_receiver(&receiver)?;
        if !is_call_kind(receiver.kind()) {
            return None;
        }
        if !arguments_match(&receiver, segment.args.as_ref(), &["arguments"]) {
            return None;
        }
        if let Some((name, multi)) = &segment.args_capture {
            if let Some(text) = capture_arguments_text(&receiver, source) {
                // The argument capture binds the LIST CONTENT (container
                // stripped), the same convention as the single-call lane's
                // `$$$` capture — sg's multi list for `()` is empty.
                bind_capture_kind(&mut captures, name, strip_container(&text), *multi)?;
            }
        }
    }
    if !arguments_match(node, last.args.as_ref(), &["arguments"]) {
        return None;
    }
    if let Some((name, multi)) = &last.args_capture {
        if let Some(text) = capture_arguments_text(node, source) {
            bind_capture_kind(&mut captures, name, strip_container(&text), *multi)?;
        }
    }
    Some(captures)
}

/// One decomposed chain path segment: its dotted text, its span start, its
/// EXPRESSION end (absent on the ruby text path), and whether the connector
/// INTO it is the spelled `?.` (PASS 67a: recorded per segment so the
/// matcher can scope the F64-7 token veto to the aligned positions).
struct ChainPathSegment {
    text: String,
    start: usize,
    end: Option<usize>,
    optional: bool,
}

/// PASS 65a (F64-2): sg-style decomposition of a candidate call's callee
/// into DOTTED COMPONENTS — `alpha.first().second()` is [alpha, first,
/// second], each `name(args)` component contributing its name (probed
/// 0.45.2: the 3-segment template binds O=alpha, M1=first, M2=second). The
/// registered call-lane path (`call_target_path`) flattens a nested call to
/// its LAST identifier, which drops the receiver segment and made every
/// three-segment face silent.
fn chain_callee_segments(node: &Node, source: &str) -> Option<Vec<ChainPathSegment>> {
    if node.kind() == "call" {
        if let Some(segs) = ruby_receiver_callee(node, source) {
            return Some(
                segs.into_iter()
                    .map(|text| ChainPathSegment {
                        text,
                        start: node.start_byte(),
                        end: None,
                        optional: false,
                    })
                    .collect(),
            );
        }
    }
    let mut segs = Vec::new();
    chain_collect_segments(node, source, &mut segs)?;
    Some(segs)
}

/// Recursive dotted-component collection over candidate nodes.
fn chain_collect_segments(
    node: &Node,
    source: &str,
    segs: &mut Vec<ChainPathSegment>,
) -> Option<()> {
    // java(/kotlin-family) member calls carry object + name fields directly.
    if node.kind() == "method_invocation" {
        let object = node.child_by_field_name("object")?;
        let name = node.child_by_field_name("name")?;
        chain_collect_segments(&object, source, segs)?;
        segs.push(ChainPathSegment {
            text: node_text(&name, source)?.to_string(),
            start: name.start_byte(),
            end: Some(node.end_byte()),
            optional: false,
        });
        return Some(());
    }
    if is_call_kind(node.kind()) {
        let field = call_field_node(node)?;
        chain_collect_segments(&field, source, segs)?;
        // The call segment's EXPRESSION ends at its argument list (sg binds
        // folded heads like `s.a()`), never at the bare callee leaf.
        if let Some(last) = segs.last_mut() {
            last.end = Some(node.end_byte());
        }
        return Some(());
    }
    if is_member_expr_kind(node.kind()) {
        // The connector INTO this segment is recorded, not vetoed: the
        // matcher applies the F64-7 token veto at the ALIGNED positions and
        // folds head-internal optionals in the registered grammars
        // (PASS 67a).
        let mut children = node.walk();
        let optional = node
            .children(&mut children)
            .any(|child| child.kind().contains('?') || child.kind().contains("optional"));
        let object = member_receiver(node)?;
        let mut children = node.walk();
        let property = node
            .named_children(&mut children)
            .find(|child| child.id() != object.id())?;
        chain_collect_segments(&object, source, segs)?;
        segs.push(ChainPathSegment {
            text: node_text(&property, source)?.to_string(),
            start: property.start_byte(),
            end: Some(node.end_byte()),
            optional,
        });
        return Some(());
    }
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        segs.push(ChainPathSegment {
            text: node_text(node, source)?.to_string(),
            start: node.start_byte(),
            end: Some(node.end_byte()),
            optional: false,
        });
        return Some(());
    }
    None
}

/// The receiver side of a chain candidate: the `object` of a
/// `method_invocation`, else the object of the callee member expression.
fn chain_receiver<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "method_invocation" {
        return node.child_by_field_name("object");
    }
    let field = call_field_node(node)?;
    member_receiver(&field)
}

/// The candidate call's argument-list text between its parentheses.
fn capture_arguments_text(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("arguments")
        .and_then(|list| node_text(&list, source))
        .map(str::to_string)
}

/// PASS 65 (F64-2): the receiver side of a member-expression callee — the
/// object field where the grammar names it, else the first named child
/// (rust `field_expression`, csharp `member_access`). `None` for plain
/// identifiers ends the receiver walk.
fn member_receiver<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if let Some(object) = node.child_by_field_name("object") {
        return Some(object);
    }
    let mut cursor = node.walk();
    let named: Vec<Node> = node.named_children(&mut cursor).collect();
    if named.len() >= 2 {
        return Some(named[0]);
    }
    None
}

/// If-node kinds matched by `NativeKind::If` across the 13 indexed languages.
/// Modifier (`x if y` in Ruby) and ternary forms are deliberately excluded.
const IF_KINDS: &[&str] = &["if_statement", "if_expression", "if"];

/// Body/consequence container kinds across grammars.
const BLOCK_KINDS: &[&str] = &[
    "block",
    "statement_block",
    "compound_statement",
    "function_body",
    "body_statement",
    "statements",
    "then",
];

/// Wrapper kinds that never hold statements directly; descend into their
/// single block-like child before counting (Swift `function_body { statements }`).
const STMT_WRAPPER_KINDS: &[&str] = &["function_body", "then", "statements"];

fn is_trivia_kind(kind: &str) -> bool {
    kind.contains("comment")
}

fn walk_ifs(
    node: Node,
    source: &str,
    pattern: &str,
    body: Option<&BodyTemplate>,
    out: &mut Vec<PatternMatch>,
) {
    if IF_KINDS.contains(&node.kind())
        && !is_in_comment_or_string(&node)
        && if_body_matches(&node, body)
    {
        push_match(&node, source, pattern, Some("if"), out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ifs(child, source, pattern, body, out);
    }
}

// ---------------------------------------------------------------------------
// PASS 51 (r9): general structural lane for the duplicate-metavariable family
// beyond the three declaration shapes (pass-48 fresh-eyes-B TrueDivergences:
// operator chains, nested-call arguments, typed params, return/assignment
// bodies). The lane parses the METAVARIABLE-SUBSTITUTED pattern with the same
// tree-sitter grammar as the candidate and compares the two trees pairwise,
// binding metavariable leaves through `bind_capture` (sg unification
// semantics: a repeated name is one variable).
//
// Scope guardrails (the registered fail-closed contract rows must keep their
// loud rejection):
// - language-free text guards refuse `$$$` templates, multi-line tails,
//   comment syntax, and bare-keyword heads outside `DECL_PATTERN_PREFIXES`
//   (`let $A = $B`, `RETURN $A`, `fun $A() { $$$B }`, `int $A($B) { $$$C }`);
// - the template root must be an expression / call / known-declaration kind;
//   `if` templates stay in the dedicated If lane (multi-statement bodies keep
//   failing closed);
// - a pattern no grammar parses cleanly (ERROR/missing nodes) is unsupported;
// - a metavariable substituted INSIDE a string/comment/regex literal is
//   unsupported (sg treats it as literal text there).
// ---------------------------------------------------------------------------

/// Placeholder prefix substituted for `$NAME` in general-lane pattern docs.
/// Underscore-led so it parses as an identifier in all 13 indexed grammars.
const GENERAL_MV_PREFIX: &str = "__asgrep_mv_";

/// Language-free eligibility guards for the general structural lane.
/// PASS 63 (F62-1): comment syntax moved OUT of this language-free guard —
/// `#` is comment syntax ONLY in python/ruby/php, so a language-free `#`
/// refusal failed every rust-attribute / C-preprocessor / swift-`#selector` /
/// js-private-field pattern closed where sg answers. The per-language
/// builders ([`cached_general_template`], [`general_lane_supported_uncached`])
/// and [`native_pattern_answerable`] apply [`lane_comment_refused`] instead.
/// PASS 73 (F72a-2): the nested-call rest-argument family. A call-shaped
/// pattern where every argument list holds only single metavariables or
/// nested calls of the same restricted shape, plus lists that are exactly
/// one `$$$` rest (`g(fetch($$$A))`, `fetch(g(fetch($$$A)))`,
/// `g(fetch($$$A), send($$$B))`, `g(o.fetch($$$A))`, `$A::bar($B)`-shaped
/// php heads included via the `::` separator). Literal atoms are NOT in the
/// family — they keep the registered H-CONF-013 mixed-concrete loud class —
/// and a `$$$` rest may never share its list with a sibling (the registered
/// H-CONF-002 trailing-concrete discipline). Paths spell `.` and `::`
/// separators with identifier/`$meta` segments.
fn nested_call_rest_template(pattern: &str) -> bool {
    let p = pattern.trim();
    if !p.contains('$') || !p.contains('(') {
        return false;
    }
    let b = p.as_bytes();
    let mut i = 0usize;
    if !parse_family_call(b, &mut i) {
        return false;
    }
    // Trailing whitespace only after the outermost call.
    b[i..].iter().all(|c| c.is_ascii_whitespace())
}

fn parse_family_call(b: &[u8], i: &mut usize) -> bool {
    // Dotted / scoped path: segment (`.` | `::` segment)*.
    let start = *i;
    loop {
        if *i >= b.len() {
            return false;
        }
        if b[*i] == b'_' || b[*i].is_ascii_alphanumeric() {
            while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                *i += 1;
            }
        } else if b[*i] == b'$' {
            *i += 1;
            while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                *i += 1;
            }
        } else {
            return false;
        }
        if *i < b.len() && b[*i] == b'.' {
            *i += 1;
            continue;
        }
        if *i + 1 < b.len() && b[*i] == b':' && b[*i + 1] == b':' {
            *i += 2;
            continue;
        }
        break;
    }
    if *i == start || *i >= b.len() || b[*i] != b'(' {
        return false;
    }
    *i += 1;
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
    // Empty list.
    if b.get(*i) == Some(&b')') {
        *i += 1;
        return true;
    }
    // Sole rest: `$$$` + optional name, then nothing but whitespace to `)`.
    if b.get(*i) == Some(&b'$') && b.get(*i + 1) == Some(&b'$') && b.get(*i + 2) == Some(&b'$') {
        *i += 3;
        while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
            *i += 1;
        }
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if b.get(*i) == Some(&b')') {
            *i += 1;
            return true;
        }
        // Rest with a sibling: out of the family (registered loud class).
        return false;
    }
    // Argument list: (arg `,`)* arg — each arg is a `$meta` or a nested call.
    loop {
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        match b.get(*i) {
            Some(b'$') => {
                if b.get(*i + 1) == Some(&b'$') && b.get(*i + 2) == Some(&b'$') {
                    return false; // rest with a sibling: out of the family
                }
                // PASS 75a (F74a-3): a canonical 2-dollar name is a family
                // argument exactly like `$NAME` (sole-or-sibling, single
                // capture); a malformed run (`$$`, `$$3`, `$$x`) stays out.
                if b.get(*i + 1) == Some(&b'$') {
                    *i += 2;
                    let start = *i;
                    while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                        *i += 1;
                    }
                    let name = std::str::from_utf8(&b[start..*i]).unwrap_or("");
                    if !is_metavar_name(name) {
                        return false;
                    }
                } else {
                    *i += 1;
                    while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                        *i += 1;
                    }
                }
            }
            Some(c) if *c == b'(' || *c == b'_' || c.is_ascii_alphanumeric() => {
                if !parse_family_call(b, i) {
                    return false;
                }
            }
            _ => return false, // literal atom: out of the family
        }
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        match b.get(*i) {
            Some(b',') => *i += 1,
            Some(b')') => {
                *i += 1;
                return true;
            }
            _ => return false,
        }
    }
}

fn general_lane_text_eligible(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.contains('\n') || p.contains(GENERAL_MV_PREFIX) {
        return false;
    }
    // PASS 73 (F72a-2): the pass-51 blanket `$$$` refusal narrows to its
    // registered core. The probed sg 0.45.2 nested-call rest family —
    // call-shaped patterns whose argument lists hold only metavariables and
    // nested calls, with `$$$` as the SOLE argument of its list — templates
    // like any other structural shape (`g(fetch($$$A))` answers all arities).
    // Everything else carrying `$$$` (flat mixed lists, rest + sibling,
    // statement templates) keeps the loud fail-closed contract.
    if p.contains("$$$") && !nested_call_rest_template(p) {
        return false;
    }
    // PASS 60 (F58-1): `//`-`/*` comment syntax is refused only OUTSIDE
    // string-literal quotes — the `//` in `parse($U, "https://default")` is
    // URL content, not a comment, and the old byte scan failed such patterns
    // closed where sg answered. The per-language template builder adds
    // the precise judgment (clean parse, allowed root, span coverage).
    if contains_comment_syntax_outside_strings(p) {
        return false;
    }
    // A bare alphabetic head is declaration-keyword territory: only the native
    // prefixes may lead a general-lane template. This keeps `let $A = $B` /
    // `RETURN $A` / `int $A($B)` / `fun $A()` fail-closed even where some
    // other grammar would happily parse the text (e.g. ruby command calls).
    // PASS 60 (F58-2): lowercase statement heads sg parses as statements are
    // native templates (`return $A` — sg answers on py/ts/java, accepted-empty
    // on rust); PASS 63 (F62-2) encodes the sg-probed family (raise/yield/
    // throw/await answer; break/continue answer `$`-less; ruby next/last stay
    // predicate-bound — subject already empty-agrees there).
    if let Some(first) = p.split_whitespace().next() {
        let bare_keyword = first.chars().all(|c| c.is_ascii_alphabetic());
        if bare_keyword
            && !DECL_PATTERN_PREFIXES.iter().any(|(prefix, _)| prefix.trim() == first)
            && !STATEMENT_HEAD_KEYWORDS.contains(&first)
        {
            // PASS 65a (F64-4): an ASSIGNMENT head (`name = "user-#{$N}"`) is
            // an ordinary identifier, not declaration-keyword territory — sg
            // answers the ruby string-interpolation face (probed 0.45.2).
            // The `let` shapes keep their registered fail-closed contract at
            // the root-kind gate (`is_general_root_kind` refuses let kinds),
            // and uppercase/other bare heads (no `=`) stay refused here.
            let after = p[first.len()..].trim_start();
            if !after.starts_with('=') {
                return false;
            }
        }
    }
    true
}

/// Lowercase statement keywords the general lane templates at statement root
/// (F58-2; family probed against sg 0.45.2 in pass 63, F62-2; PASS 65 F64-1
/// extends the probed family with go's `defer`/`go`).
const STATEMENT_HEAD_KEYWORDS: &[&str] = &[
    "return", "raise", "yield", "throw", "await", "break", "continue", "defer", "go",
];

/// PASS 63 (F62-1): true when `#` appears OUTSIDE string-literal quotes.
/// Quote state scanned with escape handling, exactly like the `//`-`/*`
/// scan below; unterminated quotes make the tail in-string (conservative).
fn contains_hash_outside_strings(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'#' => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// PASS 63 (F62-1): the per-language comment-syntax refusal. `#` is comment
/// syntax only in the hash-comment languages (python/ruby/php); in rust
/// (`#[derive]`, `#![allow]`, `r#"…"#`), C/C++ (`#include`, `#define`),
/// swift (`#selector`), and JS/TS (`this.#x`) it is real syntax and must
/// stay templatable. The registered python/ruby comment-glued faces keep
/// their fail-closed contract through this same guard.
fn lane_comment_refused(lang: Language, pattern: &str) -> bool {
    if matches!(lang, Language::Python | Language::Ruby | Language::Php)
        && contains_hash_outside_strings(pattern.trim())
    {
        return true;
    }
    contains_comment_syntax_outside_strings(pattern.trim())
}

/// PASS 63 (F62-1 boundary): true when some quote-external `#` starts a
/// tail that carries no code punctuation at all — a trailing comment glue
/// (`foo($A) # note`), never leading syntax (`#[derive($A)]`, `#include $X`
/// — their remainders contain `[`/`$`), never a mid-chain private field
/// (`this.#x = $V` — `=` follows), never a raw-string delimiter
/// (`tag(r#"$A"#)` — a quote follows).
fn contains_comment_glued_hash(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'#' => {
                let glued = bytes[i + 1..].iter().all(|&rest| {
                    !matches!(
                        rest,
                        b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'=' | b'.' | b'"' | b'\''
                            | b'$' | b'/' | b'*' | b'<' | b'>' | b'!' | b'?' | b';' | b':'
                            | b',' | b'&' | b'|' | b'^' | b'~' | b'+' | b'-'
                    )
                });
                if glued {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// True when `//` or `/*` appears OUTSIDE string-literal quotes
/// (quote state scanned with escape handling). Unterminated quotes make the
/// tail in-string — conservative in the loud direction only for comment
/// syntax, and the parse-level template checks govern the rest.
/// PASS 63 (F62-1): the `#` arm moved to the language-aware
/// [`lane_comment_refused`] — a language-free `#` refusal failed
/// rust/c/swift/js `#`-syntax patterns closed where sg answers.
fn contains_comment_syntax_outside_strings(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'/' if bytes.get(i + 1) == Some(&b'/') || bytes.get(i + 1) == Some(&b'*') => {
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Replace `$NAME` metavariables with placeholder identifiers, returning the
/// substituted document and the placeholder -> capture-name map.
///
/// H-CONF-022 v2 (pass 54): every `$`-run must be a canonical metavariable
/// (`$NAME` / `$$$NAME`, ASCII `[A-Z_][A-Za-z0-9_]*`); any other `$` shape
/// (`$3.14`, `$a`, bare `$`) refuses substitution entirely so the general
/// lane can never template — let alone wildcard-match — a pattern the v2
/// grammar classifies as universal / NeverMatches / loud-reject.
/// PASS 75a (F74a-3, corrected PASS 77b per F76c-3): a canonical `$$NAME`
/// run is NOT in the refusal set — it substitutes exactly like `$NAME` into
/// the single namespace (`(1..=3)`-dollar arm below), matching sg 0.45.2's
/// capture key; only `$$$NAME` lands in the multi namespace.
fn substitute_general_metavariables(
    pattern: &str,
) -> Option<(String, BTreeMap<String, String>, BTreeSet<String>)> {
    let mut out = String::with_capacity(pattern.len() + 32);
    let mut placeholders = BTreeMap::new();
    // PASS 73 (F72a-2): names spelled `$$$NAME` — the sole-rest arm of
    // `general_eq` binds these in the MULTI namespace over the whole
    // argument list.
    let mut multi_names = BTreeSet::new();
    let bytes = pattern.as_bytes();
    let mut chars = pattern.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        let mut dollars = 0;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        let mut end = i + dollars;
        while end < bytes.len() && (bytes[end] == b'_' || bytes[end].is_ascii_alphanumeric()) {
            end += 1;
        }
        let name = pattern.get(i + dollars..end);
        let canonical = name
            .filter(|name| !name.is_empty() && is_metavar_name(name))
            // PASS 75a (F74a-3): a canonical `$$NAME` substitutes exactly like
            // `$NAME` (same single-namespace placeholder); only `$$$NAME`
            // lands in the multi namespace.
            .filter(|_| (1..=3).contains(&dollars));
        if let Some(name) = canonical {
            if dollars == 3 {
                multi_names.insert(name.to_string());
            }
            let placeholder = format!("{GENERAL_MV_PREFIX}{name}");
            placeholders.insert(placeholder.clone(), name.to_string());
            out.push_str(&placeholder);
            // Skip the consumed ident chars in the char iterator.
            while let Some(&(j, _)) = chars.peek() {
                if j < end {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            return None;
        }
    }
    Some((out, placeholders, multi_names))
}

/// A built general-lane pattern: the substituted document, its tree, and the
/// metavariable map. Cheap to clone (`Tree` clone is reference-counted).
#[derive(Clone)]
struct GeneralTemplate {
    doc: String,
    tree: tree_sitter::Tree,
    placeholders: BTreeMap<String, String>,
    /// PASS 73 (F72a-2): metavariable NAMES spelled `$$$NAME` in the source
    /// pattern — the sole-rest arm of [`general_eq`] binds these over the
    /// whole aligned argument list (multi namespace).
    multi_names: BTreeSet<String>,
    /// Byte span of the substituted pattern inside `doc` (context wraps only).
    span: Option<(usize, usize)>,
    /// Resolved template root kind (candidate prefilter).
    root_kind: String,
}

/// Per-language context wrapping for patterns that are expressions (the
/// top-level grammar only accepts items). The substituted pattern text sits
/// exactly at byte span `[prefix.len(), prefix.len() + len)` in the doc.
fn general_expression_context(lang: Language) -> Option<(&'static str, &'static str)> {
    Some(match lang {
        // PASS 60 (H-CONF-028 / H-CONF-030-iii): java and rust statements
        // demand the terminating `;` — expression/let/return templates only
        // parse (and only align their statement children with candidate
        // sources) as terminated statements.
        Language::Rust => ("fn __asgrep_ctx() { ", "; }"),
        Language::Go => ("func __asgrep_ctx() { ", " }"),
        Language::Java => ("class __AsgrepCtx { void __m() { ", "; } }"),
        // PASS 65 (F64-1): C# statement heads demand the terminator exactly
        // like java — `throw $A` / `await $A` only parse (and only align
        // their statement children with candidate sources) as terminated
        // statements inside the method body.
        Language::CSharp => ("class __AsgrepCtx { void M() { ", "; } }"),
        // PASS 63 (F62-5): C/C++ statements demand the terminating `;` —
        // `return $A` only parses (and only aligns its statement children
        // with candidate sources) as a terminated statement; the missing
        // `;` previously left the C template build failing on a MISSING
        // node (sg answered py+c+js where the subject answered py-only).
        Language::C | Language::Cpp => ("void __asgrep_ctx(void) { ", "; }"),
        Language::Swift => ("func __asgrep_ctx() { ", " }"),
        Language::Kotlin => ("fun __asgrep_ctx() { ", " }"),
        Language::Ruby => ("def __asgrep_ctx\n  ", "\nend"),
        // Python/TS/JS grammars accept bare statements/expressions at the root.
        Language::Python | Language::TypeScript | Language::JavaScript | Language::Php => {
            return None
        }
    })
}

/// Top-level statement wrappers a template root may hide behind (single named
/// child only). PASS 63 (F62-1): `translation_unit` joins for the C/C++
/// preprocessor faces — `#include $X` parses bare at a TU root whose single
/// child is the preproc node.
const GENERAL_WRAPPER_KINDS: &[&str] = &[
    "module",
    "program",
    "source_file",
    "expression_statement",
    "translation_unit",
];

/// Allowed template root kinds for the general lane: expressions, calls, and
/// known declaration kinds. `if` templates stay in the dedicated If lane, and
/// let/bindings keep their registered fail-closed contract.
fn is_general_root_kind(kind: &str) -> bool {
    if matches!(kind, "if_statement" | "if_expression" | "if") || kind.contains("let") {
        return false;
    }
    is_call_kind(kind)
        || DECL_KIND_PREFIXES.iter().any(|(k, _)| *k == kind)
        || kind.ends_with("_expression")
        || kind.ends_with("_operator")
        || kind.ends_with("assignment")
        || kind == "assignment_statement"
        // PASS 60 (F58-2): statement-root heads (`return $A`) template to
        // return_statement roots; rust templates to return_expression via the
        // `_expression` arm above.
        || kind == "return_statement"
        // PASS 63 (F62-2): the probed statement-head family templates to
        // these roots (python raise_statement / yield, ts+java+js
        // throw_statement, js break/continue_statement).
        // PASS 65 (F64-1): the extended family — python `await` (kind
        // `await`), go `defer`/`go` statements.
        || matches!(
            kind,
            "raise_statement" | "yield" | "yield_statement" | "throw_statement"
                | "break_statement" | "continue_statement" | "await" | "defer_statement"
                | "go_statement"
        )
        // PASS 63 (F62-2): ruby `raise x` / `yield x` are receiver-less
        // command calls (kind `command`); the head guard already restricts
        // which patterns may lead, this only admits their template root.
        || kind == "command"
        // PASS 63 (F62-1): `#`-syntax faces template to these roots —
        // rust attributes (outer + inner) and C/C++ preprocessor lines.
        || matches!(
            kind,
            "attribute_item" | "inner_attribute_item" | "preproc_include" | "preproc_def"
                | "preproc_function_def" | "preproc_call"
        )
        || matches!(kind, "attribute" | "field_expression")
        // PASS 65a (F64-4): a string template root answers only when every
        // metavariable sits inside a `#{…}` INTERPOLATION subtree (sg binds
        // it; probed 0.45.2) — placeholders in PLAIN string content still
        // refuse the build in `pattern_has_placeholder_in_literal`, so the
        // registered string-metavar fail-closed rows are untouched.
        || kind == "string"
}

fn general_template_root<'a>(template: &'a GeneralTemplate) -> Option<Node<'a>> {
    let root = template.tree.root_node();
    let mut node = match template.span {
        Some((start, end)) => {
            let mut node = root.descendant_for_byte_range(start, end)?;
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
        None => root,
    };
    loop {
        if !GENERAL_WRAPPER_KINDS.contains(&node.kind()) {
            break;
        }
        let mut cursor = node.walk();
        let named: Vec<Node> = node.named_children(&mut cursor).collect();
        match named.as_slice() {
            [only] => node = *only,
            _ => break,
        }
    }
    Some(node)
}

fn pattern_has_placeholder_in_literal(node: &Node, doc: &str) -> bool {
    let kind = node.kind();
    // PASS 65 (F64-4): a placeholder inside a string INTERPOLATION is a
    // metavariable hole, not literal text — sg binds it (probed:
    // `"user-#{$N}"` answers with N = the interpolation content). The
    // interpolation subtree is exempt from the scan; plain string content
    // keeps the pass-54 literal-metavariable refusal.
    if kind == "interpolation" {
        return false;
    }
    // PASS 63 (F62-1): a raw string literal (`r#"$A"#`) is TEXT to sg — its
    // expando-replaced pattern still carries the placeholder bytes inside
    // the raw string, and sg answers accepted-empty on such faces
    // (`tag(r#"$A"#)` probes exit 1 ran-empty), not a refusal. Exempting the
    // literal AND its string_content child lets the template build; the leaf
    // text comparison then answers match-none exactly like sg (the
    // placeholder bytes never appear in a real source raw string). Ordinary
    // string literals keep the pass-54 literal-metavariable refusal.
    let exempt = kind == "raw_string_literal"
        || (kind == "string_content"
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == "raw_string_literal"));
    if !exempt
        && (kind.contains("string") || kind.contains("comment") || kind.contains("regex"))
        && node_text(node, doc).is_some_and(|text| text.contains(GENERAL_MV_PREFIX))
    {
        // PASS 65 (F64-4): the placeholder may reach a string node ONLY
        // through interpolation children (exempted below) — when every
        // placeholder occurrence inside this node sits in an interpolation
        // subtree, the node itself is container syntax, not literal text,
        // and must not refuse. Plain string content keeps the pass-54
        // literal-metavariable refusal.
        if !placeholders_only_inside_interpolations(node, doc) {
            return true;
        }
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| pattern_has_placeholder_in_literal(&child, doc));
    found
}

/// True when every direct child carrying the placeholder prefix is a string
/// interpolation node (the only string context sg treats as a metavariable
/// hole).
fn placeholders_only_inside_interpolations(node: &Node, doc: &str) -> bool {
    let mut cursor = node.walk();
    let all_clear = node.children(&mut cursor).all(|child| {
        let carries = node_text(&child, doc).is_some_and(|text| text.contains(GENERAL_MV_PREFIX));
        !carries || child.kind() == "interpolation"
    });
    all_clear
}

fn try_build_general_template(
    lang: Language,
    doc: String,
    span: Option<(usize, usize)>,
    placeholders: &BTreeMap<String, String>,
    multi_names: &BTreeSet<String>,
) -> Option<GeneralTemplate> {
    let tree = parse_source(lang, &doc).ok()?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    if pattern_has_placeholder_in_literal(&root, &doc) {
        return None;
    }
    let mut probe = GeneralTemplate {
        doc,
        tree,
        placeholders: placeholders.clone(),
        multi_names: multi_names.clone(),
        span,
        root_kind: String::new(),
    };
    let node = general_template_root(&probe)?;
    if !is_general_root_kind(node.kind()) {
        return None;
    }
    probe.root_kind.push_str(node.kind());
    Some(probe)
}

fn build_general_template(
    lang: Language,
    raw: &str,
    substituted: &str,
    placeholders: &BTreeMap<String, String>,
    multi_names: &BTreeSet<String>,
) -> Option<GeneralTemplate> {
    if let Some(template) =
        try_build_general_template(lang, substituted.to_string(), None, placeholders, multi_names)
    {
        return Some(template);
    }
    // PASS 63 (F62-1): line-oriented grammars terminate preprocessor lines
    // with the physical newline — `#include __asgrep_mv_X` without one parse
    // with a MISSING terminator token and refuse; the newline-terminated
    // twin parses clean (the span still covers only the pattern text).
    let newline_doc = format!("{substituted}\n");
    if let Some(template) = try_build_general_template(
        lang,
        newline_doc,
        Some((0, substituted.len())),
        placeholders,
        multi_names,
    ) {
        return Some(template);
    }
    // PASS 75a (F74a-4 face 3 + 74c-F4): sg pre-processes php patterns
    // behind a `<?php ` tag — a bare substituted pattern parses to a `text`
    // node. The two gated families (php_wrapped_general_lane) retry behind
    // the tag; every build still demands a clean parse and a general root
    // kind, so anything beyond them keeps the fail-closed refusal.
    if lang == Language::Php && php_wrapped_general_lane(raw) {
        // tree-sitter-php demands the terminating `;` on a statement the tag
        // leads (sg tolerates the ERROR-wrapped twin; the wrapper build does
        // not), so the retry carries the terminator inside the doc.
        for doc in [
            format!("<?php {substituted};"),
            format!("<?php {substituted}\n"),
        ] {
            if let Some(template) = try_build_general_template(
                lang,
                doc,
                Some((6, 6 + substituted.len())),
                placeholders,
                multi_names,
            ) {
                return Some(template);
            }
        }
    }
    let (prefix, suffix) = general_expression_context(lang)?;
    let doc = format!("{prefix}{substituted}{suffix}");
    try_build_general_template(
        lang,
        doc,
        Some((prefix.len(), prefix.len() + substituted.len())),
        placeholders,
        multi_names,
    )
}

/// PASS 75a (F74a-4 face 3 + 74c-F4): the two php families whose general-lane
/// template only parses behind sg's own `<?php ` pattern pre-process:
/// (i) `->`-carrying member chains (`Foo::bar($A)->baz()` — sg answers the
/// root member call), and (ii) ALL-META scoped calls `$X::$Y(...)` (74c-F4).
/// The registered §21.2 php loud cells stay loud by construction:
/// `$A::bar(1)` (meta scope + LITERAL name) fails the (ii) all-meta test and
/// carries no `->`; `Foo::nested(Foo::bar($A))` likewise. Every `<?php `-led
/// build still demands a clean parse and a general root kind, so anything
/// beyond these two families keeps the fail-closed refusal.
fn php_wrapped_general_lane(raw: &str) -> bool {
    if raw.contains("->") {
        return true;
    }
    let Some(open) = raw.find('(') else {
        return false;
    };
    let head = raw[..open].trim();
    head.contains("::") && head.split("::").all(is_pure_metavariable)
}

/// Process-wide support cache for `needs_ast_grep_fallback` (language-free
/// question: does ANY grammar build a template?).
fn general_lane_supported(pattern: &str) -> bool {
    static SUPPORTED: OnceLock<RwLock<HashMap<String, bool>>> = OnceLock::new();
    let cache = SUPPORTED.get_or_init(|| RwLock::new(HashMap::new()));
    let cached = cache
        .read()
        .ok()
        .and_then(|map| map.get(pattern).copied());
    if let Some(supported) = cached {
        return supported;
    }
    let supported = general_lane_supported_uncached(pattern);
    if let Ok(mut writer) = cache.write() {
        writer.insert(pattern.to_string(), supported);
    }
    supported
}

fn general_lane_supported_uncached(pattern: &str) -> bool {
    // PASS 65 (F64-3): the conditional-directive family is natively
    // answerable when the condition parses under the shared [`CondBinding`]
    // rule — lone metavariable, metavar-free text, a single metavariable
    // nested in the condition (`#if defined($A)` ANSWERS like sg; the
    // preproc lane binds the nested metavar, probe-corrected in 65a), or —
    // PASS 67a (F66a-4) — a parseable structural condition template over 2+
    // canonical metavariables. `#define` joins through [`parse_define_tail`]
    // (F66a-5); non-directive patterns fall through untouched.
    if let Some(supported) = preproc_directive_supported(pattern) {
        return supported;
    }
    if !general_lane_text_eligible(pattern) {
        return false;
    }
    let Some((substituted, placeholders, multi_names)) = substitute_general_metavariables(pattern)
    else {
        return false;
    };
    // Identifier soup (`$A $B`, `RETURN $A`) is not a structural template.
    // PASS 60 (F58-2): registered statement-head templates (`return $A`)
    // carry no structural punctuation but are still structural — the
    // statement-root template lane decides them; uppercase/unregistered
    // heads never reach here (the bare-keyword guard refuses them first).
    // PASS 63 (F62-2): the head check no longer demands a `$` — the probed
    // `$`-less faces (`break`/`continue`) are statement templates too.
    let statement_head_template = pattern
        .split_whitespace()
        .next()
        .is_some_and(|head| STATEMENT_HEAD_KEYWORDS.contains(&head));
    if !substituted.contains(|c: char| "()[]{}<>=:,.!?+-*/%&|;~^@#".contains(c))
        && !statement_head_template
    {
        return false;
    }
    // PASS 63 (F62-1 boundary): a `#` whose tail carries NO code structure
    // is a comment-glue tail (`foo($A) # note`, `$A + $A# note`, the python
    // `# tail` ctrl faces) — the registered pass-60 contract keeps those
    // fail-closed at the language-free ingress even though some grammars
    // (kotlin_ng) happily parse the `#` tail as syntax. The `#`-as-SYNTAX
    // faces (`#[derive($A)]`, `#include $X`, `this.#x = $V`,
    // `tag(r#"$A"#)`) all carry code punctuation after their first `#` and
    // stay templatable.
    if contains_comment_glued_hash(pattern) {
        return false;
    }
    // PASS 63 (F62-1): `#` is comment syntax only in python/ruby/php — the
    // any-language support answer must not count a hash-comment language
    // templating a pattern whose `#` is real syntax elsewhere (and vice
    // versa: rust attribute faces stay supported while python hash-glued
    // faces keep failing closed).
    Language::all().iter().any(|&lang| {
        !lane_comment_refused(lang, pattern)
            && build_general_template(
                lang,
                pattern,
                &substituted,
                &placeholders,
                &multi_names,
            )
            .is_some()
    })
}

thread_local! {
    /// Per-thread general-lane template cache (template construction parses the
    /// pattern up to twice; the search walk pays it once per thread/pattern).
    static GENERAL_TEMPLATES: RefCell<HashMap<(Language, String), Option<GeneralTemplate>>> =
        RefCell::new(HashMap::new());
}

fn cached_general_template(lang: Language, pattern: &str) -> Option<GeneralTemplate> {
    let key = (lang, pattern.trim().to_string());
    GENERAL_TEMPLATES.with(|cell| {
        let mut map = cell.borrow_mut();
        if let Some(template) = map.get(&key) {
            return template.clone();
        }
        let built = if general_lane_text_eligible(pattern) && !lane_comment_refused(lang, pattern) {
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

fn match_structural_general(lang: Language, source: &str, pattern: &str) -> Vec<PatternMatch> {
    let Some(template) = cached_general_template(lang, pattern) else {
        return Vec::new();
    };
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let root = general_template_root(&template);
    let mut out = Vec::new();
    if let Some(root) = root {
        // P7 (pass 65): the byte-range dedup is a hash set, not a linear
        // scan — the scan made lone-metavar walks quadratic in the match
        // count (measured 11.84s vs 1.89s on the 32k-line perf corpus).
        // Insertion into `out` keeps emission order byte-for-byte.
        let mut seen = std::collections::HashSet::new();
        walk_general(
            tree.root_node(),
            source,
            pattern,
            &template,
            root,
            &mut seen,
            &mut out,
        );
    }
    out
}

fn walk_general<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    template: &GeneralTemplate,
    template_root: Node<'a>,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    // PASS 65a (F64-4): the guard is ANCESTOR-only — a string-rooted
    // template must match the string node itself, while everything nested
    // inside a comment/string ancestor stays skipped (calls inside `#{…}`
    // for other templates, string_content, …).
    if node.kind() == template.root_kind && !is_inside_comment_or_string(&node) {
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
        }
        if general_eq(template, template_root, node, source, &mut captures).is_some() {
            let byte_start = node.start_byte();
            let byte_end = node.end_byte();
            if seen.insert((byte_start, byte_end)) {
                let (line_start, line_end) = node_lines(&node, source);
                out.push(PatternMatch {
                    line_start,
                    line_end,
                    byte_start,
                    byte_end,
                    excerpt: excerpt_for_node(&node, source, pattern),
                    captures,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_general(child, source, pattern, template, template_root, seen, out);
    }
}

/// First-order structural comparison of a pattern subtree against a candidate
/// subtree. A pattern node whose ENTIRE text is a substituted metavariable
/// binds the whole candidate node (sg semantics — this is how a TS
/// `required_parameter` spelled `$B` binds the full `param: type` text).
fn general_eq<'p>(
    template: &GeneralTemplate,
    p: Node<'p>,
    c: Node<'p>,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    if let Some(text) = node_text(&p, &template.doc) {
        if let Some(name) = template.placeholders.get(text.trim()) {
            let bound = node_text(&c, source)?;
            return bind_capture(captures, name, &bound);
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
            .collect()
    };
    // PASS 63 (F62-5): statement-terminator lenience. Templates parse at doc
    // roots where ASI-tolerant grammars omit the `;` (ts/js `return x`), or
    // carry it via the context suffix (C `return x;`); candidate statements
    // spell whichever the source used. A lone anonymous `;` tail difference
    // is a termination artifact, not a structural difference.
    // PASS 65 (H-CONF-034): the lenience is ONE-DIRECTIONAL — only the
    // CANDIDATE's terminator is an ASI artifact. sg keeps a PATTERN-trailing
    // `;` significant (probed: ts `return $A;` refuses the semicolon-less
    // `return 2` while answering the `;`-ful lines), so a pattern that
    // spells the terminator never pops it.
    if p_children.last().is_some_and(|n| n.kind() != ";")
        && c_children.last().is_some_and(|n| n.kind() == ";")
    {
        c_children.pop();
    }
    // PASS 63 (F62-2, ruby command args): sg binds a LONE metavariable
    // argument to the whole argument list when the source command carries
    // several (probe: `raise $A` on `raise ArgumentError, 'bad'` binds
    // A = "ArgumentError, 'bad'"). Only argument containers take this —
    // call patterns (`f($A)`) are served by the call lane, whose registered
    // arity contract is untouched.
    if p_children.len() == 1
        && c.kind().contains("argument")
        && c_children.len() > 1
    {
        if let Some(text) = node_text(&p_children[0], &template.doc) {
            if let Some(name) = template.placeholders.get(text.trim()) {
                let bound = node_text(&c, source)?;
                return bind_capture(captures, name, &bound);
            }
        }
    }
    // PASS 73 (F72a-2): sole-rest argument lists. A `$$$NAME` spelled as the
    // SOLE argument of its call (the only shape [`nested_call_rest_template`]
    // admits into this lane) matches any candidate arity of the aligned
    // arguments container — empty (`fetch()`), single (`fetch(a)`), or multi
    // (`fetch(a, b)`) — binding the container's inner text in the MULTI
    // namespace exactly like [`capture_arguments`] (H-CONF-026). sg 0.45.2
    // probes: `g(fetch($$$A))` answers all three arities and answers BOTH
    // the outer and the inner call of `fetch(g(fetch($$$A)))` faces.
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
            .map(|text| strip_container(&text).to_string())
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

fn if_body_matches(node: &Node, template: Option<&BodyTemplate>) -> bool {
    let Some(template) = template else {
        return true;
    };
    let Some(consequence) = if_consequence(node) else {
        return false;
    };
    match template {
        BodyTemplate::Any => true,
        BodyTemplate::Exactly(want) => {
            if BLOCK_KINDS.contains(&consequence.kind()) {
                count_statements(consequence) == *want
            } else {
                // Braceless consequence (`if (x) foo();`) is one statement.
                *want == 1
            }
        }
    }
}

/// The then-branch of an if node: `consequence`/`body` field, else the first
/// block-like named child (first, not last, so an else block is never picked).
fn if_consequence<'a>(node: &Node<'a>) -> Option<Node<'a>> {
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
/// templates — EXP-005 strictness is about presence agreement, not keyword
/// translation.
const RETURN_TYPE_FIELDS: &[&str] = &["return_type", "result", "returns"];

/// EXP-005 (H-CONF-005): return-type presence agreement between the template
/// and the matched declaration node.
fn function_return_type_absent(node: &Node) -> bool {
    RETURN_TYPE_FIELDS
        .iter()
        .all(|field| node.child_by_field_name(field).is_none())
}

fn function_body_matches(node: &Node, template: &BodyTemplate) -> bool {
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
fn function_body_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
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
fn count_statements(body: Node) -> usize {
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

/// When `node` is a call outside trivia that matches `path`, return its callee segments.
fn call_match_path(node: &Node, source: &str, path: &[Option<String>]) -> Option<Vec<String>> {
    if is_in_comment_or_string(node) || !is_call_kind(node.kind()) {
        return None;
    }
    let (callee, synthetic_chain) = call_callee(node, source)?;
    // A synthetic chain (java/php `object`+`name` splits, ruby receiver-dot
    // calls) spans several AST fields, so there is no single callee node for a
    // lone metavariable to bind: sg keeps `$F($$$A)`/`helper($$$A)` empty on
    // those calls (java/ruby probes, pass-22 matrix). Two-segment-plus
    // patterns resolve normally.
    if synthetic_chain && path.len() < 2 {
        return None;
    }
    path_matches(&callee, path).then_some(callee)
}

/// Callee resolution for call-pattern matching and `call:` index rows.
///
/// Returns the dotted path segments, the full callee source text, and whether
/// the chain was reassembled from split fields rather than read off one node.
///
/// - Grammars that split a member call across `object` + `name` fields
///   (java `method_invocation`) previously exposed only the trailing `name`
///   identifier: literal dotted patterns (`System.out.println($$$A)`) were
///   silently empty while trailing-name patterns (`println($$$A)`)
///   over-matched (H-CONF-009, pass-22).
/// - Ruby receiver-dot calls carry the callee as `receiver` + `method`; they
///   are path calls only with a `.` operator AND a parenthesized argument
///   list. Bare `text.upcase` and operator `"a" + "b"` calls stay unmatched,
///   exactly matching sg (pass-15 oracle-exact pins, re-probed pass-22).
fn call_callee<'a>(node: &Node<'a>, source: &'a str) -> Option<(Vec<String>, bool)> {
    // `method_invocation` is java(/kotlin-family) member calls: the member
    // operator is `.`, so dot-separated patterns express them. php's
    // `member_call_expression` also splits object/name but its operator is
    // `->`, which dot patterns can never spell — sg keeps every dot-pattern
    // empty there (pass-22 matrix), so php must keep the trailing-name shape.
    if node.kind() == "method_invocation" {
        if let (Some(obj), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            let mut segs = path_from_node(&obj, source)?;
            segs.push(node_text(&name, source)?.to_string());
            return Some((segs, true));
        }
    }
    if node.kind() == "call" {
        if let Some(segs) = ruby_receiver_callee(node, source) {
            return Some((segs, true));
        }
    }
    // PASS 75a (F74a-2): php `member_call_expression` (plain `->` calls) is
    // served by the dedicated [`NativeKind::MemberCall`] lane ONLY — the
    // plain Call lane must never see a member-call candidate, because a
    // member callee resolves to the full object->name segment chain sg
    // matches (`a->b()` == `a.b()` as segments) and a DOT template would
    // over-match the `->` call site (sg is connector token-exact: `a.b($A)`
    // answers [] on php `->` calls). The synthetic-chain flag keeps the
    // lone-metavar veto (`$F($$$A)` never answers a member call), and an
    // object the strict resolver cannot spell (nullsafe link, exotic
    // receiver) yields the EMPTY veto shape instead of the registered
    // trailing-name shape — sg keeps every bare-name pattern empty on
    // member calls. `call_target` falls back to the raw callee bytes for
    // the index row.
    if node.kind() == "member_call_expression" {
        if let (Some(object), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            if let Some(name_text) = node_text(&name, source) {
                if let Some(mut segments) = php_member_object_segments(&object, source) {
                    segments.push(name_text.to_string());
                    return Some((segments, true));
                }
                return Some((Vec::new(), true));
            }
        }
    }
    // PASS 73 (F72a-1): php `scoped_call_expression` (static `::` calls)
    // splits the callee across `scope` + `name` fields with no single callee
    // node. sg matches scope-to-scope and name-to-name (0.45.2 probes:
    // `Foo::bar($A)` answers; `$A::bar($B)` binds the RAW scope text —
    // `Foo`, `self`, `$inst`; `Foo::$M($A)` binds `$dyn` with the dollar),
    // so the dotted two-segment chain IS the sg shape here (patterns
    // normalize `::` to `.`). The synthetic-chain flag keeps the
    // lone-metavar veto: `$F($$$A)` stays empty on `Foo::bar(1)` exactly
    // like sg (only plain `helper(...)` calls answer). php `->` member
    // calls keep the registered trailing-name shape (pass-22 matrix) —
    // this arm keys on `scoped_call_expression` only.
    if node.kind() == "scoped_call_expression" {
        if let (Some(scope), Some(name)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(scope_text), Some(name_text)) =
                (node_text(&scope, source), node_text(&name, source))
            {
                return Some((vec![scope_text.to_string(), name_text.to_string()], true));
            }
        }
    }
    let field = call_field_node(node)?;
    path_from_node(&field, source).map(|segs| (segs, false))
}

/// PASS 75a (F74a-2): strict object-side segments for a php member call.
/// A `variable_name` (`$svc`) is ONE raw-text segment; a plain
/// `member_access_expression` descends (`$a->b` → `["$a", "b"]`); identifier
/// kinds pass through. Nullsafe-access and call objects are refused — their
/// connector/shape fidelity has no probed contract here, so those faces stay
/// off the plain-`->` chain lane (fail-closed, never an over-match).
fn php_member_object_segments<'a>(node: &Node<'a>, source: &'a str) -> Option<Vec<String>> {
    match node.kind() {
        "member_access_expression" => {
            let mut segments = php_member_object_segments(
                &node.child_by_field_name("object")?,
                source,
            )?;
            segments.push(node_text(&node.child_by_field_name("name")?, source)?.to_string());
            Some(segments)
        }
        "nullsafe_member_access_expression" => None,
        // PASS 75a: a scoped-call object (`Foo::bar($u)->baz()`) decomposes
        // into its scope::name segments — the sg chain shape.
        "scoped_call_expression" => call_callee(node, source).map(|(segments, _)| segments),
        _ => {
            if is_ident_kind(node.kind())
                || KEYWORD_RECEIVER_KINDS.contains(&node.kind())
                || node.kind() == "variable_name"
            {
                node_text(node, source).map(|text| vec![text.to_string()])
            } else {
                None
            }
        }
    }
}

/// Ruby `receiver.method(...)` chain segments. The top-level call must carry a
/// `.` operator AND a parenthesized argument list — bare `text.upcase` and
/// operator `"a" + "b"` calls are not call-pattern shapes (sg matches nothing
/// on them). Nested chains recurse through bare receiver calls
/// (`a.b.c(1)` → `["a", "b", "c"]`), where the intermediate `a.b` has no
/// arguments of its own.
fn ruby_receiver_callee(node: &Node, source: &str) -> Option<Vec<String>> {
    if node.kind() != "call" {
        return None;
    }
    let operator = node.child_by_field_name("operator")?;
    if node_text(&operator, source) != Some(".") {
        return None;
    }
    node.child_by_field_name("arguments")?;
    let receiver = node.child_by_field_name("receiver")?;
    let method = node.child_by_field_name("method")?;
    let mut segs = ruby_receiver_base(&receiver, source)?;
    segs.push(node_text(&method, source)?.to_string());
    Some(segs)
}

/// Receiver-side resolution: like [`ruby_receiver_callee`] but without the
/// argument-list requirement, descending through nested dot calls; non-call
/// receivers (identifiers, constants, `self`, member kinds) fall through to
/// the shared path builder.
fn ruby_receiver_base(node: &Node, source: &str) -> Option<Vec<String>> {
    if node.kind() == "call" {
        let operator = node.child_by_field_name("operator")?;
        if node_text(&operator, source) != Some(".") {
            return None;
        }
        let receiver = node.child_by_field_name("receiver")?;
        let method = node.child_by_field_name("method")?;
        let mut segs = ruby_receiver_base(&receiver, source)?;
        segs.push(node_text(&method, source)?.to_string());
        return Some(segs);
    }
    path_from_node(node, source)
}

fn call_field_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    ["function", "name"]
        .into_iter()
        .find_map(|f| node.child_by_field_name(f))
        // Swift/Kotlin call_expression has no function/name fields; callee is the
        // first named child. Safe for other langs: C# uses `invocation_expression`,
        // and field-bearing grammars hit find_map first.
        .or_else(|| {
            (node.kind() == "call_expression")
                .then(|| node.named_child(0))
                .flatten()
        })
        // Pass 15 (H-CONF-018): ruby's call kind is `call` with the callee in
        // the `method` field, so neither probe above ever fired — every ruby
        // call pattern (index rows AND native matches) was silently empty.
        // The callee is honored only on receiver-free calls: ast-grep agrees
        // with `$F($$$A)`/`upcase($$$A)` on `greet("world")` but matches
        // nothing on `text.upcase` or the operator call `"hello " + name`
        // (both are `call` nodes carrying a `receiver`).
        .or_else(|| {
            match node.kind() == "call" && node.child_by_field_name("receiver").is_none() {
                true => node.child_by_field_name("method"),
                false => None,
            }
        })
}

fn call_target_path(node: &Node, source: &str) -> Option<Vec<String>> {
    call_callee(node, source).map(|(segs, _)| segs)
}

/// PASS 60 (F58-3): the receiver path used for METAVARIABLE BINDING on
/// multi-segment callee patterns. Like `call_target_path`, but refusing
/// paths that flatten through a non-member node (a call/paren/index
/// receiver): sg decomposes a pattern callee into plain member-chain nodes
/// and rejects call-carrying receivers (probe: `$O.$O($$$A)` matches
/// `b.b(1)` but not `y.first().first()`), so a flattened tail has no
/// faithful per-segment text and must not unify via `bind_capture`.
fn call_target_path_faithful(node: &Node, source: &str) -> Option<Vec<String>> {
    if node.kind() == "method_invocation" {
        if let (Some(obj), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            let mut segs = faithful_path_from_node(&obj, source)?;
            segs.push(node_text(&name, source)?.to_string());
            return Some(segs);
        }
    }
    if node.kind() == "call" {
        // Ruby receiver-dot calls keep their registered flattening (the
        // ruby probes in the pass-22 matrix pin those semantics); only the
        // member-chain decomposition below is refined here.
        if ruby_receiver_callee(node, source).is_some() {
            return call_target_path(node, source);
        }
    }
    // PASS 73 (F72a-1): php static calls decompose into exactly the scope
    // and name positions sg unifies against (raw texts — `$inst`/`$dyn`
    // included; probed). The scope is ONE node, so its text is faithful by
    // construction; there is no flattening to veto.
    // PASS 75a (F74a-2): the plain `->` member-call twin — object and name
    // are single faithful nodes/strict chains (the strict object resolver
    // refuses flattening), so the raw texts unify exactly like sg.
    if node.kind() == "member_call_expression" {
        if let (Some(object), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(mut segments), Some(name_text)) =
                (php_member_object_segments(&object, source), node_text(&name, source))
            {
                segments.push(name_text.to_string());
                return Some(segments);
            }
        }
    }
    if node.kind() == "scoped_call_expression" {
        if let (Some(scope), Some(name)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(scope_text), Some(name_text)) =
                (node_text(&scope, source), node_text(&name, source))
            {
                return Some(vec![scope_text.to_string(), name_text.to_string()]);
            }
        }
    }
    let field = call_field_node(node)?;
    faithful_path_from_node(&field, source)
}

/// Like [`path_from_node`], but returning `None` instead of falling back to
/// `last_identifier_in_chain` — a chain segment that is not a plain
/// identifier or member expression (a call receiver, paren, subscript…)
/// has no faithful segment text.
fn faithful_path_from_node(node: &Node, source: &str) -> Option<Vec<String>> {
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        return node_text(node, source).map(|t| vec![t.to_string()]);
    }
    if !is_member_expr_kind(node.kind()) {
        return None;
    }
    let mut segs = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // PASS 65 (F64-7): an optional-chain separator (`?.`) is not a
        // plain member segment — sg refuses `?.` candidates for a `.`
        // template. The separator is an anonymous `?.` token in most
        // grammars and a named `optional_chain` node in others; either
        // spelling vetoes the path instead of being skipped like the `.`
        // punctuation.
        if child.kind().contains('?') || child.kind().contains("optional") {
            return None;
        }
        if !child.is_named() {
            // Anonymous punctuation (`.` separators) is not a chain segment.
            continue;
        }
        // F58-3: unlike `path_from_node`, a NAMED child with no faithful
        // path (a call/paren/subscript receiver) VETOES the whole chain
        // instead of being silently skipped — its "segments" would be
        // flattened lies about the receiver shape.
        let mut p = faithful_path_from_node(&child, source)?;
        segs.append(&mut p);
    }
    (!segs.is_empty()).then_some(segs)
}

/// Keyword receivers that count as a path segment in `$OBJ.$METHOD($$$)`:
/// `self.helper()` / `this.render()` must match a two-segment wildcard path
/// exactly like `app.tick()` does (ast-grep agrees). Rust/Ruby use `self`,
/// JS/TS/Java/C++ use `this`, Swift `self_expression`, Kotlin/C#
/// `this_expression`. Not added to `IDENT_KINDS`: that table also drives
/// index extraction, where keyword receivers must stay non-identifiers.
const KEYWORD_RECEIVER_KINDS: &[&str] = &["self", "this", "self_expression", "this_expression"];

fn path_from_node(node: &Node, source: &str) -> Option<Vec<String>> {
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        return node_text(node, source).map(|t| vec![t.to_string()]);
    }
    if !is_member_expr_kind(node.kind()) {
        return last_identifier_in_chain(node, source).map(|s| vec![s]);
    }
    let mut segs = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // PASS 65 (F64-7): optional-chain separators veto the path (see
        // `faithful_path_from_node`) — anonymous `?.` tokens and named
        // `optional_chain` nodes alike.
        if child.kind().contains('?') || child.kind().contains("optional") {
            return None;
        }
        if let Some(mut p) = path_from_node(&child, source) {
            segs.append(&mut p);
        }
    }
    (!segs.is_empty()).then_some(segs)
}

fn path_matches(actual: &[String], pattern: &[Option<String>]) -> bool {
    let segment_ok = |a: &String, p: &Option<String>| p.as_ref().is_none_or(|w| w == a);
    if actual.len() == pattern.len() {
        return actual
            .iter()
            .zip(pattern.iter())
            .all(|(a, p)| segment_ok(a, p));
    }
    // Pass 22 (H-CONF-009): ast-grep binds a leading metavariable to the WHOLE
    // multi-segment receiver — `$O.$M($$$A)` matches `a.b.c(2)` with
    // `$O` = `a.b` (probed on java/python/ts/js/rust/kotlin/swift). A literal
    // leading segment pins the chain to the exact length (`obj.$M($$$A)` does
    // not reach `a.b.c`), and a bare-name pattern never matches a member chain
    // (`helper($$$A)` stays empty everywhere).
    if actual.len() > pattern.len() && pattern.first().is_some_and(|p| p.is_none()) {
        return actual[actual.len() - pattern.len() + 1..]
            .iter()
            .zip(pattern[1..].iter())
            .all(|(a, p)| segment_ok(a, p));
    }
    false
}

fn walk_literal(
    node: Node,
    source: &str,
    pattern: &str,
    template: Option<&LiteralTemplate>,
    out: &mut Vec<PatternMatch>,
) {
    if !is_in_comment_or_string(&node) {
        if identifier_matches(&node, source, pattern) {
            push_match(&node, source, pattern, Some(pattern), out);
        } else if literal_content_matches(&node, source, pattern) {
            push_match(&node, source, pattern, Some(pattern), out);
        } else if let Some(template) = template {
            // H-CONF-021 rule R3 (pass 54): the structural arm — only ever
            // ADDS matches after the exact-text fast paths above declined.
            if let Some(pat_root) = literal_template_root(template) {
                // PASS 65 (F64-3): anonymous preproc directive tokens
                // (`#endif`) are not structural roots — sg answers nothing.
                if node.kind() == pat_root.kind()
                    && !pat_root.kind().starts_with('#')
                    && literal_structural_eq(pat_root, &template.doc, node, source)
                {
                    push_match(&node, source, pattern, Some(pattern), out);
                }
            }
        }
        // PASS 56 (H-AUDIT-52-2): a `$`-less pattern names the IDENTIFIER
        // itself — emit the `name` node's span, never the enclosing item.
        // Pushing `&node` here made a bare-ident pattern also match the whole
        // `fn old_name() { … }` declaration: search over-answered (parent +
        // child rows for one site) and codemod planned a destructive
        // whole-item rewrite that only the overlap validator caught.
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

fn identifier_matches(node: &Node, source: &str, pattern: &str) -> bool {
    is_ident_kind(node.kind()) && node_text(node, source).is_some_and(|t| t == pattern)
}

/// H-CONF-021 (pass 43): full-node-text arm of the literal lane. A `$`-less
/// pattern matches any non-trivia node outside comments/strings whose
/// COMPLETE text equals the pattern — number literals, literal-argument
/// calls, zero-arg member calls, whole statements (sg literal-content
/// semantics). Previously only identifier-kind nodes could match, so these
/// faces silently answered `ok:true` empty — a fail-open. The arm only ADDS
/// matches: the pass-30 loud-fallback guard (`needs_ast_grep_fallback`) keeps
/// exempting `$`-less patterns, and valid-but-empty results stay `ok:true`.
/// H-CONF-021 rule R3 (pass 54): the whitespace/trivia-variant residual from
/// pass-34 §3a is now closed by the structural arm in `walk_literal` — this
/// exact-text arm stays as the byte-stable fast path in front of it.
fn literal_content_matches(node: &Node, source: &str, pattern: &str) -> bool {
    if is_trivia_kind(node.kind()) {
        return false;
    }
    // PASS 65 (F64-3): bare preproc END/else directives are anonymous
    // tokens, not matchable nodes — sg cannot shape `#endif` into a pattern
    // tree (probes exit 1 ran-empty). The exact-text arm must not answer
    // them from the token bytes.
    if node.kind().starts_with('#') {
        return false;
    }
    node_text(node, source).is_some_and(|text| text.trim() == pattern)
}

// ---------------------------------------------------------------------------
// H-CONF-021 (pass 54): rule R3 — a `$`-less literal pattern matches a
// candidate node iff their trees are isomorphic. Whitespace of every kind
// (spaces, tabs, newlines, blank lines, operator and callee-paren gaps) and
// code-side trailing commas are invisible; comment nodes are invisible iff
// they are attached INSIDE an argument/parameters container (a comment child
// of the call node itself blocks, either side); a PATTERN-side trailing comma
// is significant (it matches only sites whose source carries one); any other
// AST delta (arg count, concatenated_string vs string, kind changes) blocks.
// The comparator integrates AFTER the pass-43 exact-text arms, so it only
// ever ADDS matches — fail-open cannot be introduced, and valid-but-empty
// results stay ok:true. Patterns no grammar parses keep today's exact-text
// behavior (`parse_pattern_tree` yields None).
// ---------------------------------------------------------------------------

/// A parsed literal-pattern template: the pattern document, its tree, and —
/// when the language required an expression context to parse it — the byte
/// span of the pattern inside that document.
struct LiteralTemplate {
    doc: String,
    tree: tree_sitter::Tree,
    span: Option<(usize, usize)>,
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
/// disables the structural arm so sg-rejected pattern text (`text# note` py)
/// keeps today's exact-text behavior with no D1 regression.
fn parse_pattern_tree(lang: Language, pattern: &str) -> Option<std::sync::Arc<LiteralTemplate>> {
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

fn build_literal_template(lang: Language, pattern: &str) -> Option<LiteralTemplate> {
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
/// yields `None` — sg reports each statement separately and the subject does
/// not approximate that here.
fn literal_template_root<'a>(template: &'a LiteralTemplate) -> Option<Node<'a>> {
    let mut node = match template.span {
        Some((start, end)) => {
            let mut node = template
                .tree
                .root_node()
                .descendant_for_byte_range(start, end)?;
            while let Some(parent) = node.parent() {
                if parent.start_byte() == node.start_byte()
                    && parent.end_byte() == node.end_byte()
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
        // PASS 65a (F64-5, H-CONF-034 in the literal lane): a
        // pattern-trailing `;` is significant — descending through an
        // expression_statement that CARRIES the terminator would let the R3
        // structural arm answer the bare sub-expression node (a second,
        // `;`-less span where sg reports the statement span, cols 4-25 on
        // the `alpha.beta().gamma();` probe). Statement roots keep the `;`
        // child in the comparison; `;`-less patterns keep descending.
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
            _ => return None,
        }
    }
}

/// Recursive R3 isomorphism check between a pattern subtree and a candidate
/// subtree: kind equality; exact-text leaves; the container-scoped comment
/// guard; comma significance; field-name alignment.
fn literal_structural_eq(p: Node, pattern_doc: &str, c: Node, source: &str) -> bool {
    if p.kind() != c.kind() {
        return false;
    }
    if p.child_count() == 0 {
        return node_text(&p, pattern_doc).is_some_and(|text| node_text(&c, source) == Some(text));
    }
    let container = is_argument_container_kind(p.kind());
    // R3 comment guard: comments are invisible only inside an arguments /
    // parameters container; a comment child anywhere else — e.g. between the
    // callee and the paren on the call node — is a real AST child and blocks
    // the match on EITHER side. T-B4 is this policy's mutation kill cell: the
    // "comments invisible everywhere" R1 mutant (this guard AND the
    // container-only skip in `comparable_children` disabled together) flips
    // that cell to a match. Each half alone is redundant defense — the other
    // half still blocks (child-count mismatch, respectively the guard) — so
    // only the combined mutant is a valid discrimination probe.
    if !container && (has_comment_child(&p) || has_comment_child(&c)) {
        return false;
    }
    // R3 comma clause: a PATTERN-side trailing comma is significant; a
    // code-side one is invisible unless the pattern demands it.
    if container && has_trailing_comma(&p) && !has_trailing_comma(&c) {
        return false;
    }
    // PASS 60 (fuzz F26-0601 family + H-CONF-030-ii): a PATTERN-side comment
    // inside an argument container is a REQUIRED SLOT (sg pins: pattern
    // `add(1, /* n */ 2)` answers only sites carrying a comment in that
    // argument position — text-free, so `/* mid */` matches `/* note */` —
    // and does NOT reach the comment-less `add(1, 2)` site). Source-side
    // comments remain invisible where no slot demands them, and trailing
    // source comments after the last matched child stay invisible.
    if container && has_comment_child(&p) {
        return container_comment_slot_eq(p, pattern_doc, c, source);
    }
    let mut p_children = comparable_children(p, container);
    let mut c_children = comparable_children(c, container);
    // PASS 69a (F68a-1): a source-side statement terminator is invisible
    // when the pattern lacks it — sg 0.45.2 probed: `const $x = 1` answers
    // `const $x = 1;` (js/ts declaration roots own the `;` byte, so the
    // child lists mismatch without this clause). A PATTERN-side `;` stays
    // significant (the registered H-CONF-034 terminator pin — this clause
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
        if !literal_structural_eq(p_child.node, pattern_doc, c_child.node, source) {
            return false;
        }
    }
    true
}

/// PASS 60: positional alignment for containers whose PATTERN carries
/// comment children (required slots). Pattern children keep comments and
/// drop commas; source children keep comments and drop commas; trailing
/// source comments after the last consumed child are invisible. A pattern
/// comment consumes exactly one source comment (any kind, text-free); a
/// pattern code child consumes the next source CODE child (leading source
/// comments are skipped only while a code child is being matched and no
/// slot precedes it positionally — once a slot has been demanded, source
/// comments are consumed by slots or block).
fn container_comment_slot_eq(
    p: Node,
    pattern_doc: &str,
    c: Node,
    source: &str,
) -> bool {
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
    // PASS 63 (F62-4): comment slots align by RAW comma-adjacency, not by
    // text and not across separator positions. sg keeps the separator
    // significant when attaching comments (probes: `calc(1, /* n */ 2)`
    // answers only the source whose comment also sits AFTER the comma — the
    // pre-comma `calc(1 /* mid */, 2)` is rejected — while a DIFFERENT
    // comment text in the same slot still answers, the registered pass-60
    // 030-ii rows). The comma-stripped child lists above erase exactly that
    // distinction, so both sides get a slot class from their RAW siblings:
    // PostComma (previous raw non-trivia sibling is `,`), Leading (container
    // opener or first child), Glued (after a code argument).
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
        // invisible (pass-54 contract — a comment-free pattern still matches
        // comment-carrying sources), so skip them here; a slot above never
        // skips, which is what keeps F26-0601 positional.
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
        if !literal_structural_eq(p_child.node, pattern_doc, c_child.node, source) {
            return false;
        }
    }
    // Source children left over (after trailing-comment strip) block.
    c_iter.next().is_none()
}

/// Raw-sibling slot class of every comment child of a container (F62-4).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotClass {
    Leading,
    PostComma,
    Glued,
}

fn slot_classes_by_id(container: Node) -> std::collections::HashMap<usize, SlotClass> {
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
/// invisible trivia. Kind-string check per the pass-53 design; exotic
/// containers could mis-guard, which is exactly what the T-B4 cell pins.
/// Known grammar-attachment variance (scoped residual, documented in the
/// ledger): sg's pinned python fork attaches a pre-first-argument comment at
/// the CALL node (so sg blocks that face) while the vendored
/// tree-sitter-python attaches it inside `argument_list` (so this guard
/// skips it and the face matches). The attachment rule itself is faithful;
/// blocking that face positionally would break sg's rust x13 face
/// (`calc(/* lead */ 1, 2)` matches) which our tree also attaches inside
/// the container.
fn is_argument_container_kind(kind: &str) -> bool {
    kind.contains("argument") || kind.contains("parameters")
}

fn has_comment_child(node: &Node) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| is_trivia_kind(child.kind()));
    found
}

struct ComparableChild<'a> {
    field: Option<&'a str>,
    node: Node<'a>,
}

/// Children aligned for comparison: inside containers, comment trivia and
/// separator/trailing commas are dropped from BOTH sides (comma significance
/// is enforced separately by `has_trailing_comma`); grammar field names are
/// captured at their raw child index so alignment survives the filtering.
fn comparable_children<'a>(node: Node<'a>, skip_trivia: bool) -> Vec<ComparableChild<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .enumerate()
        .filter(|(_, child)| {
            !skip_trivia || (!is_trivia_kind(child.kind()) && child.kind() != ",")
        })
        .map(|(index, child)| ComparableChild {
            field: node.field_name_for_child(index as u32),
            node: child,
        })
        .collect()
}

/// True when the container's source carries a trailing comma: the last
/// significant child before any closing paren token (comments are
/// transparent) is `,`.
fn has_trailing_comma(node: &Node) -> bool {
    let mut cursor = node.walk();
    let mut kinds: Vec<&str> = node.children(&mut cursor).map(|child| child.kind()).collect();
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

pub(crate) fn collect_pattern_nodes(root: Node, source: &str) -> Vec<PatternNode> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_node_signatures(root, source, &mut out, &mut seen);
    out
}

fn collect_node_signatures(
    node: Node,
    source: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    if !is_in_comment_or_string(&node) {
        record_node_signatures(&node, source, out, seen);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_signatures(child, source, out, seen);
    }
}

fn record_node_signatures(
    node: &Node,
    source: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    if is_ident_kind(node.kind()) {
        if let Some(text) = node_text(node, source) {
            push_pattern_node(*node, source, text, out, seen);
        }
    }
    if let Some(prefix) = declaration_prefix(node, source) {
        push_pattern_node(*node, source, &format!("kind:{}", node.kind()), out, seen);
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|n| node_text(&n, source))
        {
            push_pattern_node(*node, source, &format!("{prefix} {name}"), out, seen);
            push_pattern_node(*node, source, &format!("decl:{prefix}:{name}"), out, seen);
        }
    }
    if !is_call_kind(node.kind()) {
        return;
    }
    push_pattern_node(*node, source, &format!("kind:{}", node.kind()), out, seen);
    let Some(callee) = call_target(node, source) else {
        return;
    };
    push_pattern_node(*node, source, &format!("call:{callee}"), out, seen);
    if let Some(name) = callee.rsplit(['.', ':']).find(|p| !p.is_empty()) {
        push_pattern_node(*node, source, &format!("call-name:{name}"), out, seen);
    }
}

/// Map a tree-sitter declaration node to its indexed `decl:` / display prefix.
///
/// Most kinds are table-driven; `class_declaration` inspects Swift
/// `declaration_kind` / Kotlin keyword tokens so singleton forms stay exact.
pub fn declaration_prefix(node: &Node, source: &str) -> Option<&'static str> {
    let kind = node.kind();
    if kind == "class_declaration" {
        return class_declaration_prefix(node, source);
    }
    DECL_KIND_PREFIXES
        .iter()
        .find_map(|&(node_kind, prefix)| (node_kind == kind).then_some(prefix))
}

fn class_declaration_prefix(node: &Node, source: &str) -> Option<&'static str> {
    match node
        .child_by_field_name("declaration_kind")
        .and_then(|kind| node_text(&kind, source))
    {
        Some("struct" | "actor") => Some("struct"),
        Some("enum") => Some("enum"),
        Some("extension") => Some("type"),
        // No Swift declaration_kind (or unrecognised) — Kotlin reuses class_declaration.
        _ => match kotlin_class_keyword(node, source) {
            "interface" => Some("interface"),
            "enum" => Some("enum"),
            _ => Some("class"),
        },
    }
}

/// AST node kind → short declaration prefix used in `decl:{prefix}:{name}` signatures.
pub const DECL_KIND_PREFIXES: &[(&str, &str)] = &[
    ("function_item", "fn"),
    ("struct_item", "struct"),
    ("struct_declaration", "struct"),
    ("struct_specifier", "struct"),
    ("function_definition", "def"),
    ("function_declaration", "function"),
    ("protocol_function_declaration", "function"),
    ("method_definition", "function"),
    ("method_declaration", "function"),
    ("method", "function"),
    ("singleton_method", "function"),
    ("local_function_statement", "function"),
    ("class_definition", "class"),
    ("class", "class"),
    ("record_declaration", "class"),
    ("class_specifier", "class"),
    ("trait_item", "interface"),
    ("interface_declaration", "interface"),
    ("protocol_declaration", "interface"),
    ("enum_item", "enum"),
    ("enum_declaration", "enum"),
    ("enum_specifier", "enum"),
];

// e2hc/difu.5: invocation_expression is the C# tree-sitter grammar's call node.
const CALL_KINDS: &[&str] = &[
    "call_expression",
    "call",
    "method_invocation",
    "invocation_expression",
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
];

fn is_call_kind(kind: &str) -> bool {
    CALL_KINDS.contains(&kind)
}

/// Full callee text for `call:` index rows. Split-field chains (java/php
/// `object`+`name`, ruby receiver-dot calls) have no single callee node, so
/// their text is reassembled from the resolved segments; every other grammar
/// keeps the callee node's exact source bytes.
fn call_target<'a>(node: &Node<'a>, source: &'a str) -> Option<Cow<'a, str>> {
    // PASS 73 (F72a-1): php static-call rows key on the exact source callee
    // bytes (`Foo::bar`), NOT the trailing name. The old `call:bar` key made
    // `bar($$$A)` index-serve every `Foo::bar(1)` line (silent over-match vs
    // sg []) and left `Foo::bar($$$A)` — whose signature derives the raw
    // `call:Foo::bar` — with no rows to hit (silent miss). sg's own callee
    // spelling is `scope::name`.
    if node.kind() == "scoped_call_expression" {
        if let (Some(scope), Some(name)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ) {
            if let (Some(scope_text), Some(name_text)) =
                (node_text(&scope, source), node_text(&name, source))
            {
                return Some(Cow::Owned(format!("{scope_text}::{name_text}")));
            }
        }
    }
    let (segs, synthetic) = call_callee(node, source)?;
    // PASS 75a: an empty synthetic chain is the member-call veto shape (an
    // object the strict resolver cannot spell) — the index row keeps the
    // registered raw callee bytes instead of a junk key.
    if synthetic && !segs.is_empty() {
        return Some(Cow::Owned(segs.join(".")));
    }
    if synthetic && segs.is_empty() {
        return call_field_node(node)
            .and_then(|t| node_text(&t, source))
            .map(Cow::Borrowed);
    }
    call_field_node(node)
        .and_then(|t| node_text(&t, source))
        .map(Cow::Borrowed)
}

fn push_pattern_node(
    node: Node,
    source: &str,
    signature: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    let (line_start, line_end) = node_lines(&node, source);
    if !seen.insert((signature.to_string(), line_start)) {
        return;
    }
    out.push(PatternNode {
        signature: signature.to_string(),
        line_start,
        line_end,
        excerpt: excerpt_for_node(&node, source, signature),
    });
}

fn push_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_text: Option<&str>,
    out: &mut Vec<PatternMatch>,
) {
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    let byte_start = node.start_byte();
    let byte_end = node.end_byte();
    if out
        .iter()
        .any(|matched| matched.byte_start == byte_start && matched.byte_end == byte_end)
    {
        return;
    }
    let Some(captures) = captures_for_node(node, source, pattern, name_text) else {
        // H-CONF-025 (pass 43): a repeated metavariable name is bound to two
        // different texts; sg unification semantics reject the candidate.
        return;
    };
    out.push(PatternMatch {
        line_start,
        line_end,
        byte_start,
        byte_end,
        excerpt,
        captures,
    });
}

fn captures_for_node(
    node: &Node,
    source: &str,
    pattern: &str,
    name_text: Option<&str>,
) -> Option<BTreeMap<String, String>> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(variable), Some(name)) = (declaration_name_capture(pattern), name_text) {
        bind_capture(&mut captures, variable, name)?;
    }
    capture_arguments(node, source, pattern, &mut captures)?;
    if let Some((variable, multi)) = body_capture(pattern) {
        let body = if is_if_prefixed(pattern.trim()) {
            if_consequence(node)
        } else {
            function_body_node(node)
        };
        if let Some(text) = body.and_then(|body| node_text(&body, source)) {
            bind_capture_kind(&mut captures, variable, strip_container(text), multi)?;
        }
    }
    if let Some(variable) = if_condition_capture(pattern) {
        if let Some(condition) = node.child_by_field_name("condition") {
            if let Some(text) = node_text(&condition, source) {
                bind_capture(&mut captures, variable, strip_container(text))?;
            }
        }
    }
    capture_call_path(node, source, pattern, &mut captures)?;
    Some(captures)
}

/// H-CONF-025 (pass 43): unified metavariable binding. A repeated name is
/// ONE variable bound once (sg semantics): binding an already-bound name to
/// a different text returns `None` and rejects the whole candidate match;
/// equal text re-affirms the binding. The reserved envelope key `MATCH`
/// (full node text) keeps its historical overwrite behavior so a pattern
/// that spells `$MATCH` as an ordinary metavariable keeps today's envelope
/// bytes.
fn bind_capture(
    captures: &mut BTreeMap<String, String>,
    name: &str,
    text: &str,
) -> Option<()> {
    if name == "MATCH" {
        captures.insert(name.to_string(), text.to_string());
        return Some(());
    }
    match captures.get(name) {
        Some(existing) => (existing == text).then_some(()),
        None => {
            captures.insert(name.to_string(), text.to_string());
            Some(())
        }
    }
}

/// PASS 60 (H-CONF-026): sg keeps single and multi metavariable namespaces
/// DISTINCT (`metaVariables.single.B` and `metaVariables.multi.B` coexist),
/// so a name bound BOTH as `$B` and `$$$B` is two variables. Multi bindings
/// key as `$$$NAME` in the capture map and never unify with the single
/// binding of the same name; unification within one namespace is unchanged.
fn bind_capture_kind(
    captures: &mut BTreeMap<String, String>,
    name: &str,
    text: &str,
    multi: bool,
) -> Option<()> {
    if !multi || name == "MATCH" {
        return bind_capture(captures, name, text);
    }
    let key = format!("$$${name}");
    match captures.get(&key) {
        Some(existing) => (existing == text).then_some(()),
        None => {
            captures.insert(key, text.to_string());
            Some(())
        }
    }
}

fn capture_name(token: &str) -> Option<&str> {
    let token = token.trim();
    // PASS 75a (F74a-3): `$$NAME` binds in the SINGLE namespace under `NAME`
    // (sg 0.45.2 capture key probe), so it unifies with a same-name `$NAME`
    // — `$A::bar($$A)` answers [] on php exactly like sg.
    let name = token
        .strip_prefix("$$$")
        .or_else(|| token.strip_prefix("$$"))
        .or_else(|| token.strip_prefix('$'))?;
    is_metavar_name(name).then_some(name)
}

fn declaration_name_capture(pattern: &str) -> Option<&str> {
    let (declaration, _) = strip_declaration_modifiers(pattern);
    DECL_PATTERN_PREFIXES.iter().find_map(|(prefix, _)| {
        let rest = declaration.strip_prefix(prefix)?;
        let head = rest
            .split(|c: char| c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace())
            .next()?;
        capture_name(head)
    })
}

fn capture_arguments(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let Some(arguments) = pattern_argument_text(pattern) else {
        return Some(());
    };
    let fields = if is_call_kind(node.kind()) {
        &["arguments"][..]
    } else {
        &["parameters"][..]
    };
    if arguments.starts_with("$$$") {
        if let Some(variable) = capture_name(arguments) {
            let text = argument_container(node, fields)
                .and_then(|container| node_text(&container, source))
                .map(strip_container)
                .unwrap_or_default();
            // H-CONF-026: `$$$` arguments bind in the multi namespace.
            bind_capture_kind(captures, variable, text, true)?;
        }
        return Some(());
    }
    let names = arguments
        .split(',')
        .filter_map(capture_name)
        .collect::<Vec<_>>();
    let Some(nodes) = argument_nodes(node, fields) else {
        return Some(());
    };
    for (name, argument) in names.into_iter().zip(nodes) {
        if let Some(text) = node_text(&argument, source) {
            bind_capture(captures, name, text)?;
        }
    }
    Some(())
}

/// PASS 60 (H-CONF-026): returns `(name, multi)` — `multi` is true when the
/// body template capture was spelled `$$$NAME`, so the binding lands in the
/// multi namespace and never unifies with a same-named `$NAME` capture.
fn body_capture(pattern: &str) -> Option<(&str, bool)> {
    if let Some(open) = pattern.find('{') {
        let close = pattern.rfind('}')?;
        let section = pattern.get(open + 1..close)?.trim();
        let name = capture_name(section)?;
        return Some((name, section.starts_with("$$$")));
    }
    let trimmed = pattern.trim();
    if is_if_prefixed(trimmed) {
        let body = trimmed.rsplit_once(':')?.1.trim();
        let name = capture_name(body)?;
        return Some((name, body.starts_with("$$$")));
    }
    // EXP-003 (H-CONF-008): python-style suite body — `def $A($B): $$$C`.
    // The section after the parameter list is a body template, so a `$$$`
    // capture there must bind for rewrites instead of erroring unbound.
    suite_body_capture(trimmed)
}

/// `(name, multi)` for `$$$NAME` / `$NAME` in the `: body` suite after a
/// declaration's parameter list.
fn suite_body_capture(pattern: &str) -> Option<(&str, bool)> {
    let (declaration, _) = strip_declaration_modifiers(pattern);
    let open = declaration.find('(')?;
    let close = open + declaration[open..].find(')')?;
    let after = declaration[close + 1..].trim();
    let body = after.strip_prefix(':')?.trim();
    let name = capture_name(body)?;
    Some((name, body.starts_with("$$$")))
}

fn if_condition_capture(pattern: &str) -> Option<&str> {
    let rest = pattern.trim().strip_prefix("if")?.trim_start();
    if let Some(inner) = rest.strip_prefix('(') {
        let close = inner.find(')')?;
        capture_name(&inner[..close])
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '{' || c == ':')
            .unwrap_or(rest.len());
        capture_name(&rest[..end])
    }
}

fn strip_container(text: &str) -> &str {
    let trimmed = text.trim();
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        if trimmed.starts_with(open) && trimmed.ends_with(close) {
            return trimmed[open.len_utf8()..trimmed.len() - close.len_utf8()].trim();
        }
    }
    trimmed
}

fn capture_call_path(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    // PASS 65 (F64-5): the receiver-path binding only makes sense for
    // patterns that ARE a call (`callee(args)` with a single argument list).
    // Any other shape — a let/assignment whose value is a member call
    // (`let r2 = q.len();`, `hh = u.len()`), a statement compound — parses a
    // bogus callee out of the first `(` and its veto discarded matches the
    // literal lane had answered correctly while sg answers those faces.
    // Skip the binding for non-call shapes instead of vetoing.
    let simple_call = match (pattern.trim().find('('), pattern.trim().rfind(')')) {
        (Some(open), Some(close)) if close > open => {
            let trimmed = pattern.trim();
            // PASS 73 (F72a-1): normalize `::` like `parse_call_path`, so a
            // php static-call callee (`$A::bar`) decomposes into its segments
            // here too — without this the guard saw one non-ident segment
            // (`$A::bar`) and skipped scope/name capture binding entirely.
            // PASS 75a (F74a-2): the php `->` member connector normalizes the
            // same way (`$O->m` binds the object capture through the faithful
            // member arm).
            trimmed[close + 1..].trim().is_empty()
                && !trimmed[open + 1..close].contains(['(', ')'])
                && trimmed[..open]
                    .replace("::", ".")
                    .replace("->", ".")
                    .split('.')
                    .all(|segment| {
                        let segment = segment.trim();
                        is_pure_metavariable(segment)
                            || is_pattern_ident(segment)
                            || namespace_qualified_segment(segment)
                            || dollar_name_class(
                                segment.strip_prefix('$').unwrap_or(""),
                            ) == Some(DollarTokenClass::LowercaseLed)
                    })
        }
        _ => false,
    };
    if !simple_call {
        return Some(());
    }
    let Some(open) = pattern.find('(') else {
        return Some(());
    };
    let callee = pattern[..open]
        .trim()
        .strip_prefix("::")
        .unwrap_or(pattern[..open].trim())
        .replace("::", ".")
        .replace("->", ".");
    let pattern_segments = callee.split('.').collect::<Vec<_>>();
    // PASS 60 (F58-3): multi-segment callees bind through the FAITHFUL
    // receiver path — when the receiver chain flattens through a call, the
    // segments are not faithful node texts, so refuse to bind (which vetoes
    // the whole candidate) instead of unifying on flattened tails.
    // Single-segment patterns keep the registered flattening (sg binds the
    // bare callee metavariable to the last identifier there).
    //
    // PASS 63 (F62-3): the veto fires ONLY when equality could actually be
    // violated. sg structurally binds a wildcard receiver metavariable to
    // whatever node sits at the receiver position — `arr[0].len()` /
    // `vec![1].len()` / `w.v[1].len()` all answer `$O.$M($$$A)` with
    // O = the receiver's literal text (probed sg 0.45.2, 3/3 hits). The
    // faithful veto kept those faces silent-empty. For non-member receivers
    // this now binds the head metavar to the receiver TEXT and the tail to
    // the final identifier; `bind_capture`'s duplicate-name equality keeps
    // every same-name veto (`$O.$O` on `y.first().first()` still rejects —
    // `y.first()` != `first`), so the F58-3 contract is preserved through
    // unification rather than through an unconditional veto.
    let actual = if pattern_segments.len() >= 2 {
        match call_target_path_faithful(node, source) {
            Some(path) => path,
            None => return bind_nonfaithful_receiver(node, source, &pattern_segments, captures),
        }
    } else {
        match call_target_path(node, source) {
            Some(path) => path,
            None => return Some(()),
        }
    };
    if pattern_segments.len() == actual.len() {
        for (pattern_segment, actual_segment) in pattern_segments.iter().zip(&actual) {
            if let Some(variable) = capture_name(pattern_segment) {
                bind_capture(captures, variable, actual_segment)?;
            }
        }
    } else if pattern_segments.len() == 1 {
        if let (Some(variable), Some(actual_segment)) =
            (capture_name(pattern_segments[0]), actual.last())
        {
            bind_capture(captures, variable, actual_segment)?;
        }
    } else if actual.len() > pattern_segments.len() && capture_name(pattern_segments[0]).is_some()
    {
        // PASS 51 (H-CONF-020/025 boundary, ledger §9 row
        // p48-dollarO-dollarO-py-positive-face-overmatch): a leading
        // metavariable segment absorbs the WHOLE multi-segment receiver —
        // bind it so `bind_capture` enforces duplicate-name equality.
        // `$O.$O($$$A)` on `a.b.c(1)` now binds O=`a.b` then O=`c` and
        // rejects, matching sg (probed 2026-09-04).
        let head_len = actual.len() - pattern_segments.len() + 1;
        let head = actual[..head_len].join(".");
        bind_capture(captures, capture_name(pattern_segments[0]).unwrap_or_default(), &head)?;
        for (pattern_segment, actual_segment) in
            pattern_segments[1..].iter().zip(&actual[head_len..])
        {
            if let Some(variable) = capture_name(pattern_segment) {
                bind_capture(captures, variable, actual_segment)?;
            }
        }
    }
    Some(())
}

/// PASS 63 (F62-3): whole-text receiver binding for wildcard-led two-segment
/// patterns whose callee chain is NOT a plain member chain (subscript
/// receivers `arr[0]`, macro-call receivers `vec![1]`, call receivers
/// `y.first()`). sg binds the receiver metavariable to the receiver node's
/// literal text and the tail metavariable to the final identifier; a
/// duplicate head/tail name (`$O.$O`) then fails `bind_capture` equality,
/// which is exactly how sg rejects the same-name chains. Literal-head
/// patterns and split-callee grammars (java/php `object`+`name` fields,
/// ruby receiver calls) keep the F58-3 veto (`None`).
fn bind_nonfaithful_receiver(
    node: &Node,
    source: &str,
    pattern_segments: &[&str],
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    // PASS 65a (mutation adjudication): the interrupted pass exempted
    // LITERAL heads from this veto for the F64-5 faces, but no pinned face
    // reaches this arm anymore — `capture_call_path`'s `simple_call` guard
    // skips non-call shapes and the `$`-less faces never run captures at
    // all (both mutant runs survived). The F62-3 registered veto semantics
    // are restored verbatim; the F64-5 contract lives in the `simple_call`
    // guard and is killed by `f64_5_literal_lane_answers_member_call_valued_roots`.
    if pattern_segments.len() != 2 {
        return None;
    }
    let field = call_field_node(node)?;
    let (tail_node, tail_text) = chain_tail_identifier(&field, source)?;
    if tail_node.id() == field.id() {
        // A single-identifier callee always has a faithful path; reaching
        // here means the callee shape is not a member chain — keep the veto.
        return None;
    }
    let receiver = source
        .get(field.start_byte()..tail_node.start_byte())?
        .trim_end()
        .trim_end_matches('.')
        .trim_end();
    if receiver.is_empty() {
        return None;
    }
    bind_capture(
        captures,
        capture_name(pattern_segments[0]).unwrap_or_default(),
        receiver,
    )?;
    if let Some(name) = capture_name(pattern_segments[1]) {
        bind_capture(captures, name, &tail_text)?;
    }
    Some(())
}

/// The final identifier of a callee chain node: the node itself when it is
/// an identifier, else the last named child's own tail (member chains end in
/// the property identifier).
fn chain_tail_identifier<'a>(node: &Node<'a>, source: &str) -> Option<(Node<'a>, String)> {
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        let text = node_text(node, source)?.to_string();
        return Some((*node, text));
    }
    if !is_member_expr_kind(node.kind()) {
        return None;
    }
    let mut cursor = node.walk();
    let last = node.named_children(&mut cursor).last()?;
    chain_tail_identifier(&last, source)
}

fn excerpt_for_node(node: &Node, source: &str, pattern: &str) -> String {
    if let Some(text) = node_text(node, source) {
        if text.lines().count() <= 6 {
            return text.to_string();
        }
    }
    // Pass 65a keep-gate: the previous `source.lines().nth(line - 1)` fallback
    // re-scanned the whole source for every >6-line node (quadratic on the
    // universal lane). Backward/forward newline scan around start_byte yields
    // the identical first-line text in O(line) — including `lines()`'s
    // trailing-`\r` strip.
    let start = node.start_byte().min(source.len());
    if start >= source.len() && (source.is_empty() || source.ends_with('\n')) {
        // Zero-width node at EOF: `lines()` has no piece for
        // `byte_to_line(len)` here, and the old `nth(...)` fell back to the
        // pattern. (When the source does not end in `\n`, the last piece is
        // exactly the tail line — the scan below already yields it.)
        return pattern.to_string();
    }
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |i| line_start + i);
    let line = &source[line_start..line_end];
    let line = line.strip_suffix('\r').unwrap_or(line);
    line.to_string()
}


