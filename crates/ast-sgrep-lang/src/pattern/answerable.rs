//! Language-aware answerability and parse-gate checks.

use super::*;
use crate::extract::{is_ident_kind, is_member_expr_kind};
use crate::Language;
use tree_sitter::Node;

/// Language-aware answerability of a classifier-accepted shape. Decl/call
/// shapes must parse under `lang`'s grammar through the reference's
/// pattern-acceptance gate; the registered cross-language kinds (If
/// normalization, `$$A` universal, NeverMatches accepted-empty) stay
/// answerable everywhere, and shapes whose substitution refuses (bare `$$$`
/// rest args) are not probed.
pub(crate) fn native_kind_language_answerable(
    lang: Language,
    pattern: &str,
    kind: &NativeKind,
) -> bool {
    // A concrete-condition if face is answerable exactly in the covered
    // grammars AND when the cond template actually builds under that
    // grammar — uncovered grammars and unbuildable conds keep the census
    // loud (fail-closed). The meta-cond spelling keeps its registered
    // unconditional answerability below.
    match kind {
        NativeKind::If {
            cond: Some(cond), ..
        } => {
            matches!(
                lang,
                Language::JavaScript
                    | Language::TypeScript
                    | Language::Python
                    | Language::Go
                    | Language::C
                    | Language::Php
                    | Language::Ruby
                    | Language::Java
                    | Language::Rust
            ) && cached_if_cond_template(lang, cond).is_some()
        }
        NativeKind::If { .. } | NativeKind::Universal { .. } => true,
        // A NeverMatches face is answerable exactly when the reference's
        // own pattern-acceptance gate accepts the spelling — the parse
        // decides root multiplicity ("Multiple AST nodes" for the compound
        // spellings), replacing the textual `) {` scan whose 7-language
        // scope over-refused the accepted-empty IIFE / `$f($A) {` cases and
        // under-refused the rb/swift/kt/java compounds. The registered
        // silent NeverMatches cases (`$a = 1`, rust/js `$x # note`, php
        // `$A && $b`) parse to single (ERROR/text) roots and keep their
        // answerability.
        NativeKind::NeverMatches => sg_pattern_gate_accepts(lang, pattern),
        // An `interface` member-count template is answerable ONLY in the
        // receipted grammars (ts/java bind; the rest refuse or are
        // unreceipted). Csharp joins the receipted set — 1-member plain
        // binds, 2-member/extends/base-list/modifier/gap-trivia refuse —
        // and the ts `type-alias` object face is receipted for TypeScript
        // only. Other Class faces keep the pattern-parse gate byte-for-byte.
        NativeKind::Class {
            keyword,
            body: Some(BodyTemplate::Exactly(_)),
            ..
        } if *keyword == "interface" => {
            matches!(
                lang,
                Language::TypeScript | Language::Java | Language::CSharp
            )
        }
        NativeKind::Class {
            keyword,
            body: Some(BodyTemplate::Exactly(_)),
            ..
        } if *keyword == "type-alias" => matches!(lang, Language::TypeScript),
        NativeKind::Function { .. } | NativeKind::Class { .. } | NativeKind::Call { .. } => {
            // The `async def` `$$`-slot face is served by the dedicated
            // async-def lane — the general Function route's parse gate
            // refuses the multi-line spelling, which starved census-loud
            // where the reference binds.
            if lang == Language::Python && py_async_def_template(pattern).is_some() {
                return true;
            }
            sg_pattern_parses(lang, pattern)
        }
        // The `->` member-call spelling adopts the same pattern-acceptance
        // gate (php parses `µsvc->run(µA)` as a member call; grammars
        // without `->` member syntax refuse it).
        NativeKind::MemberCall { .. } => sg_pattern_parses(lang, pattern),
        // The `->` member-call CHAIN adopts the same gate — php parses the
        // expando-substituted chain; grammars without `->` member syntax
        // refuse.
        NativeKind::MemberCallChain { .. } => sg_pattern_parses(lang, pattern),
        // The chain lane adopts the same gate — the expando-substituted
        // document must parse to a single node under this language's grammar.
        NativeKind::CallChain { .. } => sg_pattern_parses(lang, pattern),
        // The optional chain additionally requires the parse to accept the
        // spelling, and the language to be in the optional-connector
        // registry — the grammars where a depth-0 `?.` IS a member
        // connector and the matcher's leaf decomposition reproduces it
        // (php's `?->` chains carry no dots and never classify here).
        // kotlin/swift wrap the connector in navigation-suffix shapes the
        // leaf checks refuse, csharp's callee never presents a
        // member-expression node, rust's `?` folds into a try-expression
        // head, and python has no `?.` spelling — those keep the
        // fail-closed refusal. Outside the registered family, languages
        // keep the general lane when it can build the template; all-call
        // chains keep the historical gate.
        NativeKind::OptionalCallChain { segments, .. } => {
            // A mixed-rest slot list is a CALL segment, not a property face
            // — keep the predicate in step with the walker's `segment_is_call`.
            let property_face = segments
                .iter()
                .skip(1)
                .any(|segment| segment.args.is_none() && segment.arg_slots.is_none());
            sg_pattern_parses(lang, pattern)
                && if property_face {
                    matches!(lang, Language::TypeScript | Language::JavaScript)
                        || (cached_general_template(lang, pattern).is_some()
                            && sg_pattern_gate_accepts(lang, pattern))
                } else {
                    matches!(lang, Language::TypeScript | Language::JavaScript)
                        // Kotlin all-call `?.` chains: the walker's
                        // decompose/receiver machinery already pairs
                        // pattern-CALL segments with candidate CALL nodes —
                        // kotlin-ng nests mid-chain calls as real
                        // `call_expression` children of
                        // `navigation_expression`, and the connectors present
                        // as plain `.`/`?.` anonymous tokens — so the census
                        // loud is pure admission; the walk answers exactly.
                        // Swift stays REFUSED: its `?`-connector +
                        // `navigation_suffix` link shape is outside the
                        // decomposition, so admission would walk silent-empty
                        // where the reference answers (fail-open).
                        || lang == Language::Kotlin
                }
        }
        // The optional lane additionally requires the parse of the template
        // to carry the `?.` as the callee's CONNECTOR token — the exact shape
        // the matcher decomposes. Grammars where `?` means something else
        // (rust folds it into a try-expression head) or that cannot parse the
        // spelling (python) keep the fail-closed refusal; both engines share
        // the grammars, so this answers identically to the reference's
        // per-language behavior by construction.
        NativeKind::OptionalCall { .. } => {
            sg_pattern_parses(lang, pattern) && sg_optional_connector_spelling(lang, pattern)
        }
        // The assignment kind is produced only by the php-only match_pattern
        // hook — [`classify_native`] never returns it (these faces keep the
        // gate's NeverMatches class there) — so this arm is unreachable from
        // the answerable gate; the php grammar parses the spelling, so keep
        // the same acceptance gate for safety.
        NativeKind::Assignment { .. } => sg_pattern_parses(lang, pattern),
    }
}

/// True when the parse of the expando-substituted optional template puts
/// the `?.` as an ANONYMOUS TOKEN CHILD of the callee member node — the
/// connector position the optional lane matches on. ts/js spell it so and
/// pass, and kotlin's `navigation_suffix` carries the same clean
/// receiver+`?.`+identifier triple so it passes too. rust's `?` lands
/// inside a try-expression head (no optional link), python has no `?.`
/// spelling, swift wraps the tail in navigation-suffix shapes the leaf
/// check refuses, and csharp's callee never presents a member-expression
/// node here — those all stay refused. php passes through the split-field
/// `nullsafe_member_call_expression` kind arm below; 3+-segment optional
/// chains go through the per-connector-flag OptionalCallChain gate instead.
pub(crate) fn sg_optional_connector_spelling(lang: Language, pattern: &str) -> bool {
    let doc: String = pattern
        .chars()
        .map(|c| if c == '$' { SG_EXPANDO_CHAR } else { c })
        .collect();
    // Php patterns are pre-processed behind a `<?php ` tag
    // (`Language::pre_process_pattern`) — without it the whole doc parses to
    // a bare `text` node and no call shape is reachable. The leading
    // `php_tag` node is pattern-irrelevant, so the descent starts at the
    // first child after it.
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
    // Php's split-field member call carries the `?->` connector as its own
    // named kind — `nullsafe_member_call_expression` (plain `->` stays
    // `member_call_expression`), so the kind is the token-exact
    // discriminator. The name leaf must be a plain identifier — the
    // matcher's decompose contract.
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
    let Some(link) = member_link_parts(lang, &field) else {
        return false;
    };
    link.optional && is_ident_kind(link.leaf.kind())
}

/// One member-link's parts classified from a member-expression node's
/// children: a receiver base, ONE connector, and an identifier leaf, in
/// that child order. The `?.` connector appears either as an anonymous
/// `?.` token or wrapped in a named `optional_chain` node
/// (tree-sitter-typescript, whose inner anonymous token must be `?.`) —
/// NOTE: csharp grammars never reach this decomposition (the
/// `invocation_expression` callee lookup refuses first, keeping csharp
/// fail-closed); `.` is always an anonymous token. Any other anonymous
/// token or extra segment vetoes the link.
pub(crate) struct MemberLink<'a> {
    pub(crate) base: Node<'a>,
    pub(crate) optional: bool,
    pub(crate) leaf: Node<'a>,
}

pub(crate) fn member_link_parts<'a>(lang: Language, node: &Node<'a>) -> Option<MemberLink<'a>> {
    let mut cursor = node.walk();
    let children: Vec<Node<'a>> = node.children(&mut cursor).collect();
    let mut base: Option<Node<'a>> = None;
    let mut leaf: Option<Node<'a>> = None;
    let mut optional: Option<bool> = None;
    // `?.`-chain decomposition is comment-transparent — the chain twin of the
    // faithful path's skip. Without it the trivia child occupied the
    // base/leaf slot and the two-slot veto silently refused. ONE
    // grammar-position veto is kept: trivia PRECEDING the NAMED
    // `optional_chain` wrapper refuses the link — but only for TypeScript:
    // `.ts` runs the typescript grammar (named wrapper; the commented link
    // refuses) while `.js` runs the javascript grammar where `?.` is an
    // ANONYMOUS token and the link answers. This workspace parses BOTH js
    // and ts with wrapping grammars (js maps to TSX), so the named-child
    // test alone cannot separate the faces and the veto must be
    // language-scoped. The veto cannot flip the wrapper's own inner-token
    // check below: the trivia is skipped, never a slot.
    let veto_trivia_before_wrapper = lang == Language::TypeScript;
    for (index, child) in children.iter().enumerate() {
        if is_trivia_kind(child.kind()) || child.is_extra() {
            if veto_trivia_before_wrapper && optional.is_none() {
                let next = children[index + 1..]
                    .iter()
                    .find(|next| !is_trivia_kind(next.kind()) && !next.is_extra());
                // The veto covers BOTH spellings of the receiver→`?.`
                // connector — the named `optional_chain` wrapper AND the
                // anonymous `?.` token the workspace's grammar emits (the
                // pre-fix wrapper-only test never fired on `a /* c */?. b`,
                // which the reference's tree-sitter-typescript refuses).
                // Comment AFTER the connector keeps binding (`optional` is
                // set by then) and the js twin never vetoes.
                if next.is_some_and(|next| {
                    (next.is_named() && next.kind().contains("optional"))
                        || (!next.is_named() && next.kind() == "?.")
                }) {
                    return None;
                }
            }
            continue;
        }
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
            let slot = if optional.is_none() {
                &mut base
            } else {
                &mut leaf
            };
            if slot.replace(*child).is_some() {
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

/// Replication of the reference pattern-acceptance gate: every `$` is
/// rewritten to the expando char, the document is parsed under `lang`'s
/// grammar, and the root must be a "single node" — exactly one child, or
/// two where the second is a missing/empty token (the golang trailing-node
/// quirk) — descended through the single-child chain. NO final-node ERROR
/// rejection: single-child ERROR-wrapped parses are accepted with a
/// warning, so the only rejections are no-content and multiple-node.
/// Both engines share the same tree-sitter grammars, so this gate answers
/// identically to the reference per-language pattern acceptance.
pub(crate) fn sg_pattern_parses(lang: Language, pattern: &str) -> bool {
    let doc: String = pattern
        .chars()
        .map(|c| if c == '$' { SG_EXPANDO_CHAR } else { c })
        .collect();
    let Ok(tree) = parse_source(lang, &doc) else {
        return false;
    };
    let mut node = tree.root_node();
    if node.child_count() == 0 {
        // Reference PatternError::NoContent.
        return false;
    }
    if !is_sg_single_node(node) {
        // Reference PatternError::MultipleNode ("Multiple AST nodes are detected").
        return false;
    }
    while is_sg_single_node(node) {
        node = node.child(0).expect("single node has a child");
    }
    true
}

/// The reference's metavariable expando char (`$` is rewritten to it before
/// the pattern is parsed; it is a valid identifier char in every indexed
/// grammar, which is why the reference picked it).
pub(crate) const SG_EXPANDO_CHAR: char = 'µ';

/// Reference `is_single_node`: one child, or two where the second is a
/// missing/empty token (some grammars emit a spurious empty node at the end).
pub(crate) fn is_sg_single_node(node: Node) -> bool {
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
        Language::Dart => tree_sitter_dart::LANGUAGE.into(),
        Language::MoonBit => tree_sitter_moonbit::LANGUAGE.into(),
    }
}
