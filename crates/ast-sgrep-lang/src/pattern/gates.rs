//! Entry predicates and semicolon/trivia acceptance gates.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::borrow::Cow;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// True when the pattern needs external ast-grep (we cannot handle it natively).
///
/// Patterns without `$` always run in-process. Patterns with `$`/`$$$` use the
/// native structural matcher when they fit a known shape; anything the
/// classifier rejects needs the (bench-only, never-delegated) external engine.
///
/// Classification rejection is always loud, matching codemod's ingress rule
/// (`plan_codemod` bails on any rejected `$`-pattern), so rejected
/// `$`-patterns fail closed instead of degrading to silent empty results.
///
/// A rejected `$`-pattern is still served natively when the general
/// structural lane below can build a template for it; everything else keeps
/// the loud fail-closed contract.
pub fn needs_ast_grep_fallback(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() || !p.contains('$') {
        return false;
    }
    if classify_native(p).is_some() {
        return false;
    }
    if general_lane_supported(p) {
        return false;
    }
    // Paren-free bracket fragments the reference accepts-EMPTY ($A ] / $A )
    // / $A } — single ERROR root) must reach the walk instead of
    // ingress refusal: the walk's empty IS the agreement. The language-free gate
    // admits the class when ANY language's bare gate accepts the spelling;
    // the language-aware census keeps the rejected spellings census-loud
    // via the backstop.
    // A pattern that is whole-token LITERAL in some no-expando language
    // (every `$`-carrying token fails whole-token meta validation) and
    // ACCEPTED there is an ordinary code face, not a priori unanswerable:
    // the glued bare tokens must reach the per-language census and the
    // literal-lane walk instead of the query-level refusal. Expando languages
    // decide per-file through their own census arms, and rejected spellings
    // never pass the per-language parse gate, so their query-level loud
    // contract is intact.
    if pattern_tokens_are_all_literal(p)
        && Language::all()
            .iter()
            .any(|&lang| sg_expando_char(lang).is_none() && sg_pattern_parses(lang, p))
    {
        return false;
    }
    // The kt `companion object { $B }` / `init { $B }` and swift
    // `deinit { $B }` accepted-empty spellings must reach the per-language
    // census and the walk's empty instead of the query-level structural
    // fallback. The union stays language-free; non-kt/swift languages keep
    // their census-loud class.
    if kt_binds_nothing_template(p) || swift_binds_nothing_template(p) {
        return false;
    }
    // The statement/decl-root lanes and the directive-family accepted-empty /
    // dotted-meta faces must reach the per-language census and their walks
    // instead of the query-level fallback. The union stays language-free.
    if statement_root_142_any_language(p) || directive_accepted_empty_any_language(p) {
        return false;
    }
    // The go semi-less `goto $L` spelling BINDS per site — reach the walk
    // instead of the query-level structural fallback. The union stays
    // language-free: the spelling parse is the admission and each language's
    // own census keeps its reference class for it.
    if go_goto_bare_template(p).is_some() {
        return false;
    }
    !Language::all()
        .iter()
        .any(|&lang| sg_bracket_fragment_accepted_empty(lang, p))
}

/// `$`-less patterns that are bare KEYWORD LITERALS — tokens every indexed
/// grammar parses as a dedicated leaf node kind (`null`/`true`/`false`,
/// python `None`/`True`/`False`, js/ts `this`/`super`) rather than as
/// identifier rows. The index stores no rows for keyword nodes, so the
/// ident-served lane would compose a silent empty where the leaf node
/// answers. The core ingress consults this predicate to route the class to
/// the native walk, which answers exactly. NOT in the class:
/// `undefined`/`self` (plain identifier nodes, index-serve correct).
pub fn pattern_is_keyword_literal_root(pattern: &str) -> bool {
    matches!(
        pattern.trim(),
        "null" | "true" | "false" | "True" | "False" | "None" | "this" | "super"
    )
}

/// True when the native engine can answer `pattern` for files of `lang` at
/// all — classifier-accepted shapes, `$`-less shapes, or a general-lane
/// template buildable IN THIS LANGUAGE. The language-free
/// `needs_ast_grep_fallback` stays permissive for ingress, so the per-file
/// unanswerable signal comes from here: core's walk uses it to separate
/// "pattern unanswerable for this file's language" (loud when the whole
/// query answers empty) from per-file source-parse robustness (silent skip).
///
/// The classifier-accepted arm is language-aware: the substituted pattern
/// must parse under THIS language's grammar (see [`sg_pattern_parses`]).
/// `#` counts as comment syntax only where it IS comment syntax
/// (python/ruby/php); rust attributes, C preprocessor lines, swift
/// `#selector`, and js private fields stay answerable.
pub fn native_pattern_answerable(lang: Language, pattern: &str) -> bool {
    // The directive-family accepted-empty and dotted-meta faces are
    // census-answerable REGARDLESS of the classify dispatch — the class is
    // defined by the reference's own acceptance, not by our classify. This
    // consult reads the RAW `$` spelling (the lanes parse canonically); the
    // expando normalization below must not rename the metas out from under
    // the templates.
    if directive_pattern_sg_accepts_empty(lang, pattern)
        || (lang == Language::CSharp && cs_using_plainface_meta_path(pattern))
    {
        return true;
    }
    // Answerability is spelling-invariant — normalize the expando META
    // spelling FIRST so every arm below (the root-multi refusal, the
    // classifier, the gate, the template build) sees the `$` spelling the
    // reference itself produces via `pre_process_pattern`.
    let pattern: Cow<'_, str> = normalize_expando_meta_spelling(lang, pattern.trim());
    let pattern = pattern.trim();
    if pattern.is_empty() || !pattern.contains('$') {
        // A degenerate statement-only semicolon pattern — ONLY `;`s and
        // whitespace, TWO or more — roots at multiple empty statements, and
        // the reference refuses it outright in every language whose grammar
        // gives `;` statement weight; refuse the class so the census takes
        // the loud fail-closed path exactly where the reference is loud.
        // python/swift parse these as lenient ERROR-node patterns and kotlin
        // finds no nodes, so those languages KEEP the silent class. A SINGLE
        // `;` is accepted everywhere and the subject already agrees — count
        // == 1 stays answerable. `$`-carrying spellings never enter this
        // degenerate class.
        if degenerate_semicolon_roots(pattern)
            && !matches!(lang, Language::Python | Language::Swift | Language::Kotlin)
        {
            return false;
        }
        // The unbalanced-bracket-tail class the reference REFUSES (js/ts/
        // ruby/swift `}` tails) is governed by the same language-aware
        // fragment census for `$`-LESS spellings too: `q }` / `µA }` (js/ts
        // have no expando) must stay census-loud exactly like their `$A }`
        // twins instead of walking silent ok-empty (the template-route
        // precedent: the census governs $-less faces). ACCEPTED tails —
        // `]`/`)` head-free fragments everywhere and the `}` tail in
        // py/rust/go/java/c/cpp/php/kotlin/csharp (lenient empty) — keep
        // the honest empty: the fragment admission accepts them below.
        if sg_bracket_fragment_shape(pattern) && !sg_bracket_fragment_accepted_empty(lang, pattern)
        {
            return false;
        }
        // Go and ruby spell statements WITHOUT `;` — the reference ACCEPTS
        // the `;`-ful spelling of these bare keyword statements (single
        // statement root, the `;` an anonymous tail) and answers
        // valid-empty on every go/rb candidate. The workspace gate port
        // splits the `;` into a second root — a known port divergence — so
        // this arm must sit AHEAD of the gate consult (the face is `$`-less
        // and never reaches the `$`-carrying arms). The walk's empty IS the
        // agreement for this exact spelling family. go:
        // return/break/continue/fallthrough; ruby: return/next/redo/retry.
        if sg_semi_keyword_accepted_empty(lang, pattern) {
            return true;
        }
        // The cs `throw;` family is candidate-side gated (the rethrow
        // binds, operand candidates refuse) — census-answerable either way,
        // never loud.
        if lang == Language::CSharp && cs_throw_semi_pattern(pattern) {
            return true;
        }
        // The `$`-less literal route folds to the reference's own parse
        // verdict — a spelling the gate refuses as multi-root (the ruby
        // multi-suffix `µA??`/`µA?!`-class and the plain `zz??` twin) is
        // unanswerable here, so the census takes the loud fail-closed path
        // exactly where the reference is loud. The expando normalization
        // above already converges the `$A??` twin onto the `µA??` bytes.
        // Spellings the gate ACCEPTS stay answerable: the single-suffix
        // `µA?`/`$A?`/`zz?` literals and the js `µA??` nullish-recovery
        // reading.
        if !sg_pattern_gate_accepts(lang, pattern) {
            return false;
        }
        return true;
    }
    // The reference's `PatternBuilder::build` refuses a bare multi-meta root
    // AFTER the single-node parse accepts it (`PatternError::RootMultiMetaVar`).
    // Root `µµµ`/`$$$` spellings are therefore UNANSWERABLE (the census keeps
    // the query loud), closing the root fail-open where the reference refuses.
    if sg_root_multi_meta_pattern(pattern) {
        return false;
    }
    if lane_comment_refused(lang, pattern) {
        return false;
    }
    // A no-expando whole-token literal face is answerable in `lang` exactly
    // when the reference accepts the spelling — every `$`-carrying token is
    // a glued literal identifier and the reference parses the pattern as a
    // single root (multi-root spellings stay census-loud through this gate).
    // The walk then answers the reference's verbatim rows through the
    // literal lane.
    if sg_expando_char(lang).is_none()
        && pattern_tokens_are_all_literal(pattern)
        && sg_pattern_parses(lang, pattern)
    {
        return true;
    }
    match classify_native(pattern) {
        Some(kind) => native_kind_language_answerable(lang, pattern, &kind),
        None => {
            // The conditional-directive family answers natively in C/C++
            // through the dedicated preproc lane.
            if matches!(lang, Language::C | Language::Cpp)
                && preproc_directive_supported(pattern) == Some(true)
            {
                return true;
            }
            // A bare php binary-expression META template is answered natively
            // by the operand lane — php files must not skip it (the skips
            // would compose into the loud census class where the reference
            // answers).
            if matches!(lang, Language::Php) && classify_php_operand_template(pattern).is_some() {
                return true;
            }
            // The php assignment-hook faces the dedicated lane serves
            // (literal targets AND the newly admitted meta targets) are
            // answerable in php exactly when the hook classifies the pattern
            // — otherwise the census counts php unanswerable and the CLI
            // refuses faces the walk answers exactly. The `;`-ful
            // whole-meta spellings and the `list()` LHS keep their census
            // class (the hook refuses them).
            if matches!(lang, Language::Php) && classify_php_assignment(pattern).is_some() {
                return true;
            }
            // The python delete-meta face is answered natively by the
            // dedicated delete lane — py files must not skip it (the skips
            // would compose into the loud census class where the reference
            // answers binding X to the whole operand list). The general
            // lane's template cannot express that whole-`expression_list`
            // binding, so its build refuses and the plain general-lane arm
            // below would count py unanswerable — the same genus the php
            // hook arms above fix.
            if lang == Language::Python && py_delete_meta_pattern(pattern) {
                return true;
            }
            // The `;`-ful del template family is ACCEPTED-EMPTY in the
            // multi-language run — census-answerable, the walk's empty IS
            // the agreement.
            if lang == Language::Python
                && pattern.starts_with("del")
                && pattern.ends_with(';')
                && pattern
                    .strip_suffix(';')
                    .is_some_and(|s| py_delete_template(s.trim()).is_some())
            {
                return true;
            }
            // The csharp statement-head lane answers its faces natively —
            // without this arm the unbuildable general template counted
            // csharp unanswerable where the reference answers. The lane's own
            // parse is the admission; the walk decides per file.
            if lang == Language::CSharp && csharp_statement_template(pattern).is_some() {
                return true;
            }
            // The java synchronized META-body, NESTED-block, and METHOD faces
            // are answered natively by the dedicated lane (the reference binds
            // the block statement, X/B captures) — the general template cannot
            // substitute a bare-meta body, and without this arm the census
            // counted java unanswerable where the reference answers.
            if lang == Language::Java
                && (ja_synchronized_meta_template(pattern).is_some()
                    || ja_sync_block_nested_template(pattern).is_some()
                    || ja_sync_method_template(pattern).is_some()
                    || classify_java_class_member_count(pattern).is_some())
            {
                return true;
            }
            // The php braced-namespace block face is answered natively by the
            // dedicated lane (the reference binds the block, N/B captures) —
            // the general template cannot substitute the bare-meta body;
            // without this arm the census counted php unanswerable where the
            // reference answers.
            if lang == Language::Php && php_namespace_block_template(pattern).is_some() {
                return true;
            }
            // The dedicated statement-root and directive lanes — each lane's
            // own parse is the admission; the walk decides per file. The go
            // `;`-ful keyword spellings are ACCEPTED-EMPTY, census-answerable
            // with the walk's empty as the agreement.
            if directive_lane_serves(lang, pattern) {
                return true;
            }
            if lang == Language::Go
                && (sg_goto_semi_pattern(pattern) || sg_go_import_semi_pattern(pattern))
            {
                return true;
            }
            // The go semi-less `goto $L` family BINDS per site — the
            // dedicated lane's parse is the admission; without it the census
            // counted go unanswerable and the shape failed closed at the
            // query level.
            if lang == Language::Go && go_goto_bare_template(pattern).is_some() {
                return true;
            }
            if lang == Language::CSharp && cs_expression_template(pattern).is_some() {
                return true;
            }
            if lang == Language::Kotlin
                && (kt_typealias_template(pattern).is_some() || kt_for_template(pattern).is_some())
            {
                return true;
            }
            if lang == Language::Swift && swift_for_template(pattern).is_some() {
                return true;
            }
            if lang == Language::Rust && rs_let_else_template(pattern).is_some() {
                return true;
            }
            if lang == Language::C && c_goto_template(pattern).is_some() {
                return true;
            }
            // The kt `companion object { $B }` / `init { $B }` and swift
            // `deinit { $B }` spellings are ACCEPTED-EMPTY on every candidate
            // (the pattern parse cannot align with the declaration shape);
            // the walk's empty IS the agreement, never loud.
            if lang == Language::Kotlin && kt_binds_nothing_template(pattern) {
                return true;
            }
            if lang == Language::Swift && swift_binds_nothing_template(pattern) {
                return true;
            }
            // The statement/decl-root lanes — each lane's own parse is the
            // admission. (The directive-family accepted-empty faces consult
            // EARLIER, before the classify dispatch — the class is
            // reference-acceptance-defined.)
            if statement_root_142_template(lang, pattern).is_some() {
                return true;
            }
            // The csharp meta-BODY lock/using templates build with
            // `force_empty` set — the reference ACCEPTS them and binds
            // nothing (valid-empty). The plain general-lane arm's gate
            // conjunct refuses the raw spelling (the workspace csharp
            // grammar ERRORs the top-level lock parse where the pinned
            // grammar accepts it — a known port divergence), so this arm
            // keeps the face census-answerable and the walk's forced empty
            // IS the agreement.
            if lang == Language::CSharp
                && cached_general_template(lang, pattern).is_some_and(|tpl| tpl.force_empty)
            {
                return true;
            }
            // A general-lane face is answerable only when the reference
            // pattern-acceptance gate accepts the spelling. The build success
            // alone admitted faces refused
            // as multi-root (`$A ;`, `$A;`, `$A ;;`, `$f($A) }` compounds) and the
            // gate is exactly the reference accept/reject decision, so the
            // intersection keeps every accepted face answerable and
            // refuses precisely the rejected class. Faces the gate accepts but
            // whose template cannot build keep the LOUD residual.
            // EXCEPT the paren-free bracket fragments accepted-EMPTY (single
            // ERROR root) — there the walk's empty is the agreement, so the
            // language is answerable.
            (cached_general_template(lang, pattern).is_some()
                && sg_pattern_gate_accepts(lang, pattern))
                || (cached_general_template(lang, pattern).is_none()
                    && sg_bracket_fragment_accepted_empty(lang, pattern))
        }
    }
}

/// The `;`-ful bare-keyword statement spellings ACCEPTED (accepted-empty)
/// where the gate port's root-split refusal (or the kind lane) would answer
/// wrong. go: `return;` `break;` `continue;` `fallthrough;` and the
/// `goto <label>;` forms — concrete AND `$`-carrying (nothing binds on any
/// go candidate); ruby: `return;` `next;` `redo;` `retry;` `break;`;
/// python: the seven `;`-ful keyword spellings (`pass; yield; global;
/// import; raise; del; assert;`); javascript/typescript/java: `throw;`
/// (the operand-less spelling binds NOTHING, even on operand-bearing and
/// error-recovered candidates). Other languages keep their own faces (java
/// `break;` answers through the kind lane).
pub(crate) fn sg_semi_keyword_accepted_empty(lang: Language, pattern: &str) -> bool {
    match lang {
        Language::Go => {
            matches!(pattern, "return;" | "break;" | "continue;" | "fallthrough;")
                || sg_goto_semi_pattern(pattern)
        }
        Language::Ruby => matches!(pattern, "return;" | "next;" | "redo;" | "retry;" | "break;"),
        Language::Python => matches!(
            pattern,
            "pass;" | "yield;" | "global;" | "import;" | "raise;" | "del;" | "assert;"
        ),
        Language::JavaScript | Language::TypeScript | Language::Java => pattern == "throw;",
        _ => false,
    }
}

/// The go `goto <label>;` spelling family — `goto`, one whitespace run, a
/// SINGLE label token (identifier or canonical meta), and the `;` tail. The
/// reference accepts every spelling and binds nothing on any go candidate;
/// the C grammar is the opposite (its `goto $L;` binds per site). The label
/// is one token: the single-root pattern parse cannot carry more.
pub(crate) fn sg_goto_semi_pattern(pattern: &str) -> bool {
    let p = pattern.trim();
    let Some(rest) = p.strip_prefix("goto") else {
        return false;
    };
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let Some(label) = rest.trim_start().strip_suffix(';') else {
        return false;
    };
    let label = label.trim();
    !label.is_empty()
        && !label.contains(char::is_whitespace)
        && (is_pattern_ident(label)
            || capture_name(label).is_some_and(|name| name == label.trim_start_matches('$')))
}

/// The cs `throw;` spelling family splits on the trivia class of record —
/// the interior run between `throw` and `;` is a parse-relevant zone. A run
/// that is empty or all trivia-class ([`is_sg_cs_trivia`]: `throw ;`,
/// `throw\u{FEFF};`, `throw\x0B;`) parses to the bare rethrow and BINDS; a
/// run of Rust-whitespace outsiders (`throw\u{2028};` — U+2028/U+0085/
/// U+2029/U+202F are NOT cs trivia) parses to a DIFFERENT root and binds
/// NOTHING on any candidate (the accepted-empty class). Anything else is
/// not this family (the operand spellings keep their kind-lane faces).
pub(crate) enum CsThrowSemiParse {
    Binds,
    AcceptedEmpty,
}

pub(crate) fn cs_throw_semi_parse(pattern: &str) -> Option<CsThrowSemiParse> {
    let rest = pattern.trim().strip_prefix("throw")?;
    if rest.is_empty() {
        // The bare `throw` spelling keeps its kind-lane face (F2_throwbare_*).
        return None;
    }
    let body = rest.strip_suffix(';')?;
    if body.is_empty() || body.chars().all(is_sg_cs_trivia) {
        return Some(CsThrowSemiParse::Binds);
    }
    // Rust-whitespace-only but not reference-class trivia: the accepted-empty class.
    (!body.trim().is_empty() && body.chars().all(char::is_whitespace))
        .then_some(CsThrowSemiParse::AcceptedEmpty)
}

pub(crate) fn cs_throw_semi_pattern(pattern: &str) -> bool {
    cs_throw_semi_parse(pattern).is_some()
}

/// The cs trivia class of record — the PATTERN-side class. The cs grammar
/// extras are /[\s\u00A0\uFEFF\u3000]+/ whose `\s` the grammar generation
/// compiles with UNICODE semantics, but the PATTERN parse answers these
/// outsider spellings accepted-empty at the nested-head seam. At the
/// throw-semi/using TOKEN GAPS the candidate parse accepts the full
/// Unicode White_Space set — that is [`is_sg_cs_gap_junk`], NOT this class.
pub(crate) fn is_sg_cs_trivia(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{00A0}' | '\u{FEFF}' | '\u{3000}'
    )
}

/// The cs CANDIDATE-side gap class. The pinned cs grammar skips the FULL
/// Unicode White_Space set plus U+FEFF at every throw-semi/using token gap,
/// and bare control junk (U+0000/U+001A/U+007F) recovers as skipped ERROR
/// tokens — all of those spellings bind. This workspace's tree-sitter-c-sharp
/// extras are ASCII-scoped, so those bytes survive as ERROR children; the
/// candidate scanners treat such junk runs as transparent while junk glued
/// INSIDE a name token still refuses.
pub(crate) fn is_sg_cs_gap_junk(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}' || c.is_control()
}

/// The php gap-trivia class of record at the use-kind clause gaps — the php
/// grammar skips exactly ASCII whitespace, U+00A0, U+FEFF and U+001A there.
/// Every other Rust-whitespace spelling GLUES into the surrounding php name
/// token (name chars run to U+0080+), which is why the zero-gap glue
/// cases refuse and why glue chars belong to name-token text.
pub(crate) fn is_sg_php_gap_trivia(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{00A0}' | '\u{FEFF}' | '\u{001A}'
    )
}

/// Strip `// …` and `/* … */` spans from directive demarcation text. The
/// reference walks the AST where comments are extras, so the candidate-side
/// directive scans see the comment-FREE clause text while the emitted
/// span/MATCH keep the original bytes. No string-literal awareness on
/// purpose: directive demarcation zones carry no string syntax.
pub(crate) fn strip_comment_spans(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else {
            let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

/// Matches the operand-less C# `throw;` rethrow and the exact `return;`
/// spelling. Gap-junk ERROR children and comments are transparent trivia;
/// any operand, or text glued to the keyword, refuses the match.
pub(crate) fn match_cs_bare_semi(
    source: &str,
    pattern: &str,
    kind: &str,
) -> Option<Vec<PatternMatch>> {
    let tree = parse_source(Language::CSharp, source).ok()?;
    let mut out = Vec::new();
    walk_cs_bare_semi(tree.root_node(), source, pattern, kind, &mut out);
    Some(out)
}

/// Operand-less `keyword expression? ';'` walk (`kind` is `return_statement`
/// or `throw_statement`); the operand-free rethrow has NO named child.
pub(crate) fn walk_cs_bare_semi(
    node: Node,
    source: &str,
    pattern: &str,
    kind: &str,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == kind && !is_in_comment_or_string(&node) {
        let mut cursor = node.walk();
        let has_operand = node.children(&mut cursor).any(|child| {
            if !child.is_named() || is_trivia_kind(child.kind()) {
                return false;
            }
            if child.is_error() {
                if let Some(text) = node_text(&child, source) {
                    if !text.is_empty() && text.chars().all(is_sg_cs_gap_junk) {
                        return false;
                    }
                }
            }
            true
        });
        if !has_operand {
            if let Some(text) = node_text(&node, source) {
                let mut captures = BTreeMap::new();
                captures.insert("MATCH".to_string(), text.to_string());
                out.push(hit_for_node(&node, source, pattern, captures));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_cs_bare_semi(child, source, pattern, kind, out);
    }
}

/// The kotlin `companion object { $B }` / `init { $B }` spellings are
/// ACCEPTED-EMPTY — the pattern parses but binds NOTHING on any candidate,
/// so the walk's empty IS the agreement. The body must be one canonical
/// meta; any other spelling keeps its prior route.
pub(crate) fn kt_binds_nothing_template(pattern: &str) -> bool {
    let parse = || -> Option<()> {
        let p = pattern.trim();
        let rest = if let Some(r) = p.strip_prefix("companion object") {
            r
        } else if let Some(r) = p.strip_prefix("init") {
            // The `init` prefix demands a word boundary (`initX` is a name).
            if r.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            r
        } else {
            return None;
        };
        let inner = rest.trim_start().strip_prefix('{')?;
        let close = balanced_brace_close(inner)?;
        if !inner[close + 1..].trim().is_empty() {
            return None;
        }
        capture_name(inner[..close].trim()).map(|_| ())
    };
    parse().is_some()
}

/// The swift `deinit { $B }` spelling is ACCEPTED-EMPTY (the reference never
/// binds the deinit body); the walk's empty IS the agreement.
pub(crate) fn swift_binds_nothing_template(pattern: &str) -> bool {
    let parse = || -> Option<()> {
        let rest = pattern.trim().strip_prefix("deinit")?;
        let inner = rest.trim_start().strip_prefix('{')?;
        let close = balanced_brace_close(inner)?;
        if !inner[close + 1..].trim().is_empty() {
            return None;
        }
        capture_name(inner[..close].trim()).map(|_| ())
    };
    parse().is_some()
}

/// True when `pattern` (already trimmed) consists ONLY of semicolons and
/// ASCII whitespace and carries TWO OR MORE semicolons — a degenerate
/// statement-only pattern whose roots are multiple empty statements (the
/// reference's "Multiple AST nodes are detected" refusal class; see the
/// language scope in [`native_pattern_answerable`]). A single `;`, any
/// `$`-carrying spelling, and any pattern with non-whitespace code never
/// enter this class.
pub(crate) fn degenerate_semicolon_roots(pattern: &str) -> bool {
    let mut semis = 0usize;
    for b in pattern.bytes() {
        if b == b';' {
            semis += 1;
        } else if !b.is_ascii_whitespace() {
            return false;
        }
    }
    semis >= 2
}
