//! Rust/PHP `use` semi-clause lane.

use super::*;
use crate::extract::node_text;
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

pub(crate) fn walk_use_semi(
    node: Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
    lang: Language,
    out: &mut Vec<PatternMatch>,
) {
    let want_kind = match lang {
        Language::Rust => "use_declaration",
        Language::Php => "namespace_use_declaration",
        _ => return,
    };
    walk_kind(node, want_kind, false, out, |n| {
        use_semi_match(n, source, pattern, template)
    });
}

pub(crate) fn use_semi_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    // The php kind faces — the clause's head token must be the pattern's
    // literal kind keyword; X binds the name (one token: the aliased kind
    // spelling keeps its uncovered route).
    if let DirectiveTemplate::PhpUseKind { kind, target } = template {
        // The kind HEAD matches on the RAW clause text (a comment BEFORE
        // the kind keyword breaks the alignment and refuses); comments
        // AFTER it are trivia — the NAME section scans comment-stripped.
        // The php gap law — after the literal `use` a gap-trivia run is
        // REQUIRED (ASCII ws, NBSP, U+FEFF, U+001A); the kind token ends at
        // the next trivia char, so a Rust-ws outsider (U+2028 et al) glues
        // into the head/name token and refuses exactly where the reference
        // refuses. The name section binds the FIRST token: trivia splits
        // it, glue chars are token chars.
        let raw = node_text(node, source)?;
        let rest = raw.trim().strip_suffix(';')?.strip_prefix("use")?;
        let gap_len: usize = rest
            .chars()
            .take_while(|c| is_sg_php_gap_trivia(*c))
            .map(char::len_utf8)
            .sum();
        if gap_len == 0 {
            return None;
        }
        let after_gap = &rest[gap_len..];
        // NUL ends the head-token scan for Meta targets (the kind clause
        // still aligns); literal targets keep the strict scan.
        let head_end = after_gap
            .find(|c| is_sg_php_gap_trivia(c) || (matches!(target, LaneName::Meta(_)) && c == '\0'))
            .unwrap_or(after_gap.len());
        if after_gap[..head_end] != kind[..] {
            return None;
        }
        // The kind-clause NAME gap admits NUL for Meta targets (capture
        // stripped); literal targets keep the strict class.
        let name_gap =
            |c: char| is_sg_php_gap_trivia(c) || (matches!(target, LaneName::Meta(_)) && c == '\0');
        let name = strip_comment_spans(&after_gap[head_end..])
            .trim_start_matches(name_gap)
            .split(name_gap)
            .next()
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            return None;
        }
        let mut captures = BTreeMap::new();
        captures.insert("MATCH".to_string(), raw.to_string());
        target.bind_or_match(&mut captures, &name)?;
        return Some(hit_for_node(node, source, pattern, captures));
    }
    let DirectiveTemplate::Semi {
        head: "use",
        target,
        alias,
        ..
    } = template
    else {
        return None;
    };
    let text = node_text(node, source)?;
    // The php KIND-LESS `use $X;` clause head obeys the same php gap law as
    // the kind lanes — after the literal `use` an is_sg_php_gap_trivia run
    // is REQUIRED (FEFF/A0/001A bind); a glue char (U+2028) means zero gap
    // → the junk glued into the name token and the reference refuses. Rust
    // `use` keeps the literal-space spelling. The use-head gap law admits
    // U+0000 as a gap unit at the META face (`use<NUL>Foo;` binds X=Foo,
    // capture stripped) while the LITERAL pattern keeps refusing the NUL
    // candidate — the admission is target-scoped.
    let meta_target = matches!(target, LaneName::Meta(_));
    let php_clause = node.kind() == "namespace_use_declaration";
    let rest = if php_clause {
        let after_use = text.trim().strip_prefix("use")?;
        let gap_len: usize = after_use
            .chars()
            .take_while(|c| is_sg_php_gap_trivia(*c) || (meta_target && *c == '\0'))
            .map(char::len_utf8)
            .sum();
        if gap_len == 0 {
            return None;
        }
        &after_use[gap_len..]
    } else {
        text.trim().strip_prefix("use ")?
    };
    let inner = rest.strip_suffix(';')?;
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), text.to_string());
    if let Some((_, y)) = alias {
        let (left, right) = split_top_level_keyword(inner, " as ")?;
        target.bind_or_match(&mut captures, left.trim())?;
        y.bind(&mut captures, right.trim())?;
    } else {
        // The reference refuses a kind-less META `use $X;` against a php
        // GROUP-use candidate — the meta capture never spans a php brace
        // group — while a literal group-use PATTERN keeps binding its
        // byte-equal candidate and plain candidates keep binding. The rust
        // brace-group face is untouched: the guard is php-clause-scoped.
        if php_clause && inner.contains('{') && matches!(target, LaneName::Meta(_)) {
            return None;
        }
        // The plain shape binds the WHOLE clause text (brace groups,
        // `function foo` kind keywords included: E_rs_use_braces,
        // R3_php_use_function).
        target.bind_or_match(&mut captures, inner.trim())?;
    }
    Some(hit_for_node(node, source, pattern, captures))
}
