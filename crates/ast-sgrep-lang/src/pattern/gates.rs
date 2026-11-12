//! Entry predicates and semicolon/trivia acceptance gates.

use super::*;
use crate::extract::node_text;
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
    let trimmed = pattern.trim();
    // No-expando literal pre-check on the RAW spelling, before the
    // language-free expando fold below: a µ-run+$ composition (`µµ$A`)
    // folds to a canonical `$$$A` meta, which the literal gate below vetoes
    // — but in the no-expando languages the token is one ordinary
    // identifier and the reference answers the verbatim rows (f100). Running
    // the admission on the un-folded bytes keeps the fold from destroying
    // the literal reading it is meant to classify. Comment-carrying faces
    // stay on the folded spelling: their admission belongs to the
    // comment-placement gates, never to this pre-check.
    if !contains_comment_syntax_outside_strings(trimmed)
        && pattern_tokens_are_all_literal(trimmed)
        && Language::all()
            .iter()
            .any(|&lang| sg_expando_char(lang).is_none() && sg_pattern_parses(lang, trimmed))
    {
        return false;
    }
    // Self-normalizing ingress (see `normalize_any_expando_meta_spelling`):
    // a µ-spelled face must take exactly its `$` twin's answerability class.
    let pattern = normalize_any_expando_meta_spelling(trimmed);
    let p: &str = &pattern;
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

/// Language-free: does ANY grammar build a general-lane template?
/// (Merged here from `lane_support.rs`: the triple admission —
/// [`match_pattern_inner`](dispatch.rs) routing, [`native_pattern_answerable`]
/// per-language verdicts, and this any-language support predicate — must
/// change together. They are one list now: this function is the only
/// any-language admission, consulted by [`needs_ast_grep_fallback`].)
pub(crate) fn general_lane_supported(pattern: &str) -> bool {
    // The comment-content facts are language-free: classify them ONCE, then
    // consult the per-language row per grammar (same rows the core placement
    // table adjudicates — see [`comment_face_refused_for_lang`]).
    let content = pattern_comment_content(pattern);
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
    if content.glued_hash {
        return false;
    }
    // `#` is comment syntax only in python/ruby/php — the any-language
    // support answer must not count a hash-comment language templating a
    // pattern whose `#` is real syntax elsewhere (and vice versa: rust
    // attribute faces stay supported while python hash-glued faces keep
    // failing closed).
    Language::all().iter().any(|&lang| {
        !comment_face_refused_for_lang(lang, &content)
            && build_general_template(lang, pattern, &substituted, &placeholders, &multi_names)
                .is_some()
    })
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
    if comment_face_refused_for_lang(lang, &pattern_comment_content(pattern)) {
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
            if lang == Language::Python && py_delete_semi_pattern(pattern) {
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
            if admitted_directive_template(lang, pattern).is_some() {
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
                && general_template_for(lang, pattern).is_some_and(|tpl| tpl.force_empty)
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
            (general_template_for(lang, pattern).is_some()
                && sg_pattern_gate_accepts(lang, pattern))
            || (general_template_for(lang, pattern).is_none()
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

/// (Trivia classes live in ONE place now: [`crate::trivia`]. The cs/php
/// gap classes and [`strip_comment_spans`] re-exported through `super::*`.)

/// Matches the operand-less C# `throw;` rethrow and the exact `return;`
/// spelling. Gap-junk ERROR children and comments are transparent trivia;
/// any operand, or text glued to the keyword, refuses the match.
pub(crate) fn match_cs_bare_semi(
    source: &str,
    pattern: &str,
    kind: &str,
) -> Option<Vec<PatternMatch>> {
    let tree = parse_source_silent(Language::CSharp, source)?;
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
    walk_kind(node, kind, false, out, |n| {
        cs_bare_semi_match(n, source, pattern)
    });
}

/// Single-candidate check for [`walk_cs_bare_semi`]: the operand-less
/// keyword statement carries no operand child (gap-junk ERROR children and
/// comments are transparent trivia); anything else is not this face.
fn cs_bare_semi_match(node: &Node, source: &str, pattern: &str) -> Option<PatternMatch> {
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
    if has_operand {
        return None;
    }
    let text = node_text(node, source)?;
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), text.to_string());
    Some(hit_for_node(node, source, pattern, captures))
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
        } else {
            let r = p.strip_prefix("init")?;
            // The `init` prefix demands a word boundary (`initX` is a name).
            if r.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            r
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
