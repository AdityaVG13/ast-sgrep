//! Top-level match dispatch over literal and structural lanes.

use super::*;
use crate::Language;
use std::borrow::Cow;

/// Unified entry: literal identifier match, or native structural match for `$` patterns.
pub fn match_pattern(
    lang: Language,
    source: &str,
    pattern: &str,
) -> anyhow::Result<Vec<PatternMatch>> {
    // A BOM-led pattern is stripped like the reference strips it; the raw
    // U+FEFF otherwise degrades every lane to a silent empty.
    let pattern = pattern.trim().trim_start_matches('\u{feff}').trim();
    // Normalize expando META spelling (`µA`/`𐐀A` runs) to the `$` spelling
    // the reference's own `pre_process_pattern` produces, so every lane
    // below sees the metavariable structure the reference's matcher extracts.
    let pattern: Cow<'_, str> = normalize_expando_meta_spelling(lang, pattern);
    let pattern = pattern.as_ref();
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    // The C/C++ conditional-compilation directives answer kind-level on the
    // directive head — a dedicated lane ahead of the general/literal routing,
    // which cannot spell compound-region roots. Non-directive patterns fall
    // through untouched.
    if matches!(lang, Language::C | Language::Cpp) {
        if let Some(hits) = match_preproc_directive(lang, source, pattern) {
            return Ok(hits);
        }
    }
    // The directive/import roots, go's `$`-carrying `;`-ful accepted-empty
    // spellings, the csharp checked/unchecked EXPRESSION root, and the
    // remaining statement roots — spelling-level lanes ahead of the
    // statement/literal routing (`using $N;` must precede the csharp
    // statement lane, and the literal `using System;` must precede its
    // over-serving literal route).
    if let Some(hits) = match_directive_root(lang, source, pattern) {
        return Ok(hits);
    }
    if lang == Language::Go && (sg_goto_semi_pattern(pattern) || sg_go_import_semi_pattern(pattern))
    {
        return Ok(Vec::new());
    }
    if lang == Language::CSharp {
        if let Some(hits) = match_csharp_expression_root(source, pattern) {
            return Ok(hits);
        }
    }
    if let Some(hits) = match_statement_root_140(lang, source, pattern) {
        return Ok(hits);
    }
    // The statement/decl root lanes (rs mod/extern crate, go type, rb module,
    // ts declare module/const, py async def, ja enum, cs record/struct) and
    // the directive-family accepted-empty faces — the empty walk IS the
    // reference agreement.
    if directive_pattern_sg_accepts_empty(lang, pattern)
        || (lang == Language::CSharp && cs_using_plainface_meta_path(pattern))
    {
        return Ok(Vec::new());
    }
    if let Some(hits) = match_statement_root_142(lang, source, pattern) {
        return Ok(hits);
    }
    // The csharp statement-head lane — fixed/checked/unchecked/unsafe
    // templates the general lane cannot root or align (see the lane's header
    // block). Placed ahead of the `$`-less literal routing because the
    // fully-concrete spellings carry no metavariable.
    if lang == Language::CSharp {
        if let Some(hits) = match_csharp_statement(source, pattern) {
            return Ok(hits);
        }
    }
    // The java synchronized META-body lane — the bare-meta body cannot
    // substitute through the general template (a placeholder alone inside
    // a block is a parse ERROR) and starved census-loud where the
    // reference binds the block statement. Concrete bodies fall through
    // (parse refuses them) to their general-lane route. The java dispatch:
    // the synchronized meta-body block face and the class member-count
    // face the reference binds.
    if lang == Language::Java {
        if let Some(hits) = match_java_synchronized_meta(source, pattern) {
            return Ok(hits);
        }
        // The synchronized NESTED-block and METHOD faces — the bare-meta lane
        // keeps the flat faces; these walks bind the outermost statement
        // exactly (modifier hop-scan with annotation blocking, the
        // one-statement body law at every slot).
        if let Some(hits) = match_java_sync_block_nested(source, pattern) {
            return Ok(hits);
        }
        if let Some(hits) = match_java_sync_method(source, pattern) {
            return Ok(hits);
        }
        // The java class member-count face — the reference binds the
        // single-member class and refuses empty/multi-member/heritage
        // candidates.
        if let Some(kind) = classify_java_class_member_count(pattern) {
            return match_structural(lang, source, pattern, &kind);
        }
    }
    // The php braced-namespace block face (`namespace $N { $B }` / global
    // `namespace { $B }`).
    if lang == Language::Php {
        if let Some(hits) = match_php_namespace_block(source, pattern) {
            return Ok(hits);
        }
    }
    // The accepted-empty kt/swift siblings — the walk's empty IS the
    // agreement.
    if lang == Language::Kotlin && kt_binds_nothing_template(pattern) {
        return Ok(Vec::new());
    }
    if lang == Language::Swift && swift_binds_nothing_template(pattern) {
        return Ok(Vec::new());
    }
    if !pattern.contains('$') {
        // The `;`-ful spelling is a DIFFERENT face on the no-semicolon
        // grammars — the reference accepts the pattern (single statement
        // root, the `;` an anonymous tail) but binds NOTHING on any go/rb
        // candidate, while the bare keyword kind template answers (the kind
        // lane just below). The empty list IS the agreement (valid-empty).
        if sg_semi_keyword_accepted_empty(lang, pattern) {
            return Ok(Vec::new());
        }
        // The cs `throw;` family — the CANDIDATE-side operand gate: the
        // rethrow binds, operand-bearing throw statements refuse. The bare
        // `throw` spelling falls through to the kind lane below. The
        // pattern-side trivia class splits the family — non-class
        // Rust-whitespace interiors (U+2028 et al) are ACCEPTED-EMPTY; the
        // walk's empty is the agreement.
        if lang == Language::CSharp && cs_throw_semi_pattern(pattern) {
            if matches!(
                cs_throw_semi_parse(pattern),
                Some(CsThrowSemiParse::AcceptedEmpty)
            ) {
                return Ok(Vec::new());
            }
            if let Some(hits) = match_cs_bare_semi(source, pattern, "throw_statement") {
                return Ok(hits);
            }
        }
        // The cs bare-return semi family — the operand-less walk with the
        // junk-gap exemption + comment transparency (see
        // [`match_cs_bare_semi`]). The general lane's child alignment
        // refused every junk-carrying candidate silently.
        if lang == Language::CSharp && pattern.trim() == "return;" {
            if let Some(hits) = match_cs_bare_semi(source, pattern, "return_statement") {
                return Ok(hits);
            }
        }
        // A bare statement head (`break`, `break;`) is a KIND template —
        // the reference answers every statement of the head's family
        // regardless of arguments or the trailing semicolon. The family
        // predates the general lane's childless-template face, which only
        // ever matched the exact childless form.
        if let Some(hits) = match_bare_statement_kind(lang, source, pattern) {
            return Ok(hits);
        }
        // A bare statement keyword (`break`, `continue`) is a statement
        // TEMPLATE, not an identifier literal — the reference matches the
        // statement node (the literal lane only ever matches identifier /
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
        // A bare connector token is the reference's lenient-parse garbage
        // class — the reference parses `->`/`::` and answers NOTHING, while
        // the literal lane matched the anonymous token at every AST connector
        // site (silent fail-open). Never answer; every other connector
        // spelling keeps its own behavior (`=>` answers array-pair sites,
        // doubled spellings `->>` match nothing in the literal lane
        // naturally).
        if matches!(pattern, "->" | "::") {
            return Ok(Vec::new());
        }
        // A bare `&&` or `.` is php garbage that answers NOTHING — the
        // reference parses `&&` leniently with an ERROR-node warning and
        // refuses `.`, while the literal lane matched the anonymous token at
        // every `&&`/concat site (silent fail-open). PHP-scoped on purpose:
        // the reference ANSWERS bare `.` member/attr sites in js/py/rust/bash
        // and bare `&&` sites in js/bash, so a cross-language carve would
        // over-refuse. `||`/`+`/`=>` keep their own answering classes;
        // `)`/`...`/`???` agree-empty in the literal lane.
        if matches!(lang, Language::Php) && matches!(pattern, "&&" | ".") {
            return Ok(Vec::new());
        }
        // A csharp `lock`/`using` statement pattern is a statement ROOT the
        // reference matches layout-insensitively — the literal lane's byte
        // equality answered only the identical single-line spelling and
        // silently dropped the multi-line bodies the reference binds. Route
        // to the general lane when the template builds (the root kinds are
        // admitted in `is_general_root_kind`); anything unbuildable keeps
        // the literal lane's faces (identifier spellings of the same heads
        // included).
        if lang == Language::CSharp
            && pattern.contains('(')
            && pattern
                .split(|c: char| c.is_ascii_whitespace() || c == '(')
                .next()
                .is_some_and(|head| matches!(head, "lock" | "using"))
            && cached_general_template(lang, pattern).is_some()
        {
            return Ok(match_structural_general(lang, source, pattern));
        }
        // A fully-concrete del TEMPLATE spelling rides the dedicated lane's
        // structural unify — the reference binds `del (x)` × `del (x)` (and
        // byte-refuses `del (y)`) where the literal route's leaf byte-match
        // answered silent 0. Only del-template spellings reroute; every
        // other `$`-less pattern keeps the literal lane.
        if lang == Language::Python && py_delete_template(pattern).is_some() {
            if let Some(hits) = match_py_delete_meta(source, pattern) {
                return Ok(hits);
            }
        }
        return match_literal_pattern(lang, source, pattern);
    }
    // A php STATIC-scope target (`C::$s = 5`, `C::$s = f(5)`, `C::$s == 5`)
    // whose `$`-tokens are all lowercase-literal rides dollar_literal_lane's
    // literal lane, whose exact-text arm answers only the byte-identical
    // spelling — the reference answers the padded candidate structurally
    // (`C :: $s = 5`, `C :: $s == 5`). Admit the assignment hook AHEAD of
    // the non-canonical gate for the static-scope family; the hook's own
    // static_target operator discipline (Assign|Augmented|Binary) stays the
    // exactness boundary, and non-static lowercase faces (`$alpha = $beta`)
    // keep the literal lane.
    if lang == Language::Php
        && split_php_binary(pattern).is_some_and(|(lhs, op, _)| {
            is_php_static_scope_target(lhs)
                || (op == "=" && php_static_target_dynamic_head(lhs).is_some())
        })
    {
        if let Some(kind) = classify_php_assignment(pattern) {
            return match_structural(lang, source, pattern, &kind);
        }
    }
    // In languages where `$` is name syntax, the reference parses
    // non-canonical `$`-tokens as literal code and answers the literal faces;
    // answer exactly through the existing literal lane (exact-text + R3
    // structural arms) instead of the silent NeverMatches empty.
    if dollar_literal_lane(lang, pattern) {
        return match_literal_pattern(lang, source, pattern);
    }
    // In the no-expando languages (js/ts/java) a pattern whose `$`-carrying
    // identifier tokens ALL fail the whole-token meta validation is pure
    // literal code — the reference parses the glued tokens (`µµµ$A`,
    // `$AµµB`, `$A$$B`, `foo$$$A`) as ordinary identifiers and answers the
    // verbatim rows. The canonical-spelling recognition inside those tokens
    // (`$A` inside `µµµ$A`) otherwise hijacked the pattern into the general
    // lane, whose leaf substitution (`µµµ__asgrep_mv_A`) can never equal
    // the candidate identifier text — the silent under-answer. The literal
    // lane's exact-text + R3 structural arms answer exactly the reference's
    // rows; existing non-canonical lanes above keep their faces (checked
    // first).
    if sg_expando_char(lang).is_none() && pattern_tokens_are_all_literal(pattern) {
        return match_literal_pattern(lang, source, pattern);
    }
    // The reference binds `del $X` to the WHOLE operand-list text
    // (`del x, y` → X=`x, y`), but the general lane's placeholder unifies
    // INSIDE the candidate's `expression_list`, so a multi-operand list can
    // never match (child-count mismatch) — the silent under-answer. A
    // python-scoped structural walk binds the list node text exactly; every
    // other `del` spelling keeps its routes.
    if lang == Language::Python {
        if let Some(hits) = match_py_delete_meta(source, pattern) {
            return Ok(hits);
        }
        // A `;`-ful del spelling whose `;`-stripped form IS a del template
        // (`del $O.$A;` and the whole family — accepted-empty for
        // `del $X;` / `del ($X);` / `del $A, $B;` / `del $O.$A;`) answers
        // the honest empty, never the loud class. (Single-language runs
        // refuse the same spellings as a multi-node parse — a registered
        // mode divergence; the multi-language run is the CLI's parity mode.)
        let del_trimmed = pattern.trim();
        if del_trimmed.starts_with("del") && del_trimmed.ends_with(';') {
            if let Some(stripped) = del_trimmed.strip_suffix(';') {
                if py_delete_template(stripped.trim()).is_some() {
                    return Ok(Vec::new());
                }
            }
        }
    }
    // `return ($X)` / `return($X)` are NOT call faces — the reference
    // answers them on `return (1);` binding X=`1` (the parenthesized
    // operand's INNER text), while the Call classification hijacked the
    // pattern into a callee-text match that can never fire. js/ts only.
    if matches!(lang, Language::JavaScript | Language::TypeScript) {
        if let Some(hits) = match_return_paren_meta(lang, source, pattern) {
            return Ok(hits);
        }
    }
    // Php-only admission ahead of the non-canonical gate — a lowercase-led
    // LHS with a bare canonical-meta RHS answers through the dedicated
    // assignment lane (the gate's NeverMatches class would silent-empty the
    // answering face).
    if matches!(lang, Language::Php) {
        if let Some(kind) = classify_php_assignment(pattern) {
            return match_structural(lang, source, pattern, &kind);
        }
        // A bare php binary-expression META template (no `=` root, no
        // literal LHS) — classified faces walk the operand lane.
        if let Some(tpl) = classify_php_operand_template(pattern) {
            let tree = parse_source(lang, source)?;
            let mut out = Vec::new();
            walk_php_operand_template(&tree, source, pattern, &tpl, &mut out);
            return Ok(out);
        }
        // A pattern ending in a DANGLING plain arrow is the lenient ERROR
        // repair to the FLAT member-call prefix (`ERROR(member_call_expression)`,
        // and the reference answers exactly the flat prefix sites). Strip the
        // arrow and serve the flat face —
        // restricted to chain-PREFIX candidates, since the repaired pattern
        // never answers the standalone statement spelling. A nullsafe
        // spelling anywhere in the remainder keeps the refusal (the
        // reference answers the dangling-`?->` faces []), and a
        // chain/property-tail remainder keeps its registered refusal (the
        // chain-dangling answer rides ERROR-object alignment — no sound
        // structural rule reproduces it).
        if let Some(stripped) = pattern.strip_suffix("->") {
            let stripped = stripped.trim();
            if !stripped.is_empty() && !stripped.contains('?') && stripped.ends_with(')') {
                if let Some(NativeKind::MemberCall {
                    path, arg_slots, ..
                }) = classify_native(stripped)
                {
                    let kind = NativeKind::MemberCall {
                        path,
                        arg_slots,
                        require_continuation: true,
                        nullsafe: false,
                    };
                    return match_structural(lang, source, pattern, &kind);
                }
            }
        }
    }
    match classify_native(pattern) {
        Some(kind) => match_structural(lang, source, pattern, &kind),
        // Classifier-rejected shapes get the general structural lane
        // (single-expression / function-declaration templates with
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
    walk_literal(
        tree.root_node(),
        source,
        pattern,
        template.as_deref(),
        &mut matches,
    );
    // The literal lane is the `$`-less member spelling path (kt `a.b(1)`,
    // swift member faces) — apply the same per-candidate link-trivia
    // doctrine the general lane consults.
    retain_member_link_trivia_free(lang, &tree, &mut matches);
    Ok(matches)
}

/// The swift/kt per-candidate member-link trivia retain shared by the
/// general structural lane and the literal lane — drops candidates whose OWN
/// subtree carries link-STRUCTURAL trivia (receiver/mid link trivia before
/// a later connector) where the reference refuses, keeps trivia-free
/// candidates (the inner link of a longer chain) and candidates whose trivia
/// sits outside their span (tail comments).
pub(crate) fn retain_member_link_trivia_free(
    lang: Language,
    tree: &tree_sitter::Tree,
    out: &mut Vec<PatternMatch>,
) {
    if matches!(lang, Language::Swift | Language::Kotlin) && !out.is_empty() {
        let root = tree.root_node();
        out.retain(|m| {
            node_with_span(root, m.byte_start, m.byte_end)
                .map_or(true, |candidate| !member_link_trivia_structural(&candidate))
        });
    }
}
