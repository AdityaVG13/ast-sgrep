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
    if general_lane_supported(p) {
        return false;
    }
    // PASS 96 (FB-95B-2): the paren-free bracket fragments sg 0.45.2
    // accepts-EMPTY ($A ] / $A ) / $A } — single ERROR root, matrix
    // mf_bracket_fragments) must reach the walk instead of ingress-rc2: the
    // walk's empty IS the sg agreement. The language-free gate admits the
    // class when ANY language's bare gate accepts the spelling; the
    // language-aware census (native_pattern_answerable's fragment arm) keeps
    // the sg-rc8 spellings (ruby/swift all tails, js/ts `}` — the workspace
    // parse splits there) census-loud via the backstop.
    // PASS 100 (FB-99A-2): a pattern that is whole-token LITERAL in some
    // no-expando language (js/ts/java — every `$`-carrying token fails sg's
    // whole-token meta validation) and sg-ACCEPTED there is an ordinary
    // code face, not a priori unanswerable: the glued bare tokens
    // (`µµµ$A`, `$AµµB`, `foo$$$A`) must reach the per-language census and
    // the literal-lane walk instead of the query-level rc2. Expando
    // languages decide per-file through their own census arms (unchanged),
    // and sg rc8 spellings (`q µµµ$A + 1`) never pass the per-language
    // parse gate, so their query-level loud contract is intact.
    if pattern_tokens_are_all_literal(p)
        && Language::all().iter().any(|&lang| {
            sg_expando_char(lang).is_none() && sg_pattern_parses(lang, p)
        })
    {
        return false;
    }
    // PASS 141 (F7, grid F7_kt_*/F7_sw_deinit; the PASS 140 del-`$$$`
    // ingress precedent): the kt `companion object { $B }` / `init { $B }`
    // and swift `deinit { $B }` accepted-empty spellings must reach the
    // per-language census and the walk's empty instead of the query-level
    // rc2. The union stays language-free; non-kt/swift languages keep
    // their census-loud class.
    if kt_binds_nothing_template(p) || swift_binds_nothing_template(p) {
        return false;
    }
    // PASS 142 (142A-F3 + 142A-F5, grids E*/H*/J*/K*/A*/I*): the
    // statement/decl-root lanes and the directive-family accepted-empty /
    // dotted-meta faces must reach the per-language census and their walks
    // instead of the query-level rc2. The union stays language-free (the
    // PASS 141 kt/swift precedent).
    if statement_root_142_any_language(p) || directive_accepted_empty_any_language(p) {
        return false;
    }
    // PASS 144 (143A-F8, grid G*): the go semi-less `goto $L` spelling BINDS
    // per site (sg G1 n1 L=end; G5 n2) — reach the walk instead of the
    // query-level rc2 structural fallback. The union stays language-free
    // (the 141/142 precedent): the spelling parse is the admission and each
    // language's own census keeps its sg class for it.
    if go_goto_bare_template(p).is_some() {
        return false;
    }
    !Language::all()
        .iter()
        .any(|&lang| sg_bracket_fragment_accepted_empty(lang, p))
}

/// PASS 122 (F3, f122c): `$`-less patterns that are bare KEYWORD LITERALS —
/// tokens every indexed grammar parses as a dedicated leaf node kind
/// (`null`/`true`/`false`, python `None`/`True`/`False`, js/ts
/// `this`/`super`) rather than as identifier rows. The index's
/// `is_pattern_ident` signature admits them as "ident-exact", but
/// `pattern_nodes` stores no rows for keyword nodes, so the ident-served
/// lane composed a silent `ok:true []` where sg 0.45.2 answers the leaf
/// node (oracle grid /tmp/phase122/f3: every keyword face sg n1 across
/// init/arg/return/array positions while the subject answered rc0 n0;
/// number roots `1`/`0` agree — `is_pattern_ident` rejects digit-led
/// spellings, so they already ride the walk). The core ingress consults
/// this predicate to route the class to the native walk, which answers
/// sg-exactly (`match_pattern` literal lane = 1 hit on every grid face).
/// NOT in the class: `undefined`/`self` (plain identifier nodes, ident rows
/// exist, index-serve correct).
pub fn pattern_is_keyword_literal_root(pattern: &str) -> bool {
    matches!(
        pattern.trim(),
        "null" | "true" | "false" | "True" | "False" | "None" | "this" | "super"
    )
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
    // PASS 142 (142A-F1 pattern-side + 142A-F5 dotted-meta, grids A21/A22/
    // A23/I1-I4/I8 + G1/G2/G8/G9/I6): the directive-family accepted-empty
    // and dotted-meta faces are census-answerable REGARDLESS of the
    // classify dispatch — the class is defined by sg's own acceptance
    // (rc1 `[]`), not by our classify. This consult reads the RAW `$`
    // spelling (the lanes parse canonically); the expando normalization
    // below must not rename the metas out from under the templates.
    if directive_pattern_sg_accepts_empty(lang, pattern)
        || (lang == Language::CSharp && cs_using_plainface_meta_path(pattern))
    {
        return true;
    }
    // PASS 96 (F-95A-1): answerability is spelling-invariant — normalize the
    // expando META spelling FIRST so every arm below (the root-multi refusal,
    // the classifier, the gate, the template build) sees the `$` spelling sg
    // itself produces via `pre_process_pattern`.
    let pattern: Cow<'_, str> = normalize_expando_meta_spelling(lang, pattern.trim());
    let pattern = pattern.trim();
    if pattern.is_empty() || !pattern.contains('$') {
        // F-r40-5 (r41, 90A-F2): a degenerate statement-only semicolon
        // pattern — ONLY `;`s and whitespace, TWO or more semicolons — roots
        // at multiple empty statements, and sg 0.45.2 refuses it outright
        // ("Cannot parse query as a valid pattern / Multiple AST nodes are
        // detected", rc8 probed 2026-09-08 ATTACHED) in every language whose
        // grammar gives `;` statement weight: javascript/typescript/php/
        // rust/go/c/cpp/csharp/ruby/java on `;;`/`;;;`/`;; ;`. The subject's
        // doc grammar parsed the same spellings into empty statements and
        // answered a silent `ok:true []` where sg exits 8 — refuse the class
        // so the census takes the loud fail-closed path exactly where sg is
        // loud. python/swift parse these spellings as lenient ERROR-node
        // patterns and kotlin finds no nodes — sg answers accepted-empty
        // rc0/rc1 there (probed), so those languages KEEP the silent class
        // (making them loud would diverge in the loud direction). A SINGLE
        // `;` (whitespace-padded or not) is sg-ACCEPTED everywhere (probed
        // answering in js/ts/php/rust/go/c/java, rc1-empty in py/rb) and the
        // subject already agrees — count == 1 stays answerable.
        // `$`-carrying spellings (`$A;;`) never enter this degenerate class.
        if degenerate_semicolon_roots(pattern)
            && !matches!(lang, Language::Python | Language::Swift | Language::Kotlin)
        {
            return false;
        }
        // PASS 98 (F-97A-2): the unbalanced-bracket-tail class sg 0.45.2
        // rc8-REFUSES (js/ts/ruby/swift `}` tails — m3b, probed 2026-09-09)
        // is governed by the same language-aware fragment census for `$`-LESS
        // spellings too: `q }` / `µA }` (js/ts have no expando) must stay
        // census-loud exactly like their `$A }` twins instead of walking
        // silent ok-empty (the pass-90/91 template-route precedent: the
        // census governs $-less faces). sg-ACCEPTED tails — `]`/`)` head
        // -free fragments everywhere and the `}` tail in py/rust/go/java/c/
        // cpp/php/kotlin/csharp (m3b: lenient rc0/rc1 empty) — keep the
        // honest empty: the fragment admission accepts them below.
        if sg_bracket_fragment_shape(pattern)
            && !sg_bracket_fragment_accepted_empty(lang, pattern)
        {
            return false;
        }
        // PASS 137 (D3 grid go_return_semi/rb_return_semi) + PASS 139
        // (grid139 F): go and ruby spell statements WITHOUT `;` — sg 0.45.2
        // ACCEPTS the `;`-ful spelling of these bare keyword statements
        // (single statement root, the `;` an anonymous tail; grid receipts
        // rc1 `[]` on both grammars) and answers valid-empty on every go/rb
        // candidate. The workspace gate port splits the `;` into a second
        // root — a registered port divergence — so this arm must sit AHEAD
        // of the gate consult (the face is `$`-less and never reaches the
        // `$`-carrying arms). The walk's empty IS the sg agreement for this
        // exact spelling family. go: return/break/continue/fallthrough;
        // ruby: return/next/redo/retry; (grid139 F01/F02/F03/F07/F08/F09).
        if sg_semi_keyword_accepted_empty(lang, pattern) {
            return true;
        }
        // PASS 141 (141A-F2, grid F2_*): the cs `throw;` family is
        // candidate-side gated (the rethrow binds, operand candidates
        // refuse) — census-answerable either way, never loud.
        if lang == Language::CSharp && cs_throw_semi_pattern(pattern) {
            return true;
        }
        // PASS 102 (F-101A-2): the `$`-less literal route folds to sg's own
        // parse verdict — a spelling sg's gate refuses (rc8 "Multiple AST
        // nodes": the ruby multi-suffix `µA??`/`µA?!`-class and the plain
        // `zz??` twin, m2 matrix) is unanswerable here, so the census takes
        // the loud fail-closed path exactly where sg is loud. The expando
        // normalization above already converges the `$A??` twin onto the
        // `µA??` bytes. Spellings sg ACCEPTS stay answerable: the
        // single-suffix `µA?`/`$A?`/`zz?` literals (m2 sg-accepted faces)
        // and the js `µA??` nullish-recovery reading (m2 SG_EXACT).
        if !sg_pattern_gate_accepts(lang, pattern) {
            return false;
        }
        return true;
    }
    // PASS 96 (F-95A-1): sg's `PatternBuilder::build` refuses a bare
    // multi-meta root AFTER the single-node parse accepts it
    // (`PatternError::RootMultiMetaVar`, exit 8 — the r44 gate ported
    // acceptance but not this post-parse check). Root `µµµ`/`$$$` spellings
    // are therefore UNANSWERABLE (the census keeps the query loud), closing
    // the root `µµµ` fail-open where sg refuses.
    if sg_root_multi_meta_pattern(pattern) {
        return false;
    }
    if lane_comment_refused(lang, pattern) {
        return false;
    }
    // PASS 100 (FB-99A-2): a no-expando whole-token literal face is
    // answerable in `lang` exactly when sg accepts the spelling — every
    // `$`-carrying token is a glued literal identifier (`µµµ$A`-class) and
    // sg parses the pattern as a single root (the `q µµµ$A` multi-root
    // spellings sg rc8s stay census-loud through this gate). The walk then
    // answers sg's verbatim rows through the literal lane.
    if sg_expando_char(lang).is_none()
        && pattern_tokens_are_all_literal(pattern)
        && sg_pattern_parses(lang, pattern)
    {
        return true;
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
            // 88a-M2 (r39): a bare php binary-expression META template is
            // answered natively by the operand lane — php files must not
            // skip it (the skips would compose into the loud census class
            // where sg answers).
            if matches!(lang, Language::Php)
                && classify_php_operand_template(pattern).is_some()
            {
                return true;
            }
            // PASS 127 (125A-F5, f127a): the php assignment-hook faces the
            // dedicated lane serves (literal targets AND the newly admitted
            // meta targets) are answerable in php exactly when the hook
            // classifies the pattern — otherwise the census counts php
            // unanswerable and the CLI rc2s faces the walk answers sg-exactly
            // (first-hand probe on the post-build binary: `$X = $Y` LOUD at
            // the CLI while match_pattern answers n1). The `;`-ful whole-meta
            // spellings and the `list()` LHS keep their census class (the
            // hook refuses them, f127a pins).
            if matches!(lang, Language::Php) && classify_php_assignment(pattern).is_some() {
                return true;
            }
            // PASS 135 (134A-F7, f135c-ingress, grid X_py_del_two): the
            // python delete-meta face is answered natively by the dedicated
            // delete lane — py files must not skip it (the skips would
            // compose into the loud census class where sg answers n1 binding
            // X to the whole operand list). The general lane's template
            // cannot express that whole-`expression_list` binding, so its
            // build refuses and the plain general-lane arm below would count
            // py unanswerable — the same genus the php hook arms above fix.
            if lang == Language::Python && py_delete_meta_pattern(pattern) {
                return true;
            }
            // PASS 137 (grid137 py_del_semi_loud): the `;`-ful del
            // template family is sg ACCEPTED-EMPTY in the multi-language
            // run (rc1 `[]` per the grid probes) — census-answerable, the
            // walk's empty IS the sg agreement.
            if lang == Language::Python
                && pattern.starts_with("del")
                && pattern.ends_with(';')
                && pattern.strip_suffix(';')
                    .is_some_and(|s| py_delete_template(s.trim()).is_some())
            {
                return true;
            }
            // PASS 137 (137A-F2, grid137a B3/D4): the csharp statement-head
            // lane answers its faces natively — without this arm the unbuild-
            // able general template counted csharp unanswerable and the CLI
            // rc2'd faces sg 0.45.2 answers n1 (`fixed ($D) { *p = 'x'; }`,
            // `checked { int v = a + b; }`, …). The lane's own parse is the
            // admission; the walk (match_csharp_statement) decides per file.
            if lang == Language::CSharp && csharp_statement_template(pattern).is_some() {
                return true;
            }
            // PASS 139 (139A-F2, grid139 B): the java synchronized META-body
            // face is answered natively by the dedicated lane (sg binds the
            // block statement, X/B captures) — the general template cannot
            // substitute a bare-meta body, and without this arm the census
            // counted java unanswerable where sg answers n1.
            // PASS 140 (grid D): the NESTED-block and METHOD faces join the
            // same lane family (D_sync_nested/D_sync_method n1).
            if lang == Language::Java
                && (ja_synchronized_meta_template(pattern).is_some()
                    || ja_sync_block_nested_template(pattern).is_some()
                    || ja_sync_method_template(pattern).is_some()
                    || classify_java_class_member_count(pattern).is_some())
            {
                return true;
            }
            // PASS 139 (grid139 I): the php braced-namespace block face is
            // answered natively by the dedicated lane (sg binds the block,
            // N/B captures) — the general template cannot substitute the
            // bare-meta body; without this arm the census counted php
            // unanswerable where sg answers n1 (the registered form-1
            // predicate's retry condition, now grid-satisfied).
            if lang == Language::Php && php_namespace_block_template(pattern).is_some() {
                return true;
            }
            // PASS 140 (grids /tmp/phase140R): the dedicated statement-root
            // and directive lanes — each lane's own parse is the admission;
            // the walk decides per file (the 137A-F2 discipline). The go
            // `;`-ful keyword spellings are sg ACCEPTED-EMPTY (rc1 `[]`),
            // census-answerable with the walk's empty as the agreement.
            if directive_lane_serves(lang, pattern) {
                return true;
            }
            if lang == Language::Go
                && (sg_goto_semi_pattern(pattern) || sg_go_import_semi_pattern(pattern))
            {
                return true;
            }
            // PASS 144 (143A-F8, grid G*): the go semi-less `goto $L` family
            // BINDS per site (sg G1/G5) — the dedicated lane's parse is the
            // admission; without it the census counted go unanswerable and
            // the shape failed closed at the query level.
            if lang == Language::Go && go_goto_bare_template(pattern).is_some() {
                return true;
            }
            if lang == Language::CSharp && cs_expression_template(pattern).is_some() {
                return true;
            }
            if lang == Language::Kotlin
                && (kt_typealias_template(pattern).is_some()
                    || kt_for_template(pattern).is_some())
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
            // PASS 141 (F7, grid F7_*): the kt `companion object { $B }` /
            // `init { $B }` and swift `deinit { $B }` spellings are sg
            // ACCEPTED-EMPTY (rc1 `[]` on every candidate — the pattern
            // parse cannot align with the declaration shape); the walk's
            // empty IS the agreement, never loud.
            if lang == Language::Kotlin && kt_binds_nothing_template(pattern) {
                return true;
            }
            if lang == Language::Swift && swift_binds_nothing_template(pattern) {
                return true;
            }
            // PASS 142 (142A-F3, grids E*/H*/J*/K*): the statement/decl-root
            // lanes — each lane's own parse is the admission. (The
            // directive-family accepted-empty faces consult EARLIER, before
            // the classify dispatch — the class is sg-acceptance-defined.)
            if statement_root_142_template(lang, pattern).is_some() {
                return true;
            }
            // PASS 137 (137B-F4, grid137a D4 cs_lock_meta_body): the csharp
            // meta-BODY lock/using templates build with `force_empty` set —
            // sg ACCEPTS them and binds nothing (valid-empty). The plain
            // general-lane arm's sg-gate conjunct refuses the raw spelling
            // (the workspace csharp grammar ERRORs the top-level lock parse
            // where sg's pinned grammar accepts it — registered port
            // divergence), so this arm keeps the face census-answerable and
            // the walk's forced empty IS the sg agreement.
            if lang == Language::CSharp
                && cached_general_template(lang, pattern).is_some_and(|tpl| tpl.force_empty)
            {
                return true;
            }
            // PASS 94b (FB-93A-1/FB-93A-2): a general-lane face is
            // answerable only when sg's own pattern-acceptance gate accepts
            // the spelling. The build success alone admitted faces sg rc8s
            // as multi-root (`$A ;` go/py, `$A;` go/py, `$A ;;` java — the
            // FB-93A-2 had_semi over-answers; `$f($A) }` compounds) and the
            // gate is exactly sg's accept/reject decision, so the
            // intersection keeps every sg-accepted face answerable and
            // refuses precisely the rc8 class. Faces the gate accepts but
            // whose template cannot build keep the registered LOUD residual
            // (php/kotlin single-semi spellings — 90B-T4).
            // PASS 96 (FB-95B-2): EXCEPT the paren-free bracket fragments sg
            // accepts-EMPTY (single ERROR root) — there the walk's empty is
            // the sg agreement, so the language is answerable.
            (cached_general_template(lang, pattern).is_some()
                && sg_pattern_gate_accepts(lang, pattern))
                || (cached_general_template(lang, pattern).is_none()
                    && sg_bracket_fragment_accepted_empty(lang, pattern))
        }
    }
}

/// PASS 137 + PASS 139 (grid139 F) + PASS 140 (grids B/H): the `;`-ful
/// bare-keyword statement spellings sg ACCEPTS (rc1 `[]` — accepted-empty)
/// where the workspace gate port's root-split refusal (or the kind lane)
/// would answer wrong. go: `return;` (137) `break;` `continue;`
/// `fallthrough;` (139) and the `goto <label>;` forms — concrete AND
/// `$`-carrying (140 grid H_go_goto_meta/plain: sg binds NOTHING on any go
/// candidate); ruby: `return;` (137) `next;` `redo;` `retry;` (139)
/// `break;` (140); python: the seven `;`-ful keyword spellings of the
/// 140A-F9 sweep (`pass; yield; global; import; raise; del; assert;`);
/// javascript/typescript/java: `throw;` (140 grid B — sg accepts the
/// operand-less spelling and binds NOTHING, even on operand-bearing and
/// error-recovered candidates). Other languages keep their own faces (java
/// `break;` answers n1 through the kind lane — the f64-6 family).
fn sg_semi_keyword_accepted_empty(lang: Language, pattern: &str) -> bool {
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

/// PASS 140 (grid H): the go `goto <label>;` spelling family — `goto`, one
/// whitespace run, a SINGLE label token (identifier or canonical meta), and
/// the `;` tail. sg 0.45.2 accepts every spelling (rc1 `[]`) and binds
/// nothing on any go candidate (H_go_goto_meta/plain); the C grammar is the
/// opposite (its `goto $L;` binds per site — the f140f lane). The label is
/// one token: sg's single-root pattern parse cannot carry more.
fn sg_goto_semi_pattern(pattern: &str) -> bool {
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

/// PASS 142 (142B-F1, grids C_*): the cs `throw;` spelling family splits on
/// the sg trivia class of record — the interior run between `throw` and `;`
/// is a parse-relevant zone. A run that is empty or all sg-class trivia
/// ([`is_sg_cs_trivia`]: `throw ;`, `throw\u{FEFF};`, `throw\x0B;`) parses
/// to the bare rethrow and BINDS (C4/C5/C7 sg n1); a run of Rust-whitespace
/// outsiders (`throw\u{2028};` — U+2028/U+0085/U+2029/U+202F are NOT sg cs
/// trivia) parses to a DIFFERENT root and binds NOTHING on any candidate
/// (C1/C2/C3/C6 sg rc1 `[]` — the accepted-empty class). Anything else is
/// not this family (the operand spellings keep their kind-lane faces).
enum CsThrowSemiParse {
    Binds,
    AcceptedEmpty,
}

fn cs_throw_semi_parse(pattern: &str) -> Option<CsThrowSemiParse> {
    let rest = pattern.trim().strip_prefix("throw")?;
    if rest.is_empty() {
        // The bare `throw` spelling keeps its kind-lane face (F2_throwbare_*).
        return None;
    }
    let body = rest.strip_suffix(';')?;
    if body.is_empty() || body.chars().all(is_sg_cs_trivia) {
        return Some(CsThrowSemiParse::Binds);
    }
    // Rust-whitespace-only but not sg-class trivia: the accepted-empty class.
    (!body.trim().is_empty() && body.chars().all(char::is_whitespace))
        .then_some(CsThrowSemiParse::AcceptedEmpty)
}

fn cs_throw_semi_pattern(pattern: &str) -> bool {
    cs_throw_semi_parse(pattern).is_some()
}

/// PASS 141 (141A-F4 seam law) + PASS 142 (142B-F1) + PASS 143 (142E-F1
/// annotation): the sg cs trivia class of record — the PATTERN-side class.
/// sg's cs grammar extras are /[\s\u00A0\uFEFF\u3000]+/ whose `\s` the sg
/// grammar generation compiles with UNICODE semantics (143 grid), but the
/// sg PATTERN parse answers these outsider spellings accepted-empty at the
/// nested-head seam (grids D_d_*/C_*; 142E re-verified the seam posture).
/// At the throw-semi/using TOKEN GAPS sg's candidate parse accepts the full
/// Unicode White_Space set — that is [`is_sg_cs_gap_junk`], NOT this class.
fn is_sg_cs_trivia(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{00A0}' | '\u{FEFF}' | '\u{3000}'
    )
}

/// PASS 143 (142E-F1): the cs CANDIDATE-side gap class. sg's pinned cs
/// grammar skips the FULL Unicode White_Space set plus U+FEFF at every
/// throw-semi/using token gap, and bare control junk (U+0000/U+001A/U+007F)
/// recovers as skipped ERROR tokens — the oracle binds all of those
/// spellings (phase143R grid cs_throw_*/cs_using_*/cs_global_* and the
/// cs_gap_*/cs_name_glue_* neighbors). The subject's tree-sitter-c-sharp
/// 0.23.5 extras are ASCII-scoped, so those bytes survive as ERROR
/// children; the candidate scanners treat such junk runs as sg-transparent
/// while junk glued INSIDE a name token still refuses (sg cs_name_glue n0).
fn is_sg_cs_gap_junk(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}' || c.is_control()
}

/// PASS 143 (142E-F1): the php gap-trivia class of record at the use-kind
/// clause gaps — sg's php grammar skips exactly ASCII whitespace, U+00A0,
/// U+FEFF and U+001A there (grid php_use_* bind-set). Every other
/// Rust-whitespace spelling GLUES into the surrounding php name token
/// (name chars run to U+0080+), which is why the zero-gap glue cells
/// refuse and why glue chars belong to name-token text.
fn is_sg_php_gap_trivia(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{00A0}' | '\u{FEFF}' | '\u{001A}'
    )
}

/// PASS 142 (142A-F1): strip `// …` and `/* … */` spans from directive
/// demarcation text. sg walks the AST where comments are extras, so the
/// candidate-side directive scans see the comment-FREE clause text while
/// the emitted span/MATCH keep the original bytes. No string-literal
/// awareness on purpose: directive demarcation zones carry no string
/// syntax (grids A1-A20).
fn strip_comment_spans(text: &str) -> String {
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

fn match_cs_throw_semi(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let tree = parse_source(Language::CSharp, source).ok()?;
    let mut out = Vec::new();
    walk_cs_throw_semi(tree.root_node(), source, pattern, &mut out);
    Some(out)
}

/// PASS 144 (143A-F1): the cs `return;` family lane — the exact `return;`
/// spelling only (the bare `return` kind face and the operand spellings keep
/// their routes). sg binds the operand-less return_statement across the
/// candidate gap-junk class (grid R1-R6/R12: Unicode White_Space outsiders,
/// FEFF, control junk, junk runs) and across comments (R5), while the
/// structural general lane's child alignment refused every junk-carrying
/// candidate silently (su n0 SILENT). Operand-bearing candidates refuse
/// (R9/R10 sg rc1 — the `;`-ful cs spelling is operand-less discipline,
/// the 134A-F2 js/ts law); glued `returnx` refuses (R8).
fn match_cs_return_semi(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let tree = parse_source(Language::CSharp, source).ok()?;
    let mut out = Vec::new();
    walk_cs_return_semi(tree.root_node(), source, pattern, &mut out);
    Some(out)
}

fn walk_cs_return_semi(node: Node, source: &str, pattern: &str, out: &mut Vec<PatternMatch>) {
    if node.kind() == "return_statement" && !is_in_comment_or_string(&node) {
        // tree-sitter-c-sharp: return_statement = 'return' expression? ';'.
        // The junk-gap ERROR child (a run the subject's cs parser could not
        // skip — ASCII-scoped extras) is sg-transparent trivia, not an
        // operand, mirroring walk_cs_throw_semi's PASS 143 exemption and the
        // break/continue/yield siblings (grid A06-A08). Comments are trivia
        // kinds; a real operand refuses.
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
                let (line_start, line_end) = node_lines(&node, source);
                let excerpt = excerpt_for_node(&node, source, pattern);
                let mut captures = BTreeMap::new();
                captures.insert("MATCH".to_string(), text.to_string());
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
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_cs_return_semi(child, source, pattern, out);
    }
}

fn walk_cs_throw_semi(node: Node, source: &str, pattern: &str, out: &mut Vec<PatternMatch>) {
    if node.kind() == "throw_statement" && !is_in_comment_or_string(&node) {
        // tree-sitter-c-sharp: throw_statement = 'throw' expression? ';' —
        // the operand-free rethrow has NO named child.
        // PASS 143 (142E-F1): a junk-gap ERROR child (a Unicode
        // White_Space/FEFF/control run the subject's cs parser could not
        // skip) is sg-transparent trivia, not an operand — sg binds the bare
        // rethrow across those spellings (grid cs_throw_*); junk glued to a
        // real operand still refuses (cs_throw_operand_2028 n0).
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
                let (line_start, line_end) = node_lines(&node, source);
                let excerpt = excerpt_for_node(&node, source, pattern);
                let mut captures = BTreeMap::new();
                captures.insert("MATCH".to_string(), text.to_string());
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
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_cs_throw_semi(child, source, pattern, out);
    }
}

/// PASS 141 (F7, grid F7_kt_*): the kotlin `companion object { $B }` /
/// `init { $B }` spellings are sg ACCEPTED-EMPTY — the pattern parses but
/// binds NOTHING on any candidate (rc1 `[]` even on the matching
/// declaration shape), so the walk's empty IS the agreement. The body must
/// be one canonical meta; any other spelling keeps its prior route.
fn kt_binds_nothing_template(pattern: &str) -> bool {
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

/// PASS 141 (F7, grid F7_sw_deinit): the swift `deinit { $B }` spelling is
/// sg ACCEPTED-EMPTY (rc1 `[]` — sg never binds the deinit body); the
/// walk's empty IS the agreement.
fn swift_binds_nothing_template(pattern: &str) -> bool {
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

/// F-r40-5 (r41): true when `pattern` (already trimmed) consists ONLY of
/// semicolons and ASCII whitespace and carries TWO OR MORE semicolons — a
/// degenerate statement-only pattern whose roots are multiple empty
/// statements (sg's "Multiple AST nodes are detected" rc8 class; see the
/// language scope in [`native_pattern_answerable`]). A single `;`, any
/// `$`-carrying spelling, and any pattern with non-whitespace code never
/// enter this class.
fn degenerate_semicolon_roots(pattern: &str) -> bool {
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

/// PASS 94b (r44, FB-93A-1): the sg 0.45.2 pattern-acceptance gate, ported
/// faithfully from `PatternBuilder::single` (ast-grep-core 0.45.2
/// src/matcher/pattern.rs): pre-process the pattern (the conditional sigil
/// rewrite of ast-grep-language 0.45.2 `pre_process_pattern`), parse it
/// under the language's grammar, and require the root to be a SINGLE node
/// (`is_single_node`: one child, or two where the second is a
/// missing/empty-kind token). sg rejects multi-root patterns with
/// "Multiple AST nodes are detected" (rc8); ERROR nodes never reject and
/// there is no descent at acceptance time. This replaces the pass-92
/// textual `) {` two-statement scan, whose 7-language scope both
/// over-refused (the sg accepted-empty IIFE / `$f($A) {` cells went loud
/// rc2) and under-refused (the rb/swift/kt/java compound spellings sg rc8s
/// walked silent-empty, and the go/py/java semicolon statement roots
/// answered hits). Grammar drift is sharpened in one probed spot: php
/// without an opening tag folds the whole document into a single `text`
/// node, so the bare parse cannot count statement roots — sg splits them on
/// `;` (php `$A ;;` / `; $A` / `;;;` probed rc8), so two or more
/// quote-external semicolons refuse. The kotlin residuals (this repo pins
/// tree-sitter-kotlin-ng, sg ships the fwcd grammar, which folds `;` into
/// call roots differently) keep the registered silent class — form-1
/// predicate: adopt sg's pinned grammar. Validated cell-by-cell against the
/// 230-cell probe matrix (artifacts/conformance/pass94b/probes_run1.jsonl,
/// oracle 0.45.2 ATTACHED 2026-09-08): 200/211 R-matrix pattern cells agree
/// before the php sharpening, all 62 divergent cells resolve to sg truth
/// after it modulo the registered residuals.
fn sg_pattern_gate_accepts(lang: Language, pattern: &str) -> bool {
    if lang == Language::Php && quote_external_semicolon_count(pattern) >= 2 {
        return false;
    }
    let doc = sg_preprocess_pattern(lang, pattern);
    let Ok(tree) = parse_source(lang, &doc) else {
        return false;
    };
    let root = tree.root_node();
    // PASS 94b: sg's pinned grammars (older minors than this workspace's
    // python/javascript/java grammars) recover parse errors as a SINGLE
    // ERROR root where these grammars split the document into an ERROR
    // fragment plus the recovered remainder. sg ACCEPTS those faces in the
    // probed languages (registered silent witnesses py `$a = 1` / js
    // `$x # note` probed accepted-empty; java `$A ;` probed hits), so an
    // ERROR-led two-fragment PAREN-FREE root is accepted THERE — every
    // probed acceptance is a paren-free statement face, while the
    // paren-bearing compounds rc8 (`$f($A) $g($B)` py) or keep parse
    // verdicts elsewhere (csharp `$A ;` / `$f($A) ;` probed rc8). Three or
    // more fragments is a real multi-root (java `$A ;;` probed rc8) and
    // stays refused.
    is_sg_single_node(root)
        || (matches!(lang, Language::Python | Language::JavaScript | Language::Java)
            && !pattern.contains('(')
            && root.child_count() == 2
            && root.child(0).is_some_and(|c| c.kind() == "ERROR"))
}

/// PASS 96 (FB-95B-2): the BARE sg acceptance (preprocess + parse +
/// `is_sg_single_node`) WITHOUT the documented workspace-grammar-drift arm —
/// the discriminator the bracket-fragment class rides. A face sg's own parse
/// splits into multiple roots (ruby/swift `$A ]`, js/ts `$A }` — probed rc8)
/// must NOT enter the sg-accepted-empty fragment admission, and those are
/// exactly the parses where the bare single-node check refuses while the
/// 2-fragment drift arm would accept.
fn sg_pattern_gate_bare_single(lang: Language, pattern: &str) -> bool {
    if lang == Language::Php && quote_external_semicolon_count(pattern) >= 2 {
        return false;
    }
    let doc = sg_preprocess_pattern(lang, pattern);
    let Ok(tree) = parse_source(lang, &doc) else {
        return false;
    };
    is_sg_single_node(tree.root_node())
}

/// PASS 98 (F-97A-2): the SHAPE half of the bracket-fragment class — paren
/// -free, ends in an unbalanced `]`/`)`/`}` closer. Shared by the fragment
/// admission below and by the `$`-less census arm of
/// [`native_pattern_answerable`], so both spellings consult the identical
/// per-language gate.
fn sg_bracket_fragment_shape(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.contains('(') {
        return false;
    }
    let Some(tail) = p.chars().last() else {
        return false;
    };
    if !matches!(tail, ']' | ')' | '}') {
        return false;
    }
    let (open, close) = match tail {
        ']' => ('[', ']'),
        ')' => ('(', ')'),
        _ => ('{', '}'),
    };
    let opens = p.chars().filter(|c| *c == open).count();
    let closes = p.chars().filter(|c| *c == close).count();
    closes > opens
}

/// PASS 96 (FB-95B-2): true for the paren-free ERROR-repair BRACKET-fragment
/// faces sg 0.45.2 accepts and answers EMPTY (matrix mf_bracket_fragments:
/// `$A ]` / `$A )` / `$A }` across rust/go/py/php/kt/cs/c/cpp/java/js/ts —
/// sg parses the stray closer into a single ERROR root that matches no clean
/// source node). The subject keeps these walk-admissible (the walk's empty
/// IS the sg agreement) instead of ingress-rc2, while sg-rc8 spellings stay
/// census-loud: the class requires the BARE single-node gate, so ruby/swift
/// (workspace parse splits, 2 roots) and js/ts `$A }` keep the registered
/// loud fold. `;`/operator tails are NOT brackets and keep their registered
/// classes (90B-T4 `$A ;`, trailing-operator `$A +`). `pattern` must already
/// be general-lane-unsupported at the call sites (the walk answering empty
/// is what makes the face an honest empty).
fn sg_bracket_fragment_accepted_empty(lang: Language, pattern: &str) -> bool {
    if !sg_bracket_fragment_shape(pattern) {
        return false;
    }
    sg_pattern_gate_bare_single(lang, pattern.trim())
}

/// PASS 94b: the number of `;` bytes OUTSIDE string-literal quotes — the
/// statement-root counter behind the php sharpening. Quote-aware
/// like the rest of the gates; a `;` inside a string literal is text, not
/// a root.
fn quote_external_semicolon_count(pattern: &str) -> usize {
    let bytes = pattern.as_bytes();
    let mut quote: Option<u8> = None;
    let mut semis = 0usize;
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
            b'\'' | b'"' => quote = Some(b),
            b';' => semis += 1,
            _ => {}
        }
        i += 1;
    }
    semis
}

/// sg 0.45.2 `pre_process_pattern` (ast-grep-language 0.45.2 src/lib.rs):
/// CONDITIONAL sigil replacement — a `$` run is rewritten to the language's
/// expando char when the run is followed by `[A-Z_]` or when it is a
/// 3-dollar run; a lowercase-led run (`$a`) stays LITERAL `$` text (php
/// variable syntax / js-ts identifier syntax). Java, JavaScript and
/// TypeScript have no preprocessing (sg uses the raw `$` there), and no
/// language here gets the php `<?php ` pattern wrapping this crate's other
/// gates apply (sg parses php pattern documents bare).
/// PASS 96 (F-95A-1): the language's expando char — the shared table behind
/// both sg 0.45.2 halves: `pre_process_pattern` (the gate, below) AND
/// `extract_meta_var` (the matcher). C/Cpp use U+10000, CSharp/Go/Kotlin/
/// Php/Python/Ruby/Rust/Swift use µ, and Java/JavaScript/TypeScript have no
/// expando (sg `impl_lang!` trait defaults — `None` here).
fn sg_expando_char(lang: Language) -> Option<char> {
    match lang {
        Language::C | Language::Cpp => Some('\u{10000}'),
        Language::CSharp | Language::Go | Language::Kotlin | Language::Php
        | Language::Python | Language::Ruby | Language::Rust | Language::Swift => Some('µ'),
        Language::Java | Language::JavaScript | Language::TypeScript => None,
        // Dart identifiers may contain `$` (and strings interpolate `$var`),
        // so it joins the µ-expando group; MoonBit has no `$` syntax at all,
        // so `$` stays the raw metavariable sigil (the Java/JS/TS group).
        Language::Dart => Some('µ'),
        Language::MoonBit => None,
    }
}

fn sg_preprocess_pattern(lang: Language, pattern: &str) -> String {
    let Some(expando) = sg_expando_char(lang) else {
        return pattern.to_string();
    };
    let mut ret = String::with_capacity(pattern.len());
    let mut dollar_count = 0usize;
    for c in pattern.chars() {
        if c == '$' {
            dollar_count += 1;
            continue;
        }
        let expando_run = c.is_ascii_uppercase() || c == '_' || dollar_count == 3;
        let sigil = if expando_run { expando } else { '$' };
        for _ in 0..dollar_count {
            ret.push(sigil);
        }
        dollar_count = 0;
        ret.push(c);
    }
    for _ in 0..dollar_count {
        ret.push(if dollar_count == 3 { expando } else { '$' });
    }
    ret
}

/// PASS 96 (F-95A-1): sg 0.45.2 treats expando-spelled identifiers as
/// METAVARIABLES at pattern build/match time — `Pattern::try_new` runs
/// `pre_process_pattern` and then `convert_node_to_pattern` calls
/// `lang.extract_meta_var(node_text)` (ast-grep-core 0.45.2
/// src/matcher/pattern.rs:249 → src/meta_var.rs:235), whose rules over an
/// expando run of length n with tail T are:
/// - n==1 (`µA`) or n==2 (`µµA`): meta iff T = `[A-Z_][A-Z_0-9]*`
///   (Capture/Dropped);
/// - n==3 (`µµµ`/`µµµA`/`µµµ_`): meta iff T empty or T = `[A-Z_0-9]+`
///   (Multiple/MultiCapture — digit-led tails included, the ellipsis branch);
/// - n>=4 or a non-matching tail (`µµµµ`, `µabc`, `f(µµ)`): NOT a meta — a
///   plain identifier sg matches literally.
/// PASS 98 (FB-97B-1): the validation runs over the ENTIRE parsed node text
/// (meta_var.rs:260-264 — `!trimmed.chars().all(is_valid_meta_var_char)` ⇒
/// None), so a token that CONTINUES past the `[A-Z_0-9]` prefix with an
/// identifier character (`µAble`, `µAx`) is a LITERAL node, never a meta.
/// PASS 98 (F97X-0072): sg's preprocess maps EVERY `$` of a run to the
/// expando char, so a µ-run immediately followed by `$`s is ONE combined run
/// in sg's expando space (py `µµ$A` preprocesses to `µµµA` = MultiCapture).
/// This scanner returns the byte spans of the META-shaped expando runs — with
/// the combined run count and the NAME-tail start — under those rules (the
/// exact inverse of the `pre_process_pattern` rewrite: sg's own preprocessing
/// makes `$A` and `µA` THE SAME parsed pattern, so rewriting the meta-shaped
/// runs back to `$` makes the subject's `$` machinery see the identical
/// metavariable structure).
struct ExpandoSpan {
    /// Byte offset of the first expando char of the run.
    start: usize,
    /// Byte offset of the NAME tail inside the span (after the run — expando
    /// chars plus any absorbed `$`s).
    name_start: usize,
    /// End of the `[A-Z_0-9]` tail = end of the rewritten span.
    end: usize,
    /// Combined run count to emit as `$`s (expando chars + absorbed `$`s).
    run_count: usize,
}

fn expando_meta_spans(
    pattern: &str,
    expando: char,
    suffix_continuations: &[char],
) -> Vec<ExpandoSpan> {
    let mut spans = Vec::new();
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let Some(ch) = pattern[i..].chars().next() else { break };
        if ch != expando {
            i += ch.len_utf8();
            continue;
        }
        let mut run = 0usize;
        while pattern[i + run..].chars().next() == Some(expando) {
            run += expando.len_utf8();
        }
        let mut run_count = run / expando.len_utf8();
        // F97X-0072: absorb an adjacent `$` run — sg's preprocess turns it
        // into expando chars too, so `µµ$A` is the 3-run `µµµA`, not a µ-run
        // plus a separate `$A` token.
        let mut name_start = i + run;
        while pattern[name_start..].starts_with('$') {
            name_start += 1;
            run_count += 1;
        }
        let mut end = name_start;
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase() || bytes[end] == b'_' || bytes[end].is_ascii_digit())
        {
            end += 1;
        }
        let tail = &bytes[name_start..end];
        let first_ok = tail
            .first()
            .is_some_and(|b| b.is_ascii_uppercase() || *b == b'_');
        let tail_all_valid = tail
            .iter()
            .all(|b| b.is_ascii_uppercase() || *b == b'_' || b.is_ascii_digit());
        // FB-97B-1: whole-node validation. sg's `extract_meta_var` runs on the
        // full node text, and every probed grammar keeps ascii-lowercase and
        // non-ASCII letters INSIDE one identifier node — so any such
        // continuation after the valid prefix means the node text cannot
        // match the meta grammar (`µAble` is a literal identifier). The
        // literal verdict is the sg-lean direction: rewriting the truncated
        // prefix (`µA` + junk) was the F-97A-1 silent miss.
        // PASS 100 (FB-99B-1): ruby folds the `?` method-name suffix INTO
        // the identifier node (`µA?` is ONE call/identifier node — sg's
        // whole-node validation reads `µA?` as a literal and answers the
        // verbatim rows), so an immediately adjacent `?` is node-text
        // continuation too and the run must NOT fold into a meta. The `!`
        // twin is NOT admitted: in expression position `!` is the negation
        // operator (sg parses `µA!` as µA + `!` and rc8s), so `µA!` keeps
        // its registered loud meta fold.
        let continues_ident = pattern[end..]
            .chars()
            .next()
            .is_some_and(|c| {
                c.is_ascii_lowercase()
                    || !c.is_ascii()
                    || suffix_continuations.contains(&c)
            });
        let is_meta = !continues_ident
            && match run_count {
                1 | 2 => first_ok && tail_all_valid,
                3 => tail.is_empty() || tail_all_valid,
                _ => false,
            };
        if is_meta {
            spans.push(ExpandoSpan {
                start: i,
                name_start,
                end,
                run_count,
            });
            i = end;
        } else {
            // Literal token: skip past the WHOLE node text (run + absorbed
            // `$`s + tail + ident continuation) so a later expando char
            // inside the same literal node is not rescanned as a new run
            // (`µµAµB` is one literal node in sg, never `µµA` + meta `µB`).
            let mut skip = end;
            while let Some(c) = pattern[skip..].chars().next() {
                if c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii() {
                    skip += c.len_utf8();
                } else {
                    break;
                }
            }
            i = skip;
        }
    }
    spans
}

/// PASS 98 (F-97A-1 layer 2): byte spans of 1-run UPPERCASE-led MixedCase
/// `$`-tokens (`$Bx` — uppercase-led with a lowercase continuation). sg's
/// preprocess maps the run to the expando char and `extract_meta_var`
/// then REJECTS the whole node text (the lowercase continuation), so sg
/// parses the token as literal code and answers the verbatim rows under BOTH
/// spellings (matrix m3a2, probed 2026-09-09 across rust/py/go/java/c/cpp/
/// php/ruby/swift/kotlin/csharp). Rewriting the run to the expando spelling
/// hands the pattern to the plain `$`-less literal lane on exactly the bytes
/// sg's parse matches. Underscore-led mixed tokens (`$_a` — the f92 php
/// brace-compound faces), multi-run mixed (`$$Bx`), lowercase-led (`$x`) and
/// canonical runs never enter this scanner — they keep their registered
/// classes.
fn dollar_mixed_case_literal_spans(pattern: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut dollars = 0usize;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        if dollars == 1 {
            let name_start = i + 1;
            let mut name_end = name_start;
            while name_end < bytes.len()
                && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
            {
                name_end += 1;
            }
            if name_end > name_start
                && bytes[name_start].is_ascii_uppercase()
                && dollar_name_class(pattern.get(name_start..name_end).unwrap_or(""))
                    == Some(DollarTokenClass::MixedCase)
            {
                spans.push(i..name_end);
                i = name_end;
                continue;
            }
        }
        i += dollars;
    }
    spans
}

/// PASS 100 (FB-99B-1): byte spans of 1-run CANONICAL `$`-tokens
/// immediately followed by ruby's `?` method-name suffix (`$A?`). sg's
/// preprocess maps the run to the expando char and the ruby grammar folds
/// the suffix INTO the identifier node, so `extract_meta_var` rejects the
/// whole node text (`µA?` — the trailing `?`) and sg parses the token as a
/// LITERAL, answering the verbatim rows under BOTH spellings (m2 oracle
/// sets: `µA?` {1,5,6} == `$A?`). Rewriting the run to the expando spelling
/// (`$A?` → `µA?`) hands the pattern to the `$`-less literal lane on the
/// exact bytes sg's parse matches — the µ≡$ twin convergence. MixedCase
/// names keep the [`dollar_mixed_case_literal_spans`] arm (same
/// destination), lowercase-led names keep their registered classes, and
/// multi-run tokens never enter this scanner.
fn dollar_suffix_literal_spans(
    pattern: &str,
    suffixes: &[char],
) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    if suffixes.is_empty() {
        return spans;
    }
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut dollars = 0usize;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        if dollars == 1 {
            let name_start = i + 1;
            let mut name_end = name_start;
            while name_end < bytes.len()
                && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
            {
                name_end += 1;
            }
            let canonical = name_end > name_start
                && dollar_name_class(pattern.get(name_start..name_end).unwrap_or(""))
                    == Some(DollarTokenClass::Canonical);
            let suffixed = pattern[name_end..]
                .chars()
                .next()
                .is_some_and(|c| suffixes.contains(&c));
            if canonical && suffixed {
                spans.push(i..name_end);
                i = name_end;
                continue;
            }
        }
        i += dollars;
    }
    spans
}

/// PASS 96 (F-95A-1): rewrite expando META runs to `$` runs so the native
/// `$` machinery (classification, templates, the sg gate's own preprocess)
/// sees exactly the pattern sg's `extract_meta_var` would extract. Non-meta
/// expando text stays verbatim (literal-identifier faces keep their sg
/// literal class), and languages without an expando are returned untouched.
/// PASS 98 (FB-97B-1): the meta decision is whole-node, so `µAble`-class
/// tokens stay verbatim instead of being truncated into non-canonical
/// `$`-tokens. PASS 98 (F97X-0072): a µ-run immediately followed by `$`s
/// composes into one combined `$`-run (`µµ$A` → `$$$A` semantics). PASS 98
/// (F-97A-1 layer 2): 1-run MixedCase `$`-tokens are rewritten the INVERSE
/// way — to the expando spelling — because sg's parse of them is literal
/// expando-space code (`$Bx` ≡ `µBx`); the plain `$`-less literal lane then
/// answers the same verbatim rows sg answers.
/// Note the gated convergence: `preprocess(normalize(src)) == preprocess(src)`
/// for every spelling, so the parse-shape gate sees sg's exact bytes either
/// way. Pathological hand-mixed runs (`$µA`) normalize where sg's blind byte
/// preprocess would keep them literal — the disclosed boundary of a
/// byte-level ingress normalization (sg extracts from PARSED node text).
fn normalize_expando_meta_spelling(lang: Language, pattern: &str) -> Cow<'_, str> {
    let Some(expando) = sg_expando_char(lang) else {
        return Cow::Borrowed(pattern);
    };
    // PASS 100 (FB-99B-1): ruby folds the `?` method-name suffix into the
    // identifier node, so `µA?`/`$A?` are literal node texts in sg — the
    // suffix blocks the meta fold on the µ side and inversely rewrites the
    // `$` spelling onto the expando bytes on the other.
    let suffix_continuations: &[char] = if lang == Language::Ruby { &['?'] } else { &[] };
    let meta_spans = expando_meta_spans(pattern, expando, suffix_continuations);
    let mixed_spans = dollar_mixed_case_literal_spans(pattern);
    let suffix_spans = dollar_suffix_literal_spans(pattern, suffix_continuations);
    if meta_spans.is_empty() && mixed_spans.is_empty() && suffix_spans.is_empty() {
        return Cow::Borrowed(pattern);
    }
    // Merge both span sets in byte order — disjoint by construction (µ-spans
    // start at the expando char, mixed `$`-spans at `$`; a µ-span's absorbed
    // `$`s are followed by a META name, which the mixed scanner rejects).
    let mut edits: Vec<(usize, usize, usize, Option<usize>)> = meta_spans
        .into_iter()
        .map(|s| (s.start, s.name_start, s.end, Some(s.run_count)))
        .collect();
    edits.extend(
        mixed_spans
            .into_iter()
            .map(|r| (r.start, r.start + 1, r.end, None)),
    );
    edits.extend(
        suffix_spans
            .into_iter()
            .map(|r| (r.start, r.start + 1, r.end, None)),
    );
    edits.sort_by_key(|(start, _, _, _)| *start);
    let mut out = String::with_capacity(pattern.len());
    let mut last = 0usize;
    for (start, name_start, end, to_dollar) in edits {
        out.push_str(&pattern[last..start]);
        match to_dollar {
            // The expando RUN (plus absorbed `$`s) maps 1:1 to `$`s; the NAME
            // tail is copied verbatim (`µµA` -> `$$A`, `µµ$A` -> `$$$A`).
            Some(run_count) => {
                for _ in 0..run_count {
                    out.push('$');
                }
            }
            // The inverse rewrite: one MixedCase `$` becomes the expando char
            // (`$Bx` -> `µBx`), the literal NAME tail is copied verbatim.
            None => out.push(expando),
        }
        out.push_str(&pattern[name_start..end]);
        last = end;
    }
    out.push_str(&pattern[last..]);
    Cow::Owned(out)
}

/// PASS 96 (F-95A-1): sg's `PatternBuilder::build` REFUSES a pattern whose
/// root is a bare multi meta variable (`PatternError::RootMultiMetaVar`,
/// exit 8 — `$$$`, `$$$NAME`, `$$$_`; the r44 gate ported acceptance but not
/// this check, which sg runs AFTER the single-node parse). The check runs on
/// the NORMALIZED spelling, so `µµµ`-style roots fold to the registered loud
/// class instead of the fail-open silent empty.
fn sg_root_multi_meta_pattern(pattern: &str) -> bool {
    let Some(rest) = pattern.trim().strip_prefix("$$$") else {
        return false;
    };
    rest.is_empty()
        || rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit())
}

/// PASS 63 (H-CONF-031): language-aware answerability of a
/// classifier-accepted shape. Decl/call shapes must parse under `lang`'s
/// grammar through sg's pattern-acceptance gate; the registered
/// cross-language kinds (If normalization, `$$A` universal, NeverMatches
/// accepted-empty) stay answerable everywhere, and shapes whose
/// substitution refuses (bare `$$$` rest args — the registered rest-arg
/// faces) are not probed.
fn native_kind_language_answerable(lang: Language, pattern: &str, kind: &NativeKind) -> bool {
    // PASS 131 (130A-F6, f131e): a concrete-condition if face is answerable
    // exactly in the d2-grid-receipted grammars AND when the cond template
    // actually builds under that grammar — unprobed grammars and
    // unbuildable conds keep the census loud (fail-closed). The meta-cond
    // spelling keeps its registered unconditional answerability below.
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
        // PASS 94b (FB-93A-1): a NeverMatches face is answerable exactly
        // when sg's own pattern-acceptance gate accepts the spelling — the
        // parse decides root multiplicity (sg rc8 "Multiple AST nodes" for
        // the compound spellings), replacing the pass-92 textual `) {` scan
        // whose 7-language scope over-refused the accepted-empty IIFE /
        // `$f($A) {` cells and under-refused the rb/swift/kt/java
        // compounds. The registered silent NeverMatches cells (`$a = 1`,
        // rust/js `$x # note`, php `$A && $b`) parse to single (ERROR/text)
        // roots and keep their answerability.
        NativeKind::NeverMatches => sg_pattern_gate_accepts(lang, pattern),
        // PASS 131 (130A-F8, f131f): an `interface` member-count template is
        // answerable ONLY in the receipted grammars (sg oracle d2_iface +
        // d4: ts/java bind; the rest refuse or are unreceipted). PASS 135
        // (grids F_cs_iface1/F_cs_iface2/X_cs_iface_*): csharp joins the
        // receipted set — 1-member plain binds (sg n1 N/B), 2-member/
        // extends/base-list/modifier/gap-trivia refuse (sg rc1) — and the
        // ts `type-alias` object face is receipted for TypeScript only
        // (F_ts_alias_multi/one sg n1, F_ts_alias2 rc1).
        // Other Class faces keep the sg pattern-parse gate byte-for-byte.
        NativeKind::Class {
            keyword,
            body: Some(BodyTemplate::Exactly(_)),
            ..
        } if *keyword == "interface" => {
            matches!(lang, Language::TypeScript | Language::Java | Language::CSharp)
        }
        NativeKind::Class {
            keyword,
            body: Some(BodyTemplate::Exactly(_)),
            ..
        } if *keyword == "type-alias" => matches!(lang, Language::TypeScript),
        NativeKind::Function { .. } | NativeKind::Class { .. } | NativeKind::Call { .. } => {
            // PASS 142 (142A-F3, grid E8): the `async def` `$$`-slot face is
            // served by the dedicated async-def lane — the general
            // Function route's sg parse gate refuses the multi-line
            // spelling, which starved census-loud where sg binds n1.
            if lang == Language::Python && py_async_def_template(pattern).is_some() {
                return true;
            }
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
        // PASS 115 (114A-F1/F2): a property-segment face (a mid-chain
        // argument-free link) classified for the FIRST time in 115. Outside
        // the registered family those languages keep the general lane they
        // rode pre-115 — answerable exactly when that lane can build the
        // template (kotlin/swift/rust clean `a?.b?.c($X)` faces answer
        // sg-exactly through it; a `$$$`-rest chain builds nowhere and stays
        // census-loud, the registered fail-closed class). All-call chains
        // keep the historical gate byte-for-byte.
        NativeKind::OptionalCallChain { segments, .. } => {
            // PASS 117: a mixed-rest slot list is a CALL segment, not a
            // property face — keep the predicate in step with the walker's
            // `segment_is_call`.
            let property_face = segments.iter().skip(1).any(|segment| {
                segment.args.is_none() && segment.arg_slots.is_none()
            });
            sg_pattern_parses(lang, pattern)
                && if property_face {
                    matches!(lang, Language::TypeScript | Language::JavaScript)
                        || (cached_general_template(lang, pattern).is_some()
                            && sg_pattern_gate_accepts(lang, pattern))
                } else {
                    matches!(lang, Language::TypeScript | Language::JavaScript)
                        // PASS 118 (117E-Info kt half): kotlin all-call `?.`
                        // chains. The walker's decompose/receiver machinery
                        // already pairs pattern-CALL segments with candidate
                        // CALL nodes — kotlin-ng nests mid-chain calls as
                        // real `call_expression` children of
                        // `navigation_expression` (first-hand CST dump
                        // 2026-09-10), and the connectors present as plain
                        // `.`/`?.` anonymous tokens — so the census loud is
                        // pure admission; the walk answers sg-exact (clean
                        // 3/4-link, `$$$`-rest, negative + connector-flag
                        // controls, f118b). Swift stays REFUSED: its
                        // `?`-connector + `navigation_suffix` link shape is
                        // outside `member_link_parts`' decomposition, so
                        // admission would walk silent-empty where sg answers
                        // (fail-open) — the swift census loud is the
                        // REGISTERED 117E-Info swift class (CNR §39.13).
                        || lang == Language::Kotlin
                }
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
        // PASS 81a (FB-80a-03): the assignment kind is produced only by the
        // php-only match_pattern hook — [`classify_native`] never returns it
        // (these faces keep the gate's NeverMatches class there) — so this
        // arm is unreachable from the answerable gate; the php grammar
        // parses the spelling, so keep the same acceptance gate for safety.
        NativeKind::Assignment { .. } => sg_pattern_parses(lang, pattern),
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
    let Some(link) = member_link_parts(lang, &field) else {
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

fn member_link_parts<'a>(lang: Language, node: &Node<'a>) -> Option<MemberLink<'a>> {
    let mut cursor = node.walk();
    let children: Vec<Node<'a>> = node.children(&mut cursor).collect();
    let mut base: Option<Node<'a>> = None;
    let mut leaf: Option<Node<'a>> = None;
    let mut optional: Option<bool> = None;
    // PASS 113 (112A-F1): sg's `?.`-chain decomposition is comment-
    // transparent — the chain twin of `faithful_path_from_node`'s PASS 111
    // skip. Without it the trivia child occupied the base/leaf slot and the
    // two-slot veto silently refused (js `a /*c*/ ?.b(1)` answered [] where
    // sg answers `a?.b($X)` / `a?.b($$A)` / `$A?.b($B)`; the same for both
    // comments around the token and for trivia AFTER the link in BOTH
    // grammars, oracle grid 2026-09-08). ONE grammar-position veto is kept:
    // trivia PRECEDING the NAMED `optional_chain` wrapper refuses the link
    // — but only for Language::TypeScript. sg 0.45.2 runs tree-sitter-
    // typescript for `.ts` (named `optional_chain` wrapper — its CST dump
    // shows `comment` as a direct member_expression sibling of the wrapper)
    // and refuses the commented link there, while `.js` runs tree-sitter-
    // javascript where `?.` is an ANONYMOUS token and sg answers. This
    // workspace parses BOTH js and ts with wrapping grammars (js maps to
    // TSX), so the named-child test alone cannot separate the faces and the
    // veto must be language-scoped. The veto cannot flip the wrapper's own
    // inner-token check below: the trivia is skipped, never a slot.
    let veto_trivia_before_wrapper = lang == Language::TypeScript;
    for (index, child) in children.iter().enumerate() {
        if is_trivia_kind(child.kind()) || child.is_extra() {
            if veto_trivia_before_wrapper && optional.is_none() {
                let next = children[index + 1..]
                    .iter()
                    .find(|next| !is_trivia_kind(next.kind()) && !next.is_extra());
                // PASS 144 (143A-F9, grid C*): the veto covers BOTH spellings
                // of the receiver→`?.` connector — the named `optional_chain`
                // wrapper AND the anonymous `?.` token the workspace's
                // grammar emits (the pre-fix wrapper-only test never fired
                // on `a /* c */?. b`, which sg's tree-sitter-typescript
                // refuses). Comment AFTER the connector keeps binding (C2 —
                // `optional` is set by then) and the js twin never vetoes
                // (CJ1 sg n1; the 113 language scope).
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
            let slot = if optional.is_none() { &mut base } else { &mut leaf };
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
        Language::Dart => tree_sitter_dart::LANGUAGE.into(),
        Language::MoonBit => tree_sitter_moonbit::LANGUAGE.into(),
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
    // PASS 96 (F-95A-1): normalize expando META spelling (`µA`/`𐐀A` runs)
    // to the `$` spelling sg's own `pre_process_pattern` produces, so every
    // lane below sees the metavariable structure sg's matcher extracts.
    let pattern: Cow<'_, str> = normalize_expando_meta_spelling(lang, pattern);
    let pattern = pattern.as_ref();
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
    // PASS 140 (grids /tmp/phase140R): the directive/import roots, go's
    // `$`-carrying `;`-ful accepted-empty spellings, the csharp checked/
    // unchecked EXPRESSION root, and the remaining statement roots —
    // spelling-level lanes ahead of the statement/literal routing (the
    // 137A-F2 placement discipline: `using $N;` must precede the csharp
    // statement lane, and the literal `using System;` must precede its
    // over-serving literal route, R2_cs_using_lit).
    if let Some(hits) = match_directive_root(lang, source, pattern) {
        return Ok(hits);
    }
    if lang == Language::Go
        && (sg_goto_semi_pattern(pattern) || sg_go_import_semi_pattern(pattern))
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
    // PASS 142 (142A-F3 + 142A-F5, grids E*/H*/J*/K*): the statement/decl
    // root lanes (rs mod/extern crate, go type, rb module, ts declare
    // module/const, py async def, ja enum, cs record/struct) and the
    // directive-family accepted-empty faces — the empty walk IS the sg
    // rc1 agreement.
    if directive_pattern_sg_accepts_empty(lang, pattern)
        || (lang == Language::CSharp && cs_using_plainface_meta_path(pattern))
    {
        return Ok(Vec::new());
    }
    if let Some(hits) = match_statement_root_142(lang, source, pattern) {
        return Ok(hits);
    }
    // PASS 137 (137A-F2): the csharp statement-head lane — fixed/checked/
    // unchecked/unsafe templates the general lane cannot root or align (see
    // the lane's header receipt block). Placed ahead of the `$`-less literal
    // routing because the fully-concrete spellings carry no metavariable.
    if lang == Language::CSharp {
        if let Some(hits) = match_csharp_statement(source, pattern) {
            return Ok(hits);
        }
    }
    // PASS 139 (139A-F2, grid139 B): the java synchronized META-body lane —
    // the bare-meta body cannot substitute through the general template (a
    // placeholder alone inside a block is a parse ERROR) and starved
    // census-loud where sg binds the block statement. Concrete bodies fall
    // through (parse refuses them) to their PASS 137 general-lane route.
    // PASS 139 (139A-F2 grid139 B + 139A-F5 grid139 E): the java dispatch —
    // the synchronized meta-body block face and the class member-count face
    // sg binds (B01/B02 n1, E01/E02/E08 n1).
    if lang == Language::Java {
        if let Some(hits) = match_java_synchronized_meta(source, pattern) {
            return Ok(hits);
        }
        // PASS 140 (grid D): the synchronized NESTED-block and METHOD faces
        // — the 139 bare-meta lane keeps the flat faces; these walks bind
        // the outermost statement sg-exactly (modifier hop-scan with
        // annotation blocking, the one-statement body law at every slot).
        if let Some(hits) = match_java_sync_block_nested(source, pattern) {
            return Ok(hits);
        }
        if let Some(hits) = match_java_sync_method(source, pattern) {
            return Ok(hits);
        }
        // PASS 139 (139A-F5, grid139 E): the java class member-count face —
        // sg binds the single-member class and refuses empty/multi-member/
        // heritage candidates (E01/E02/E08 n1; E03/E04/E07 rc1 `[]`).
        if let Some(kind) = classify_java_class_member_count(pattern) {
            return match_structural(lang, source, pattern, &kind);
        }
    }
    // PASS 139 (grid139 I): the php braced-namespace block face sg binds
    // n1 (`namespace $N { $B }` / global `namespace { $B }`).
    if lang == Language::Php {
        if let Some(hits) = match_php_namespace_block(source, pattern) {
            return Ok(hits);
        }
    }
    // PASS 141 (F7, grids F7_kt_*/F7_sw_deinit): the accepted-empty
    // kt/swift siblings — the walk's empty IS the sg agreement.
    if lang == Language::Kotlin && kt_binds_nothing_template(pattern) {
        return Ok(Vec::new());
    }
    if lang == Language::Swift && swift_binds_nothing_template(pattern) {
        return Ok(Vec::new());
    }
    if !pattern.contains('$') {
        // PASS 137 (f137j, D3 grid go_return_semi/rb_return_semi) + PASS 139
        // (grid139 F): the `;`-ful spelling is a DIFFERENT sg face on the
        // no-semicolon grammars — sg accepts the pattern (single statement
        // root, the `;` an anonymous tail) but binds NOTHING on any go/rb
        // candidate, while the bare keyword kind template answers (the kind
        // lane just below). The empty list IS the sg agreement (valid-empty).
        if sg_semi_keyword_accepted_empty(lang, pattern) {
            return Ok(Vec::new());
        }
        // PASS 141 (141A-F2, grid F2_*): the cs `throw;` family — the
        // CANDIDATE-side operand gate: the rethrow binds, operand-bearing
        // throw statements refuse. The bare `throw` spelling falls through
        // to the kind lane below (F2_throwbare_* n1 on both shapes).
        // PASS 142 (142B-F1, grid C_*): the pattern-side trivia class splits
        // the family — non-class Rust-whitespace interiors (U+2028 et al)
        // are sg ACCEPTED-EMPTY (C1/C2/C3/C6 rc1 `[]`); the walk's empty is
        // the agreement.
        if lang == Language::CSharp && cs_throw_semi_pattern(pattern) {
            if matches!(cs_throw_semi_parse(pattern), Some(CsThrowSemiParse::AcceptedEmpty)) {
                return Ok(Vec::new());
            }
            if let Some(hits) = match_cs_throw_semi(source, pattern) {
                return Ok(hits);
            }
        }
        // PASS 144 (143A-F1, grid R*): the cs bare-return semi family — the
        // operand-less walk with the 143 junk-gap exemption + comment
        // transparency (see [`match_cs_return_semi`]). The general lane's
        // child alignment refused every junk-carrying candidate silently.
        if lang == Language::CSharp && pattern.trim() == "return;" {
            if let Some(hits) = match_cs_return_semi(source, pattern) {
                return Ok(hits);
            }
        }
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
        // FB-84a-02 (r34): a bare connector token is sg's lenient-parse
        // garbage class — sg 0.45.2 parses `->`/`::` and answers NOTHING
        // (probed [] rc0, "parsed the pattern but it matched nothing"),
        // while the literal lane matched the anonymous token at every AST
        // connector site (silent rc0 fail-open). Never answer; every other
        // connector spelling keeps its own sg behavior (`=>` answers
        // array-pair sites, doubled spellings `->>` match nothing in the
        // literal lane naturally).
        if matches!(pattern, "->" | "::") {
            return Ok(Vec::new());
        }
        // 86a-M4 (r37): a bare `&&` or `.` is php garbage that answers
        // NOTHING — sg 0.45.3 parses `&&` leniently with an ERROR-node
        // warning and answers [] rc0, and rc1-refuses `.` (probed; answer
        // sets agree, the registered silent-vs-loud genus) — while the
        // literal lane matched the anonymous token at every `&&`/concat
        // site (silent rc0 fail-open). PHP-scoped on purpose: sg ANSWERS
        // bare `.` member/attr sites in js/py/rust/bash and bare `&&`
        // sites in js/bash (probed), so a cross-language carve would
        // over-refuse. `||`/`+`/`=>` keep their own sg-answering classes
        // (probed agreeing); `)`/`...`/`???` agree-empty in the literal
        // lane.
        if matches!(lang, Language::Php) && matches!(pattern, "&&" | ".") {
            return Ok(Vec::new());
        }
        // PASS 135 (134A-F1, grids A1/A9/A10): a csharp `lock`/`using`
        // statement pattern is a statement ROOT sg matches
        // layout-insensitively — the literal lane's byte equality answered
        // only the identical single-line spelling (A1) and silently dropped
        // the multi-line bodies sg binds (A9/A10). Route to the general
        // lane when the template builds (the root kinds are admitted in
        // `is_general_root_kind`); anything unbuildable keeps the literal
        // lane's faces (identifier spellings of the same heads included).
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
        // PASS 139 (grid139 G07): a fully-concrete del TEMPLATE spelling
        // rides the dedicated lane's structural unify — sg binds `del (x)`
        // × `del (x)` n1 (and byte-refuses `del (y)`) where the literal
        // route's leaf byte-match answered silent 0. Only del-template
        // spellings reroute; every other `$`-less pattern keeps the
        // literal lane.
        if lang == Language::Python && py_delete_template(pattern).is_some() {
            if let Some(hits) = match_py_delete_meta(source, pattern) {
                return Ok(hits);
            }
        }
        return match_literal_pattern(lang, source, pattern);
    }
    // PASS 135 (134B-F4, f135f, oracle grid /tmp/phase135/iso2/f.php): a php
    // STATIC-scope target (`C::$s = 5`, `C::$s = f(5)`, `C::$s == 5`) whose
    // `$`-tokens are all lowercase-literal rides dollar_literal_lane's
    // literal lane, whose exact-text arm answers only the byte-identical
    // spelling — sg 0.45.2 answers the padded candidate structurally
    // (`C :: $s = 5` row 4, `C :: $s == 5` row 7). Admit the assignment
    // hook AHEAD of the non-canonical gate for the static-scope family (the
    // PASS 81a precedent); the hook's own static_target operator discipline
    // (Assign|Augmented|Binary, sg-probed pass-131) stays the sg-exactness
    // boundary, and non-static lowercase faces (`$alpha = $beta`) keep the
    // literal lane.
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
    // PASS 69a (F68a-1): in languages where `$` is name syntax, sg parses
    // non-canonical `$`-tokens as literal code and answers the literal
    // faces; the honest-ladder rung (a) — answer sg-exactly through the
    // existing literal lane (exact-text + R3 structural arms) instead of
    // the silent NeverMatches empty.
    if dollar_literal_lane(lang, pattern) {
        return match_literal_pattern(lang, source, pattern);
    }
    // PASS 100 (FB-99A-2): in the no-expando languages (js/ts/java) a
    // pattern whose `$`-carrying identifier tokens ALL fail sg's
    // whole-token meta validation is pure literal code — sg parses the
    // glued tokens (`µµµ$A`, `$AµµB`, `$A$$B`, `foo$$$A`) as ordinary
    // identifiers and answers the verbatim rows (m1 oracle sets). The
    // canonical-spelling recognition inside those tokens (`$A` inside
    // `µµµ$A`) otherwise hijacked the pattern into the general lane, whose
    // leaf substitution (`µµµ__asgrep_mv_A`) can never equal the candidate
    // identifier text — the silent under-answer. The literal lane's
    // exact-text + R3 structural arms answer exactly sg's rows; existing
    // non-canonical lanes above keep their faces (checked first).
    if sg_expando_char(lang).is_none() && pattern_tokens_are_all_literal(pattern) {
        return match_literal_pattern(lang, source, pattern);
    }
    // PASS 135 (134A-F7, f135e, grid X_py_del_two): sg binds `del $X` to the
    // WHOLE operand-list text (`del x, y` → X=`x, y`), but the general
    // lane's placeholder unifies INSIDE the candidate's `expression_list`,
    // so a multi-operand list can never match (child-count mismatch) — the
    // silent under-answer. A python-scoped structural walk binds the list
    // node text sg-exactly; every other `del` spelling keeps its routes.
    if lang == Language::Python {
        if let Some(hits) = match_py_delete_meta(source, pattern) {
            return Ok(hits);
        }
        // PASS 137 (grid137 py_del_semi_loud): a `;`-ful del spelling whose
        // `;`-stripped form IS a del template (`del $O.$A;` and the whole
        // family — oracle multi-language probes 2026-09-11: rc1 `[]`
        // accepted-empty for `del $X;` / `del ($X);` / `del $A, $B;` /
        // `del $O.$A;`) answers the honest empty, never the loud class.
        // (Single-`-l` oracle runs rc8 the same spellings as a multi-node
        // parse — a registered mode divergence; the multi-language run is
        // the CLI's parity mode.)
        let del_trimmed = pattern.trim();
        if del_trimmed.starts_with("del") && del_trimmed.ends_with(';') {
            if let Some(stripped) = del_trimmed.strip_suffix(';') {
                if py_delete_template(stripped.trim()).is_some() {
                    return Ok(Vec::new());
                }
            }
        }
    }
    // PASS 137 (137A-D3 grid js_return_paren_lookalike/js_return_calllooka-
    // like_pat): `return ($X)` / `return($X)` are NOT call faces — sg 0.45.2
    // answers them n1 on `return (1);` binding X=`1` (the parenthesized
    // operand's INNER text), while the Call classification hijacked the
    // pattern into a callee-text match that can never fire (subject n0).
    // js/ts only: the grammars the grid pins.
    if matches!(lang, Language::JavaScript | Language::TypeScript) {
        if let Some(hits) = match_return_paren_meta(lang, source, pattern) {
            return Ok(hits);
        }
    }
    // PASS 81a (FB-80a-03): php-only admission ahead of the non-canonical
    // gate — a lowercase-led LHS with a bare canonical-meta RHS answers
    // through the dedicated assignment lane (the gate's NeverMatches class
    // would silent-empty the sg-answering face, the F78-2 fall-through
    // genus).
    if matches!(lang, Language::Php) {
        if let Some(kind) = classify_php_assignment(pattern) {
            return match_structural(lang, source, pattern, &kind);
        }
        // 88a-M2 (r39): a bare php binary-expression META template (no `=`
        // root, no literal LHS) — sg 0.45.2 answers the whole probed family
        // at expression level; classified faces walk the operand lane.
        if let Some(tpl) = classify_php_operand_template(pattern) {
            let tree = parse_source(lang, source)?;
            let mut out = Vec::new();
            walk_php_operand_template(&tree, source, pattern, &tpl, &mut out);
            return Ok(out);
        }
        // PASS 83a (FB-82a-05): a pattern ending in a DANGLING plain arrow
        // is sg's lenient ERROR repair to the FLAT member-call prefix (the
        // debug-query probe shows `ERROR(member_call_expression)`, and sg
        // answers exactly the flat prefix sites). Strip the arrow and serve
        // the flat face — restricted to chain-PREFIX candidates, since sg's
        // repaired pattern never answers the standalone statement spelling
        // (probed [] on `$o->c1($u);`). A nullsafe spelling anywhere in the
        // remainder keeps the refusal (sg answers the dangling-`?->` faces
        // []), and a chain/property-tail remainder keeps its registered
        // refusal (sg's chain-dangling answer rides ERROR-object
        // alignment — no sound structural rule reproduces it).
        if let Some(stripped) = pattern.strip_suffix("->") {
            let stripped = stripped.trim();
            if !stripped.is_empty() && !stripped.contains('?') && stripped.ends_with(')') {
                if let Some(NativeKind::MemberCall {
                    path,
                    arg_slots,
                    ..
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
    // PASS 124 (F2, f124b): the literal lane is the `$`-less member spelling
    // path (kt `a.b(1)`, swift §41.5 faces) — apply the same per-candidate
    // link-trivia doctrine the general lane consults.
    retain_member_link_trivia_free(lang, &tree, &mut matches);
    Ok(matches)
}

/// PASS 124 (F2, f124b): the swift/kt per-candidate member-link trivia
/// retain shared by the general structural lane and the literal lane —
/// drops candidates whose OWN subtree carries link-STRUCTURAL trivia
/// (receiver/mid link trivia before a later connector) where sg refuses,
/// keeps trivia-free candidates (the inner link of a longer chain) and
/// candidates whose trivia sits outside their span (tail comments).
fn retain_member_link_trivia_free(lang: Language, tree: &tree_sitter::Tree, out: &mut Vec<PatternMatch>) {
    if matches!(lang, Language::Swift | Language::Kotlin) && !out.is_empty() {
        let root = tree.root_node();
        out.retain(|m| {
            node_with_span(root, m.byte_start, m.byte_end)
                .map_or(true, |candidate| !member_link_trivia_structural(&candidate))
        });
    }
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

/// PASS 83a (FB-82a-04): one per-position slot in a php member-call
/// argument list that mixes literal tokens with canonical metas — sg binds
/// metas positionally and matches literal tokens byte-exactly, and a
/// `$$$NAME` slot binds the remaining arguments' source text from any
/// position (leading/mid/trailing, zero-length included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgSlot {
    /// `$NAME` / `$$NAME` — binds its positional candidate argument.
    Meta(String),
    /// `$$$NAME` — binds the remaining arguments' source text (multi
    /// namespace), zero or more.
    Rest(String),
    /// A literal token — the candidate argument text must equal it
    /// byte-exactly.
    Literal(String),
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
    /// PASS 81a (FB-80a-05): per-argument capture names when the argument
    /// list is a comma list of TWO OR MORE single canonical metas
    /// (`$A, $B`) — each name binds its positional candidate argument node
    /// text (sg 0.45.2: `$w->q2($A, $B)->r2()` + `-r QQ($A, $B)` applies
    /// `QQ($u, $v)`). Single-meta/rest lists keep the whole-list
    /// `args_capture` arm (`None` here) so their registered bindings stay
    /// byte-compatible.
    pub arg_metas: Option<Vec<String>>,
    /// PASS 83a (FB-82a-04): per-position slots when the member-call
    /// argument list MIXES literal tokens with metas (`1, $B`,
    /// `$$$A, 3`). Meta-only lists keep the registered `arg_metas`/
    /// `args_capture` arms (this stays `None`) so their bindings stay
    /// byte-compatible.
    pub arg_slots: Option<Vec<ArgSlot>>,
}

/// PASS 65 (F64-2): classify a dotted member-call chain with per-segment
/// argument lists. `None` unless MORE THAN ONE segment carries an argument
/// list — every single-argument-list face keeps the simple
/// [`NativeKind::Call`] lane unchanged.
///
/// PASS 115 (114A-F1/F2): a `?.`-SPELLED chain (`optional_flags` any) with
/// PROPERTY links (`a?.b?.c($X)`, `a.b?.c($X)`, `a?.b?.c?.d($X)` — mid-chain
/// argument-free segments) classifies into the gated
/// [`NativeKind::OptionalCallChain`] lane instead of falling to the general
/// structural lane: the general lane consults none of the
/// `call_junction_exact` / `member_link_parts` gates, so junction-comment
/// files were answered where sg refuses (114A-F1), and a `$$$`-rest argument
/// list built no template anywhere and failed closed rc2-loud where sg
/// answers (114A-F2). The admission stays conservative: at least one `?.`
/// connector anywhere in the chain, at least three segments, at least one
/// call segment, the TERMINAL segment must be a call (property tails have no
/// call-node walk), every admitted property segment is an identifier-shaped
/// literal or a pure metavariable (the walk's text-unification contract),
/// and all other segment rules are unchanged. All-dotted chains keep the
/// historical ≥2-call threshold byte-for-byte.
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
    // PASS 115 (114A-F1/F2): the chain spells at least one `?.` connector —
    // the admission that unlocks property segments below.
    let optional_spelled = optional_flags.iter().any(|flag| *flag);
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
                // Only the leading receiver segment may be argument-free —
                // EXCEPT in a `?.`-spelled chain, where a mid-chain PROPERTY
                // link classifies (PASS 115, 114A-F1/F2: the general lane's
                // ungated `?.` path over-answered junction comments and went
                // ingress-loud on `$$$`-rest lists). The segment must be
                // identifier-shaped — the walk's text-unification contract.
                // PASS 117: template-literal (`` `b` ``) and numeric (`0`)
                // link TEXTS were probed for admission and DELIBERATELY NOT
                // admitted — sg 0.45.2's answers on those faces ride its own
                // grammar's parse of `?.` + non-ident properties, while this
                // workspace's TSX parse shapes the same sources differently
                // (the template link parses as a connector-less glued
                // sibling above a zero-width property; `a?.0` at a statement
                // root parses as an ERROR extra plus a number-rooted chain).
                // Emulating those shapes would need new walker machinery AND
                // would still leave the js semicolon-less numeric face
                // silently empty where sg ANSWERS — so the identifier-shape
                // refusal stands and every template/numeric face keeps the
                // fail-closed loud envelope (CNR §39.12: probed sg grids +
                // retry predicates).
                if index != 0 {
                    if !optional_spelled
                        || !(is_pure_metavariable(raw) || is_pattern_ident(raw))
                    {
                        return None;
                    }
                }
                (raw, None)
            }
        };
        let (literal, capture) = if is_pure_metavariable(name_part) {
            (None, capture_name(name_part).map(str::to_string))
        } else {
            (Some(name_part.to_string()), None)
        };
        let mut arg_slots: Option<Vec<ArgSlot>> = None;
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
            // PASS 117 (116A-F2): a MIXED rest list (one or two rests with
            // singles, or two rests) classifies ONLY for `?.`-spelled chains,
            // riding the registered plain-call rest-slot semantics
            // (`parse_call_arg_slots` + `call_arg_slots_match`): first-hand
            // sg grid 2026-09-08 answers the chain faces at exactly those
            // arities (trailing rest + single ⇒ n ≥ 2, k ≥ 2 rests ⇒
            // n ≥ k-1, non-trailing rest ⇒ n == 1). Dotted chains keep the
            // refusal (sg refuses `a.b.c($A, $$$B)` at every arity), and
            // `parse_call_arg_slots`' same-name collision refusal keeps the
            // H-CONF-026 census-loud envelope (`a?.b?.c($$$A, $A)`).
            Some(args) if args.contains("$$$") => {
                if !optional_spelled {
                    return None;
                }
                match parse_call_arg_slots(args) {
                    Some(slots) => arg_slots = Some(slots),
                    None => return None,
                }
                None
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
        // PASS 81a (FB-80a-05): a comma list of 2+ single metas binds each
        // name to its positional candidate argument (sg -r probe:
        // `r.m1($A, $B).m2()` + `QQ($A, $B)` applies `QQ(1, 2)`).
        let arg_metas = match args_text {
            Some(args)
                if !args.contains("$$$")
                    && args.split(',').all(is_pure_metavariable)
                    && args.contains(',') =>
            {
                let names: Vec<String> = args
                    .split(',')
                    .filter_map(capture_name)
                    .map(str::to_string)
                    .collect();
                (!names.is_empty()).then_some(names)
            }
            _ => None,
        };
        segments.push(CallChainSegment {
            literal,
            capture,
            args,
            args_capture,
            arg_metas,
            // PASS 83a: the dotted chain lane keeps its registered
            // meta-only argument contract (every probed mixed dotted face
            // already answers through the general lane). PASS 117: the
            // `?.`-spelled chain lane carries mixed-rest slot lists here
            // (116A-F2) — the walker enforces them with the registered
            // plain-call rest-slot semantics.
            arg_slots,
        });
    }
    // PASS 67a (F66a-3): the 3+-segment optional chain is its own kind —
    // the two-segment spelling keeps the registered [`NativeKind::OptionalCall`]
    // lane (call_segments == 1 falls through to it via the single-call arm).
    // The per-connector flags ride along: `optional_flags[j]` is true when
    // the connector INTO pattern segment j is the spelled `?.`.
    // PASS 115 (114A-F1/F2): a `?.`-spelled chain with property links admits
    // on ONE call segment when the chain is ≥3 segments and the TERMINAL
    // segment is a call (property tails have no call-node walk). The
    // historical ≥2-call admission keeps any segment count (the 2-seg
    // all-call face `fetch()?.$M($$$A)` stays here, NOT `OptionalCall`), the
    // `CallChain` arm stays byte-identical for all-dotted chains, and a
    // 2-segment single-call `?.` spelling keeps `OptionalCall`.
    if optional_spelled {
        let terminal_is_call = segments
            .last()
            .is_some_and(|segment| segment.args.is_some() || segment.arg_slots.is_some());
        if !terminal_is_call
            || call_segments < 1
            || (call_segments < 2 && segments.len() < 3)
        {
            return None;
        }
        return Some(NativeKind::OptionalCallChain {
            segments,
            optional_flags,
        });
    }
    if call_segments < 2 {
        return None;
    }
    Some(NativeKind::CallChain { segments })
}

/// PASS 77b (F76-1): classify a php `->` member-call chain with per-segment
/// argument lists. `None` unless MORE THAN ONE segment carries an argument
/// list — single-call-segment member faces keep the simple
/// [`NativeKind::MemberCall`] lane, and a refused shape falls through to the
/// historical arms unchanged (the pre-77b posture is preserved verbatim).
/// The segment/argument grammar mirrors [`classify_call_chain`] with the
/// depth-0 connector spelled `->`. PASS 79 (F78-1/F78-2): mid-chain
/// PROPERTY segments (`->prop` / `?->prop`, no call parens) classify —
/// sg 0.45.2 answers the intermixed faces (`$svc->c1()->prop->c2($A)`,
/// `$x->prop->m($A)`) — as does the nullsafe `?->` connector anywhere in
/// the chain, recorded per link so the walk stays connector token-exact.
/// PASS 81a (FB-80a-01/FB-80a-02): a property segment may also END the
/// pattern (sg answers the property-tail member-access node and its
/// depth-equal prefix subnodes), and a lowercase-led `$name` property link
/// admits as byte-exact literal dynamic-property text. A sole-call chain
/// still needs one of those extra links to reach this lane (zero-call
/// member-access faces keep their registered lanes), and any OTHER `?`
/// (ternary text, `??->`) still refuses like the `.`-lane's `?.` refusal.
/// PASS 94b (FB-93A-5): classify the FLAT php member-property face —
/// `receiver->PROP` / `receiver?->PROP` where the receiver is a LITERAL
/// segment (the same admission [`call_path_segment`] gives chain receivers:
/// a lowercase-led `$variable` text or `$this`), the name tail is a pure
/// metavariable, and the spelling carries no call parens. The tail meta
/// binds the candidate property name text. A pure-meta receiver head
/// (`$A->$M`) keeps its refusal (unprobed wildcard face), as does any other
/// `?` (ternary text) — the same connector discipline the chain lane
/// registers.
fn classify_php_member_property(p: &str) -> Option<NativeKind> {
    let (head, tail) = p.split_once("->")?;
    let (head, nullsafe) = match head.strip_suffix('?') {
        Some(head) => (head, true),
        None => (head, false),
    };
    if head.contains('?') || tail.contains('?') {
        return None;
    }
    let literal = call_path_segment(head.trim())??;
    let tail = tail.trim();
    if !is_pure_metavariable(tail) {
        return None;
    }
    Some(NativeKind::MemberCallChain {
        segments: vec![
            CallChainSegment {
                literal: Some(literal),
                capture: None,
                args: None,
                args_capture: None,
                arg_metas: None,
                arg_slots: None,
            },
            CallChainSegment {
                literal: None,
                capture: capture_name(tail).map(str::to_string),
                args: None,
                args_capture: None,
                arg_metas: None,
                arg_slots: None,
            },
        ],
        nullsafe_flags: vec![false, nullsafe],
    })
}

fn classify_member_call_chain(p: &str) -> Option<NativeKind> {
    // MED-1 (84c, r35): the `;`-terminated chain spelling is the SAME face
    // (sg answers `$w->q9(1, $$$A)->r9();` = the bare spelling's line set);
    // the terminator used to ride into the last segment and fail its
    // `)`-close check.
    let p = p.trim();
    let p = p.strip_suffix(';').unwrap_or(p).trim();
    if !p.contains("->") {
        return None;
    }
    // PASS 94b (FB-93A-5): the FLAT member-property face — a literal
    // receiver, one `->`/`?->` link, a pure-metavariable name tail, and no
    // call parens anywhere — classifies as a two-segment property chain.
    // sg 0.45.2 answers these (probes_run1.jsonl matrix M, probed
    // 2026-09-08 ATTACHED: `$o->$M;` {5}, `$this->$P;` {4}, `$o?->$M;`
    // {6}); pre-fix the zero-call shape fell through every arm to a silent
    // ok:true []. All other no-paren shapes keep the refusal.
    if !p.contains('(') {
        return classify_php_member_property(p);
    }
    // Split on depth-0 `->`; parentheses and subscripts nest. A `?`
    // immediately before a split is the nullsafe `?->` connector INTO the
    // next segment — stripped from the left segment and recorded.
    let bytes = p.as_bytes();
    let mut raw_segments: Vec<String> = Vec::new();
    let mut nullsafe_flags: Vec<bool> = vec![false];
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
                let mut segment = p[segment_start..i].to_string();
                let nullsafe = segment.ends_with('?');
                if nullsafe {
                    segment.pop();
                }
                raw_segments.push(segment);
                nullsafe_flags.push(nullsafe);
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
        // Any `?` that survived the connector strip (ternaries, `??->`,
        // a trailing marker with no `->` after it) has no probed chain
        // contract here — fail-closed.
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
            None if index == 0 => {
                // Only the leading receiver segment may be argument-free.
                // Receiver class discipline (PASS 77b): a canonical metavar
                // (`$A`/`$$A`), a plain identifier, or a lowercase-led
                // literal variable (`$obj` — sg's literal-source reading,
                // matched byte-exactly) classify; a MixedCase or garbage-led
                // `$`-token refuses (php poisons the whole pattern — the
                // NeverMatches gate keeps those faces).
                let receiver_ok = is_pure_metavariable(raw)
                    || is_pattern_ident(raw)
                    || dollar_name_class(raw.strip_prefix('$').unwrap_or(""))
                        == Some(DollarTokenClass::LowercaseLed);
                if !receiver_ok {
                    return None;
                }
                (raw, None)
            }
            None => {
                // PASS 79 (F78-1): a bare segment is a PROPERTY link. sg
                // matches the property bytes exactly and binds a canonical
                // metavariable to the property text (0.45.2 probe:
                // `$svc->c1()->$P->c2($A)` answers with `P` = `prop`), so
                // plain identifiers and canonical metas admit.
                // PASS 81a (FB-80a-01): a property segment may also END the
                // pattern — sg answers the property-tail faces
                // (`$o->c1($A)->tailProp` = the member-access node plus its
                // depth-equal prefix subnodes), and the pre-79 fall-through
                // answered silent [] whenever a meta arg was present.
                // PASS 81a (FB-80a-02): a lowercase-led `$name` is a
                // DYNAMIC property link matched as BYTE-EXACT literal text
                // (0.45.2: `$svc->c1()->$dyn->c2($A)` answers only the
                // `$dyn` line, `$other` only its own line, while a canonical
                // `$$DYN` wildcards every property link) — the registered
                // "refuses" posture fell through silent, never sg's answer.
                // PASS 83a (FB-82a-01): the same holds for a `$$name`
                // lowercase double-dollar link (sg answers
                // `$svc->c1()->$$dyn->c2($A)` on the `$$dyn` line) — the
                // crack between the `$`+lowercase and `$$`+CAPS families.
                // Mixed-case `$$Dyn` keeps the registered refusal (sg []).
                if is_pattern_ident(raw)
                    || is_pure_metavariable(raw)
                    || dollar_name_class(raw.strip_prefix('$').unwrap_or(""))
                        == Some(DollarTokenClass::LowercaseLed)
                    || is_dollar2_lowercase_token(raw)
                {
                    (raw, None)
                } else {
                    return None;
                }
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
        // PASS 83a (FB-82a-04): lists mixing literal tokens with metas
        // classify through per-position slots (sg answers `$w->q5(1, $B)`,
        // `$w->q5($A, $v)`, `$w->q9(1, $$$A)` with literal/metal/rest
        // binding); meta-only lists keep the registered arms byte-compatible.
        let (args, arg_slots) = match args_text {
            None => (None, None),
            Some(args) if args.is_empty() => (Some(ArgumentTemplate::Exactly(0)), None),
            Some(args) if args == "$$$" => (Some(ArgumentTemplate::Any), None),
            Some(args) if args.starts_with("$$$") && is_metavar_name(&args[3..]) => {
                (Some(ArgumentTemplate::Any), None)
            }
            Some(args) if !args.contains("$$$") && args.split(',').all(is_pure_metavariable) => {
                (
                    Some(ArgumentTemplate::Exactly(args.split(',').count())),
                    None,
                )
            }
            Some(args) => {
                let slots = parse_member_arg_slots(args)?;
                let has_rest = slots
                    .iter()
                    .any(|slot| matches!(slot, ArgSlot::Rest(_)));
                let template = if has_rest {
                    ArgumentTemplate::Any
                } else {
                    ArgumentTemplate::Exactly(slots.len())
                };
                (Some(template), Some(slots))
            }
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
        // PASS 81a (FB-80a-05): same per-argument table as
        // [`classify_call_chain`] — `$w->q2($A, $B)` binds A and B
        // positionally (sg -r `QQ($A, $B)` applies `QQ($u, $v)`).
        let arg_metas = match args_text {
            Some(args)
                if !args.contains("$$$")
                    && args.split(',').all(is_pure_metavariable)
                    && args.contains(',') =>
            {
                let names: Vec<String> = args
                    .split(',')
                    .filter_map(capture_name)
                    .map(str::to_string)
                    .collect();
                (!names.is_empty()).then_some(names)
            }
            _ => None,
        };
        segments.push(CallChainSegment {
            literal,
            capture,
            args,
            args_capture,
            arg_metas,
            arg_slots,
        });
    }
    // Single-call-segment faces keep the [`NativeKind::MemberCall`] /
    // [`NativeKind::OptionalCall`] lanes — unless a property or nullsafe
    // link makes the chain deeper than those two-segment shapes
    // (PASS 79 F78-1/F78-2: sg 0.45.2 answers `$x->prop->m($A)` and
    // `$x?->p1->q1($A)`, which no other lane serves). PASS 81a (FB-80a-01):
    // a call followed by a property TAIL also classifies
    // (`$o->c1($A)->tailProp`), while the plain single-call shape keeps its
    // flat lane. Zero-call chains stay out (plain member-access faces keep
    // their registered lanes).
    let last_is_property = segments.len() >= 2 && segments.last().is_some_and(|s| s.args.is_none());
    if call_segments < 2
        && (call_segments != 1
            || segments.len() < 3 && !(segments.len() == 2 && last_is_property))
    {
        return None;
    }
    Some(NativeKind::MemberCallChain {
        segments,
        nullsafe_flags,
    })
}

/// The callee text before the argument list of a call segment.
fn name_head(raw: &str, open: usize) -> &str {
    raw[..open].trim()
}

/// PASS 81a (FB-80a-03); PASS 83a (FB-82a-02/82c-1, FB-82a-03): classify a
/// php assignment/binary pattern whose LHS is a LOWERCASE-LITERAL operand
/// (`$alpha`, `$$zeta`, `$this->prop`, `$eps[0]` — byte-exact literal
/// source text) and whose operator is the plain `=`, an augmented
/// assignment (`+= -= *= /= %= **= .= ??= |= &= ^= <<= >>=`), or a binary
/// operator (`== === != !== < > <= >= <=> + - * / % . && || ??`) — sg
/// 0.45.2 answers EVERY cell of this family (the r31 registration's
/// "compound operators and mixed expressions refuse" wording was
/// fixture-limited: the registered `[]` cells only lacked matching source
/// lines). The RHS is either ONE bare canonical metavariable (the
/// registered capture path, `$V`/`$$V`/`$$$V`) or an expression pattern
/// mixing canonical metas with literal operands (`$V + 1`, `f($V)`,
/// `[$V, $W]`, `"x" . $V` — matched structurally with meta binding).
/// Refusals sg agrees with: a canonical or MixedCase LHS (`$ALPHA = $V;`,
/// `$Alpha = $V;` — sg answers []), and a META assignment-target anywhere
/// in the pattern (`$alpha = $V = $W;` — sg refuses meta LHS targets).
/// Called for PHP ONLY (the match_pattern hook) — js/ts mixed faces keep
/// their registered NeverMatches residual. Statement discipline
/// (FB-82a-03): the walker binds `;`-terminated patterns at
/// expression-statement roots only (sg's `;`-rooted answer node), while a
/// `;`-less pattern also answers condition/argument embedded nodes (probed
/// sg behavior).
fn classify_php_assignment(p: &str) -> Option<NativeKind> {
    let p = p.trim();
    let (lhs, op, rhs) = split_php_binary(p)?;
    // The statement `;` rides the RHS side only, and is OPTIONAL (sg
    // answers `$alpha = $V;` and `$alpha = $V` with the same nodes).
    let rhs = rhs.trim();
    let rhs = rhs.strip_suffix(';').unwrap_or(rhs).trim();
    // FB-84a-01 (r34) + 86a-M3 (r37): sg's tree-sitter-php parse ERRORS on
    // every doubled-sign face whose `--`/`++` token is TIGHT to an operand
    // — prefix (`--$V`/`++$V`), postfix (`$V--`/`$V++`), ANYWHERE in the
    // RHS, parens included (`$a = ($V--) + 1;` probed sg [] rc1 even
    // against the byte-compatible source line; the r34 gate was end-anchored
    // and missed the mid-RHS postfix). A sign SPACED on both sides is the
    // lenient binary-minus + unary-minus spelling both engines answer
    // (`$a = 1 -- $V;` probed sg-answering; `$x = $y -- $z;` the registered
    // control), so the scan is tightness-exact, not substring-exact. This
    // grammar parses the tight faces as real unary operators, so refuse the
    // classification explicitly: token-exact, the single-sign unary faces
    // (`-$V`/`+$V`, sg-answering) and the bare-meta wildcard over postfix
    // lines (`$alpha = $V;` answers `$alpha = $v--;`) keep their lanes.
    if php_rhs_has_tight_doubled_sign(rhs) {
        return None;
    }
    // PASS 131 (130A-F1/F2, f131a): a fully-literal STATIC scoped target
    // (`C::$s`, `Foo\Bar::$s`, `\Foo::$s`, literal + meta-index subscripts)
    // is sg-ANSWERED — byte-exact LHS compare (structural when a subscript
    // index is one canonical metavariable), `;`-optional. The operator gate
    // is sg-exact across ALL THREE candidate families: assignments,
    // augmented assignments, AND binary faces (`C::$s == $V` sg n1, oracle
    // a_bin/a_bin2/a_bin3 — the pass-129 Assign|Augmented restriction is
    // corrected of record in CNR §44.1). Literal `$`-targets never satisfy
    // the predicate (disjoint classes).
    // PASS 135 (134A-F5/F6, f135d): a DYNAMIC-CLASS head (`$C::$s`,
    // `$$C::$s` — [`php_static_target_dynamic_head`]) rides the static lane
    // for `=` assignments only (grid gridE: sg answers both spellings,
    // binding the whole candidate scope text dollar-included); the
    // augmented/binary dynamic faces are unprobed and stay refused.
    let static_target = (is_php_static_scope_target(lhs)
        || (op == "=" && php_static_target_dynamic_head(lhs).is_some()))
        && php_op_class(op).is_some();
    if !is_php_literal_target(lhs) {
        // PASS 127 (125A-F5, f127a): sg 0.45.2 BINDS php `=`-assignment
        // targets that carry canonical metavariables — the whole-meta
        // target (`$X = $Y` binds X to the candidate LHS text: px_php_mm/
        // mixed/alpha sg n1 each, grid /tmp/phase127/g1) and the meta
        // member LINK (`$o->$A = $Y` binds A to the link name text "a"
        // literal / "$a" dynamic: f5a_propmeta + f5a_propmeta2 sg n1 each).
        // The whole-meta `;`-FUL spelling stays REFUSED — sg answers
        // `$X = $Y` but returns [] on `$X = $Y;` (f5c_semi + f5c_alpha_semi
        // sg n0, the §26.2 controls' own spellings) — so the semi faces
        // keep their registered census class. The whole-meta admission is
        // `=`-ONLY: the meta-LHS binary faces (`$A && $B && $C`, f89b)
        // belong to the operand-template lane, whose no-meta-target rule
        // is unchanged. The old blanket refusal served the sg-answering
        // faces a silent `ok:true []` from lanes that never consulted
        // sg-answerability (the F5a silent-under cells).
        // PASS 129 (128B-F1, f129c): the `;`-discipline is WHOLE-META-LHS
        // ONLY. The meta-LINK semi face `$o->$A = $Y;` is sg-ANSWERED
        // (oracle n1, A=$k — grid /tmp/phase129/cells/f1_semi_link), so the
        // link arm correctly carries no `;` guard; the earlier "`;`-ful
        // meta-LHS stays a refusal" wording (report + CNR §43.1) is
        // corrected of record in CNR §44.
        let meta_target = if op == "=" {
            if is_pure_metavariable(lhs) {
                !p.ends_with(';')
            } else {
                php_meta_link_target(lhs)
            }
        } else {
            false
        };
        if !meta_target && !static_target {
            return None;
        }
    }
    // F-r40-3 (r41, 90A-F5): a block comment at the RHS HEAD (`$a = /* h */
    // $V + 1;`) is sg trivia at the operator slot — sg's CST puts it as a
    // direct child of the assignment node between `=` and `right`, and the
    // face ANSWERS when the candidate carries the text-exact comment there
    // (probed {3}/{3,4}/{6} ATTACHED 2026-09-08). Strip the head comments
    // and decide bare-meta vs rhs_expr on the stripped body; the walker
    // re-derives the same sequence from the pattern text and enforces the
    // positional text-exactness at the candidate's operator node. The
    // tight-sign scan above deliberately ran on the UNSTRIPPED rhs (M1
    // skips comment bodies itself, so its straddle bytes are unchanged).
    let (_, rhs_body) = php_rhs_head_block_comments(rhs);
    let rhs_body = rhs_body.trim();
    // PASS 131 (130A-F1, f131a): the pass-129 "single-bare-meta-arg call
    // RHS is the ONE sg-refused static-target shape" veto is DELETED — the
    // r68 oracle re-probe REFUTED its receipt (`C::$s = f($V)` sg n1 V=5,
    // oracle cell a_fcall; corrected of record in CNR §44.1). The face now
    // rides the structural rhs_expr machinery below and binds V sg-exactly.
    // One bare canonical meta keeps the registered capture path.
    if let Some((value, value_multi)) = php_bare_meta(rhs_body) {
        return Some(NativeKind::Assignment {
            target: lhs.to_string(),
            op: op,
            op_class: php_op_class(op)?,
            rhs_expr: None,
            value: value.to_string(),
            value_multi,
        });
    }
    // Anything else is an expression pattern: it must parse and carry no
    // META assignment-target (sg refuses those faces).
    let rhs = rhs_body.to_string();
    validate_php_rhs_expr(&rhs)?;
    Some(NativeKind::Assignment {
        target: lhs.to_string(),
        op: op,
        op_class: php_op_class(op)?,
        rhs_expr: Some(rhs),
        value: String::new(),
        value_multi: false,
    })
}

/// F-r40-3 (r41): split leading `/* */` block comments off the head of a
/// php assignment RHS — returns the comment texts (byte-exact, sg compares
/// comment NODE text) and the remainder. Only a comment run that starts at
/// the very head (whitespace-separated) is head placement; comments after
/// code stay inside the RHS expression where the structural matcher's
/// FB-84a-03 discipline already rules. An unterminated `/*` is not a head
/// comment (the doc parse refuses that pattern, fail-closed either way).
fn php_rhs_head_block_comments(rhs: &str) -> (Vec<String>, &str) {
    let mut rest = rhs.trim_start();
    let mut out = Vec::new();
    while let Some(tail) = rest.strip_prefix("/*") {
        let Some(end) = tail.find("*/") else {
            break;
        };
        out.push(format!("/*{}*/", &tail[..end]));
        // FB-93A-4c (r44): comments in the head run are whitespace-
        // separated (`$a = /* a */ /* b */ $V + 1;` — sg answers the line
        // carrying both slots, matrix C {4}); the run must trim BETWEEN the
        // comments or the second `/*` hides behind the leading space and
        // rides into the RHS body where the expr grammar refuses it.
        rest = tail[end + 2..].trim_start();
    }
    (out, rest)
}

/// F-r40-3 (r41): the walker-side derivation of the same head-comment
/// sequence from the raw pattern text — the pattern's literal `target`
/// (byte-exact LHS), the operator, and then a whitespace-separated run of
/// block comments. An operator-continuation byte (`=` of `==`, `=>`, ...)
/// right after the stripped op bails (no head comments — the spellings the
/// assignment lane admits never carry one there).
fn php_assignment_head_comments(pattern: &str, target: &str, op: &str) -> Vec<String> {
    let Some(rest) = pattern.trim().strip_prefix(target) else {
        return Vec::new();
    };
    let Some(rest) = rest.trim_start().strip_prefix(op) else {
        return Vec::new();
    };
    if rest.starts_with('=') {
        return Vec::new();
    }
    let (comments, _) = php_rhs_head_block_comments(rest.trim_start());
    comments
}

/// 86a-M3 (r37): true when `rhs` carries a `--`/`++` token TIGHT to an
/// operand on either side — sg's parse-error class for doubled signs (see
/// [`classify_php_assignment`]). A sign surrounded by whitespace on both
/// sides is the lenient binary+unary spelling both engines answer, so it is
/// not a refusal face. String literal bodies are skipped (a sign inside a
/// literal is text, not an operator), and — 88a-M1 (r39) — so are
/// pattern-side `/* */` block-comment bodies (comment content is not an
/// operand: `$a = $V /*--*/ + 1;` is sg-ANSWERING, probed, while the
/// spaced-inside-comment face agreed even pre-fix only because the scan's
/// own space rule saw the comment bytes). `//`/`#` comment kinds are NOT
/// skipped: sg REFUSES patterns carrying them outright (rc8 probed for
/// every trailing spelling), so their sg-agreed empty class rides the
/// unskipped bytes. The scan is ASCII-byte-exact, so a multi-byte
/// character can never match `-`/`+`/space/`/`/`*` and no slice can land
/// mid-code-point.
fn php_rhs_has_tight_doubled_sign(rhs: &str) -> bool {
    let bytes = rhs.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        break;
                    }
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                // 88a-M1 (r39): a block comment is sg parse trivia — skip to
                // the closing `*/`. An unterminated comment consumes the
                // rest of the rhs (the doc parse refuses that pattern,
                // fail-closed either way).
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            sign @ (b'-' | b'+') if bytes.get(i + 1) == Some(&sign) => {
                let spaced_left = i > 0 && matches!(bytes[i - 1], b' ' | b'\t');
                let spaced_right = bytes[i + 2..]
                    .first()
                    .map_or(true, |&b| matches!(b, b' ' | b'\t'));
                if !(spaced_left && spaced_right) {
                    return true;
                }
                i += 2;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// One top-level binary operator of the widened php operand family,
/// longest-first so prefix operators (`===` vs `==` vs `=`, `??=` vs `??`,
/// `<=>` vs `<=`) split correctly.
const PHP_BINARY_OPERATORS: &[&str] = &[
    "**=", "<<=", ">>=", "===", "!==", "<=>", "??=", "+=", "-=", "*=", "/=", "%=", ".=", "|=",
    "&=", "^=", "==", "!=", "<=", ">=", "&&", "||", "??", "=", "<", ">", "+", "-", "*", "/",
    "%", ".",
];

/// Split `p` at its FIRST top-level operator (paren/bracket-depth aware,
/// php string literals skipped). Returns None when no family operator is
/// present at the top level.
fn split_php_binary(p: &str) -> Option<(&str, &'static str, &str)> {
    let bytes = p.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'\'' | b'"' => {
                // Skip the string literal body (quote escapes included).
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        break;
                    }
                    i += 1;
                }
            }
            _ if depth == 0 => {
                // The `->` member connector (and `=>`) are NOT binary
                // operators — skip them whole.
                if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'>') {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'=' && bytes.get(i + 1) == Some(&b'>') {
                    i += 2;
                    continue;
                }
                for &op in PHP_BINARY_OPERATORS {
                    // HIGH-2 (84c, r35): `i` advances one BYTE at a time, so
                    // a multi-byte character outside a string literal
                    // (`$α = $V;` — PHP allows non-ASCII identifier bytes)
                    // lands mid-code-point; slice only at char boundaries.
                    if p.is_char_boundary(i) && p[i..].starts_with(op) {
                        return Some((&p[..i].trim(), op, &p[i + op.len()..]));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// PASS 83a: which candidate node kind serves the pattern operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhpBinaryClass {
    /// `=` — `assignment_expression` candidates.
    Assign,
    /// `+=` etc — `augmented_assignment_expression` candidates.
    Augmented,
    /// `==`, `+`, ... — `binary_expression` candidates.
    Binary,
}

fn php_op_class(op: &str) -> Option<PhpBinaryClass> {
    match op {
        "=" => Some(PhpBinaryClass::Assign),
        "**=" | "<<=" | ">>=" | "+=" | "-=" | "*=" | "/=" | "%=" | ".=" | "??=" | "|=" | "&="
        | "^=" => Some(PhpBinaryClass::Augmented),
        "===" | "!==" | "<=>" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "??" | "<" | ">"
        | "+" | "-" | "*" | "/" | "%" | "." => Some(PhpBinaryClass::Binary),
        _ => None,
    }
}

/// PASS 127 (125A-F5, f127a): a php assignment target whose `->` member
/// LINK carries a canonical metavariable — a lowercase-led `$var` root with
/// links of a plain identifier OR a canonical `$META` (`$o->$A`), plus the
/// literal-subscript tail rule of [`is_php_literal_target`]. At least one
/// meta link must be present (pure-literal targets stay on the text-exact
/// path). sg 0.45.2 binds such faces (f5a_propmeta/propmeta2 sg n1, oracle
/// grid /tmp/phase127/g1); the structural comparison runs through
/// [`php_rhs_expr_matches`], whose `variable_name` arm binds the candidate
/// link node text — `a` for a literal `name`, `$a` for a dynamic
/// `variable_name` — exactly sg's metaVariables.
fn php_meta_link_target(lhs: &str) -> bool {
    let mut rest = lhs.trim();
    let Some(root) = rest.strip_prefix('$') else {
        return false;
    };
    let name_len = root
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(root.len());
    if dollar_name_class(&root[..name_len]) != Some(DollarTokenClass::LowercaseLed) {
        return false;
    }
    rest = &root[name_len..];
    let mut saw_meta_link = false;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("->") {
            let after = after.trim_start();
            rest = match after.strip_prefix('$') {
                Some(name_rest) => {
                    let len = name_rest
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(name_rest.len());
                    if !is_metavar_name(&name_rest[..len]) {
                        return false;
                    }
                    saw_meta_link = true;
                    &name_rest[len..]
                }
                None => {
                    let len = after
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(after.len());
                    if !is_pattern_ident(&after[..len]) {
                        return false;
                    }
                    &after[len..]
                }
            };
            continue;
        }
        if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return false;
            };
            let index = &after[..close];
            if index.trim().is_empty() || index.contains('$') {
                return false;
            }
            rest = &after[close + 1..];
            continue;
        }
        break;
    }
    saw_meta_link && rest.trim().is_empty()
}

/// PASS 131 (130A-F1/F2 + 130B-F2/F3, f131a; supersedes the pass-129
/// predicate whose meta-call/meta-index veto receipts the r68 oracle
/// re-probes REFUTED — corrected of record in CNR §44.1): a fully-literal
/// STATIC scoped assignment/binary target — a class-name-ish head (`C`,
/// `self`, `parent`, `static`) optionally NAMESPACED (`Foo\Bar` and the
/// leading-`\` `\Foo\Bar` FQN spellings; sg binds every one — oracle cells
/// a_ns / a2_lead1, refuting the pass-129 "`\Foo::$s` refuses" receipt)
/// followed by `::$prop` with a lowercase-led `$var` property, plus optional
/// `[index]` subscript tails admitted per [`php_static_index_admissible`].
/// Doubled-dollar props and canonical-meta props stay out (sg rc1, oracle
/// a2_metaprop / a2_metaprop2). Disjoint from [`is_php_literal_target`]
/// (`$`-rooted) and [`php_meta_link_target`] (meta links) by construction.
fn is_php_static_scope_target(lhs: &str) -> bool {
    let mut rest = lhs.trim();
    if let Some(after) = rest.strip_prefix('\\') {
        rest = after.trim_start();
    }
    // Head segments: plain identifier-ish tokens joined by `\` (alphabetic/
    // underscore-led — covers class names and the self/parent/static
    // keywords).
    loop {
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if name_len == 0 || !rest.is_char_boundary(name_len) {
            return false;
        }
        let head = &rest[..name_len];
        let head_ok = head.starts_with('_')
            || head
                .chars()
                .next()
                .is_some_and(|c: char| c.is_ascii_alphabetic());
        if !head_ok {
            return false;
        }
        rest = rest[name_len..].trim_start();
        match rest.strip_prefix('\\') {
            Some(after) => rest = after.trim_start(),
            None => break,
        }
    }
    let Some(after) = rest.strip_prefix("::") else {
        return false;
    };
    rest = after.trim_start();
    let Some(prop) = rest.strip_prefix('$') else {
        return false;
    };
    let prop_len = prop
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(prop.len());
    if dollar_name_class(&prop[..prop_len]) != Some(DollarTokenClass::LowercaseLed) {
        return false;
    }
    rest = &prop[prop_len..];
    // Optional subscript tails.
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return false;
            };
            if !php_static_index_admissible(after[..close].trim()) {
                return false;
            }
            rest = &after[close + 1..];
            continue;
        }
        return false;
    }
}

/// PASS 131 (f131a): the subscript-index admission of a static scoped
/// target, sg-exact per the r68 grids. `$`-free indexes are literal text
/// (verbatim byte compare, oracle a2_stridx). ONE canonical metavariable is
/// a whole-index structural bind (oracle a_msub: `[$K]` binds K to the
/// candidate's whole index text, `f($V)` and `$i + 1` included — a2_callsub
/// / a2_idxbin). A bare LOWERCASE php variable spelling is sg's LITERAL
/// variable read: text-equal candidates answer with no capture (oracle
/// a2_lowsub). Any other `$`-carrying shape (a meta nested inside a call,
/// member links, multi-`$` expressions) is unprobed — fail-closed refusal.
fn php_static_index_admissible(index: &str) -> bool {
    if index.is_empty() || index.contains('[') || index.contains(']') {
        return false;
    }
    if !index.contains('$') {
        return true;
    }
    let literal_var = index.strip_prefix('$').is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .next()
                .is_some_and(|c: char| c.is_ascii_lowercase() || c == '_')
            && rest
                .chars()
                .all(|c: char| c.is_ascii_alphanumeric() || c == '_')
    });
    literal_var || is_single_metavariable(index)
}

/// PASS 131 (f131a): true when a static scoped target carries a
/// single-canonical-meta subscript index — the walker must parse the LHS
/// structurally (binding the meta to the whole candidate index text,
/// [`php_static_index_admissible`]) instead of the verbatim byte compare
/// the literal tails keep.
fn php_static_target_has_meta_index(target: &str) -> bool {
    let mut rest = target.trim();
    if let Some(after) = rest.strip_prefix('\\') {
        rest = after.trim_start();
    }
    loop {
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if name_len == 0 {
            return false;
        }
        rest = rest[name_len..].trim_start();
        match rest.strip_prefix('\\') {
            Some(after) => rest = after.trim_start(),
            None => break,
        }
    }
    let Some(after) = rest.strip_prefix("::") else {
        return false;
    };
    rest = after.trim_start();
    let Some(prop) = rest.strip_prefix('$') else {
        return false;
    };
    let prop_len = prop
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(prop.len());
    rest = &prop[prop_len..];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return false;
        }
        if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return false;
            };
            if is_single_metavariable(after[..close].trim()) {
                return true;
            }
            rest = &after[close + 1..];
            continue;
        }
        return false;
    }
}

/// PASS 135 (134A-F5/F6, f135d): the DYNAMIC-CLASS head of a static scoped
/// assignment target — `$C::$s` / `$$C::$s` where the head is ONE canonical
/// metavariable (optionally behind one literal `$`, php's dynamic-variable
/// spelling) and the prop is a literal lowercase-led `$var` with NO
/// subscript tails (unprobed → fail-closed). sg binds the WHOLE candidate
/// scope text, dollar included (grid /tmp/phase135/cells gridE:
/// `$C::$s = $V` → C=`$name`; `$$C::$s = $V` → C=`$c`); a literal-class
/// candidate head is unprobed and stays refused. Returns the metavariable
/// NAME for the walker's scope binding.
fn php_static_target_dynamic_head(target: &str) -> Option<&str> {
    let rest = target.trim().strip_prefix('$')?;
    let (head, prop) = rest.split_once("::")?;
    let meta_name = match head.strip_prefix('$') {
        Some(inner) => inner, // the `$$C` spelling
        None => head,         // the `$C` spelling
    };
    if !is_metavar_name(meta_name) {
        return None;
    }
    let prop_name = prop.trim().strip_prefix('$')?;
    if dollar_name_class(prop_name) != Some(DollarTokenClass::LowercaseLed) {
        return None;
    }
    Some(meta_name)
}

/// PASS 127 (125A-F5): true when the classified assignment target carries a
/// canonical metavariable the walker must bind structurally (whole-meta
/// `$X` or a [`php_meta_link_target`] link) instead of comparing the
/// candidate LHS text verbatim.
fn php_assignment_target_is_meta(target: &str) -> bool {
    is_pure_metavariable(target) || php_meta_link_target(target)
}

/// PASS 83a: a literal assignment TARGET — a lowercase-led `$var` / `$$var`
/// root optionally followed by `->` literal links and `[literal]` subscripts
/// (`$alpha`, `$$zeta`, `$this->prop`, `$eps[0]`). No metavariables anywhere
/// (sg refuses meta assignment-targets), no calls.
fn is_php_literal_target(lhs: &str) -> bool {
    let (mut rest, _) = match lhs.strip_prefix("$$") {
        Some(rest) => (rest, 2usize),
        None => match lhs.strip_prefix('$') {
            Some(rest) => (rest, 1usize),
            None => return false,
        },
    };
    // The root name must be a lowercase-led variable name.
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let (name, tail) = rest.split_at(name_len);
    if dollar_name_class(name) != Some(DollarTokenClass::LowercaseLed) {
        return false;
    }
    rest = tail;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if let Some(after) = rest.strip_prefix("->") {
            let after = after.trim_start();
            // A member link: a plain identifier or a lowercase/2-dollar var.
            let (link_rest, link_dollars) = match after.strip_prefix("$$") {
                Some(r) => (r, 2usize),
                None => match after.strip_prefix('$') {
                    Some(r) => (r, 1usize),
                    None => (after, 0usize),
                },
            };
            let name_len = link_rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(link_rest.len());
            let (name, tail) = link_rest.split_at(name_len);
            if link_dollars == 0 {
                if !is_pattern_ident(name) {
                    return false;
                }
            } else if dollar_name_class(name) != Some(DollarTokenClass::LowercaseLed) {
                return false;
            }
            rest = tail;
            continue;
        }
        if let Some(after) = rest.strip_prefix('[') {
            // A subscript: literal content only (no `$`), closed.
            let Some(close) = after.find(']') else {
                return false;
            };
            let index = &after[..close];
            if index.trim().is_empty() || index.contains('$') {
                return false;
            }
            rest = &after[close + 1..];
            continue;
        }
        return false;
    }
}

/// PASS 83a: the RHS is exactly one bare canonical metavariable
/// (`$V` / `$$V` single namespace, `$$$V` multi).
fn php_bare_meta(rhs: &str) -> Option<(&str, bool)> {
    if rhs.starts_with("$$$") {
        let name = &rhs[3..];
        is_metavar_name(name).then_some((name, true))
    } else {
        capture_name(rhs).map(|name| (name, false))
    }
}

/// PASS 83a (FB-82a-02): validate a general RHS expression pattern — it must
/// parse under php and contain no META assignment-target (`$V = $W` — sg
/// refuses meta LHS targets anywhere in the pattern). HIGH-1 (84c, r35): the
/// veto MUST slice against the wrapper DOC — the tree was parsed from
/// `<?php {rhs};`, so every pattern-node byte offset is doc-relative
/// (shifted by the 6-byte prefix). Slicing the bare `rhs` made the verdict a
/// function of byte-length coincidence: on `$alpha = $V = $W;` the inner
/// target's doc range [6..8] is out of bounds for the 7-byte rhs (veto
/// skipped — the registered sg-agreed refusal answered) while longer rhs
/// texts mis-sliced onto unrelated nodes (sg-answering faces falsely
/// refused).
fn validate_php_rhs_expr(rhs: &str) -> Option<()> {
    let template = parse_php_rhs_tree(rhs)?;
    validate_no_meta_target(template.tree.root_node(), &template.doc)
}

/// PASS 83a (FB-82a-02): the php RHS expression pattern parses as a
/// `;`-terminated statement under the `<?php` tag — the grammar's `program`
/// rule admits statements ONLY after the tag, and the bare expression is a
/// parse ERROR (expression statements demand the terminator), so with php's
/// absent `general_expression_context` wrapper every rhs_expr face would
/// classify None and fall to the NeverMatches gate. Pattern node byte
/// offsets are DOC-relative — callers pass `template.doc` as the pattern
/// source for `php_rhs_expr_matches`.
fn parse_php_rhs_tree(rhs: &str) -> Option<std::sync::Arc<LiteralTemplate>> {
    let doc = format!("<?php {rhs};");
    parse_pattern_tree(Language::Php, &doc)
}

fn validate_no_meta_target(node: Node, source: &str) -> Option<()> {
    if matches!(node.kind(), "assignment_expression" | "augmented_assignment_expression") {
        if let Some(left) = node.child_by_field_name("left") {
            if let Some(text) = node_text(&left, source) {
                if capture_name(text.trim()).is_some() {
                    return None;
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        validate_no_meta_target(child, source)?;
    }
    Some(())
}

/// 88a-M2 (r39): a classified bare php binary-expression META template —
/// the parsed `<?php {core};` doc plus whether the raw pattern spelled a
/// trailing `;` (sg roots that spelling at the statement).
struct PhpOperandTemplate {
    parsed: std::sync::Arc<LiteralTemplate>,
    had_semi: bool,
}

impl PhpOperandTemplate {
    /// The comparison root inside the doc: `program` → `expression_statement`
    /// → the binary/paren expression (no-semi), or the statement itself
    /// (semi — sg's statement-rooted `;` discipline).
    fn comparison_root(&self) -> Option<Node<'_>> {
        php_operand_comparison_root(&self.parsed, self.had_semi)
    }
}

/// 88a-M2 (r39): admission of a bare php binary-expression META template —
/// no `=` assignment root, no literal LHS: `$A && $B`, `$A . $B`, `$A + $B`,
/// `$A == $B`, `$A || $B`, `$A === $B`, `$A ?? $B`, `$A <=> $B`, the nested
/// `$A && $B && $C`, the paren spelling `($A && $B)`, the universal
/// `$$A && $B`, literal-operand tokens (`$A === 2`), and the
/// `;`-terminated statement-root spellings (`$A && $B;`, `$A === $B;`).
/// sg 0.45.2 answers every probed face (2026-09-08, ATTACHED) through
/// expression-level metavariable binds; the php operand lane was
/// assignment/augmented-ROOTED only, so these classified None and failed
/// closed loudly at the census. Admission rules: every `$`-token canonical
/// (the mixed face `$A && $b` keeps its registered NeverMatches silent
/// class — sg refuses the lowercase spelling), and the `;`-stripped doc's
/// comparison root must be a `binary_expression` or
/// `parenthesized_expression` (no-semi: kind-exact expression bind — the
/// paren face answers only paren-wrapped sites) or the
/// `expression_statement` wrapping a binary (semi: only bare `expr;`
/// statements answer — `$A && $B;` {13} / `$A === $B;` {6} probed, while
/// `$A . $B;` answers nothing on a corpus without a bare concat statement).
/// Rootless NON-binary faces (`$A = $B` assignment root, `$A;` bare-meta
/// statement) are NOT admitted — they keep their registered loud riders.
/// F-r40-2 (r41, 90B-89M2-1): the sibling assignment lane's
/// [`validate_no_meta_target`] veto runs over the parsed doc HERE TOO — an
/// operand template carrying an embedded assignment/augmented-assignment
/// expression with a META target (`($A = $B) && $C`, `$A && ($B = $C)`,
/// `($A .= $B) && $C`, `($A += $B) && $C`) is refused the lane: sg 0.45.2
/// answers nothing on the whole family (probed rc1/rc0 [] ATTACHED
/// 2026-09-08 — the meta assignment target never binds), and the walk's
/// kind-equal unification otherwise over-matched ordinary source. The
/// refusal follows the sibling's exact mechanism and genus: the face leaves
/// the lane, `native_pattern_answerable` goes false, and the census takes
/// the loud fail-closed class (the registered `$A = $B` rider's class).
fn classify_php_operand_template(pattern: &str) -> Option<PhpOperandTemplate> {
    let p = pattern.trim();
    let had_semi = p.ends_with(';');
    let core = p.strip_suffix(';').unwrap_or(p).trim();
    if !core.contains('$')
        || pattern_has_noncanonical_metavar(core)
        || php_operand_has_two_dollar_token(core)
        || php_operand_has_literal_operand(core)
    {
        return None;
    }
    let parsed = parse_php_rhs_tree(core)?;
    php_operand_comparison_root(&parsed, had_semi)?;
    validate_no_meta_target(parsed.tree.root_node(), &parsed.doc)?;
    Some(PhpOperandTemplate { parsed, had_semi })
}

/// 88a-M2 (r39) scope guard: a LITERAL leaf operand anywhere in the
/// pattern (an identifier/number/quoted byte outside a `$`-token). The
/// deduped 88a-M2 family is all-meta; literal-operand faces keep their
/// registered classes — `$A + 1` is the §21.2 registered LOUD cell
/// (f74c pin), and the probed sg-answering literal faces (`$A === 2` {2,5,6})
/// stay subject-stricter riders for the register.
fn php_operand_has_literal_operand(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'$' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'(' | b')' | b';' | b'+' | b'-' | b'*' | b'/'
            | b'%' | b'=' | b'<' | b'>' | b'!' | b'?' | b'.' | b'|' | b'&' | b':' => i += 1,
            _ => return true,
        }
    }
    false
}

/// 88a-M2 (r39) scope guard: a `$$NAME` dynamic-variable token. The
/// registered `$$` wildcard binds plain AND dynamic VARIABLE candidates
/// only ([`php_rhs_expr_matches`]), while sg answers the universal
/// spelling over every operand kind (probed `$$A && $B` == the `$A && $B`
/// set, integers included) — the lane must not serve it with narrower
/// semantics, so the face keeps its registered loud rider.
fn php_operand_has_two_dollar_token(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let mut run = 0usize;
            while i < bytes.len() && bytes[i] == b'$' {
                run += 1;
                i += 1;
            }
            let name_len = bytes[i..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            if run == 2 && name_len > 0 {
                return true;
            }
            i += name_len;
        } else {
            i += 1;
        }
    }
    false
}

/// Named children for the operand-template descent: the php tag and
/// comment trivia never shape the single-child chain (a pattern-side
/// comment stays INSIDE the comparison root, where
/// [`php_rhs_expr_matches`] text-matches it).
fn php_operand_children<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() != "php_tag" && !child.kind().contains("comment"))
        .collect()
}

fn php_operand_comparison_root<'a>(
    parsed: &'a LiteralTemplate,
    had_semi: bool,
) -> Option<Node<'a>> {
    let mut node = parsed.tree.root_node();
    // `program`: descend through the single named child once the php tag
    // and trivia are filtered (the `unwrap_pattern_expression` discipline).
    while node.kind() == "program" {
        let named = php_operand_children(node);
        match named.as_slice() {
            [only] => node = *only,
            _ => return None,
        }
    }
    if node.kind() != "expression_statement" {
        return None;
    }
    let named = php_operand_children(node);
    match named.as_slice() {
        [only] => {
            if had_semi {
                // sg roots the `;`-terminated pattern at the statement; the
                // statement's single child must still be the binary shape.
                (only.kind() == "binary_expression").then_some(node)
            } else if matches!(
                only.kind(),
                "binary_expression" | "parenthesized_expression"
            ) {
                Some(*only)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// PASS 137 (D6 elseif_same_line_F5): the parsed `else …` tail of an if
/// template. `ElseIf` nests so a deep `else if … else if …` chain parses
/// recursively (sg binds every level's captures, oracle grid137a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfAlternative {
    /// `else { B }` / `else: B` — the fallback branch. `body_meta` is the
    /// bare `$NAME` of THIS branch's body section (per-level: the
    /// pattern-global last-section scan cannot serve chained tails).
    Else {
        body: Option<BodyTemplate>,
        body_braced: bool,
        body_meta: Option<String>,
    },
    /// `else if (COND) { B }` — the chained if; `alternative` carries any
    /// further tail. `cond_meta`/`body_meta` are THIS level's bare-meta
    /// captures (None when the section is not a single canonical meta).
    ElseIf {
        cond: Option<String>,
        body: Option<BodyTemplate>,
        body_braced: bool,
        cond_meta: Option<String>,
        body_meta: Option<String>,
        alternative: Option<Box<IfAlternative>>,
    },
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
    /// PASS 131 (130A-F8, f131f): `body` carries the member-count template
    /// for the `interface` prefix ONLY — sg 0.45.2 binds `interface $N { $B }`
    /// against single-member ts/java interfaces and refuses multi-member
    /// candidates (oracle d2_iface cells); other grammars keep the census
    /// loud (unreceipted). `class`/`struct`/`type` Exactly templates keep
    /// their registered refusal.
    Class {
        keyword: &'static str,
        name: Option<String>,
        body: Option<BodyTemplate>,
    },
    /// Free or method call; method path segments may be `$` wildcards.
    Call {
        /// Exact path like `foo.bar` or single name; segments that were `$X` are None.
        path: Vec<Option<String>>,
        /// PASS 105 (FB-104A-2): per-position slots when the argument list
        /// mixes a `$$$NAME` rest with positional `$NAME`/`$$NAME` singles
        /// (`q($A, $$$B)`). sg 0.45.2 answers the rest-slot grid uniformly in
        /// the seven probed languages (m3 matrix) and the text-derived
        /// arity template cannot express it; `None` keeps the registered
        /// empty/`$$$`/pure-singles templates.
        arg_slots: Option<Vec<ArgSlot>>,
    },
    /// PASS 75a (F74a-2): the php plain `->` member-call spelling
    /// (`$svc->run($A)`). sg is connector token-exact — a `->`-spelled callee
    /// never answers `.`/`::`/`?->` call sites and vice versa — so the
    /// spelling gets its own lane over `member_call_expression` candidates
    /// with the full object->name segment chain (raw object text, meta
    /// segments wildcard). Nullsafe candidates: plain spellings never
    /// answer them (token-exact); a pattern that SPELLS `?->` rides this
    /// lane with `nullsafe: true` over `nullsafe_member_call_expression`
    /// candidates (86a-L5, r37 — the flat nullsafe slot faces; shapes the
    /// carve refuses keep the [`NativeKind::OptionalCall`] lane).
    MemberCall {
        /// Exact path; segments that were `$X` are None.
        path: Vec<Option<String>>,
        /// PASS 83a (FB-82a-04): per-position slots when the argument list
        /// mixes literal tokens with canonical metas (`$w->q5(1, $B)`).
        /// 86a-L5 (r37): meta-only flat lists classify through the slots
        /// too (the `;`-terminated spelling had no answering arm).
        arg_slots: Option<Vec<ArgSlot>>,
        /// PASS 83a (FB-82a-05): set only for the DANGLING-arrow repair —
        /// sg's repaired pattern never answers the standalone statement
        /// spelling (probed []), only chain-prefix sites whose member-call
        /// node is continued by a member-access/call link.
        require_continuation: bool,
        /// 86a-L5 (r37): the pattern's member connector is the nullsafe
        /// `?->` — candidates switch to `nullsafe_member_call_expression`
        /// (token-exact in both directions, probed sg).
        nullsafe: bool,
    },
    /// PASS 77b (F76-1): the php plain `->` member-call CHAIN spelling with
    /// MORE THAN ONE argument list (`$obj->m1()->m2($A)`). Pass-75's
    /// [`NativeKind::MemberCall`] lane serves single-call-segment faces only;
    /// every deeper chain fell through the classifier into the silent general
    /// lane and answered `ok:true []` where sg 0.45.2 answers the chain node
    /// (and its inner prefix subnodes, which the walk visits as their own
    /// member-call nodes). Per-segment contract mirrors
    /// [`NativeKind::CallChain`], but candidates are the php member-call
    /// nodes only — connector token-exact, never a `.` site — and each
    /// call segment's argument list must sit on a REAL call link (a property
    /// access in the receiver chain is not a call). Canonical `$$A` argument
    /// slots ride the F74a-3 family binding (single namespace, key `A`).
    /// PASS 79 (F78-1/F78-2): mid-chain property segments classify, and the
    /// nullsafe `?->` connector is spelled per link. PASS 81a (FB-80a-01):
    /// the LAST segment may be a property link — the walker then visits the
    /// member-access candidate kinds as well and answers the property-tail
    /// node plus its depth-equal prefix subnodes, sg-exact.
    MemberCallChain {
        /// Outermost-first segments. The leading segment is a plain receiver;
        /// later segments are calls or property links (mid-chain or tail).
        segments: Vec<CallChainSegment>,
        /// `nullsafe_flags[j]` is true when the connector INTO segment j is
        /// the nullsafe `?->` (flags[0] always false). Token-exact: a plain
        /// `->` link never answers a `?->` site and vice versa (0.45.2
        /// cross probes, both directions).
        nullsafe_flags: Vec<bool>,
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
    /// `if $COND { $BODY }`, or `if $COND: $BODY`. A metavariable condition
    /// rides `cond == None` (registered capture); PASS 131 (130A-F6, f131e)
    /// admits CONCRETE conditions through `cond == Some(text)` — the text
    /// parses as a per-language general-lane expression template and
    /// `general_eq` compares it against the candidate's own condition node
    /// (oracle d2 cells: `f($X)`, `a && $X`, fully-concrete `a`), binding
    /// cond metas sg-exactly. Paren, brace, and colon forms are normalized
    /// so one pattern matches if-nodes across all indexed languages.
    /// PASS 122 (F1, f122a): `body_braced` records whether the PATTERN's body
    /// section was spelled with braces — sg 0.45.2 treats brace-ness as
    /// STRUCTURAL there (oracle grid /tmp/phase122/f1: a `{ $B }` pattern
    /// answers only candidates whose consequence is a braced block; the
    /// brace-less js/ts/c/php `if (c) d();` twin is refused `[]` rc1 silent,
    /// while a brace-less pattern answers both spellings). The flag gates
    /// `if_body_matches`; a `:`-spelled suite stays brace-less.
    If {
        cond: Option<String>,
        body: Option<BodyTemplate>,
        body_braced: bool,
        /// PASS 137 (137A-F5/D6 elseif_same_line_F5, oracle grid
        /// /tmp/phase137A grid137a): the `else { B }` / `else if (Y) { B }`
        /// tail. sg 0.45.2 answers the js else-if chain face n1 binding ALL
        /// FOUR captures (X=a A=`f();` Y=b B=`g();`) where the un-parsed tail
        /// composed the loud census class. `None` keeps the registered
        /// else-less contract byte-for-byte (a pattern without an else never
        /// refuses an else-carrying candidate — the pre-137 behavior).
        alternative: Option<IfAlternative>,
    },
    /// PASS 81a (FB-80a-03); PASS 83a (FB-82a-02/82c-1, -03, -06): a php
    /// assignment/binary pattern whose LHS is a LOWERCASE-LITERAL operand
    /// (byte-exact variable/member/dim text under sg's literal-source
    /// reading) and whose operator is `=`, an augmented assignment, or a
    /// binary operator — sg 0.45.2 answers the whole probed family
    /// (`$alpha = $V;`, `$alpha += $V;`, `$gamma == $V;`, `$this->prop =
    /// $V;`, `$eps[0] = $V;`, `$$zeta = $V;`), binding the RHS expression
    /// texts. The match span is the `;`-rooted EXPRESSION STATEMENT when
    /// the pattern carries a trailing `;` (sg -U consumes it) and the
    /// assignment node otherwise; `;`-terminated patterns bind statement
    /// roots only (sg never answers condition/argument embedded nodes with
    /// them) while `;`-less patterns also answer embedded nodes. A
    /// canonical or MixedCase LHS and any META assignment-target stay out
    /// (sg refuses those faces); the lane is PHP-ONLY (the js/ts mixed
    /// class keeps its registered NeverMatches residual).
    Assignment {
        /// Byte-exact LHS text (`$alpha`, `$this->prop`, `$eps[0]`).
        target: String,
        /// The spelled operator (`=`, `+=`, `==`, ...).
        op: &'static str,
        /// Which candidate node kind serves the operator.
        op_class: PhpBinaryClass,
        /// When the RHS is NOT one bare meta: the RHS expression pattern
        /// text (parsed at walk time, matched structurally with meta
        /// binding).
        rhs_expr: Option<String>,
        /// RHS capture name for the bare-meta path (`V` for `$V`/`$$V`/`$$$V`).
        value: String,
        /// The bare-meta RHS was spelled `$$$NAME` (multi namespace).
        value_multi: bool,
    },
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
        // PASS 137 (137A-F12 / 137B-F6): the "two-modifier residual" is the
        // two-token SPELLING gap class, not `sealed` alone. sg 0.45.2 binds
        // every one of these pattern spellings on the gridded faces
        // (grid137 B-lane: cs `sealed interface $N { $B }`,
        // `partial interface …`, `public partial interface …`; java
        // `strictfp interface …`, `sealed interface …` — all sg n1 N/B
        // against the matching candidate while the subject rc2'd LOUD: the
        // pattern could not classify Class{interface}). The cross-language
        // faces are sg-REFUSED and stay refused here: a ts/java pattern
        // carrying a csharp-only spelling keeps modifiers Some(spelling),
        // so declaration_modifiers_match refuses every plain candidate and
        // the ts blanket arm refuses the rest (grid137: `strictfp interface
        // $N { $B }` on ts plain/export sg rc1; `partial` on java sg rc1).
        "sealed ",
        "partial ",
        "strictfp ",
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
        // PASS 137 (grid137 B-lane + follow-up): a java ANNOTATION leads the
        // modifiers list (`@Deprecated public interface …`) and sg's
        // pattern-side modifiers include it — the annotation-carrying
        // PATTERN faces bind sg n1 when the candidate carries the same
        // annotation (`@Deprecated public interface $N { $B }` ×
        // `@Deprecated interface K` n1; `@Deprecated` alone ⊆
        // `@Deprecated @SafeVarargs` n1) and refuse otherwise (rc1).
        // Consume `@ident` optionally followed by ONE balanced `(...)` group
        // plus trailing whitespace; an unterminated group keeps the face's
        // fail-closed class (no strip).
        if let Some(after_at) = rest.strip_prefix('@') {
            let ident_len = after_at
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .map(char::len_utf8)
                .sum::<usize>();
            let annotation_len = if ident_len == 0 {
                None
            } else if let Some(after_paren) = after_at[ident_len..].strip_prefix('(') {
                // `@Ident(…)` — the arg group must balance to strip.
                balanced_paren_close(after_paren)
                    .map(|close| ident_len + 1 + close + 1)
            } else {
                Some(ident_len)
            };
            if let Some(len) = annotation_len {
                rest = after_at[len..].trim_start();
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
/// PASS 83a (FB-82a-04): the pre-gate carve for a FLAT php `->` member call
/// whose argument list carries slot-worthy shapes (mixed literal/meta
/// lists, whole-list rests, and — 86a-L5, r37 — meta-only lists and the
/// `;`-terminated nullsafe spelling). Every other shape returns None so
/// its registered route stays untouched: non-member callees keep the plain
/// Call lane, no-semi nullsafe spellings keep the OptionalCall lane, and a
/// slot grammar refusal (MixedCase tokens, composite literals) falls
/// through to the NeverMatches gate. Single-call discipline mirrors
/// `literal_variable_callee_admitted`: the one argument list must close the
/// pattern with no nested parens.
fn classify_member_call_mixed_args(p: &str) -> Option<NativeKind> {
    // MED-1 (84c, r35): the `;`-terminated flat spelling is the SAME face
    // (sg answers `$w->q9(1, $$$A);` identically to the bare spelling) —
    // strip the terminator the way the assignment lane does instead of
    // letting the `)`-close check refuse it. 86a-L5 (r37): whether the
    // `;` rode the pattern decides the nullsafe admission below.
    let p = p.trim();
    let had_semi = p.ends_with(';');
    let p = p.strip_suffix(';').unwrap_or(p).trim();
    let open = p.find('(')?;
    let close = p.rfind(')')?;
    if close <= open || !p[close + 1..].trim().is_empty() {
        return None;
    }
    if p[open + 1..close].contains(['(', ')']) {
        return None;
    }
    let args = p[open + 1..close].trim();
    // Whole-list rest `$$$A`: the chained spelling answers it (Any arity +
    // whole-list capture) while the post-gate arm never classifies a
    // non-canonical receiver (gate NeverMatches), so the flat spelling
    // silent-emptied sg-answering faces (MED-1's flat/chained duopoly;
    // probed sg `$w->q9($$$A);` answers every arity incl. empty).
    // 86a-L5 (r37): META-ONLY lists (`$A`, `$A, $B`, same-name `$A, $A`)
    // classify through the slots too — the `;`-terminated flat spelling
    // never reached an answering arm before (routed to the post-gate arm,
    // whose F74a-1 carve refuses the `;` tail → gate NeverMatches
    // silent-[]; the registered §27.8-2 residual) while sg answers the
    // statement-root sites (probed `$o->c1($A);` {2}, `$o?->c1($A);` {3},
    // `$o->c1($A, $B);` {4}; the same-name list answers EQUAL args only —
    // bind_capture's conflict veto). Empty and bare-`$$$` lists keep their
    // own routes (probed agreeing / residual).
    let whole_rest = args.strip_prefix("$$$").filter(|name| is_metavar_name(name));
    let meta_only = !args.is_empty() && args != "$$$" && validate_argument_pattern(args).is_some();
    // PASS 94b (FB-93A-5): an EMPTY argument list classifies when the
    // callee's TAIL segment is a pure metavariable — sg 0.45.2 answers
    // `$o->$M();` {2} and the `;`-terminated nullsafe `$o?->$M();` {3}
    // (probes_run1.jsonl matrix M, probed 2026-09-08 ATTACHED); pre-fix the
    // empty list hit the meta-list carve below and the face walked a silent
    // ok:true []. Literal-name empty calls keep their registered routes
    // (`$o?->m1()` via OptionalCall).
    let callee = p[..open].trim();
    // PASS 94b (FB-93A-5): the nullsafe spelling normalizes to the plain
    // connector for the path check (`?->` splits below; `parse_call_path`
    // refuses `?` bytes).
    let callee_plain = callee.replace("?->", "->");
    let empty_meta_call = args.is_empty()
        && callee_plain.contains("->")
        && parse_call_path(&callee_plain)
            .and_then(|path| path.last().cloned())
            .is_some_and(|last| last.is_none());
    if whole_rest.is_none()
        && validate_argument_pattern(args).is_some()
        && !meta_only
        && !empty_meta_call
    {
        return None;
    }
    if !callee.contains("->") {
        return None;
    }
    // 86a-L5 (r37): the php nullsafe connector rides BETWEEN the receiver
    // and the call (`$o?->c1`). The `;`-TERMINATED spelling admits here
    // (probed sg `$o?->c1($A);` {3}, `$o?->c1($A, $B);` {10},
    // `$o?->c1($$$A);` {3,10}, `$o?->c1($A, 9);` {10}; the semi spelling
    // keeps the statement-root discipline — the chained-embedded inner
    // call stays unanswered): pre-fix these faces died in the gate (the
    // F74a-1 carve refuses the `;` tail) where the post-gate OptionalCall
    // lane could never serve them. The `;`-LESS nullsafe spellings keep
    // their registered OptionalCall routes untouched (`$o?->m($A)` {4},
    // `$a?->b($A)` {5}, `$o?->m1()` {6}, and the wildcard-head fold face
    // `$O?->$M($$$A)` {3,4,5} whose O binds the folded `g?->h` text no
    // flat path can spell) — the carve refuses them (fail-closed, like
    // parse_call_path's own `?`-refusal) and the gate lets them through
    // exactly as before.
    let (callee, nullsafe) = match callee.split_once("?->") {
        Some((head, tail))
            if had_semi && !head.is_empty() && !head.contains('?') && !tail.contains('?') =>
        {
            (alloc_member_callee(head, tail), true)
        }
        _ => {
            if callee.contains('?') {
                return None;
            }
            (callee.to_string(), false)
        }
    };
    let slots = if let Some(name) = whole_rest {
        vec![ArgSlot::Rest(name.to_string())]
    } else if args.is_empty() {
        // PASS 94b (FB-93A-5): the empty meta-name call binds zero
        // arguments (`$o->$M();` answers the zero-arity sites).
        Vec::new()
    } else {
        parse_member_arg_slots(args)?
    };
    let path = parse_call_path(&callee)?;
    Some(NativeKind::MemberCall {
        path,
        arg_slots: Some(slots),
        require_continuation: false,
        nullsafe,
    })
}

/// 86a-L5 (r37): rejoin the nullsafe split (`head?->tail` → `head->tail`)
/// so the plain dotted-path parser serves the nullsafe spelling.
fn alloc_member_callee(head: &str, tail: &str) -> String {
    format!("{head}->{tail}")
}

/// PASS 105 (FB-104B-1): true when `pattern` classifies into one of the two
/// php comment-TRANSPARENT faces — the assignment-hook lane
/// ([`classify_php_assignment`], the r35 FB-84a-03 family:
/// `$a = $V + /* c */ 1;` answers sg's {4}) or the bare php operand-template
/// lane ([`classify_php_operand_template`], 88a-M2). Both lanes decide
/// comment-carrying spellings by AST structure (the comment is trivia), so
/// core's NeverMatches exemption may hand them to the walk. Every OTHER
/// comment-carrying spelling — notably a member-call pattern whose comment
/// sits in an ARGUMENT slot (`$obj->m(/* m */ $A)`, the 94a cell-94 class:
/// the slot grammar refuses the comment trivia, so the structural arm
/// answers structurally empty) — is NOT walk-decidable and must keep its
/// census-loud fail-closed class. Core consumes this via
/// `matcher_decides`; the pre-fix `!comment_free` blanket short-circuit
/// exempted the whole comment-carrying NeverMatches class and silenced
/// exactly those non-decidable faces into ok:true [] where sg answers the
/// row (m1 oracle).
pub fn php_comment_transparent_operand_lane(pattern: &str) -> bool {
    classify_php_assignment(pattern).is_some() || classify_php_operand_template(pattern).is_some()
}

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
    // PASS 83a (FB-82a-04): a flat php `->` member call whose argument list
    // MIXES literal tokens with canonical metas classifies BEFORE the
    // non-canonical gate — the same genus as the chain carve above (sg
    // 0.45.2 answers `$w->q5(1, $B)` under the lowercase receiver's
    // literal-source reading, and the F74a-1 carve below the gate refuses
    // exactly these lists: `literal_variable_callee_admitted` demands a
    // pure-metavar argument list). Meta-only lists keep the registered
    // post-gate arm, all-lowercase faces keep `dollar_literal_lane`, and a
    // refused slot grammar (MixedCase anywhere) falls through to the gate's
    // NeverMatches class unchanged.
    if let Some(kind) = classify_member_call_mixed_args(p) {
        return Some(kind);
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
            // PASS 135 (134A-F7, grids F_ts_alias_*): `type $N = { $B }` —
            // the ts alias-OBJECT face joins the member-count lane under a
            // dedicated `type-alias` keyword (its query row lands on
            // type_alias_declaration, never the class rows the plain
            // `type` keyword serves). Brace-less alias tails (`type $N =
            // $V`) stay unclassified here and ride the general lane, whose
            // type_alias_declaration root binds V sg-exactly.
            let (tail, class_keyword) = if prefix.trim() == "type" {
                match tail.strip_prefix('=') {
                    Some(after) => (after.trim(), "type-alias"),
                    None => (tail, "type"),
                }
            } else {
                (tail, prefix.trim())
            };
            let body = if is_class {
                parse_body_template(tail)?
            } else {
                parse_function_tail(tail)?
            };
            return Some(if is_class {
                // Statement-count templates on type bodies are language-specific
                // (fields vs methods); only `{ $$$ }` / no body are supported
                // — EXCEPT the `interface` prefix: PASS 131 (130A-F8, f131f)
                // admits the member-count template there (sg binds
                // `interface $N { $B }` on single-member ts/java interfaces,
                // oracle d2_iface), scoped to the receipted grammars at the
                // answerability gate and the walk. PASS 135: the ts
                // `type-alias` object face joins with the same discipline
                // (1-member binds N/B, 2-member refuses — F_ts_alias2 sg
                // rc1). PASS 139: the plain `class` prefix KEEPS the refusal
                // here — the f122e bare-colon empty-suite contract rides it
                // for every language — and the JAVA member-count face
                // classifies through [`classify_java_class_member_count`]
                // instead (grid139 E).
                if matches!(body, Some(BodyTemplate::Exactly(_)))
                    && !matches!(class_keyword, "interface" | "type-alias")
                {
                    return None;
                }
                NativeKind::Class {
                    keyword: class_keyword,
                    name,
                    body,
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
    let callee = p[..open].trim();
    if callee.is_empty() {
        return None;
    }
    // PASS 83a (FB-82a-04): a `->`-spelled member call may carry a MIXED
    // literal/meta argument list (sg answers `$w->q5(1, $B)`); every other
    // call shape keeps the meta-only argument contract.
    let arg_slots = if callee.contains("->") {
        match validate_argument_pattern(args) {
            Some(()) => None,
            None => {
                let slots = parse_member_arg_slots(args)?;
                Some(slots)
            }
        }
    } else {
        // PASS 105 (FB-104A-2): a plain call whose argument list mixes a
        // `$$$NAME` rest with positional singles classifies through the
        // per-position slot grammar — sg 0.45.2 answers the rest-slot grid
        // uniformly across go/js/php/py/rb/rs/ts (m3 matrix: trailing rest
        // binds >= 1, non-trailing rest binds exactly zero, two rests split
        // freely) and the general lane refuses every sibling-rest list.
        // Sole rests and pure-single lists keep the registered templates;
        // literal tokens and unprobed shapes (two rests with singles) stay
        // unclassified here exactly as before.
        match validate_argument_pattern(args) {
            Some(()) => None,
            None => parse_call_arg_slots(args).map(Some)?,
        }
    };
    if let Some(path) = parse_call_path(callee) {
        // PASS 75a (F74a-2): a `->`-spelled callee is the php member-call
        // lane — connector token-exact, never the plain Call lane. This arm
        // is plain-`->` only: parse_call_path refuses the `?`-carrying
        // nullsafe spelling (it keeps the dedicated optional lane below,
        // and the pre-gate carve serves its slot faces with
        // `nullsafe: true` — 86a-L5).
        if callee.contains("->") {
            return Some(NativeKind::MemberCall {
                path,
                arg_slots,
                require_continuation: false,
                nullsafe: false,
            });
        }
        return Some(NativeKind::Call { path, arg_slots });
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
/// PASS 127 (125B-F1, f127h): the reading is aligned with
/// `classify_if_template`, which tolerates ANY whitespace after the keyword
/// (`trim_start`). The old space/paren-only acceptance re-armed the exact
/// F1 silent drop one whitespace away: `if\n($X) { $B }` classified as If
/// yet skipped the argument-capture skip here, so `capture_arguments`
/// misbound `$X` from the condition's inner call and `if_condition_capture`
/// conflicted → candidates silently dropped (and the paren-free py spelling
/// lost its `: $B` body binding via [`body_capture`]).
fn is_if_prefixed(p: &str) -> bool {
    p.strip_prefix("if").is_some_and(|rest| {
        rest.starts_with('(') || rest.starts_with(char::is_whitespace)
    })
}

/// Parse `if ($COND) { $BODY }` / `if $COND { $BODY }` / `if $COND: $BODY`.
///
/// PASS 131 (130A-F6, f131e): a SINGLE-METAVARIABLE condition rides the
/// registered capture path (`cond: None`); a CONCRETE condition joins the
/// lane when it is spelling-admissible ([`if_cond_admissible`]) — the
/// per-language template build then decides answerability at the gate, and
/// the walk binds cond metas through `general_eq`. Unsupported shapes keep
/// the `None` fail-closed refusal — never fall through to call
/// classification.
///
/// PASS 122 (F1, f122a): the pattern body's brace-ness is threaded through
/// (`body_braced`) because sg treats it structurally — see the
/// [`NativeKind::If`] doc. A brace-less meta body still refuses here
/// (`parse_body_template` has no bare-identifier arm), keeping the registered
/// H-CONF-IFBODY loud class for `if ($X) $B` byte-stable.
fn classify_if_template(p: &str) -> Option<NativeKind> {
    let rest = p.strip_prefix("if")?.trim_start();
    let (condition, after) = if let Some(inner) = rest.strip_prefix('(') {
        // PASS 131 (130A-F6, f131e): the section close is the paren that
        // BALANCES the leading `(` — a depth scan. The old first-`)` slice
        // truncated any paren-bearing condition (`if (f($X))` read cond
        // `f($X`), a latent bug the registered concrete-cond refusal masked;
        // the f131e admission surfaces it.
        let Some(close) = balanced_paren_close(inner) else {
            return None;
        };
        (inner[..close].trim(), inner[close + 1..].trim_start())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '{' || c == ':')
            .unwrap_or(rest.len());
        (rest[..end].trim(), rest[end..].trim_start())
    };
    let cond = if is_single_metavariable(condition) {
        None
    } else if if_cond_admissible(condition) {
        Some(condition.to_string())
    } else {
            return None;
    };
    // PASS 137 (D6 elseif_same_line_F5): the else tail parses only after a
    // BRACED body — the balanced-brace scan finds the body section's true
    // extent so the tail after it can be read. A `:`-suite form keeps its
    // registered else-less class (py `else` clauses are unprobed; the
    // suite-text extent is line-ambiguous at the spelling level).
    let (body, body_braced, rest) = if let Some(inner) = after.trim_start().strip_prefix('{') {
        let Some(close) = balanced_brace_close(inner) else {
            return None;
        };
        (
            parse_body_section(inner[..close].trim())?,
            true,
            Some(&inner[close + 1..]),
        )
    } else {
        let (body, _) = parse_body_template_braced(after)?;
        (body, false, None)
    };
    // A tail that is neither `else {…}` nor `else if (…) {…}` keeps the
    // face's registered census-loud class (`if ($X) { $B } else $C`,
    // f124e) — never a silent else-less classification. An empty tail is
    // the plain else-less spelling.
    let alternative = match rest {
        Some(r) if r.trim().is_empty() => None,
        Some(r) => match parse_else_tail(r) {
            Some(alternative) => Some(alternative),
            None => return None,
        },
        None => None,
    };
    Some(NativeKind::If {
        cond,
        body,
        body_braced,
        alternative,
    })
}

/// PASS 137: byte index of the `}` that closes the section opened by a
/// leading `{` — the same depth scan [`balanced_paren_close`] runs for
/// conditions. String literals carrying unbalanced braces inside a body are
/// an unprobed residual (the scan may mis-terminate; the face keeps its
/// fail-closed class downstream).
fn balanced_brace_close(inner: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// PASS 137: the body-section grammar shared by the head and the else tail —
/// the [`parse_body_template_braced`] acceptance WITHOUT its outer-brace
/// spelling (the caller already unwrapped and consumed the braces).
fn parse_body_section(inner: &str) -> Option<Option<BodyTemplate>> {
    let inner = inner.trim();
    if inner.is_empty() {
        return Some(Some(BodyTemplate::Exactly(0)));
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

/// PASS 137: the `else …` tail after an if template's body section. Only the
/// two sg-probed spellings admit: `else { B }` and `else if (COND) { B }`
/// (recursively, so deep chains parse). Anything else keeps the face's
/// registered census-loud class.
fn parse_else_tail(rest: &str) -> Option<IfAlternative> {
    let rest = rest.trim();
    let tail = rest.strip_prefix("else")?.trim_start();
    if let Some(if_rest) = tail.strip_prefix("if") {
        let if_rest = if_rest.trim_start();
        let inner = if_rest.strip_prefix('(')?;
        let close = balanced_paren_close(inner)?;
        let condition = inner[..close].trim();
        let cond = if is_single_metavariable(condition) {
            None
        } else if if_cond_admissible(condition) {
            Some(condition.to_string())
        } else {
            return None;
        };
        // Per-level bare-meta captures (f137h chain): each tail level binds
        // ITS OWN section's meta — the pattern-global second-paren /
        // last-brace scans re-bind the wrong names at depth ≥ 2 and the
        // same-name conflict refuses the whole chain.
        let cond_meta = if is_single_metavariable(condition) {
            capture_name(condition).map(str::to_string)
        } else {
            None
        };
        let after = inner[close + 1..].trim_start();
        let inner_body = after.strip_prefix('{')?;
        let close_body = balanced_brace_close(inner_body)?;
        let body_section = inner_body[..close_body].trim();
        let body_meta = capture_name(body_section).map(str::to_string);
        let body = parse_body_section(body_section)?;
        let nested = parse_else_tail(&inner_body[close_body + 1..]);
        return Some(IfAlternative::ElseIf {
            cond,
            body,
            body_braced: true,
            cond_meta,
            body_meta,
            alternative: nested.map(Box::new),
        });
    }
    let inner_body = tail.strip_prefix('{')?;
    let close_body = balanced_brace_close(inner_body)?;
    let body_section = inner_body[..close_body].trim();
    let body_meta = capture_name(body_section).map(str::to_string);
    let body = parse_body_section(body_section)?;
    Some(IfAlternative::Else {
        body,
        body_braced: true,
        body_meta,
    })
}

/// PASS 131 (130A-F6): spelling-level admission for a concrete if condition.
/// Trivia and container spellings (comments, braces, terminators, ternaries,
/// colon families) stay out; every `$` token must substitute canonically.
/// Deeper sg-exactness is the per-language template build (the answerability
/// gate demands it) plus `general_eq` at walk time.
fn if_cond_admissible(cond: &str) -> bool {
    let trimmed = cond.trim();
    if trimmed.is_empty()
        || trimmed.contains("//")
        || trimmed.contains("/*")
        || trimmed.contains('{')
        || trimmed.contains('}')
        || trimmed.contains(';')
        || trimmed.contains('?')
        || trimmed.contains(':')
        // `$$` spellings (multi/universal namespace) are NOT the receipted
        // concrete-cond subset — `if ($$$B) { $A }` keeps its registered
        // census-loud class (f124e).
        || trimmed.contains("$$")
    {
        return false;
    }
    substitute_general_metavariables(trimmed).is_some()
}

/// PASS 131 (f131e): byte index of the `)` that closes the section opened
/// by a leading `(` — a depth scan over the raw text. String literals
/// carrying unbalanced parens inside a condition are an unprobed residual
/// (CNR §45 form-1): the scan may mis-terminate and the face keeps its
/// fail-closed refusal downstream.
fn balanced_paren_close(inner: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse the nested body section of a template, reporting whether the section
/// was BRACED. PASS 122 (F1): sg's if-matching is brace-ness structural, so
/// the If lane needs the flag; the shared [`parse_body_template`] keeps its
/// function/class signature and discards the flag.
///
/// PASS 122 (f122e): a BARE `:` suite section is sg's EMPTY suite, not "no
/// constraint" — first-hand oracle grid (`def $A($B):` / `class $A:` /
/// `if $X:` on python files with real bodies: sg answers `[]` rc1 silent
/// every cell — a suite-less statement never exists), so the bare-colon arm
/// now yields `Exactly(0)` instead of an unconstrained body. The old
/// unconstrained reading over-answered those py faces at the subject.
fn parse_body_template_braced(after: &str) -> Option<(Option<BodyTemplate>, bool)> {
    let after = after.trim();
    if after.is_empty() {
        return Some((None, false));
    }
    let (inner, braced) = if let Some(rest) = after.strip_prefix('{') {
        (rest.strip_suffix('}')?, true)
    } else {
        (after.strip_prefix(':')?, false)
    };
    let inner = inner.trim();
    if inner.is_empty() {
        // `{}` matches an empty body; a bare `:` suite is sg's EMPTY suite.
        return Some((Some(BodyTemplate::Exactly(0)), braced));
    }
    if inner
        .strip_prefix("$$$")
        .is_some_and(|rest| rest.is_empty() || is_metavar_name(rest))
    {
        return Some((Some(BodyTemplate::Any), braced));
    }
    if is_single_metavariable(inner) {
        return Some((Some(BodyTemplate::Exactly(1)), braced));
    }
    None
}

/// Function/class callers of the body grammar — brace-ness is inherent to a
/// declaration body in every grammar that reaches here, so the flag is
/// discarded (PASS 122 f122e: the bare-colon EMPTY-suite correction still
/// applies through the shared arm).
fn parse_body_template(after: &str) -> Option<Option<BodyTemplate>> {
    parse_body_template_braced(after).map(|(body, _)| body)
}

/// PASS 131 (130A-F4): true when the pattern's root is the sg universal
/// node metavariable (`$$NAME`), expando spellings normalized first. The
/// core census uses this to keep the universal lane's REGISTERED
/// line-collapsed rows (f96) by suppressing dedup byte spans there.
pub fn is_universal_root_pattern(lang: Language, pattern: &str) -> bool {
    let normalized = normalize_expando_meta_spelling(lang, pattern.trim());
    normalized
        .trim()
        .strip_prefix("$$")
        .is_some_and(is_metavar_name)
}

/// `$NAME` — exactly one metavariable, not `$$$`.
fn is_single_metavariable(s: &str) -> bool {
    s.strip_prefix('$')
        .is_some_and(|rest| !rest.starts_with('$') && is_metavar_name(rest))
}

/// Identifier token check shared with index signature builders.
#[inline]
pub fn is_pattern_ident(s: &str) -> bool {
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

/// PASS 83a (FB-82a-01): true for a `$$name` LOWERCASE double-dollar token
/// (`$$dyn`) — a php variable-variable literal that sg 0.45.2 matches
/// byte-exactly in dynamic-property LINK positions. Canonical `$$DYN` runs
/// are pure metavariables (handled before this check); mixed-case `$$Dyn`
/// keeps the registered refusal.
fn is_dollar2_lowercase_token(raw: &str) -> bool {
    raw.strip_prefix("$$")
        .is_some_and(|name| dollar_name_class(name) == Some(DollarTokenClass::LowercaseLed))
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
        // PASS 98 (F-97A-1 layer 2): java has NO expando preprocessing in sg
        // and its meta char IS `$` — a 1-run MixedCase token (`$Bx`) fails
        // `extract_meta_var` over the whole node text and parses as a literal
        // java IDENTIFIER: sg answers the verbatim rows (m3a2 oracle sets:
        // `k($Bx)` {14}, `$Bx + 2` {13}, probed 2026-09-09). Route the
        // all-MixedCase class to the literal lane on the raw bytes.
        // PASS 100 (FB-99B-2): the arm is ALL-MixedCase only, as this
        // comment always claimed — a canonical token is a real java meta
        // (`$$A + 1` answers the meta set through the structural lanes, m1),
        // so a canonical+mixed mix (`$A + $Bx`) is NOT a literal face: the
        // former `Canonical | MixedCase` admission hijacked the mix into
        // the literal lane where sg answers the meta rows (m3: java
        // `$A + $Bx` sg {J.java:3} vs subject []) — that divergence keeps
        // its registered form-1 row instead. Lowercase-led / multi-run
        // faces keep their registered classes.
        Language::Java => classes
            .iter()
            .all(|&class| class == DollarTokenClass::MixedCase),
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

/// PASS 100 (FB-99A-2): sg's `extract_meta_var` runs over the WHOLE node
/// text (meta_var.rs). In the no-expando languages (js/ts/java) a `$`-run
/// glued inside a larger identifier token (`µµµ$A`, `$AµµB`, `foo$$$A`)
/// never validates: the full token text fails the `[A-Z_0-9]` grammar, so
/// sg parses the token as an ordinary identifier and answers the verbatim
/// rows. This mirrors that whole-token validation for one candidate token.
fn sg_whole_token_is_meta(token: &str) -> bool {
    let Some(rest) = token.strip_prefix('$') else {
        return false;
    };
    let dollars = 1 + rest.chars().take_while(|&c| c == '$').count();
    let Some(tail) = token.get(dollars..) else {
        return false;
    };
    let valid_tail = |c: char| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit();
    match dollars {
        1 | 2 => {
            tail.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c == '_')
                && tail.chars().all(valid_tail)
        }
        // sg's ellipsis branch: 3 runs strip to an empty or all-valid name;
        // 4+ runs are never metas (literal).
        3 => tail.is_empty() || tail.chars().all(valid_tail),
        _ => false,
    }
}

/// PASS 100 (FB-99A-2): true when every `$`-carrying identifier token in
/// `pattern` is a GLUED literal identifier — its `$`-run continues into a
/// would-be meta name ([A-Z_]) but the WHOLE token text fails sg's
/// whole-token meta validation (`µµµ$A`, `$AµµB`, `$A$$B`, `foo$$$A`): in
/// the no-expando languages sg parses these as ordinary identifiers and
/// answers the verbatim rows. Registered genera stay OUT of the class:
/// dollars-only tokens (`$`, `$$`, `g($$)`, `$)(` — the bare-meta loud
/// class), runs continuing into anything but a meta name (`$3`, `$Ü`,
/// `$x`-led faces keep their own lanes), whole canonical meta tokens
/// (`$A`, `$$A`, `$$$A` — the structural meta lanes), and string-literal
/// spellings (`"$A$B"`, `"pre-$A"` — the F96 in-string meta faces).
fn pattern_tokens_are_all_literal(pattern: &str) -> bool {
    if !pattern.contains('$') || pattern.contains('"') || pattern.contains('\'') {
        return false;
    }
    let is_ident_char =
        |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$' || !c.is_ascii();
    // True when `token` KEEPS the pattern out of the glued-literal class.
    fn glued_veto(token: &str) -> bool {
        if !token.contains('$') {
            return false;
        }
        if token.bytes().all(|b| b == b'$') {
            return true;
        }
        let Some(first) = token.find('$') else {
            return false;
        };
        let dollars = 1 + token[first + 1..].bytes().take_while(|&b| b == b'$').count();
        match token.as_bytes().get(dollars + first) {
            Some(&b) if b == b'_' || b.is_ascii_uppercase() => sg_whole_token_is_meta(token),
            _ => true,
        }
    }
    let mut current = String::new();
    for c in pattern.chars() {
        if is_ident_char(c) {
            current.push(c);
        } else {
            if glued_veto(&current) {
                return false;
            }
            current.clear();
        }
    }
    !glued_veto(&current)
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

/// PASS 83a (FB-82a-04): parse a php member-call argument list into
/// per-position slots — the same token grammar as
/// [`parse_rhs_arg_slots`] (pure metavariable `$$$NAME`/`$NAME`/`$$NAME`
/// slots or single literal TOKENs, at most one rest). The r33-era
/// "at least one literal" floor is GONE (86a-L5, r37): meta-only lists
/// classify through the slots now (the `;`-terminated flat spelling had no
/// answering arm — see [`classify_member_call_mixed_args`]) and same-name
/// meta lists inherit bind_capture's sg conflict semantics. The floor was
/// vacuous at the other two call sites (pure-meta lists never reach them).
fn parse_member_arg_slots(args: &str) -> Option<Vec<ArgSlot>> {
    parse_rhs_arg_slots(args)
}

/// FB-84a-04 (r35): the assignment-RHS argument-list slot grammar — the
/// shared member/rhs token grammar (86a-L5, r37 unified
/// [`parse_member_arg_slots`] onto this body when the member carve's
/// >=1-literal floor fell): a meta/rest-only list (`f($A, $$$B)` — probed
/// sg answers) and the sole whole-list rest (`f($$$A)` — answers every
/// arity incl. the empty list) parse here. At most one rest (multiple
/// rests have no unambiguous contract).
fn parse_rhs_arg_slots(args: &str) -> Option<Vec<ArgSlot>> {
    let mut slots = Vec::new();
    let mut rests = 0usize;
    for part in args.split(',') {
        let part = part.trim();
        if let Some(name) = part.strip_prefix("$$$") {
            is_metavar_name(name).then_some(())?;
            slots.push(ArgSlot::Rest(name.to_string()));
            rests += 1;
        } else if is_pure_metavariable(part) {
            slots.push(ArgSlot::Meta(capture_name(part)?.to_string()));
        } else if is_dollar_literal_token(part) || is_literal_arg_token(part) {
            slots.push(ArgSlot::Literal(part.to_string()));
        } else {
            return None;
        }
    }
    (rests <= 1).then_some(slots)
}

/// PASS 105 (FB-104A-2): the PLAIN-call rest-slot grammar — pure
/// metavariable slots where at least one is a `$$$NAME` rest. sg 0.45.2
/// answers this family uniformly in the seven probed languages (m3 matrix);
/// pre-fix every mixed spelling failed [`validate_argument_pattern`] and
/// classified None (the ingress rc2'd where sg answers). Sole rests and
/// pure-single lists never reach here (the registered templates classify
/// them first); literal tokens and 2+-rests-with-singles have no probed
/// contract and refuse (fail-closed). A rest sharing its NAME with a single
/// in the SAME list also refuses (the registered 67c semantics keep the
/// `$O.log($$$A, $A)` collision face classifier-REJECTED — loud at the
/// ingress), and PASS 107 (FB-106A-1) extends the same refusal to TWO RESTS
/// sharing one name (`q($$$A, $$$A)`): the k >= 2 arm binds every HEAD rest
/// to `""` and only the tail rest to the whole-args text, so a shared name
/// conflict-binds (`$$$A=""` vs `$$$A=<whole args>` fails the equality
/// check) and would SILENTLY EMPTY the face — including the 1-arg row sg
/// 0.45.2 answers (m1_samespace oracle, 11 languages). The parse refusal
/// keeps that would-be-under-answer in the registered ingress-LOUD class
/// instead (hconf036 contract; the pre-r55 posture for the face). Distinct
/// names only.
/// PASS 107 (FB-106A-10): the two-rest admission generalizes to k >= 2
/// rests with no singles — sg's rest grid continues past two rests
/// (m1_samespace: `q($$$A, $$$B, $$$C)` answers the >= 2-arg rows in all
/// 11 probed languages).
fn parse_call_arg_slots(args: &str) -> Option<Vec<ArgSlot>> {
    let mut slots = Vec::new();
    let mut rests = 0usize;
    let mut singles = 0usize;
    for part in args.split(',') {
        let part = part.trim();
        if let Some(name) = part.strip_prefix("$$$") {
            is_metavar_name(name).then_some(())?;
            slots.push(ArgSlot::Rest(name.to_string()));
            rests += 1;
        } else if is_pure_metavariable(part) {
            slots.push(ArgSlot::Meta(capture_name(part)?.to_string()));
            singles += 1;
        } else {
            return None;
        }
    }
    if !((rests == 1 && singles >= 1) || (rests >= 2 && singles == 0)) {
        return None;
    }
    // Same-name collision refusal, order-independent: a rest sharing its
    // name with a single (either position), or any two rests sharing a name,
    // refuses (classify None -> ingress loud).
    let rest_names: Vec<&str> = slots
        .iter()
        .filter_map(|slot| match slot {
            ArgSlot::Rest(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    for (index, name) in rest_names.iter().enumerate() {
        if rest_names[index + 1..].contains(name) {
            return None;
        }
    }
    for slot in &slots {
        if let ArgSlot::Meta(name) = slot {
            if rest_names.contains(&name.as_str()) {
                return None;
            }
        }
    }
    Some(slots)
}

/// PASS 105 (FB-104A-2): sg's REST-SLOT argument semantics for plain calls —
/// deliberately NOT the php member lane's backtracking
/// [`arg_slots_match_from`]: the m3 oracle shows sg does NOT backtrack the
/// split (`q($$$A, $B)` answers ONLY the 1-arg row, never the 2-arg row).
///   - a TRAILING rest sharing the list with singles binds >= 1 argument
///     (`q($A, $$$B)` refuses the 1-arg row);
///   - a NON-TRAILING rest binds EXACTLY ZERO and the singles anchor the
///     candidate arity to the single count (`q($$$A, $B)` only the 1-arg
///     row; `q($A, $$$B, $C)` only the 2-arg row);
///   - k >= 2 rests with no singles answer every arity >= k-1 (head rests
///     bind zero, the tail binds all — the split choice is unobservable;
///     PASS 107 FB-106A-10 generalizes the two-rest arm, sg grid probed in
///     m1_samespace across 11 languages).
fn call_arg_slots_match(
    slots: &[ArgSlot],
    nodes: &[Node],
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let rest_count = slots
        .iter()
        .filter(|slot| matches!(slot, ArgSlot::Rest(_)))
        .count();
    let single_count = slots.len() - rest_count;
    let n = nodes.len();
    let (bound_nodes, rest_text) = match (rest_count, slots.last()) {
        // k >= 2 rests: bind zero to every head rest, all arguments to the
        // tail (sg answers n >= k-1: two rests -> >= 1 arg, three -> >= 2).
        (k, _) if k >= 2 => {
            if n < k - 1 {
                return None;
            }
            (
                0usize,
                source
                    .get(nodes[0].start_byte()..nodes[n - 1].end_byte())?
                    .to_string(),
            )
        }
        // trailing rest binds at least one argument.
        (1, Some(ArgSlot::Rest(_))) => {
            if n <= single_count {
                return None;
            }
            (
                single_count,
                source
                    .get(nodes[single_count].start_byte()..nodes[n - 1].end_byte())?
                    .to_string(),
            )
        }
        // non-trailing rest binds exactly zero; singles anchor the arity.
        (1, _) => {
            if n != single_count {
                return None;
            }
            (single_count, String::new())
        }
        _ => return None,
    };
    let mut consumed = 0usize;
    // The TAIL rest binds the whole argument text; every head rest binds
    // zero (the split is unobservable in hit sets — 106B INFO-3). Names are
    // distinct by parse_call_arg_slots's collision refusal.
    let tail_rest_index = slots
        .iter()
        .rposition(|slot| matches!(slot, ArgSlot::Rest(_)));
    for (index, slot) in slots.iter().enumerate() {
        match slot {
            ArgSlot::Meta(name) => {
                let text = node_text(nodes.get(consumed)?, source)?;
                bind_capture(captures, name, &text)?;
                consumed += 1;
            }
            ArgSlot::Rest(name) => {
                let text = if Some(index) == tail_rest_index {
                    rest_text.as_str()
                } else {
                    ""
                };
                bind_capture_kind(captures, name, text, true)?;
            }
            ArgSlot::Literal(_) => return None,
        }
    }
    debug_assert_eq!(consumed, bound_nodes);
    Some(())
}

/// PASS 83a (FB-82a-04): a LOWERCASE-led `$var` / `$$var` token in a mixed
/// argument list is sg LITERAL code — byte-compared against the candidate
/// argument, never a capture (canonical metas stay Meta slots; mixed-case
/// keeps its registered refusal by failing here).
fn is_dollar_literal_token(part: &str) -> bool {
    let dollars = part.bytes().take_while(|&b| b == b'$').count();
    (dollars == 1 || dollars == 2)
        && dollar_name_class(&part[dollars..]) == Some(DollarTokenClass::LowercaseLed)
}

/// PASS 83a: a single literal argument token — a balanced quoted string
/// (`'a'`, `"x"`, no `$`) or a plain alphanumeric token (`1`, `-1`, `1.5`,
/// `true`, `FOO`). A comma inside a string literal splits the list at the
/// comma; the unbalanced halves refuse here (fail-closed).
fn is_literal_arg_token(part: &str) -> bool {
    if part.len() >= 2 {
        let bytes = part.as_bytes();
        let quote = bytes[0];
        if quote == b'\'' || quote == b'"' {
            return bytes[bytes.len() - 1] == quote
                && !part[1..part.len() - 1].contains('$')
                && !part[1..part.len() - 1].contains(quote as char);
        }
    }
    !part.is_empty()
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// PASS 83a (FB-82a-04): per-position slot matching. Literal slots compare
/// the candidate argument text byte-exactly; meta slots bind positionally;
/// a rest slot binds the remaining arguments' ORIGINAL source bytes
/// (comma-joined in the source, zero-or-more) in the multi namespace and
/// backtracks over split points. MED-1 (84c, r35): a trailing rest SHARING
/// the list with an earlier slot must bind at least one argument (probed
/// sg: `$w->q9(1, $$$A)` refuses `q9(1)`, `f($v, $$$A)` refuses `f($v)`);
/// a whole-list rest may bind zero (`q9($$$A)` answers `q9()`) and a
/// non-trailing rest may bind zero (the registered mid-rest cells).
fn arg_slots_match(
    slots: &[ArgSlot],
    nodes: &[Node],
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    arg_slots_match_from(slots, nodes, source, captures, true)
}

/// `first` marks the un-recursed entry call: a sole-slot rest (first and
/// last at once) is the whole-list rest that may bind an empty argument
/// list.
fn arg_slots_match_from(
    slots: &[ArgSlot],
    nodes: &[Node],
    source: &str,
    captures: &mut BTreeMap<String, String>,
    first: bool,
) -> Option<()> {
    match slots.split_first() {
        None => nodes.is_empty().then_some(()),
        Some((ArgSlot::Literal(lit), rest)) => {
            let text = node_text(nodes.first()?, source)?;
            (text == lit).then_some(())?;
            arg_slots_match_from(rest, &nodes[1..], source, captures, false)
        }
        Some((ArgSlot::Meta(name), rest)) => {
            let text = node_text(nodes.first()?, source)?;
            bind_capture(captures, name, &text)?;
            arg_slots_match_from(rest, &nodes[1..], source, captures, false)
        }
        Some((ArgSlot::Rest(name), rest)) => {
            let lower = if rest.is_empty() && !(first && slots.len() == 1) {
                1
            } else {
                0
            };
            for end in lower..=nodes.len() {
                let (taken, remain) = nodes.split_at(end);
                let text = if taken.is_empty() {
                    String::new()
                } else {
                    source
                        .get(taken[0].start_byte()..taken[taken.len() - 1].end_byte())?
                        .to_string()
                };
                let mut trial = captures.clone();
                if bind_capture_kind(&mut trial, name, &text, true).is_some()
                    && arg_slots_match_from(rest, remain, source, &mut trial, false).is_some()
                {
                    *captures = trial;
                    return Some(());
                }
            }
            None
        }
    }
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
/// PASS 135 (134A-F7, f135e, grid X_py_del_two): python's `del $X` with ONE
/// canonical operand metavariable. sg binds X to the WHOLE operand-list
/// text (`del x, y` → X=`x, y`, sg n1), which the general lane's
/// placeholder-inside-expression_list unification cannot express (a
/// multi-operand candidate has more list children than the template), so
/// the face silent-emptied. This lane walks the `delete_statement`
/// candidates directly and binds the `expression_list` child's text
/// (single-operand candidates bind the same node text sg reports). Any
/// other spelling — concrete operands, multi-metavariable lists, `$$`
/// rest spellings — returns None and keeps its registered route
/// (fail-closed; unprobed faces stay unadmitted).
/// PASS 135 (134A-F7, f135e + f135c-ingress): the python `del $X` shape the
/// dedicated delete lane serves — `del`, whitespace, ONE canonical operand
/// metavariable. Shared by the matcher dispatch ([`match_py_delete_meta`]),
/// the language-free support gate, and the per-language census, so all three
/// gates agree on exactly the faces the lane answers sg-exactly.
/// PASS 137 (137A-F1): the lane's FIRST-NAMED-CHILD bind covered only the
/// head shape; the operand-position meta families sg 0.45.2 answers stayed
/// loud or silent (grid137 C-lane, sg receipts verbatim):
///   * `del $A, $B` × `del x, y` n1 A=`x` B=`y` — per-element binding;
///   * `del $A, $B` × `del x, y, z` n1 A=`x` B=`y` — candidate extras
///     absorb (prefix); `del $A, $B, $C` × 2 operands rc1 (short refuses);
///   * `del x, $B` / `del $A, y` n1 — mixed literal/meta lists bind the
///     metas positionally, literals byte-match;
///   * `del d[$K]` × `del d[k]` n1 K=`k` — literal-head subscript with a
///     meta index; × a 2-operand candidate rc1 (structural elements demand
///     the exact candidate element count);
///   * `del $O.$A` × `del o.a.b` n1 O=`o.a` A=`b` (× `del o.a` O=`o` A=`a`)
///     — the receiver meta absorbs all-but-last-attribute, the attr meta
///     binds the last identifier; the literal receiver form (`o.$A`) needs
///     the candidate object subtree byte-equal (`o.$A` × `o.x.a` rc1);
///   * `del ($X)` / `del($X)` × `del (x)` n1 X=`x` — the paren spelling
///     binds the INNER text (the plain `del $X` twin keeps binding the
///     parenthesized node whole: X=`(x)` — the f136 pin); `del ($X, $Y)`
///     × `del (x, y)` n1 X=`x` Y=`y`;
///   * `del $A, $B,` (trailing comma) rc1 — an empty element refuses.
/// PASS 139 (139A-F3 + 139B-F1, grids /tmp/phase139R/gridA + gridBCDEF,
/// sg receipts verbatim): the structural operand grammar is a full postfix
/// chain, unified LEVEL BY LEVEL over the left-nested tree —
///   * `del $O[$K1][$K2]` × `del d[k1][k2]` n1 O=`d` K1=`k1` K2=`k2`;
///     `del $O[$K].$A` × `del d[k].b` n1; `del $O.$A[$K]` × `del d.b[k]`
///     n1; `del $O.$A.$B` × `del x.y.z` n1 O=`x` A=`y` B=`z`;
///     `del d[$K][$J]` (literal head, meta indexes) n1; `del $O[k]` (meta
///     head, literal index) n1; literals BYTE-MATCH per level (`del
///     $O[$K][j]` × `del d[k1][k2]` rc1, `del m.n.$A` × `del x.y.z` rc1,
///     `del m.n[$K]` rc1) and candidate kinds must agree at every level
///     (`del $O[$K]` × `del x.y` rc1);
///   * the paren wrap is structurally SIGNIFICANT both directions: the
///     paren-required pattern refuses the paren-free candidate (`del
///     ($X, $Y)` × `del x, y` rc1 `[]` — the 137 comment wrongly claimed
///     this refusal existed; the expression_list branch now enforces it),
///     and `del (x)` × `del (x)` BINDS (G07) while `del (x)` × `del x`
///     stays rc1 `[]` (G02);
///   * `del ($O[$K])` — a structural element under the paren wrap — is
///     sg-ACCEPTED binds-nothing (C16 rc1 `[]`).
/// The predicate is shared by the dispatch, the census arm, and this lane,
/// so the three gates agree on exactly the admitted faces (the
/// F-135E-2 three-gates-agree contract).
/// PASS 140 (grids /tmp/phase140R G/R2/I, sg receipts verbatim): the
/// per-element operand grammar grows the CALL and PER-ELEMENT-PAREN
/// flavors and the `$$$` multi-namespace slots —
///   * `del $F($A)` × `del f(1)` n1 F=`f` A=`1`; `del f($A)` ×
///     `del f(x.y)` n1 A=`x.y` (the single argument node text);
///     `del f()` refuses (R2_del_call_emptyargs rc1); the call element is
///     EXACT-COUNT structural (`del $F($A)` × `del f(1), y` rc1,
///     G_del_call_2op);
///   * `del ($X), $Y` — the paren wrap is PER-ELEMENT when it does not
///     span the whole list: binds `(a), b` X=`a`(inner) Y=`b` (G_del_pm2),
///     `(d[k]), y` X=`d[k]` (G_del_pm2_struct), `($O[$K]), $y` slotwise
///     (G_del_pm_structmix), `($X), ($Y)` × `(a), (b)` (G_del_bothparen),
///     absorbs candidate extras (R2_del_pm2_extra_cand n1) and REFUSES the
///     bare-element candidate (G_del_pm2_free_cand rc1 — the Paren slot
///     demands a parenthesized_expression);
///   * `del d[$$$K]` binds K in the MULTI namespace (I_py_dollaratom);
///     `del $$X` / `del d[$$K]` keep the single namespace (already AGREE).
/// One PATTERN operand element of the PASS 137/139 delete lane.
#[derive(Debug, Clone, PartialEq)]
enum PyDelOperand {
    /// `$V` — binds the candidate operand text (the WHOLE list text when the
    /// single-element template faces a multi-operand candidate, f135e).
    Whole(String),
    /// PASS 140 (I_py_dollaratom law): `$$$V` — binds the whole candidate
    /// operand text in the MULTI namespace.
    WholeMulti(String),
    /// A bare meta under the top-level paren spelling (`del ($X)`) — binds
    /// the candidate's INNER text (sg X=`x` on `del (x)`).
    Paren(String),
    /// PASS 139 (grid139 G07): a fully literal element under the paren
    /// spelling (`del (x)`) — sg binds the paren candidate byte-exactly
    /// (n1) and refuses the paren-free spelling (rc1 `[]`).
    ParenLiteral(String),
    /// PASS 139 (grid139 C16): a STRUCTURAL element under the paren
    /// spelling (`del ($O[$K])`) — sg ACCEPTS and binds NOTHING (rc1 `[]`
    /// = valid empty): sg's pattern parse roots the paren group as a
    /// parenthesized expression, which no delete operand aligns with
    /// (PASS 141 standing probe: `del ($O[$K])`/`del ($O.$K)` × `del d[1]`
    /// both rc1 `[]` — the postfix faces HOLD).
    ParenStructural,
    /// PASS 141 standing-face correction (S_del_d3_paren): `del ($$$X)` —
    /// the multi slot under a single-element LIST-level wrap is NOT
    /// accepted-empty: sg BINDS the candidate's single inner operand in
    /// the multi namespace (`del (x)` n1, X=["x"]) and refuses 0/≥2
    /// elements (`del ()` / `del (a, b)` rc1 `[]`).
    ParenMulti(String),
    /// PASS 139 (grid139 C): a postfix chain `head[i1][i2].a…` — the head
    /// and every bracket/dot atom are a canonical meta or a literal
    /// identifier. sg unifies structurally LEVEL BY LEVEL left-nested
    /// (`d[k1][k2]` is `subscript(subscript(d,k1),k2)`): metas bind their
    /// level's node text, literals byte-match (`del $O[$K][j]` ×
    /// `del d[k1][k2]` rc1 — `j`≠`k2`). Pure-attribute chains keep the 137
    /// multi-element width; any bracket group demands the single-element
    /// list (the 137 subscript width).
    Postfix {
        head: PyDelAtom,
        ops: Vec<PyDelPostfix>,
    },
    /// PASS 140 (G_del_call_meta): `head(arg)` — exactly ONE argument; the
    /// element is EXACT-COUNT structural. The head is a meta or literal
    /// identifier; the argument a meta or literal identifier.
    Call {
        head: PyDelAtom,
        arg: PyDelAtom,
    },
    /// PASS 140 (G_del_pm2_struct): `($O[$K])` — a postfix chain under a
    /// PER-ELEMENT paren; unifies the chain against the parenthesized
    /// candidate's inner node.
    ParenPostfix {
        head: PyDelAtom,
        ops: Vec<PyDelPostfix>,
    },
    /// A literal identifier element (`del x, $B`) — byte-match.
    Literal(String),
}

/// One atom of a PASS 139 postfix chain: a canonical meta (`$K`) or a
/// literal identifier (`k`). PASS 140: `$$$K` binds the MULTI namespace
/// (I_py_dollaratom: `del d[$$$K]` × `del d[1]` multi K=`1`).
#[derive(Debug, Clone, PartialEq)]
enum PyDelAtom {
    Meta(String),
    MetaMulti(String),
    Literal(String),
    /// PASS 141 (grid F6_py_del_chain*): a dotted call head `$A.b` — the
    /// FIRST segment is a canonical meta binding the receiver BASE, every
    /// later segment a literal identifier byte-matched innermost-first.
    ChainBase { base: String, attrs: Vec<String> },
}

/// One postfix operator: a `[atom]` bracket group, a `.atom` attribute
/// access, or (PASS 142, 142A-F4 grids H10-H17) a `[lo:hi]` slice — the
/// step-slot spelling (`[$A:$B:$C]`, H17) demands a candidate step field;
/// the step-less pattern absorbs one (H10: `del $O[$A:$B]` binds
/// `del d[1:9:2]` with A=1 B=9).
#[derive(Debug, Clone, PartialEq)]
enum PyDelPostfix {
    Index(PyDelAtom),
    Attr(PyDelAtom),
    Slice {
        lo: PyDelAtom,
        hi: PyDelAtom,
        step: Option<PyDelAtom>,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct PyDelTemplate {
    operands: Vec<PyDelOperand>,
    /// The whole operand section was paren-wrapped (`del ($X, $Y)`).
    paren_wrapped: bool,
}

fn py_delete_meta_pattern(pattern: &str) -> bool {
    py_delete_template(pattern).is_some()
}

/// Parse the PASS 137 delete-lane pattern shapes (see the lane doc). Every
/// element must be one of the gridded spellings; anything else (trailing
/// commas, meta indexes, deep chains) keeps its registered route
/// (fail-closed). PASS 140: the per-element grammar grows the call
/// (`$F($A)` / `f($A)`) and per-element-paren (`($X)` / `($O[$K])` among
/// several elements) flavors, and `$$$`-prefixed slots bind the MULTI
/// namespace — the old "`$$` rests fail closed" note is superseded by the
/// I_py_dollaratom/dollarwhole cells (single `$$X` was always admitted via
/// `capture_name`; `$$$X` now binds multi instead of failing).
fn py_delete_template(pattern: &str) -> Option<PyDelTemplate> {
    let trimmed = pattern.trim();
    let after_del = trimmed.strip_prefix("del")?;
    // `del $X` demands the whitespace; the tight paren spelling `del($X)`
    // may glue the paren (sg answers both, grid137 C-lane).
    if !(after_del.starts_with(char::is_whitespace) || after_del.starts_with('(')) {
        return None;
    }
    let section = after_del.trim();
    if section.is_empty() {
        return None;
    }
    let (inner, paren_wrapped) = if section.starts_with('(') {
        let after_paren = section.strip_prefix('(')?;
        let close = balanced_paren_close(after_paren)?;
        if after_paren[close + 1..].trim().is_empty() {
            (after_paren[..close].trim(), true)
        } else {
            // PASS 140 (G_del_pm2): `del ($X), $Y` — the paren wraps only
            // the FIRST element; the list keeps its elements and each
            // element parses its own per-element shape (paren_wrapped
            // stays FALSE so the expression_list extraction branch stays
            // open and the per-element kinds decide).
            (section, false)
        }
    } else {
        (section, false)
    };
    let elements = split_top_level_commas(inner);
    if elements.is_empty() || elements.iter().any(|e| e.trim().is_empty()) {
        // `del $A, $B,` (trailing comma) is sg rc1 (grid137 follow-up).
        return None;
    }
    let mut operands = Vec::new();
    for element in &elements {
        let text = element.trim();
        if let Some(name) = capture_name(text) {
            if text.starts_with("$$$") {
                // PASS 140 (I_py_dollaratom law): `del $$$X` binds the
                // whole candidate operand in the MULTI namespace.
                operands.push(PyDelOperand::WholeMulti(name.to_string()));
            } else {
                operands.push(PyDelOperand::Whole(name.to_string()));
            }
            continue;
        }
        if is_pattern_ident(text) {
            operands.push(PyDelOperand::Literal(text.to_string()));
            continue;
        }
        // Structural shapes are gridded UNWRAPPED only (`del ($X)`/`del
        // ($X, $Y)` bind bare metas). PASS 139 (grid139 C16): a SINGLE
        // structural element under the paren spelling is sg's
        // ACCEPTED-binds-nothing class (`del ($O[$K])` rc1 `[]` — sg's
        // pattern parse roots the paren group as a parenthesized
        // expression no delete operand aligns with); anything else
        // structural under parens (incl. mixed lists) stays refused.
        if paren_wrapped {
            if elements.len() == 1
                && parse_py_del_postfix(text).is_some_and(|(_, ops)| !ops.is_empty())
            {
                operands.push(PyDelOperand::ParenStructural);
                continue;
            }
            return None;
        }
        // PASS 140 (G_del_pm2): a PER-ELEMENT paren (`($X)` /
        // `($O[$K])` as one list element among several) — a bare meta
        // inside re-tags to the Paren slot, a postfix chain to the
        // ParenPostfix slot; anything else inside refuses.
        if let Some(element_inner) = py_del_element_paren_section(text) {
            if let Some(name) = capture_name(element_inner) {
                operands.push(PyDelOperand::Paren(name.to_string()));
                continue;
            }
            if let Some((head, ops)) = parse_py_del_postfix(element_inner) {
                if !ops.is_empty() {
                    operands.push(PyDelOperand::ParenPostfix { head, ops });
                    continue;
                }
            }
            return None;
        }
        // PASS 140 (G_del_call_meta): the call element `head(arg)` —
        // exactly one meta/literal argument.
        if let Some((head, arg)) = parse_py_del_call(text) {
            operands.push(PyDelOperand::Call { head, arg });
            continue;
        }
        // PASS 139 (grid139 C): postfix chains — `head[i1][i2]`, `a.b.c`,
        // `$O[$K].$A`, `d.b[k]`… The head is a meta or literal identifier;
        // every bracket/dot atom likewise. PASS 141 (S_del G03_chain, the
        // 12-cell oracle law): index-bearing chains are admitted in
        // MULTI-element lists too (`del $O[$K], $Y` sg n1; `del $A, $O[$K]`
        // n1; `del f($G), $Y` n1) — the old "bracket groups keep the 137
        // single-element width" refusal was an unregistered over-refusal
        // (CNR holds no such row; the post-walk backstop kept the face
        // loud where sg 0.45.2 answers).
        if let Some((head, ops)) = parse_py_del_postfix(text) {
            operands.push(PyDelOperand::Postfix { head, ops });
            continue;
        }
        return None;
    }
    // A single Paren element is the `del ($X)` shape (the dedicated arm
    // binds the parenthesized candidate's INNER text). Multi-element
    // paren-wrapped lists (`del ($X, $Y)`) keep the inner flavors — the
    // wrap is LIST-level (one paren pair around the whole list), so the
    // elementwise path's list unwrap already consumed it and each operand
    // unifies against the bare element (pinprobe: X=`x` Y=`y`).
    // PASS 139 (grid139 G07): a single fully-literal element re-tags to
    // ParenLiteral — sg binds the paren candidate byte-exactly (`del (x)`
    // × `del (x)` n1) and refuses the paren-free spelling (`del (x)` ×
    // `del x` rc1 `[]`); the old code left it a plain Literal whose
    // paren-candidate element extraction found no named children and
    // answered silent-empty.
    if paren_wrapped && operands.len() == 1 {
        for operand in &mut operands {
            match operand {
                PyDelOperand::Whole(name) => {
                    *operand = PyDelOperand::Paren(name.clone());
                }
                PyDelOperand::Literal(lit) => {
                    *operand = PyDelOperand::ParenLiteral(lit.clone());
                }
                // PASS 141 (S_del_d3_paren, refuting the 140
                // R3_py_del_paren_d3 accepted-empty reading): `del ($$$X)`
                // sg-BINDS the single-element candidate (multi X=["x"]) and
                // refuses 0/≥2 inner operands — re-tag to the dedicated
                // ParenMulti slot, not ParenStructural.
                PyDelOperand::WholeMulti(name) => {
                    *operand = PyDelOperand::ParenMulti(name.clone());
                }
                _ => {}
            }
        }
    }
    Some(PyDelTemplate {
        operands,
        paren_wrapped,
    })
}

/// PASS 140: a WHOLE element that is one balanced paren group — `($X)`,
/// `($O[$K])`. Returns the trimmed inner text.
fn py_del_element_paren_section(text: &str) -> Option<&str> {
    let after = text.strip_prefix('(')?;
    let close = balanced_paren_close(after)?;
    if !after[close + 1..].trim().is_empty() {
        return None;
    }
    Some(after[..close].trim())
}

/// PASS 140 (G_del_call_meta): `head(arg)` — the head is a meta or literal
/// identifier; the paren group holds EXACTLY ONE meta/literal argument;
/// nothing after the group.
fn parse_py_del_call(text: &str) -> Option<(PyDelAtom, PyDelAtom)> {
    let open = text.find('(')?;
    let head_text = text[..open].trim();
    let head = py_del_call_atom(head_text).or_else(|| py_del_chain_head(head_text))?;
    let after = &text[open + 1..];
    let close = balanced_paren_close(after)?;
    if !after[close + 1..].trim().is_empty() {
        return None;
    }
    let arg_text = after[..close].trim();
    if arg_text.is_empty() {
        return None;
    }
    let arg = py_del_call_atom(arg_text)?;
    Some((head, arg))
}

/// PASS 141 (grid F6_py_del_chain*): a DOTTED call head `$A.b` — the FIRST
/// segment is a canonical meta (binding the receiver base; a `$$$` base is
/// ungridded and refuses) and every later segment a literal identifier; the
/// last segment is the call's literal tail. A chain-free head (`f`) never
/// reaches here.
fn py_del_chain_head(text: &str) -> Option<PyDelAtom> {
    if !text.contains('.') {
        return None;
    }
    let mut segments = text.split('.');
    let base = segments.next()?.trim();
    if base.starts_with("$$$") {
        return None;
    }
    let base_name = capture_name(base)?;
    let attrs: Vec<String> = segments
        .map(|segment| segment.trim().to_string())
        .collect();
    if attrs.is_empty() || attrs.iter().any(|segment| !is_pattern_ident(segment)) {
        return None;
    }
    Some(PyDelAtom::ChainBase {
        base: base_name.to_string(),
        attrs,
    })
}

fn py_del_call_atom(text: &str) -> Option<PyDelAtom> {
    if let Some(name) = capture_name(text) {
        if text.starts_with("$$$") {
            return Some(PyDelAtom::MetaMulti(name.to_string()));
        }
        return Some(PyDelAtom::Meta(name.to_string()));
    }
    is_pattern_ident(text)
        .then(|| PyDelAtom::Literal(text.to_string()))
}

/// PASS 139 (grid139 C): parse a postfix chain operand — `head`, then one
/// or more `[atom]` / `.atom` operators; every atom is a canonical meta or
/// a literal identifier; the whole text must be consumed; no commas inside
/// a bracket group (`d[k, j]` stays refused, the 137 discipline).
fn parse_py_del_postfix(text: &str) -> Option<(PyDelAtom, Vec<PyDelPostfix>)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let atom_at = |segment: &str| -> Option<PyDelAtom> {
        if let Some(name) = capture_name(segment) {
            if segment.starts_with("$$$") {
                Some(PyDelAtom::MetaMulti(name.to_string()))
            } else {
                Some(PyDelAtom::Meta(name.to_string()))
            }
        } else if is_pattern_ident(segment) {
            Some(PyDelAtom::Literal(segment.to_string()))
        } else {
            None
        }
    };
    // Head: the identifier/meta run before the first `[` or `.`.
    let head_end = text
        .find(['[', '.'])
        .unwrap_or(text.len());
    let head = atom_at(text[..head_end].trim())?;
    let mut ops = Vec::new();
    let mut rest = &text[head_end..];
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after
                .find(['[', '.'])
                .unwrap_or(after.len());
            let atom = atom_at(after[..end].trim())?;
            ops.push(PyDelPostfix::Attr(atom));
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix('[') {
            let close = after.find(']')?;
            // One atom per bracket group; a comma (or nested brackets)
            // refuses (the 137 `d[k, j]` discipline).
            let inner = after[..close].trim();
            if inner.contains([',', '[', ']']) {
                return None;
            }
            // PASS 142 (142A-F4, grids H10-H17): the slice spellings — one
            // or two `:` separators, every populated slot an atom (a
            // pattern-side OPEN slot is ungridded and keeps the loud
            // fail-closed route).
            if inner.contains(':') {
                let parts: Vec<&str> = inner.split(':').collect();
                if parts.len() > 3 {
                    return None;
                }
                let slot = |segment: &str| -> Option<PyDelAtom> {
                    let segment = segment.trim();
                    if let Some(name) = capture_name(segment) {
                        if segment.starts_with("$$$") {
                            Some(PyDelAtom::MetaMulti(name.to_string()))
                        } else {
                            Some(PyDelAtom::Meta(name.to_string()))
                        }
                    } else if is_pattern_ident(segment)
                        || !segment.is_empty()
                            && segment.chars().all(|c| c.is_ascii_digit())
                    {
                        Some(PyDelAtom::Literal(segment.to_string()))
                    } else {
                        None
                    }
                };
                let (lo, hi, step) = match parts.len() {
                    2 => (slot(parts[0])?, slot(parts[1])?, None),
                    3 => (
                        slot(parts[0])?,
                        slot(parts[1])?,
                        Some(slot(parts[2])?),
                    ),
                    _ => return None,
                };
                ops.push(PyDelPostfix::Slice { lo, hi, step });
                rest = &after[close + 1..];
                continue;
            }
            let atom = atom_at(inner)?;
            ops.push(PyDelPostfix::Index(atom));
            rest = &after[close + 1..];
        } else {
            return None;
        }
    }
    Some((head, ops))
}

/// Top-level comma split — nesting over `()[]{}"` keeps structural elements
/// (`d[k, j]` stays one element... though the lane refuses it later) whole.
fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

fn match_py_delete_meta(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    if !py_delete_meta_pattern(pattern) {
        return None;
    }
    let template = py_delete_template(pattern)?;
    let tree = parse_source(Language::Python, source).ok()?;
    let mut out = Vec::new();
    walk_py_delete_meta(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

fn walk_py_delete_meta(
    node: Node,
    source: &str,
    pattern: &str,
    template: &PyDelTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "delete_statement" && !is_in_comment_or_string(&node) {
        if let Some(hits) = py_delete_statement_match(&node, source, pattern, template) {
            out.extend(hits);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_py_delete_meta(child, source, pattern, template, out);
    }
}

/// The PASS 136/137 delete_statement unification. `operand_node` is the
/// statement's single named child: the `expression_list` for 2+ operands,
/// the bare operand expression for the inlined single (tree-sitter-python
/// inlines the one-operand list).
fn py_delete_statement_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &PyDelTemplate,
) -> Option<Vec<PatternMatch>> {
    let mut cursor = node.walk();
    let operand_node = node.children(&mut cursor).find(|child| child.is_named())?;
    let operand_kind = operand_node.kind();
    // Whole-bind: the single `$V` template keeps its f135e/f136 semantics —
    // the whole operand-list text (multi-operand candidates) or the bare
    // operand text (inlined single, parens included: X=`(x)`).
    if template.operands.len() == 1 {
        match template.operands.first() {
            Some(PyDelOperand::Whole(name)) => {
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture(&mut captures, name, node_text(&operand_node, source)?).is_none() {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            // PASS 140 (I_py_dollaratom law): `del $$$X` — the same
            // whole-bind in the MULTI namespace.
            Some(PyDelOperand::WholeMulti(name)) => {
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture_kind(
                    &mut captures,
                    name,
                    node_text(&operand_node, source)?,
                    true,
                )
                .is_none()
                {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            Some(PyDelOperand::Paren(name)) => {
                if operand_kind != "parenthesized_expression" {
                    return Some(Vec::new());
                }
                let Some(inner) = operand_node.named_child(0) else {
                    return Some(Vec::new());
                };
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture(&mut captures, name, node_text(&inner, source)?).is_none() {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            // PASS 139 (grid139 G07): `del (x)` × `del (x)` — sg n1, the
            // inner text byte-matching; the paren-free spelling refuses
            // (grid139 G02: `del (x)` × `del x` rc1 `[]`).
            Some(PyDelOperand::ParenLiteral(lit)) => {
                if operand_kind != "parenthesized_expression" {
                    return Some(Vec::new());
                }
                let Some(inner) = operand_node.named_child(0) else {
                    return Some(Vec::new());
                };
                if node_text(&inner, source).is_some_and(|text| text.trim() != lit) {
                    return Some(Vec::new());
                }
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            // PASS 139 (grid139 C16): the paren-structural spelling binds
            // NOTHING — sg rc1 `[]` with the candidate present; the walk's
            // empty IS the sg agreement.
            Some(PyDelOperand::ParenStructural) => {
                return Some(Vec::new());
            }
            // PASS 141 (S_del_d3_paren): `del ($$$X)` — the multi slot
            // demands the parenthesized single-element candidate; the inner
            // operand binds under the `$$$NAME` key. 0/≥2 inner operands
            // and paren-free candidates refuse (sg rc1 `[]`: `del ()`,
            // `del (a, b)`, `del x`).
            Some(PyDelOperand::ParenMulti(name)) => {
                if operand_kind != "parenthesized_expression" {
                    return Some(Vec::new());
                }
                let Some(inner) = operand_node.named_child(0) else {
                    return Some(Vec::new());
                };
                let mut captures = BTreeMap::new();
                if let Some(whole) = node_text(node, source) {
                    captures.insert("MATCH".to_string(), whole.to_string());
                }
                if bind_capture_kind(
                    &mut captures,
                    name,
                    node_text(&inner, source)?,
                    true,
                )
                .is_none()
                {
                    return Some(Vec::new());
                }
                let mut out = Vec::new();
                push_match_with_captures(node, source, pattern, captures, &mut out);
                return Some(out);
            }
            _ => {}
        }
    }
    // Elementwise paths need the candidate ELEMENTS (operand expressions).
    let paren_required = template.paren_wrapped
        || template
            .operands
            .iter()
            .any(|operand| matches!(operand, PyDelOperand::Paren(_)));
    let elements: Vec<Node> = if paren_required
        && matches!(operand_kind, "parenthesized_expression")
    {
        let Some(inner) = operand_node.named_child(0) else {
            return Some(Vec::new());
        };
        let mut inner_cursor = inner.walk();
        let inner_children: Vec<Node> =
            inner.children(&mut inner_cursor).filter(|c| c.is_named()).collect();
        match inner_children.len() {
            0 => return Some(Vec::new()),
            1 => {
                let only = &inner_children[0];
                if matches!(only.kind(), "tuple" | "expression_list" | "list") {
                    let mut only_cursor = only.walk();
                    let nested: Vec<Node> =
                        only.children(&mut only_cursor).filter(|c| c.is_named()).collect();
                    if nested.is_empty() {
                        vec![*only]
                    } else {
                        nested
                    }
                } else {
                    inner_children
                }
            }
            _ => inner_children,
        }
    } else if paren_required && operand_kind == "tuple" {
        // `del (x, y)` — python parses the paren-wrapped list as a bare
        // `tuple` (the parens belong to the tuple itself), so the wrap is
        // already consumed and the tuple's named children ARE the elements
        // (f137c pinprobe: X=`x` Y=`y`).
        let mut tuple_cursor = operand_node.walk();
        operand_node
            .children(&mut tuple_cursor)
            .filter(|c| c.is_named())
            .collect()
    } else if operand_kind == "expression_list" && !template.paren_wrapped {
        // PASS 139 (grid139 G01/G03): a LIST-level wrap (`del ($X, $Y)`,
        // paren_wrapped=true) REFUSES the paren-free candidate spellings —
        // sg rc1 `[]` (the paren wrap is structurally significant on the
        // pattern side). PASS 140 (G_del_pm2): a PER-ELEMENT paren
        // (`del ($X), $Y`) does NOT refuse the free candidate — the
        // elementwise unify's per-slot kind checks decide (the Paren slot
        // demands a parenthesized_expression element, G_del_pm2_free_cand
        // rc1; the postfix/meta slots bind, G_del_pm2 n1).
        let mut list_cursor = operand_node.walk();
        operand_node
            .children(&mut list_cursor)
            .filter(|c| c.is_named())
            .collect()
    } else {
        vec![operand_node]
    };
    // PASS 141 (S_del G03_chain, the 12-cell oracle law): operand-count
    // alignment — a SINGLE-operand template demands the exact candidate
    // count (`del $O[$K]` × `del d[k], y` sg rc1 `[]`, C17 re-verified),
    // while a MULTI-operand template aligns the operands as an in-order
    // PREFIX and absorbs TRAILING candidate extras (`del $O[$K], $Y` ×
    // `del d[k], y, z` sg n1 binding Y=`y`; `del $A, $O[$K]` ×
    // `del x, d[k], w` n1; `del f($G), $Y` × `del f(x), y, z` n1) —
    // leading extras and short candidates refuse either way
    // (`del z, d[k], y` / `del d[k]` rc1 `[]`). The zip below performs
    // the in-order prefix unification, so absorbing is implicit.
    if template.operands.len() == 1 {
        if elements.len() != 1 {
            return Some(Vec::new());
        }
    } else if elements.len() < template.operands.len() {
        return Some(Vec::new());
    }
    let mut captures = BTreeMap::new();
    if let Some(whole) = node_text(node, source) {
        captures.insert("MATCH".to_string(), whole.to_string());
    }
    for (operand, element) in template.operands.iter().zip(elements.iter()) {
        if !py_del_operand_unify(operand, element, source, &mut captures) {
            return Some(Vec::new());
        }
    }
    let mut out = Vec::new();
    push_match_with_captures(node, source, pattern, captures, &mut out);
    Some(out)
}

/// One candidate operand element against one PATTERN operand (PASS 137):
/// metas bind the element text, literals byte-match, the subscript shape
/// byte-matches its head and binds the index meta, the attribute shape
/// binds the all-but-last-attribute receiver and the last identifier.
fn py_del_operand_unify(
    operand: &PyDelOperand,
    element: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    match operand {
        PyDelOperand::Whole(name) => {
            bind_capture(captures, name, &node_text(element, source).unwrap_or_default())
                .is_some()
        }
        // PASS 140 (I_py_dollaratom law): `$$$X` binds in the MULTI
        // namespace.
        PyDelOperand::WholeMulti(name) => bind_capture_kind(
            captures,
            name,
            &node_text(element, source).unwrap_or_default(),
            true,
        )
        .is_some(),
        PyDelOperand::Paren(name) => {
            element.kind() == "parenthesized_expression"
                && element
                    .named_child(0)
                    .and_then(|inner| node_text(&inner, source))
                    .is_some_and(|text| bind_capture(captures, name, &text).is_some())
        }
        PyDelOperand::Literal(lit) => node_text(element, source)
            .is_some_and(|text| text.trim() == lit),
        PyDelOperand::Postfix { head, ops } => {
            py_del_postfix_unify(head, ops, element, source, captures)
        }
        // PASS 140 (G_del_call_meta): the candidate element must be a call
        // with EXACTLY ONE argument node; head/arg unify against the
        // function child and the argument node respectively (the argument
        // meta binds the whole node text: A=`x.y`).
        PyDelOperand::Call { head, arg } => {
            if element.kind() != "call" {
                return false;
            }
            let Some(function) = element.child_by_field_name("function") else {
                return false;
            };
            let Some(arguments) = element.child_by_field_name("arguments") else {
                return false;
            };
            let mut arg_cursor = arguments.walk();
            let args: Vec<Node> = arguments
                .children(&mut arg_cursor)
                .filter(|c| c.is_named())
                .collect();
            let [only] = args.as_slice() else {
                return false;
            };
            py_del_atom_unify(head, &function, source, captures)
                && py_del_atom_unify(arg, only, source, captures)
        }
        // PASS 140 (G_del_pm2_struct): the chain unifies against the
        // parenthesized candidate's INNER node (X=`d[k]`).
        PyDelOperand::ParenPostfix { head, ops } => {
            if element.kind() != "parenthesized_expression" {
                return false;
            }
            let Some(inner) = element.named_child(0) else {
                return false;
            };
            py_del_postfix_unify(head, ops, &inner, source, captures)
        }
        // ParenLiteral/ParenStructural never reach the elementwise unifier:
        // both exist only as single-element templates, which take their
        // dedicated arms before any elementwise work.
        PyDelOperand::ParenLiteral(_) | PyDelOperand::ParenStructural => true,
        // ParenMulti (PASS 141) also never reaches the elementwise
        // unifier: its dedicated arm returns before elementwise work.
        PyDelOperand::ParenMulti(_) => true,
    }
}

/// PASS 139 (grid139 C): one postfix-chain operand against one candidate
/// element — structural alignment LEVEL BY LEVEL over the left-nested
/// tree (`d[k1][k2]` is `subscript(subscript(d,k1),k2)`, `x.y.z` is
/// `attribute(attribute(x,y),z)`). The LAST operator aligns the element's
/// own kind; the prefix chain recurses into the object child. Metas bind
/// their level's node text, literals byte-match (`del $O[$K][j]` ×
/// `del d[k1][k2]` is sg rc1 — `j`≠`k2`), and the candidate kind must
/// agree at every level (`del $O[$K]` × `del x.y` rc1 — attribute vs
/// subscript).
fn py_del_postfix_unify(
    head: &PyDelAtom,
    ops: &[PyDelPostfix],
    element: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    let Some((last, prefix)) = ops.split_last() else {
        return py_del_atom_unify(head, element, source, captures);
    };
    match last {
        PyDelPostfix::Index(atom) => {
            if element.kind() != "subscript" {
                return false;
            }
            let object = element.child_by_field_name("object").or_else(|| element.named_child(0));
            let subscripts = element
                .child_by_field_name("subscripts")
                .or_else(|| element.named_child(1));
            let (Some(object), Some(subscripts)) = (object, subscripts) else {
                return false;
            };
            py_del_atom_unify(atom, &subscripts, source, captures)
                && py_del_prefix_unify(head, prefix, &object, source, captures)
        }
        PyDelPostfix::Slice { lo, hi, step } => {
            // PASS 142 (142A-F4, grids H10-H17): the slice element law.
            // The tree-sitter-python `slice` node has NO field names — its
            // bounds are POSITIONAL named children (absent bounds produce
            // no child) and the step marker is a SECOND anonymous `:`. A
            // 2-named slice is [start, stop]; a 3-named slice (two `:`s)
            // is [start, stop, step]; 0/1-named slices (open bounds) and
            // partial-step shapes refuse (H13/H14).
            if element.kind() != "subscript" {
                return false;
            }
            let object = element.child_by_field_name("object").or_else(|| element.named_child(0));
            let subscripts = element
                .child_by_field_name("subscripts")
                .or_else(|| element.named_child(1));
            let (Some(object), Some(subscripts)) = (object, subscripts) else {
                return false;
            };
            if subscripts.kind() != "slice" {
                return false;
            }
            // PASS 144 (143A-F7, grid D*): comments are sg trivia BETWEEN the
            // slice bounds/colons (D1 before `:` and D2 after `:` bind O=d
            // A=1 B=2) but refuse in the START-bound position (D3 sg rc1 —
            // the comment occupies the slot sg's exact-children match can
            // not skip there) or between the object and the slice node.
            // Filter trivia children from the positional bounds and veto the
            // leading position (both extra-attachment shapes).
            let mut sub_cursor = element.walk();
            if element.children(&mut sub_cursor).any(|c| {
                c.is_named()
                    && is_trivia_kind(c.kind())
                    && c.start_byte() >= object.end_byte()
                    && c.end_byte() <= subscripts.start_byte()
            }) {
                return false;
            }
            let mut slice_cursor = subscripts.walk();
            let slice_children: Vec<Node> = subscripts.children(&mut slice_cursor).collect();
            if slice_children
                .iter()
                .find(|c| c.is_named())
                .is_some_and(|first| is_trivia_kind(first.kind()))
            {
                return false;
            }
            let bounds: Vec<Node> = slice_children
                .iter()
                .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
                .copied()
                .collect();
            let mut colons_cursor = subscripts.walk();
            let colons = subscripts
                .children(&mut colons_cursor)
                .filter(|c| !c.is_named() && node_text(&c, source).is_some_and(|t| t == ":"))
                .count();
            let mut bind_atom = |atom: &PyDelAtom, bound: &Node| -> bool {
                node_text(bound, source)
                    .is_some_and(|t| py_del_atom_bind_text(atom, t.trim(), captures))
            };
            let (lo_node, hi_node, step_node) = match (bounds.len(), colons) {
                (2, 1) => (Some(&bounds[0]), Some(&bounds[1]), None),
                (3, 2) => (Some(&bounds[0]), Some(&bounds[1]), Some(&bounds[2])),
                _ => return false,
            };
            let (Some(lo_node), Some(hi_node)) = (lo_node, hi_node) else {
                return false;
            };
            if !bind_atom(lo, lo_node) || !bind_atom(hi, hi_node) {
                return false;
            }
            match step {
                Some(step_atom) => {
                    let Some(step_bound) = step_node else {
                        return false;
                    };
                    if !bind_atom(step_atom, step_bound) {
                        return false;
                    }
                }
                None => {}
            }
            py_del_prefix_unify(head, prefix, &object, source, captures)
        }
        PyDelPostfix::Attr(atom) => {
            if element.kind() != "attribute" {
                return false;
            }
            let object = element.child_by_field_name("object").or_else(|| element.named_child(0));
            let attribute = element
                .child_by_field_name("attr")
                .or_else(|| element.named_child(1));
            let (Some(object), Some(attribute)) = (object, attribute) else {
                return false;
            };
            py_del_atom_unify(atom, &attribute, source, captures)
                && py_del_prefix_unify(head, prefix, &object, source, captures)
        }
    }
}

/// The prefix recursion: an empty operator list means the HEAD atom
/// compares against the object node itself (the 137 receiver law — the
/// head meta absorbs the WHOLE all-but-last chain: `del $O.$A` ×
/// `del o.a.b` binds O=`o.a`); otherwise the prefix chain re-unifies.
fn py_del_prefix_unify(
    head: &PyDelAtom,
    prefix: &[PyDelPostfix],
    object: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    if prefix.is_empty() {
        py_del_atom_unify(head, object, source, captures)
    } else {
        py_del_postfix_unify(head, prefix, object, source, captures)
    }
}

/// PASS 142 (142A-F4): bind one slice-bound atom against its bound TEXT
/// (the slice arms compare field texts, not nodes).
fn py_del_atom_bind_text(
    atom: &PyDelAtom,
    text: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    match atom {
        PyDelAtom::Meta(name) => bind_capture(captures, name, text).is_some(),
        PyDelAtom::MetaMulti(name) => bind_capture_kind(captures, name, text, true).is_some(),
        PyDelAtom::Literal(literal) => text == literal,
        PyDelAtom::ChainBase { .. } => false,
    }
}

fn py_del_atom_unify(
    atom: &PyDelAtom,
    node: &Node,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    let Some(text) = node_text(node, source) else {
        return false;
    };
    match atom {
        PyDelAtom::Meta(name) => bind_capture(captures, name, text.trim()).is_some(),
        // PASS 140 (I_py_dollaratom): `$$$K` binds the MULTI namespace.
        PyDelAtom::MetaMulti(name) => {
            bind_capture_kind(captures, name, text.trim(), true).is_some()
        }
        PyDelAtom::Literal(lit) => text.trim() == lit,
        // PASS 141 (grid F6_py_del_chain*): peel the candidate attribute
        // chain innermost-first; every pattern segment byte-matches the
        // attr field and the receiver BASE binds the remaining object
        // text (`del $A.b($C)` × `del a.b(c)` → A=`a`; a non-attribute
        // function refuses — x_plain rc1 `[]`).
        PyDelAtom::ChainBase { base, attrs } => {
            let mut current = node.clone();
            for want in attrs.iter().rev() {
                if current.kind() != "attribute" {
                    return false;
                }
                let Some(attr) = current.child_by_field_name("attribute") else {
                    return false;
                };
                let Some(attr_text) = node_text(&attr, source) else {
                    return false;
                };
                if attr_text.trim() != want {
                    return false;
                }
                let Some(object) = current.child_by_field_name("object") else {
                    return false;
                };
                current = object;
            }
            node_text(&current, source)
                .is_some_and(|t| bind_capture(captures, base, t.trim()).is_some())
        }
    }
}

// ===========================================================================
// PASS 139 (139A-F2, grid139 B + 137A-F5 predicate trigger, grid139 I):
// dedicated statement-head lanes for the java `synchronized (R) { $B }`
// META-body face and the php `namespace [N] { $B }` BLOCK face — the
// spellings whose bare-meta body the general template cannot substitute
// (a placeholder alone inside a block is a parse ERROR) so they starved
// census-loud where sg 0.45.2 binds.
//
// The probed sg laws these lanes encode:
//   * java `synchronized ($X) { $B }` / `synchronized (lock) { $B }`:
//     binds the synchronized BLOCK statement — `$X` = the resource's inner
//     text (`lock`), `$B` = the single body statement text (`doIt();`);
//     a 0- or 2+-statement body refuses (grid B02/B06: sg rc1 `[]` —
//     accepted-empty); the CONCRETE-body spellings keep their PASS 137
//     general-lane route (grid B03/B04 AGREE) — this lane admits ONLY the
//     bare-meta body.
//   * php `namespace $N { $B }` / `namespace { $B }`: binds the block form
//     — `$N` = the namespace name text (`App`; the GLOBAL form binds no
//     name and refuses a named candidate, grid I04), `$B` = the single
//     member text (`function f() {}`); a 0- or 2+-member body refuses
//     (grid I03: sg rc1 `[]`). The `namespace $N;` STATEMENT form keeps
//     its PASS 137 general-lane route.
// ===========================================================================

/// PASS 139 (139A-F5, grid139 E): the JAVA class member-count classifier —
/// the shared [`classify_native`] refuses `class` Exactly bodies (the
/// f122e bare-colon empty-suite contract rides that refusal for every
/// language), so the java-scoped face classifies here and rides the SAME
/// Class member-count machinery through a constructed kind (run_queries:
/// the hopping scan, heritage/trivia refusals, and the member-count body
/// filter). sg law (grid139 E): `class $N { $B }` binds the single-member
/// java class (E01/E02/E08 n1), refuses empty/multi-member bodies and
/// heritage clauses (E03/E04/E07 rc1 `[]`).
fn classify_java_class_member_count(pattern: &str) -> Option<NativeKind> {
    let p = pattern.trim();
    let (declaration, _) = strip_declaration_modifiers(p);
    let rest = declaration.strip_prefix("class ")?;
    let head = rest
        .split(|c: char| c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim();
    if head.is_empty() {
        return None;
    }
    let name = match head.strip_prefix('$') {
        // PASS 140 (I_ja_class_dollar): `$$`/`$$$`-prefixed heads answer
        // like sg (`class $$N { $B }` n1, N single) — admit with no direct
        // name bind; the generic declaration-head capture path binds it.
        Some(_) if capture_name(head).is_some() => None,
        Some(_) => return None,
        None if is_pattern_ident(head) => Some(head.to_string()),
        None => return None,
    };
    let tail = rest[head.len()..].trim();
    let body = parse_body_template(tail)?;
    if !matches!(body, Some(BodyTemplate::Exactly(_))) {
        return None;
    }
    Some(NativeKind::Class {
        keyword: "class",
        name,
        body,
    })
}

/// The PASS 139 java synchronized META-body template: `synchronized`,
/// whitespace, balanced parens (any non-empty resource section), balanced
/// braces, a BARE-meta body, nothing after. Concrete bodies stay on the
/// general lane (the PASS 137 receipts).
fn ja_synchronized_meta_template(pattern: &str) -> Option<String> {
    let p = pattern.trim();
    let rest = p.strip_prefix("synchronized")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    let resource = inner[..close].trim();
    let rest = inner[close + 1..].trim_start();
    let inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(inner)?;
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    if resource.is_empty() {
        return None;
    }
    capture_name(inner[..close].trim()).map(str::to_string)
}

fn match_java_synchronized_meta(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let body_name = ja_synchronized_meta_template(pattern)?;
    let tree = parse_source(Language::Java, source).ok()?;
    let mut out = Vec::new();
    walk_java_synchronized_meta(
        tree.root_node(),
        source,
        pattern,
        &body_name,
        &mut out,
    );
    Some(out)
}

fn walk_java_synchronized_meta(
    node: Node,
    source: &str,
    pattern: &str,
    body_name: &str,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "synchronized_statement" && node.is_named() && !is_in_comment_or_string(&node)
    {
        if let Some(hit) = java_synchronized_meta_match(&node, source, pattern, body_name) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java_synchronized_meta(child, source, pattern, body_name, out);
    }
}

fn java_synchronized_meta_match(
    node: &Node,
    source: &str,
    pattern: &str,
    body_name: &str,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    // The resource is the parenthesized expression child; sg binds its
    // INNER text (grid139 B01: X=`lock`).
    let resource = children
        .iter()
        .copied()
        .find(|c| c.kind() == "parenthesized_expression")?;
    let resource_inner = resource.named_child(0)?;
    let resource_text = node_text(&resource_inner, source)?;
    // The resource section of the PATTERN binds like the general lane: a
    // meta takes the inner text; any other spelling byte-matches.
    if let Some(meta) = ja_synchronized_resource_meta_name(pattern) {
        if meta.bind(&mut captures, resource_text.trim()).is_none() {
            return None;
        }
    } else {
        let want = ja_synchronized_resource_literal_text(pattern)?;
        if resource_text.trim() != want {
            return None;
        }
    }
    // The body: the block child; `$B` binds a ONE-statement body's
    // statement text (0/2+-statement candidates refuse — grid B02/B06).
    let body = children.iter().copied().find(|c| c.kind() == "block")?;
    let mut body_cursor = body.walk();
    let stmts: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    let text = node_text(only, source)?;
    if bind_capture(&mut captures, body_name, text).is_none() {
        return None;
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// The resource section between the balanced parens of a PASS 139
/// synchronized META-body pattern.
fn ja_synchronized_resource_section(pattern: &str) -> Option<&str> {
    let p = pattern.trim();
    let rest = p.strip_prefix("synchronized")?;
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    Some(inner[..close].trim())
}

fn ja_synchronized_resource_meta_name(pattern: &str) -> Option<LaneMeta> {
    // PASS 140 (I_ja_dollarres/I_ja_dollarres3): the resource meta keeps
    // its sg namespace — `$$X` binds single (already AGREE), `$$$X` binds
    // the MULTI namespace (grid I_ja_dollarres3: multi X=`lock`).
    ja_synchronized_resource_section(pattern).and_then(lane_meta)
}

fn ja_synchronized_resource_literal_text(pattern: &str) -> Option<String> {
    let section = ja_synchronized_resource_section(pattern)?;
    if section.is_empty() || capture_name(section).is_some() {
        return None;
    }
    Some(section.to_string())
}

/// The PASS 139 php braced-namespace template: `namespace [NAME] { $B }` —
/// the name is a bare canonical meta (any sg namespace: `$$N` single,
/// `$$$N` multi per R3_php_name_dollar3), a literal name byte-matched
/// (PASS 140 J_php_litname: `namespace App { $B }` n1; J_php_litname_neg
/// `namespace Other { $B }` rc1 `[]`), or ABSENT (the global block form);
/// the body is any canonical meta — PASS 141 (141A-F3, grid F3_*): `$$B`
/// follows the ONE-member `$B` law and `$$$B` binds the MULTI list at any
/// member count (the 140 RefusedEmpty reading is refuted of record; see
/// the CNR §49 annotation). The `namespace $N;` statement form keeps its
/// PASS 137 route.
struct PhpNamespaceBlockTemplate {
    name: Option<LaneName>,
    body: PhpNamespaceBody,
}

/// PASS 144 (143A-F13 / CNR §51.c row 3 predicate FIRED): the body slot's
/// MIXED exact-order face — literal prefix statements followed by ONE
/// trailing single-member meta (`const A = 1; $$B`). sg BINDS the
/// exact-order face (grid M1 B=`f();`, refuting the registered row's
/// "sg binds nothing on every mixed-body cell" premise) and binds nothing
/// on the mixed-ORDER (M2), zero-trailing (M3), two-trailing (M4), and
/// prefix-mismatch (M5) faces.
#[derive(Debug, Clone)]
enum PhpNamespaceBody {
    Meta(LaneMeta),
    Mixed { prefix: String, tail: LaneMeta },
}

/// Split `const A = 1; $$B` into the literal prefix statement run and the
/// ONE trailing single-member meta (last whitespace-delimited word). The
/// `$$$B` multi tail keeps its ungridded route and a prefix not ending in
/// `;` is not a statement run.
fn php_mixed_namespace_body(section: &str) -> Option<(String, LaneMeta)> {
    let s = section.trim();
    let tail_start = s.rfind(char::is_whitespace)? + 1;
    let tail = &s[tail_start..];
    if tail.is_empty() {
        return None;
    }
    let meta = lane_meta(tail)?;
    if meta.multi {
        return None;
    }
    let prefix = s[..tail_start].trim_end();
    if prefix.is_empty() || !prefix.ends_with(';') {
        return None;
    }
    Some((prefix.to_string(), meta))
}

fn php_namespace_block_template(pattern: &str) -> Option<PhpNamespaceBlockTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("namespace")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let (name, brace_section) = if let Some(inner) = rest.strip_prefix('{') {
        (None, inner)
    } else {
        let brace_at = rest.find('{')?;
        let name_section = rest[..brace_at].trim();
        if name_section.is_empty() {
            return None;
        }
        let name = if let Some(meta) = lane_meta(name_section) {
            LaneName::Meta(meta)
        } else if !name_section.contains('$')
            && !name_section.starts_with(|c: char| c.is_ascii_digit())
            && name_section
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '\\')
        {
            LaneName::Literal(name_section.to_string())
        } else {
            return None;
        };
        (Some(name), &rest[brace_at + 1..])
    };
    let close = balanced_brace_close(brace_section)?;
    if !brace_section[close + 1..].trim().is_empty() {
        return None;
    }
    let body_section = brace_section[..close].trim();
    // PASS 141 (141A-F3, grid F3_*): the 140 `$$`-body RefusedEmpty reading
    // is REFUTED of record — sg BINDS `$$B` under the ONE-member `$B` law
    // (any single member) and `$$$B` under the MULTI list law (ANY member
    // count, 0 → the empty list; refuse only at ≥2 for the single
    // namespace). The parse therefore treats every canonical meta body
    // alike; the member-count law lives in the walk.
    // PASS 144 (143A-F13): the MIXED exact-order face joins the slot (see
    // [`PhpNamespaceBody`]).
    let body = if let Some(meta) = lane_meta(body_section) {
        PhpNamespaceBody::Meta(meta)
    } else if let Some((prefix, tail)) = php_mixed_namespace_body(body_section) {
        PhpNamespaceBody::Mixed { prefix, tail }
    } else {
        return None;
    };
    Some(PhpNamespaceBlockTemplate { name, body })
}

fn match_php_namespace_block(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = php_namespace_block_template(pattern)?;
    let tree = parse_source(Language::Php, source).ok()?;
    let mut out = Vec::new();
    walk_php_namespace_block(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

fn walk_php_namespace_block(
    node: Node,
    source: &str,
    pattern: &str,
    template: &PhpNamespaceBlockTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "namespace_definition" && node.is_named() && !is_in_comment_or_string(&node)
    {
        if let Some(hit) = php_namespace_block_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_namespace_block(child, source, pattern, template, out);
    }
}

fn php_namespace_block_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &PhpNamespaceBlockTemplate,
) -> Option<PatternMatch> {
    let name_node = node.child_by_field_name("name");
    match (&template.name, &name_node) {
        // The global pattern refuses a named candidate and vice versa
        // (grid139 I04: `namespace { $B }` × `namespace App { … }` rc1 `[]`).
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(want), Some(name_node)) = (&template.name, name_node) {
        let name_text = node_text(&name_node, source)?;
        want.bind_or_match(&mut captures, name_text.trim())?;
    }
    // The body: the compound_statement child. PASS 141 (grid F3_*): the
    // member-count law follows the meta's namespace — `$B`/`$$B` (single)
    // bind a ONE-member body's member text (0/2+-member candidates refuse —
    // grid139 I03 + F3_app_d2_2mem); `$$$B` (multi) binds EVERY member at
    // ANY count, 0 members → the empty list (F3_app_d3_0mem n1).
    let body = node.child_by_field_name("body")?;
    let mut body_cursor = body.walk();
    let members: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let body_meta = match (&template.body, &members[..]) {
        (PhpNamespaceBody::Meta(body_meta), _) => body_meta,
        (PhpNamespaceBody::Mixed { prefix, tail }, members) => {
            // PASS 144 (143A-F13, grid M*): the candidate's members must be
            // the prefix statement run EXACTLY (statement-wise,
            // whitespace-normalized, comment-stripped) followed by exactly
            // ONE trailing member — the mixed-ORDER (M2), zero-trailing
            // (M3), two-trailing (M4), and prefix-mismatch (M5) faces all
            // refuse (sg rc1).
            let doc = format!("<?php\nnamespace __Px {{ {prefix} }}\n");
            let tpl_tree = parse_source(Language::Php, &doc).ok()?;
            if tpl_tree.root_node().has_error() {
                return None;
            }
            let mut root_cursor = tpl_tree.root_node().walk();
            let prefix_members: Vec<Node> = tpl_tree
                .root_node()
                .children(&mut root_cursor)
                .find(|n| n.kind() == "namespace_definition")
                .and_then(|ns| ns.child_by_field_name("body"))
                .map(|body| {
                    let mut c = body.walk();
                    body.children(&mut c)
                        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
                        .collect()
                })
                .unwrap_or_default();
            if prefix_members.is_empty() || members.len() != prefix_members.len() + 1 {
                return None;
            }
            let normalize = |node: &Node, src: &str| -> Option<String> {
                Some(strip_comment_spans(node_text(node, src)?).split_whitespace().collect())
            };
            for (p_stmt, c_stmt) in prefix_members.iter().zip(members.iter()) {
                if normalize(p_stmt, &doc) != normalize(c_stmt, source) {
                    return None;
                }
            }
            let tail_text = node_text(&members[members.len() - 1], source)?.trim();
            if tail.bind(&mut captures, tail_text).is_none() {
                return None;
            }
            return namespace_block_hit(node, source, pattern, captures);
        }
    };
    if body_meta.multi {
        // The subject's capture map is text-valued: the MULTI encoding of
        // record joins the member texts with '\n' (sg emits a JSON array —
        // an encoding difference of record; count/span parity holds).
        let mut texts = Vec::new();
        for member in &members {
            texts.push(node_text(member, source)?.trim().to_string());
        }
        if body_meta.bind(&mut captures, &texts.join("\n")).is_none() {
            return None;
        }
    } else {
        let [only] = members.as_slice() else {
            return None;
        };
        let text = node_text(only, source)?;
        if body_meta.bind(&mut captures, text).is_none() {
            return None;
        }
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// PASS 144: the shared php namespace-block hit constructor (the Mixed
/// exact-order arm returns through here).
fn namespace_block_hit(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: BTreeMap<String, String>,
) -> Option<PatternMatch> {
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

// ===========================================================================
// PASS 140 (r72 remediation; oracle grids /tmp/phase140R grid_pre/r2/r3):
// the directive/import root family, the csharp checked/unchecked EXPRESSION
// root, the java synchronized nested-block + METHOD faces, the remaining
// statement roots (kotlin typealias/for, swift for±where, rust let-else,
// c goto), and go's `$`-carrying `;`-ful accepted-empty pair — every face
// oracle-verified against sg 0.45.2 before coding.
//
// The probed sg laws encoded here (verbatim receipt cells):
//   * py `import $X` binds each import_statement, X = the FIRST module
//     name (`import os, sys` → X=`os`, E_py_import_two; per-statement
//     answers E_py_import_multi n2); an aliased child binds the WHOLE
//     child text under the plain shape (X=`os as o`,
//     R2_py_importX_alias_cand); `import $X as $Y` splits name/alias
//     (E_py_import_alias); `from $M import $X` binds M and the FIRST
//     imported name (E_py_from).
//   * java `import [static] T[.*];` — the static/star decorations
//     demarcate DISTINCT faces: `import $X;` refuses static/star
//     candidates (R2_ja_import_plain_pat_static_cand /
//     R2_ja_import_star_pat_plain_cand rc1 []), `import static $X;` binds
//     everything after `static` (E_ja_import_static), `import $X.*;`
//     binds the dot-prefix (E_ja_import_star).
//   * rs `use $X;` binds the whole use argument incl. brace groups
//     (E_rs_use_braces); `use $X as $Y;` splits at the top-level `as`
//     (E_rs_use_alias). php `use $X;` binds the whole clause incl. a
//     `function`/`const` kind keyword (R3_php_use_function). cs
//     `using $N;` binds the namespace name (E_cs_using1), REFUSES the
//     `=` alias spelling (E_cs_using_alias rc1) and never answers the
//     using-STATEMENT face (R3_cs_using_stmt_neg rc1 — using_directive
//     kind only).
//   * go `goto $L;` / `import $X;` are sg-ACCEPTED bind-nothing spellings
//     (H_go_goto_meta / E_go_importblock rc1 []) — walk-empty, never
//     loud — while c `goto $L;` BINDS every goto_statement's label
//     (F_c_goto n1; R2_c_goto_2sites n2): the language scoping is
//     load-bearing.
//   * kotlin `typealias $N = $T` binds both slots (F_kt_typealias);
//     kotlin `for ($X in $C) { $B }` binds X/C and the block-INNER text
//     trimmed BOTH ends (F_kt_for B=`g(x)`; R2_kt_for_2stmt
//     B=`g(x)\n        h(x)`) and refuses the brace-less body
//     (R2_kt_for_nobrace rc1).
//   * swift `for $X in $C [where $W] { $B }` — the `where` clause
//     presence must agree on BOTH sides (F_sw_for_plain × where-src rc1;
//     R2_sw_where_pat_nowhere_src / R2_sw_where_pat_plain_src rc1); W =
//     the where-condition text; B = the body-inner text trim_start ONLY
//     (sg KEEPS the trailing whitespace: B=`g(x)\n    `, F_sw_for_where).
//   * rs `let $P = $E else { $B };` binds P/E and the ONE-statement else
//     body (F_rs_letelse B=`return;`; R2_rs_letelse_2stmt rc1); a
//     no-else candidate refuses (R2_rs_letelse_noelse rc1).
//   * csharp `checked($E)` / `unchecked($E)` are EXPRESSION roots — sg
//     binds the checked_expression node with E = the inner expression
//     text (C_checkedE_assign/ret/arg n1) and refuses the statement-
//     position candidate (C_checkedE_vs_stmt rc1 — the checked STATEMENT
//     keeps its 137 lane).
//   * java synchronized compositions: the nested block
//     (`synchronized ($X) { synchronized ($Y) { $B } }`, D_sync_nested
//     n1, D_sync_nested_3 n1, D_sync_nested_lit n1) binds every slot at
//     its own level; the METHOD face (`synchronized void $M() { $B }`,
//     D_sync_method n1) binds the method_declaration, HOPS ordinary
//     keyword modifiers positionally (`static synchronized void m()`
//     answers the modifier-less pattern, D_sync_static n1) while the
//     pattern's modifiers demand the candidate carry them in order
//     (`static synchronized …` pattern × plain candidate rc1,
//     D_sync_static_pat); the method-BODY face (`void $M() {
//     synchronized ($X) { $B } }`, D_sync_method_body n1) binds the
//     method and the inner block's slots; the ONE-statement body law
//     holds at every slot (D_sync_method_2stmt rc1).
// ===========================================================================

/// A lane metavariable slot with its sg namespace: `$$$NAME` binds the MULTI
/// namespace (key `$$$NAME` in the capture map), `$`/`$$NAME` the single
/// one (grids I/R2/R3: ja `synchronized ($$$X)` binds multi X,
/// `synchronized ($$X)` binds single X, py `del d[$$$K]` binds multi K).
#[derive(Debug, Clone)]
struct LaneMeta {
    name: String,
    multi: bool,
}

fn lane_meta(token: &str) -> Option<LaneMeta> {
    let token = token.trim();
    if let Some(name) = token.strip_prefix("$$$") {
        return is_metavar_name(name).then_some(LaneMeta {
            name: name.to_string(),
            multi: true,
        });
    }
    capture_name(token).map(|name| LaneMeta {
        name: name.to_string(),
        multi: false,
    })
}

impl LaneMeta {
    fn bind(&self, captures: &mut BTreeMap<String, String>, text: &str) -> Option<()> {
        bind_capture_kind(captures, &self.name, text, self.multi)
    }
}

/// A name slot that is either a lane meta or a byte-matched literal
/// (`using System;` — R2_cs_using_lit: sg answers the directive node n1
/// where the cs statement lane over-served).
#[derive(Debug, Clone)]
enum LaneName {
    Meta(LaneMeta),
    Literal(String),
}

impl LaneName {
    fn bind_or_match(&self, captures: &mut BTreeMap<String, String>, text: &str) -> Option<()> {
        match self {
            LaneName::Meta(meta) => meta.bind(captures, text),
            LaneName::Literal(literal) => (text == literal).then_some(()),
        }
    }
}

/// PASS 142 (142A-F4, grids F1/F4/F7/F10/H4-H5/I7): the cs using-alias rhs
/// slot faces — sg's structural law beyond the single-meta spelling:
/// `($T, $U)` is the TUPLE face (element slots, count-exact — F7 refuses a
/// 3-element candidate), `$T[]` is the ARRAY-SUFFIX face (T binds the
/// element type; `int[][]` binds T=`int[]`, H4 — the suffix strips
/// recursively), and `global::$T` is the QUALIFIED face (T binds the text
/// AFTER the `global::` qualifier — I7 T='S').
#[derive(Debug, Clone)]
enum AliasRhs {
    One(LaneName),
    Tuple(Vec<LaneName>),
    ArraySuffix(LaneName),
    QualifiedGlobalMeta(LaneMeta),
}

impl AliasRhs {
    /// Bind the alias rhs against the candidate's rhs text (already
    /// comment-stripped and trimmed by the caller).
    fn bind_or_match(&self, captures: &mut BTreeMap<String, String>, text: &str) -> Option<()> {
        match self {
            AliasRhs::One(name) => name.bind_or_match(captures, text),
            AliasRhs::Tuple(slots) => {
                // Count-exact element alignment over the depth-aware comma
                // split (F7: 3-element candidate refuses a 2-slot pattern).
                let inner = text.strip_prefix('(')?.strip_suffix(')')?;
                let elements = split_top_level_commas(inner);
                if elements.len() != slots.len() {
                    return None;
                }
                for (slot, element) in slots.iter().zip(elements.iter()) {
                    slot.bind_or_match(captures, element.trim())?;
                }
                Some(())
            }
            AliasRhs::ArraySuffix(name) => {
                // The candidate must carry the suffix; the element binds the
                // trimmed prefix (H4 recursion: `int[][]` → `int[]`).
                let element = text.strip_suffix(']')?.strip_suffix('[')?;
                name.bind_or_match(captures, element.trim())
            }
            AliasRhs::QualifiedGlobalMeta(meta) => {
                let rest = text.strip_prefix("global::")?;
                if rest.is_empty() || rest.contains(char::is_whitespace) {
                    return None;
                }
                meta.bind(captures, rest)
            }
        }
    }
}

/// Parse the ALIAS rhs slot faces. `text` is the trimmed rhs of the `=`
/// split. Falls back to the single-slot face only when no tuple/array/
/// qualified shape applies.
fn alias_rhs_slot(text: &str) -> Option<AliasRhs> {
    if let Some(element) = text.strip_suffix("[]").map(str::trim) {
        if let Some(slot) = lane_slot(element) {
            return Some(AliasRhs::ArraySuffix(slot));
        }
    }
    if text.starts_with('(') && text.ends_with(')') {
        let inner = &text[1..text.len() - 1];
        let slots = split_top_level_commas(inner)
            .into_iter()
            .map(|element| lane_slot(element.trim()))
            .collect::<Option<Vec<_>>>()?;
        if !slots.is_empty() {
            return Some(AliasRhs::Tuple(slots));
        }
    }
    if let Some(rest) = text.strip_prefix("global::") {
        if let Some(meta) = lane_meta(rest) {
            return Some(AliasRhs::QualifiedGlobalMeta(meta));
        }
    }
    lane_slot(text).map(AliasRhs::One)
}

/// A lane name slot: canonical meta or dotted ident path literal.
fn lane_slot(text: &str) -> Option<LaneName> {
    if let Some(meta) = lane_meta(text) {
        return Some(LaneName::Meta(meta));
    }
    directive_path_literal(text).then(|| LaneName::Literal(text.to_string()))
}

/// A dotted/literal path text the directive lanes byte-match
/// (`java.util.List`, `System.Text`). PASS 142 (142A-F2, grids B1-B4/B14):
/// `::`-qualified C# namespace-alias paths (`global::System.Math`,
/// alias-qualified `System::Math`) ride the same ident-segment law — sg
/// binds the whole qualified text (B15 N='System::Math'); the `::` spelling
/// was never gridded before 142 and the refusal dropped the literal patterns
/// onto a dual-arm emit (directive walk + literal byte-match).
fn directive_path_literal(text: &str) -> bool {
    let normalized = text.replace("::", ".");
    let mut segments = normalized.split('.');
    let first_ok = segments.next().is_some_and(|s| {
        !s.is_empty()
            && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    first_ok && segments.all(|s| {
        !s.is_empty()
            && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Split at a top-level ` keyword ` occurrence (bracket-depth aware) —
/// the `as`-alias splits (`import $X as $Y`, `use $X as $Y;`).
fn split_top_level_keyword<'a>(text: &'a str, keyword: &str) -> Option<(&'a str, &'a str)> {
    let mut depth = 0usize;
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b' ' if depth == 0 && text[i..].starts_with(keyword) => {
                return Some((&text[..i], &text[i + keyword.len()..]));
            }
            _ => {}
        }
    }
    None
}

// --- E (140A-F5): the import/directive root family. ---

#[derive(Debug, Clone)]
enum DirectiveTemplate {
    /// py `import …` (NO `;`): plain names or the alias face.
    PyImport {
        names: Vec<LaneMeta>,
        alias: Option<(LaneMeta, LaneMeta)>,
    },
    /// py `from $M import …` — PASS 141 (grids F6_from_*/G_from*/R2_from_*):
    /// comma-separated name slots (meta or literal), the `*` star face, and
    /// the parenthesized-list face (which DEMARCATES: a paren face refuses
    /// a paren-free candidate and vice versa).
    PyFromImport {
        module: LaneMeta,
        names: Vec<LaneName>,
        star: bool,
        paren_wrapped: bool,
    },
    /// java `import [static] T[.*];` / rs|php `use …;`.
    Semi {
        head: &'static str,
        static_prefix: bool,
        star_suffix: bool,
        target: LaneName,
        alias: Option<(LaneMeta, LaneMeta)>,
    },
    /// PASS 141 (grids F1_*/F5_*/A_*/R2_cs_*): the cs using-directive
    /// family — `using [static|unsafe] T;`, `using A = T;`, and the
    /// `global `-prefixed faces. `global` is DEMAND-ONLY (a pattern
    /// without it binds global candidates too; a pattern WITH it refuses
    /// non-global candidates); the keyword run must EQUAL the candidate's.
    CsUsing {
        global_prefix: bool,
        keyword_run: Vec<CsUsingKeyword>,
        target: LaneName,
        alias_type: Option<AliasRhs>,
    },
    /// PASS 141 (grid F6_php_use_*): the php `use function|const $X;` kind
    /// faces — the literal kind keyword must agree with the candidate's
    /// kind keyword; X binds the name.
    PhpUseKind { kind: &'static str, target: LaneName },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CsUsingKeyword {
    Static,
    Unsafe,
}

impl CsUsingKeyword {
    fn spell(self) -> &'static str {
        match self {
            CsUsingKeyword::Static => "static",
            CsUsingKeyword::Unsafe => "unsafe",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "static" => Some(CsUsingKeyword::Static),
            "unsafe" => Some(CsUsingKeyword::Unsafe),
            _ => None,
        }
    }
}

fn directive_template(pattern: &str) -> Option<DirectiveTemplate> {
    let p = pattern.trim();
    if let Some(rest) = p.strip_prefix("from ") {
        // `from $M import …` — PASS 141 (grids F6_from_*/G_from*/R2_from_*):
        // the module is a canonical meta; the import list is comma
        // separated name slots (meta or literal), the `*` star face, or
        // the parenthesized spelling of either — the parens DEMARCATE
        // (a paren face refuses a paren-free candidate and vice versa:
        // F6_from_paren_ab_x_plain_src / R2_from_plain_x_paren_src rc1).
        let import_at = rest.find(" import ")?;
        let module = lane_meta(rest[..import_at].trim())?;
        let mut name_section = rest[import_at + " import ".len()..].trim();
        let paren_wrapped = name_section.starts_with('(') && name_section.ends_with(')');
        if paren_wrapped {
            name_section = name_section[1..name_section.len() - 1].trim();
        }
        if name_section == "*" {
            if paren_wrapped {
                return None;
            }
            return Some(DirectiveTemplate::PyFromImport {
                module,
                names: Vec::new(),
                star: true,
                paren_wrapped: false,
            });
        }
        if name_section.is_empty() {
            return None;
        }
        let slot = |element: &str| -> Option<LaneName> {
            let element = element.trim();
            if let Some(meta) = lane_meta(element) {
                return Some(LaneName::Meta(meta));
            }
            is_pattern_ident(element).then(|| LaneName::Literal(element.to_string()))
        };
        let names = split_top_level_commas(name_section)
            .into_iter()
            .map(slot)
            .collect::<Option<Vec<_>>>()?;
        if names.is_empty() {
            return None;
        }
        return Some(DirectiveTemplate::PyFromImport {
            module,
            names,
            star: false,
            paren_wrapped,
        });
    }
    if let Some(rest) = p.strip_prefix("import") {
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let rest = rest.trim_start();
        if rest.contains(';') {
            // java shape: `import [static] T[.*];` — the decorations are
            // part of the FACE (R2 demarcation cells).
            let mut rest = rest;
            let mut static_prefix = false;
            if let Some(after) = rest.strip_prefix("static") {
                if after.starts_with(char::is_whitespace) {
                    static_prefix = true;
                    rest = after.trim_start();
                }
            }
            let inner = rest.strip_suffix(';')?;
            let mut star_suffix = false;
            let mut target_text = inner.trim();
            if let Some(prefix) = target_text.strip_suffix(".*") {
                star_suffix = true;
                target_text = prefix.trim_end();
            }
            if target_text.is_empty() || target_text.contains(char::is_whitespace) {
                return None;
            }
            let target = if let Some(meta) = lane_meta(target_text) {
                LaneName::Meta(meta)
            } else if directive_path_literal(target_text) {
                LaneName::Literal(target_text.to_string())
            } else {
                return None;
            };
            return Some(DirectiveTemplate::Semi {
                head: "import",
                static_prefix,
                star_suffix,
                target,
                alias: None,
            });
        }
        // python faces: names / alias; `;`-ful spellings never reach here.
        if let Some((left, right)) = split_top_level_keyword(rest, " as ") {
            let x = lane_meta(left.trim())?;
            let y = lane_meta(right.trim())?;
            return Some(DirectiveTemplate::PyImport {
                names: Vec::new(),
                alias: Some((x, y)),
            });
        }
        let names = split_top_level_commas(rest)
            .into_iter()
            .map(|element| lane_meta(element.trim()))
            .collect::<Option<Vec<_>>>()?;
        // PASS 141 (grid G_abc_*): any slot count — the zip law absorbs
        // trailing candidate names (G_ab_x3name n1) and refuses a
        // short candidate (G_abc_x2name rc1 `[]`).
        if names.is_empty() {
            return None;
        }
        return Some(DirectiveTemplate::PyImport { names, alias: None });
    }
    if let Some(rest) = p.strip_prefix("use ") {
        let inner = rest.trim().strip_suffix(';')?;
        if inner.is_empty() || inner.contains(',') {
            return None;
        }
        // PASS 141 (grid F6_php_use_*): the php kind faces — a literal
        // `function`/`const` keyword + one canonical name slot. The kind
        // keyword must AGREE with the candidate's (x_plain rc1); the plain
        // `use $X;` face keeps its whole-clause law.
        for kind in ["function", "const"] {
            if let Some(name) = inner.strip_prefix(kind) {
                if name.starts_with(char::is_whitespace) {
                    let name = name.trim();
                    let target = if let Some(meta) = lane_meta(name) {
                        LaneName::Meta(meta)
                    } else if is_pattern_ident(name) {
                        LaneName::Literal(name.to_string())
                    } else {
                        return None;
                    };
                    return Some(DirectiveTemplate::PhpUseKind { kind, target });
                }
            }
        }
        if let Some((left, right)) = split_top_level_keyword(inner, " as ") {
            let x = lane_meta(left.trim())?;
            let y = lane_meta(right.trim())?;
            return Some(DirectiveTemplate::Semi {
                head: "use",
                static_prefix: false,
                star_suffix: false,
                target: LaneName::Meta(x.clone()),
                alias: Some((x, y)),
            });
        }
        // meta-only: the literal rs/php `use` spellings AGREE on their
        // current literal-lane route (R2_rs_use_lit / R2_php_use_lit).
        let target = lane_meta(inner.trim()).map(LaneName::Meta)?;
        return Some(DirectiveTemplate::Semi {
            head: "use",
            static_prefix: false,
            star_suffix: false,
            target,
            alias: None,
        });
    }
    // PASS 141 (grids F1_*/F5_*/A_*/R2_cs_*): the cs using-directive
    // family — `[global ]using [static|unsafe] (T | A = T);`. A `global`
    // prefix is a DEMAND-ONLY marker (a pattern without it binds global
    // candidates too — F1_a_global n1; a pattern with it refuses
    // non-global candidates); the `static`/`unsafe` keyword run is part of
    // the FACE (the run must equal the candidate's —
    // R2_cs_static_pat_x_unsafe_cand n0); the `= ` alias face splits
    // name/type (A_cs_using_alias_pat n1).
    // PASS 142 (142B-F1, grids C_*): every demarcation skip is CLASS-STRICT
    // (is_sg_cs_trivia) — sg-class trivia runs (U+FEFF/NBSP/VT) parse and
    // BIND (C9/C10/C11/C12) while Rust-whitespace outsiders (U+2028 et al)
    // refuse here and fall to the accepted-empty class (C8/C13).
    let mut global_prefix = false;
    let mut using_body = p;
    if let Some(after) = p.strip_prefix("global") {
        if after.starts_with(is_sg_cs_trivia) {
            global_prefix = true;
            using_body = after.trim_start_matches(is_sg_cs_trivia);
        }
    }
    if let Some(rest) = using_body.strip_prefix("using") {
        if !rest.starts_with(is_sg_cs_trivia) {
            return None;
        }
        let mut keyword_run = Vec::new();
        let mut rest = rest.trim_start_matches(is_sg_cs_trivia);
        loop {
            let mut consumed = false;
            for keyword in [CsUsingKeyword::Static, CsUsingKeyword::Unsafe] {
                if let Some(after) = rest.strip_prefix(keyword.spell()) {
                    if after.starts_with(is_sg_cs_trivia) {
                        keyword_run.push(keyword);
                        rest = after.trim_start_matches(is_sg_cs_trivia);
                        consumed = true;
                        break;
                    }
                }
            }
            if !consumed {
                break;
            }
        }
        let Some(inner) = rest.strip_suffix(';') else {
            return None;
        };
        // The alias face: split at the TOP-LEVEL `=` (depth-aware over
        // paren/bracket/brace groups).
        if let Some(eq_at) = top_level_equals(inner) {
            let (lhs, rhs) = (
                inner[..eq_at].trim_matches(is_sg_cs_trivia),
                inner[eq_at + 1..].trim_matches(is_sg_cs_trivia),
            );
            if lhs.is_empty() || rhs.is_empty() {
                return None;
            }
            let alias_type = alias_rhs_slot(rhs)?;
            return Some(DirectiveTemplate::CsUsing {
                global_prefix,
                alias_type: Some(alias_type),
                keyword_run,
                target: lane_slot(lhs)?,
            });
        }
        // The plain face: the name is ONE token (whitespace is never a
        // namespace name); the edges are trivia-trimmed. PASS 146 (z8, grid
        // m*): sg's matcher treats comments as trivia on the PATTERN face
        // too (`using static Sys/* c */.IO;` answers its own
        // comment-bearing candidate, m-grid n1), so comment spans are
        // stripped before the token checks — leftover comment-padding
        // trivia keeps refusing.
        let stripped = strip_comment_spans(inner);
        let had_comment = stripped.len() != inner.len();
        let trimmed = stripped.trim_matches(is_sg_cs_trivia);
        // PASS 142A: a comment in a META face keeps the accepted-empty
        // posture (A21/A22/I-cells) — z8 transparency is the LITERAL-face
        // law (`using static Sys/* c */.IO;` answers its comment-bearing
        // candidate, m-grid n1).
        if had_comment && trimmed.contains('$') {
            return None;
        }
        let inner = trimmed.to_string();
        if inner.is_empty() || inner.chars().any(is_sg_cs_trivia) {
            return None;
        }
        let Some(target) = lane_slot(&inner) else {
            return None;
        };
        return Some(DirectiveTemplate::CsUsing {
            global_prefix,
            keyword_run,
            target,
            alias_type: None,
        });
    }
    None
}

/// The top-level `=` scanner for the cs using-alias face — depth-aware
/// over `(`/`[`/`{` groups, ASCII positions only.
fn top_level_equals(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '=' if depth == 0 => {
                // `==`, `+=`, `=>` never occur in a using directive's name;
                // the first bare `=` is the alias separator.
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// The (template, language) pairing the walk dispatches — the census arm
/// and the dispatch share this exact admission.
fn directive_lane_serves(lang: Language, pattern: &str) -> bool {
    let Some(template) = directive_template(pattern) else {
        return false;
    };
    match (&template, lang) {
        (DirectiveTemplate::Semi { head: "import", .. }, Language::Java) => true,
        (DirectiveTemplate::Semi { head: "use", .. }, Language::Rust | Language::Php) => true,
        (DirectiveTemplate::CsUsing { .. }, Language::CSharp) => true,
        (DirectiveTemplate::PhpUseKind { .. }, Language::Php) => true,
        (
            DirectiveTemplate::PyImport { .. } | DirectiveTemplate::PyFromImport { .. },
            Language::Python,
        ) => true,
        _ => false,
    }
}

fn match_directive_root(lang: Language, source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    if !directive_lane_serves(lang, pattern) {
        return None;
    }
    let template = directive_template(pattern)?;
    let tree = parse_source(lang, source).ok()?;
    let mut out = Vec::new();
    match &template {
        DirectiveTemplate::PyImport { .. } => {
            walk_py_import(tree.root_node(), source, pattern, &template, &mut out);
        }
        DirectiveTemplate::PyFromImport { .. } => {
            walk_py_from_import(tree.root_node(), source, pattern, &template, &mut out);
        }
        DirectiveTemplate::Semi { head: "import", .. } => {
            walk_java_import(tree.root_node(), source, pattern, &template, &mut out);
        }
        DirectiveTemplate::Semi { head: "use", .. } => {
            walk_use_semi(tree.root_node(), source, pattern, &template, lang, &mut out);
        }
        DirectiveTemplate::CsUsing { .. } => {
            walk_cs_using(tree.root_node(), source, pattern, &template, &mut out);
        }
        DirectiveTemplate::PhpUseKind { .. } => {
            walk_use_semi(tree.root_node(), source, pattern, &template, lang, &mut out);
        }
        _ => {}
    }
    Some(out)
}

fn walk_py_import(
    node: Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "import_statement" && !is_in_comment_or_string(&node) {
        if let Some(hit) = py_import_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_py_import(child, source, pattern, template, out);
    }
}

fn py_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::PyImport { names, alias } = template else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    if let Some((x, y)) = alias {
        // `import $X as $Y` — PASS 141 (grid G_alias_*): the FIRST child
        // must be an aliased_import (a leading plain name refuses,
        // G_alias_x_firstplain rc1 `[]`); candidate children AFTER it are
        // sg-skipped (a trailing plain name keeps the bind,
        // G_alias_x_secondplain n1 with X=`o` Y=`a`).
        let Some(first) = children.first() else {
            return None;
        };
        if first.kind() != "aliased_import" {
            return None;
        }
        let name_node = first.child_by_field_name("name")?;
        let alias_node = first.child_by_field_name("alias")?;
        x.bind(&mut captures, node_text(&name_node, source)?.trim())?;
        y.bind(&mut captures, node_text(&alias_node, source)?.trim())?;
    } else {
        // The plain shape binds the FIRST child text — an aliased child
        // binds WHOLE (`os as o`, R2_py_importX_alias_cand); candidate
        // extras absorb as a prefix (E_py_import_two n1).
        if children.len() < names.len() {
            return None;
        }
        for (slot, child) in names.iter().zip(children.iter()) {
            slot.bind(&mut captures, node_text(child, source)?.trim())?;
        }
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

fn walk_py_from_import(
    node: Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "import_from_statement" && !is_in_comment_or_string(&node) {
        if let Some(hit) = py_from_import_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_py_from_import(child, source, pattern, template, out);
    }
}

fn py_from_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::PyFromImport {
        module,
        names,
        star,
        paren_wrapped,
    } = template
    else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    // PASS 141: the candidate's parentheses DEMARCATE both ways (the
    // anonymous `(`/`)` tokens are part of the tree; a paren face refuses
    // a paren-free candidate and vice versa — R2_from_plain_x_paren_src /
    // F6_from_paren_ab_x_plain_src rc1 `[]`).
    let has_paren = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && node_text(&c, source).is_some_and(|t| t == "("));
    if has_paren != *paren_wrapped {
        return None;
    }
    let [module_node, imported @ ..] = children.as_slice() else {
        return None;
    };
    module.bind(&mut captures, node_text(module_node, source)?.trim())?;
    if *star {
        // The star face binds the module only and refuses a named list
        // (F6_from_star_x_names rc1 `[]`); a name-SLOT pattern binds the
        // wildcard text under its slot (R2_from_nameY_x_star_src: Y=`*`).
        if imported.first().is_some_and(|c| c.kind() != "wildcard_import") {
            return None;
        }
    } else if imported.len() < names.len() {
        return None;
    } else {
        for (slot, child) in names.iter().zip(imported.iter()) {
            slot.bind_or_match(&mut captures, node_text(child, source)?.trim())?;
        }
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

fn walk_java_import(
    node: Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "import_declaration" && !is_in_comment_or_string(&node) {
        if let Some(hit) = java_import_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java_import(child, source, pattern, template, out);
    }
}

fn java_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::Semi {
        head: "import",
        static_prefix,
        star_suffix,
        target,
        alias: None,
    } = template
    else {
        return None;
    };
    let text = node_text(node, source)?;
    let rest = text.trim().strip_prefix("import")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut rest = rest.trim_start();
    let mut had_static = false;
    if let Some(after) = rest.strip_prefix("static") {
        if after.starts_with(char::is_whitespace) {
            had_static = true;
            rest = after.trim_start();
        }
    }
    if had_static != *static_prefix {
        return None;
    }
    let inner = rest.strip_suffix(';')?;
    let mut had_star = false;
    let mut target_text = inner.trim();
    if let Some(prefix) = target_text.strip_suffix(".*") {
        had_star = true;
        target_text = prefix.trim_end();
    }
    if had_star != *star_suffix {
        return None;
    }
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), text.to_string());
    target.bind_or_match(&mut captures, target_text)?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

fn walk_use_semi(
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
    if node.kind() == want_kind && !is_in_comment_or_string(&node) {
        if let Some(hit) = use_semi_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_use_semi(child, source, pattern, template, lang, out);
    }
}

fn use_semi_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    // PASS 141 (grid F6_php_use_*): the php kind faces — the clause's head
    // token must be the pattern's literal kind keyword; X binds the name
    // (one token: the aliased kind spelling keeps its ungridded route).
    if let DirectiveTemplate::PhpUseKind { kind, target } = template {
        // PASS 142 (142A-F1, grids A7/A13/A16): the kind HEAD matches on the
        // RAW clause text (a comment BEFORE the kind keyword breaks the
        // sg alignment — A13 refuses); comments AFTER it are trivia — the
        // NAME section scans comment-stripped.
        // PASS 143 (142E-F1, grids php_use_*): the sg php gap law — after
        // the literal `use` an sg-php-gap-trivia run is REQUIRED (ASCII ws,
        // NBSP, U+FEFF, U+001A: sg php_head_feff binds, php_nogap refuses);
        // the kind token ends at the next trivia char, so a Rust-ws
        // outsider (U+2028 et al) glues into the head/name token and
        // refuses exactly where sg refuses (zero-gap glue cells n0). The
        // name section binds the FIRST token: trivia splits it, glue chars
        // are token chars (sg space_glue X='\u{2028}strlen',
        // internal_feff/two_names X='str', tail glue X='strlen\u{2028}').
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
        // PASS 146 (145B-F3, grid l*): NUL ends the head-token scan for
        // Meta targets (l3 `use function<NUL>Foo;` — the kind clause still
        // aligns; literal targets keep the strict scan).
        let head_end = after_gap
            .find(|c| is_sg_php_gap_trivia(c) || (matches!(target, LaneName::Meta(_)) && c == '\0'))
            .unwrap_or(after_gap.len());
        if &after_gap[..head_end] != &kind[..] {
            return None;
        }
        // PASS 146 (145B-F3, grid l*): the kind-clause NAME gap admits
        // NUL for Meta targets (l3 `use function<NUL>Foo;` binds X=Foo,
        // capture stripped); literal targets keep the strict class.
        let name_gap = |c: char| {
            is_sg_php_gap_trivia(c) || (matches!(target, LaneName::Meta(_)) && c == '\0')
        };
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
        let (line_start, line_end) = node_lines(node, source);
        let excerpt = excerpt_for_node(node, source, pattern);
        return Some(PatternMatch {
            line_start,
            line_end,
            byte_start: node.start_byte(),
            byte_end: node.end_byte(),
            excerpt,
            captures,
        });
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
    // PASS 144 (143A-F5, grid P*): the php KIND-LESS `use $X;` clause head
    // obeys the same sg php gap law as the 143-fixed kind lanes — after the
    // literal `use` an is_sg_php_gap_trivia run is REQUIRED (FEFF/A0/001A
    // bind, P1-P3); a glue char (U+2028) means zero gap → the junk glued
    // into the name token and sg refuses (P5). Rust `use` keeps the
    // literal-space spelling.
    // PASS 146 (145B-F3, grid l*): sg's use-head gap law admits U+0000 as
    // a gap unit at the META face (l1 `use<NUL>Foo;` binds X=Foo, capture
    // stripped) while the LITERAL pattern keeps refusing the NUL candidate
    // (l4 rc1 `[]`) — the admission is target-scoped.
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
        // PASS 146 (145A-F11, grid j*): sg refuses a kind-less META `use
        // $X;` against a php GROUP-use candidate (j1 rc1 `[]`) — the meta
        // capture never spans a php brace group — while a literal
        // group-use PATTERN keeps binding its byte-equal candidate (j2,
        // `use My\\{A, B};` n1) and plain candidates keep binding (j4).
        // The RUST brace-group cell (E_rs_use_braces) is untouched: the
        // guard is php-clause-scoped.
        if php_clause && inner.contains('{') && matches!(target, LaneName::Meta(_)) {
            return None;
        }
        // The plain shape binds the WHOLE clause text (brace groups,
        // `function foo` kind keywords included: E_rs_use_braces,
        // R3_php_use_function).
        target.bind_or_match(&mut captures, inner.trim())?;
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

fn walk_cs_using(
    node: Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "using_directive" && !is_in_comment_or_string(&node) {
        if let Some(hit) = cs_using_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_cs_using(child, source, pattern, template, out);
    }
}

fn cs_using_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::CsUsing {
        global_prefix,
        keyword_run,
        target,
        alias_type,
    } = template
    else {
        return None;
    };
    // PASS 141 candidate-side demarcation (grid F1_*): parse the
    // directive's own shape — optional `global`, the `using` keyword, the
    // `static`/`unsafe` keyword run, then a name or `name = type` alias.
    // PASS 142 (142A-F1, grids A_*): sg walks the AST where comments are
    // trivia, so the SCANS see the comment-stripped clause text (and the
    // captures bind trivia-free slot texts, A10 A='M') while the emitted
    // span/MATCH keep the original bytes. The demarcation skips are
    // CLASS-STRICT — since PASS 143 (142E-F1) the CANDIDATE-side class is
    // is_sg_cs_gap_junk (sg's parse skips full Unicode White_Space + FEFF
    // + control junk here; grids cs_using_*), while the PATTERN-side class
    // of record (directive_template) stays is_sg_cs_trivia.
    let raw = node_text(node, source)?;
    let text = strip_comment_spans(raw);
    let mut rest = text.trim().trim_start_matches(is_sg_cs_gap_junk);
    let mut had_global = false;
    if let Some(after) = rest.strip_prefix("global") {
        if after.starts_with(is_sg_cs_gap_junk) {
            had_global = true;
            rest = after.trim_start_matches(is_sg_cs_gap_junk);
        }
    }
    let Some(after_using) = rest.strip_prefix("using") else {
        return None;
    };
    if !after_using.starts_with(is_sg_cs_gap_junk) {
        return None;
    }
    let mut rest = after_using.trim_start_matches(is_sg_cs_gap_junk);
    let mut had_keywords: Vec<CsUsingKeyword> = Vec::new();
    loop {
        let mut consumed = false;
        for keyword in [CsUsingKeyword::Static, CsUsingKeyword::Unsafe] {
            if let Some(after) = rest.strip_prefix(keyword.spell()) {
                if after.starts_with(is_sg_cs_gap_junk) {
                    had_keywords.push(keyword);
                    rest = after.trim_start_matches(is_sg_cs_gap_junk);
                    consumed = true;
                    break;
                }
            }
        }
        if !consumed {
            break;
        }
    }
    // `global` is demand-only; the keyword run must EQUAL the pattern's.
    if *global_prefix && !had_global {
        return None;
    }
    if *keyword_run != had_keywords {
        return None;
    }
    let inner = rest
        .trim()
        .trim_start_matches(is_sg_cs_gap_junk)
        .strip_suffix(';')?
        .trim_end_matches(is_sg_cs_gap_junk);
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), raw.to_string());
    match alias_type {
        Some(alias) => {
            let eq_at = top_level_equals(inner)?;
            let (lhs, rhs) = (
                inner[..eq_at].trim_matches(is_sg_cs_gap_junk),
                inner[eq_at + 1..].trim_matches(is_sg_cs_gap_junk),
            );
            if lhs.is_empty() || rhs.is_empty() {
                return None;
            }
            target.bind_or_match(&mut captures, lhs)?;
            alias.bind_or_match(&mut captures, rhs)?;
        }
        None => {
            // The plain face: one name token. The `=` alias spelling and
            // any decorated run never reach here (the run equality and
            // the trivia guard refuse them: E_cs_using_alias,
            // F1_a_static, F1_a_unsafe). PASS 143: junk glued inside ONE
            // name token refuses (sg cs_name_glue2028/cs_name_001a n0).
            // PASS 144 (143B-F1, grid Q*): sg's law is PER-TOKEN-GAP — junk
            // at an identifier↔`.` boundary is skipped by the parse and the
            // qualified name BINDS with the junk RETAINED in the capture
            // (N=`Sys<U+2028>.IO`, oracle metaVariables) on the plain,
            // static, and global lanes (Q1-Q6/Q10/Q11). The pre-fix
            // wholesale interior-junk refusal under-served all of them
            // (silent ok:true []). Segment law: split the top-level `.`,
            // trim each segment's EDGE junk runs, refuse a segment whose
            // INTERIOR carries junk or that empties out; the capture binds
            // the raw (junk-retained) section text.
            if inner.is_empty() || top_level_equals(inner).is_some() {
                return None;
            }
            let cand_segs: Vec<&str> = inner.split('.').collect();
            if cand_segs.iter().any(|seg| {
                let core = seg.trim_matches(is_sg_cs_gap_junk);
                core.is_empty() || core.chars().any(is_sg_cs_gap_junk)
            }) {
                return None;
            }
            // PASS 146 (145B-F4, grid m*): a LITERAL qualified-name pattern
            // obeys the same per-segment law — sg skips boundary junk and
            // comment gaps on the literal face too (m1 `Sys<U+2028>.IO` /
            // m3 `Sys /* c */ .IO` bind n1) while glue INSIDE one segment
            // refuses (m2 n0). Meta targets keep the junk-retained capture
            // (Q-grid law of record).
            match target {
                LaneName::Meta(_) => {
                    target.bind_or_match(&mut captures, inner)?;
                }
                LaneName::Literal(literal) => {
                    let lit_segs: Vec<&str> = literal.split('.').collect();
                    if lit_segs.len() != cand_segs.len() {
                        return None;
                    }
                    for (seg, lit) in cand_segs.iter().zip(lit_segs.iter()) {
                        if seg.trim_matches(is_sg_cs_gap_junk) != *lit {
                            return None;
                        }
                    }
                }
            }
        }
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

// --- C (140A-F3): the csharp checked/unchecked EXPRESSION root lane. ---

struct CsExpressionTemplate {
    keyword: &'static str,
    operand: String,
}

fn cs_expression_template(pattern: &str) -> Option<CsExpressionTemplate> {
    let p = pattern.trim();
    let (keyword, rest) = if let Some(rest) = p.strip_prefix("checked") {
        ("checked", rest)
    } else if let Some(rest) = p.strip_prefix("unchecked") {
        ("unchecked", rest)
    } else {
        return None;
    };
    if !rest.starts_with('(') {
        return None;
    }
    let inner = &rest[1..];
    let close = balanced_paren_close(inner)?;
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    let operand = capture_name(inner[..close].trim())?.to_string();
    Some(CsExpressionTemplate { keyword, operand })
}

fn match_csharp_expression_root(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = cs_expression_template(pattern)?;
    let tree = parse_source(Language::CSharp, source).ok()?;
    let mut out = Vec::new();
    walk_csharp_expression_root(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

fn walk_csharp_expression_root(
    node: Node,
    source: &str,
    pattern: &str,
    template: &CsExpressionTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "checked_expression"
        && node.is_named()
        && !is_in_comment_or_string(&node)
    {
        if let Some(hit) = csharp_expression_root_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_csharp_expression_root(child, source, pattern, template, out);
    }
}

fn csharp_expression_root_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &CsExpressionTemplate,
) -> Option<PatternMatch> {
    // checked/unchecked SHARE the checked_expression kind — the keyword
    // token must agree token-exactly (the 137 statement-lane discipline).
    let mut cursor = node.walk();
    let keyword = node
        .children(&mut cursor)
        .find(|c| !c.is_named() && !is_trivia_kind(c.kind()))
        .and_then(|c| node_text(&c, source))?;
    if keyword != template.keyword {
        return None;
    }
    let mut cursor = node.walk();
    let expr = node
        .children(&mut cursor)
        .find(|c| c.is_named() && !is_trivia_kind(c.kind()))?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(&expr, source)?;
    bind_capture(&mut captures, &template.operand, text.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

// --- D (140A-F4): the java synchronized nested-block + METHOD faces. The
// 139 bare-meta lane keeps the flat faces (D_sync_plain_ctl AGREE n2); the
// lanes here take what it refuses. ---

/// The BLOCK slots of a synchronized face. Resource: `Ok(meta)` binds the
/// resource's inner text in the meta's namespace; `Err(literal)` byte-
/// matches. Body: a bare meta binds the ONE statement's text; a nested
/// `synchronized (…) { … }` recurses.
#[derive(Debug, Clone)]
struct JaSyncMetaBlock {
    resource: Result<LaneMeta, String>,
    body: JaSyncBlockBody,
}

#[derive(Debug, Clone)]
enum JaSyncBlockBody {
    Meta(LaneMeta),
    Nested(Box<JaSyncMetaBlock>),
}

#[derive(Debug, Clone)]
struct JaSyncMethodTemplate {
    /// Pattern modifiers IN ORDER (the head `synchronized` included). The
    /// walk hops ordinary keyword modifiers positionally; an annotation
    /// beyond the DEMANDED prefix blocks passage (the 137 java modifier
    /// discipline, D_sync_static / D_sync_static_pat).
    modifiers: Vec<String>,
    /// PASS 141 (grid F6_ja_annot_pat): the PATTERN-side leading
    /// annotation run — each demanded annotation must match a candidate
    /// annotation in order (an absent one refuses, x_noannot rc1 `[]`).
    annotations: Vec<String>,
    name: LaneMeta,
    body: JaSyncMethodBody,
}

#[derive(Debug, Clone)]
enum JaSyncMethodBody {
    /// `{ $B }` — exactly one statement, B = its text.
    Meta(LaneMeta),
    /// `{ synchronized (…) { … } }` — the body IS a synchronized block.
    SyncBlock(JaSyncMetaBlock),
}

const JA_SYNC_METHOD_MODIFIERS: &[&str] = &[
    "public",
    "protected",
    "private",
    "static",
    "final",
    "abstract",
    "strictfp",
    "default",
    "native",
    "transient",
    "volatile",
    "synchronized",
];

/// Strip the leading modifier-keyword run; returns the modifiers in order
/// and whether the run CARRIES `synchronized` anywhere — PASS 141 (grid
/// R2_ja_syncstatic_patorder_cand): `synchronized static …` is the method
/// face too (sg n1 on the pattern-order candidate; the in-order demand
/// refuses the static-first candidate, F7_ja_syncstatic_pat rc1 `[]`).
fn ja_strip_modifier_run(pattern: &str) -> (Vec<String>, bool, &str) {
    let mut modifiers = Vec::new();
    let mut rest = pattern;
    loop {
        let mut consumed = false;
        for keyword in JA_SYNC_METHOD_MODIFIERS {
            if let Some(after) = rest.strip_prefix(keyword) {
                if after.starts_with(char::is_whitespace) {
                    modifiers.push((*keyword).to_string());
                    rest = after.trim_start();
                    consumed = true;
                    break;
                }
            }
        }
        if !consumed {
            break;
        }
    }
    let head_sync = modifiers.iter().any(|m| m == "synchronized");
    (modifiers, head_sync, rest)
}

/// PASS 141 (grid F6_ja_annot_pat): strip the PATTERN's leading annotation
/// run — bare `@Name` tokens (identifier, no argument list: the gridded
/// face) separated by whitespace/newlines. Each demanded annotation must
/// match a candidate annotation IN ORDER (f141g); a pattern annotation
/// with no candidate annotation refuses (x_noannot rc1 `[]`).
fn ja_strip_annotation_run(pattern: &str) -> (Vec<String>, &str) {
    let mut out = Vec::new();
    let mut rest = pattern;
    loop {
        let Some(after) = rest.strip_prefix('@') else {
            break;
        };
        let name_end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let name = &after[..name_end];
        if name.is_empty() || !is_pattern_ident(name) {
            break;
        }
        out.push(format!("@{name}"));
        rest = after[name_end..].trim_start();
    }
    (out, rest)
}

fn ja_strip_ret(rest: &str) -> Option<(&str, &str)> {
    let rest = rest.trim_start();
    let ret_end = rest.find(char::is_whitespace)?;
    let ret = &rest[..ret_end];
    if ret.is_empty() || ret.starts_with('$') || !is_pattern_ident(ret) {
        return None;
    }
    Some((ret, &rest[ret_end..]))
}

fn ja_name_then_parens(rest: &str) -> Option<(LaneMeta, &str)> {
    let rest = rest.trim_start();
    let name_end = rest
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(rest.len());
    let name = lane_meta(&rest[..name_end])?;
    let rest = rest[name_end..].trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    if !inner[..close].trim().is_empty() {
        return None;
    }
    Some((name, &inner[close + 1..]))
}

fn ja_method_body(rest: &str) -> Option<JaSyncMethodBody> {
    let rest = rest.trim_start();
    let brace_inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(brace_inner)?;
    if !brace_inner[close + 1..].trim().is_empty() {
        return None;
    }
    let section = brace_inner[..close].trim();
    if let Some(meta) = lane_meta(section) {
        return Some(JaSyncMethodBody::Meta(meta));
    }
    let after = section.strip_prefix("synchronized")?;
    if !after.starts_with(char::is_whitespace) {
        return None;
    }
    let block = ja_sync_block_slots(after.trim_start())?;
    Some(JaSyncMethodBody::SyncBlock(block))
}

fn ja_sync_block_nested_template(pattern: &str) -> Option<JaSyncMetaBlock> {
    let p = pattern.trim();
    let rest = p.strip_prefix("synchronized")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    ja_sync_block_slots(rest.trim_start())
}

fn ja_sync_block_slots(rest: &str) -> Option<JaSyncMetaBlock> {
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    let resource_section = inner[..close].trim();
    let resource = if resource_section.is_empty() {
        return None;
    } else if let Some(meta) = lane_meta(resource_section) {
        Ok(meta)
    } else if capture_name(resource_section).is_none()
        && !resource_section.contains('$')
        && !resource_section.contains('(')
        && !resource_section.contains('{')
    {
        Err(resource_section.to_string())
    } else {
        return None;
    };
    let rest = inner[close + 1..].trim_start();
    let brace_inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(brace_inner)?;
    if !brace_inner[close + 1..].trim().is_empty() {
        return None;
    }
    let body_section = brace_inner[..close].trim();
    let body = if let Some(meta) = lane_meta(body_section) {
        JaSyncBlockBody::Meta(meta)
    } else {
        let after = body_section.strip_prefix("synchronized")?;
        if !after.starts_with(char::is_whitespace) {
            return None;
        }
        // BOUNDEDNESS-OF-RECORD (141B-F3): PATTERN-controlled recursion —
        // one frame per nested `synchronized` brace group in the QUERY
        // string, unbounded without a grid-verified bound (same posture as
        // the csharp_statement_template Nested recursion of record).
        JaSyncBlockBody::Nested(Box::new(ja_sync_block_slots(after.trim_start())?))
    };
    Some(JaSyncMetaBlock { resource, body })
}

fn ja_sync_method_template(pattern: &str) -> Option<JaSyncMethodTemplate> {
    let p = pattern.trim();
    let (annotations, p) = ja_strip_annotation_run(p);
    let (modifiers, head_sync, rest) = ja_strip_modifier_run(p);
    let (ret, rest) = ja_strip_ret(rest)?;
    let _ = ret;
    let (name, rest) = ja_name_then_parens(rest)?;
    let body = ja_method_body(rest)?;
    if head_sync {
        // `synchronized`-carrying METHOD face:
        // `[@Anno…] [modifiers] ret $M() { body }`.
        return Some(JaSyncMethodTemplate {
            modifiers,
            annotations,
            name,
            body,
        });
    }
    // Method-BODY face: `ret $M() { synchronized (…) { … } }` — the body
    // MUST parse as a synchronized block; `ret $M() { $B }` without the
    // synchronized head is a different (ungridded) face.
    match body {
        JaSyncMethodBody::SyncBlock(_) => Some(JaSyncMethodTemplate {
            modifiers,
            annotations,
            name,
            body,
        }),
        JaSyncMethodBody::Meta(_) => None,
    }
}

fn match_java_sync_block_nested(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let block = ja_sync_block_nested_template(pattern)?;
    let tree = parse_source(Language::Java, source).ok()?;
    let mut out = Vec::new();
    walk_java_sync_block_nested(tree.root_node(), source, pattern, &block, &mut out);
    Some(out)
}

fn walk_java_sync_block_nested(
    node: Node,
    source: &str,
    pattern: &str,
    block: &JaSyncMetaBlock,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "synchronized_statement" && node.is_named() && !is_in_comment_or_string(&node)
    {
        let mut captures = BTreeMap::new();
        if let Some(text) = node_text(&node, source) {
            captures.insert("MATCH".to_string(), text.to_string());
        }
        if java_sync_block_bind(&node, source, block, &mut captures).is_some() {
            push_match_with_captures(&node, source, pattern, captures, out);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java_sync_block_nested(child, source, pattern, block, out);
    }
}

fn match_java_sync_method(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = ja_sync_method_template(pattern)?;
    let tree = parse_source(Language::Java, source).ok()?;
    let mut out = Vec::new();
    walk_java_sync_method(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

fn walk_java_sync_method(
    node: Node,
    source: &str,
    pattern: &str,
    template: &JaSyncMethodTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "method_declaration" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = java_sync_method_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java_sync_method(child, source, pattern, template, out);
    }
}

fn java_sync_block_bind(
    node: &Node,
    source: &str,
    block: &JaSyncMetaBlock,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let resource = children
        .iter()
        .copied()
        .find(|c| c.kind() == "parenthesized_expression")?;
    let resource_inner = resource.named_child(0)?;
    let resource_text = node_text(&resource_inner, source)?;
    match &block.resource {
        Ok(meta) => meta.bind(captures, resource_text.trim())?,
        Err(literal) => {
            if resource_text.trim() != literal {
                return None;
            }
        }
    }
    let body = children.iter().copied().find(|c| c.kind() == "block")?;
    let mut body_cursor = body.walk();
    let stmts: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    match &block.body {
        JaSyncBlockBody::Meta(meta) => {
            let text = node_text(only, source)?;
            meta.bind(captures, text)?;
        }
        JaSyncBlockBody::Nested(nested) => {
            if only.kind() != "synchronized_statement" {
                return None;
            }
            java_sync_block_bind(only, source, nested, captures)?;
        }
    }
    Some(())
}

fn java_sync_method_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JaSyncMethodTemplate,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    // The candidate's modifier tokens: `Some(text)` for keyword modifiers,
    // `None` for annotations (which BLOCK the hop-scan's passage). A
    // modifier-less candidate may carry NO modifiers node at all — the
    // scan then refuses any pattern modifier (D_sync_static_pat rc1) and
    // passes a modifier-less pattern (D_sync_method_body).
    let modifiers_node = children.iter().copied().find(|c| c.kind() == "modifiers");
    let mut tokens: Vec<Option<String>> = Vec::new();
    let mut candidate_annotations: Vec<String> = Vec::new();
    if let Some(modifiers_node) = modifiers_node {
        let mut modifier_cursor = modifiers_node.walk();
        for child in modifiers_node.children(&mut modifier_cursor) {
            if child.is_named() {
                if child.kind().contains("annotation") {
                    tokens.push(None);
                    if let Some(text) = node_text(&child, source) {
                        candidate_annotations.push(text.to_string());
                    }
                }
            } else {
                tokens.push(node_text(&child, source).map(str::to_string));
            }
        }
    }
    // PASS 141 (grid F6_ja_annot_pat): the pattern's leading annotation
    // run must match a PREFIX of the candidate's annotation run, in order
    // (an absent candidate annotation refuses — x_noannot rc1 `[]`). The
    // consumed prefix is then skipped; any annotation BEYOND the demanded
    // prefix still blocks the modifier hop (the standing law).
    if !template.annotations.is_empty() {
        if candidate_annotations.len() < template.annotations.len() {
            return None;
        }
        for (want, got) in template.annotations.iter().zip(candidate_annotations.iter()) {
            if want.trim() != got.trim() {
                return None;
            }
        }
        tokens = tokens
            .into_iter()
            .skip(template.annotations.len())
            .collect();
    }
    let mut cursor_index = 0usize;
    for want in &template.modifiers {
        loop {
            let Some(token) = tokens.get(cursor_index) else {
                return None;
            };
            match token {
                // An annotation must not sit between pattern modifiers.
                None => return None,
                Some(text) if text == want => {
                    cursor_index += 1;
                    break;
                }
                Some(_) => cursor_index += 1,
            }
        }
    }
    let name_node = node.child_by_field_name("name")?;
    let name_text = node_text(&name_node, source)?;
    template.name.bind(&mut captures, name_text.trim())?;
    // The gridded method shape carries an EMPTY parameter list.
    let parameters = node.child_by_field_name("parameters")?;
    if parameters.named_child_count() != 0 {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let mut body_cursor = body.walk();
    let stmts: Vec<Node> = body
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    match &template.body {
        JaSyncMethodBody::Meta(meta) => {
            let text = node_text(only, source)?;
            meta.bind(&mut captures, text)?;
        }
        JaSyncMethodBody::SyncBlock(block) => {
            if only.kind() != "synchronized_statement" {
                return None;
            }
            java_sync_block_bind(only, source, block, &mut captures)?;
        }
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

// --- F (140A-F6): the remaining statement roots. ---

struct KtTypealiasTemplate {
    name: LaneMeta,
    target: LaneMeta,
}

fn kt_typealias_template(pattern: &str) -> Option<KtTypealiasTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("typealias")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let eq_at = rest.find('=')?;
    let name = lane_meta(rest[..eq_at].trim())?;
    let target = lane_meta(rest[eq_at + 1..].trim())?;
    Some(KtTypealiasTemplate { name, target })
}

fn kt_typealias_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &KtTypealiasTemplate,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    let [name_node, target_node] = children.as_slice() else {
        return None;
    };
    template
        .name
        .bind(&mut captures, node_text(name_node, source)?.trim())?;
    template
        .target
        .bind(&mut captures, node_text(target_node, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

struct KtForTemplate {
    item: LaneMeta,
    collection: LaneMeta,
    body: LaneMeta,
}

fn kt_for_template(pattern: &str) -> Option<KtForTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("for")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?;
    let close = balanced_paren_close(inner)?;
    let header = &inner[..close];
    let (left, right) = split_top_level_keyword(header, " in ")?;
    let item = lane_meta(left.trim())?;
    let collection = lane_meta(right.trim())?;
    let rest = inner[close + 1..].trim_start();
    let brace_inner = rest.strip_prefix('{')?;
    let close = balanced_brace_close(brace_inner)?;
    if !brace_inner[close + 1..].trim().is_empty() {
        return None;
    }
    let body = lane_meta(brace_inner[..close].trim())?;
    Some(KtForTemplate {
        item,
        collection,
        body,
    })
}

fn walk_kt_for(
    node: Node,
    source: &str,
    pattern: &str,
    template: &KtForTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "for_statement" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = kt_for_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kt_for(child, source, pattern, template, out);
    }
}

fn kt_for_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &KtForTemplate,
) -> Option<PatternMatch> {
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    // kotlin for_statement carries NO field list — the positions are the
    // law: item, collection, then the control-structure body LAST.
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    if children.len() < 3 {
        return None;
    }
    let item = &children[0];
    let collection = &children[1];
    let body = children.last()?;
    let body_text = node_text(body, source)?;
    // The brace-less body refuses (R2_kt_for_nobrace rc1 `[]`).
    let brace_inner = body_text.trim_start().strip_prefix('{')?;
    if !brace_inner.trim_end().ends_with('}') {
        return None;
    }
    let inner = brace_inner.trim_end().strip_suffix('}')?;
    template
        .item
        .bind(&mut captures, node_text(item, source)?.trim())?;
    template
        .collection
        .bind(&mut captures, node_text(collection, source)?.trim())?;
    // B = the block inner trimmed BOTH ends (F_kt_for / R2_kt_for_2stmt).
    template.body.bind(&mut captures, inner.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

struct SwiftForTemplate {
    item: LaneMeta,
    collection: LaneMeta,
    where_clause: Option<LaneMeta>,
    body: LaneMeta,
}

fn swift_for_template(pattern: &str) -> Option<SwiftForTemplate> {
    let mut tokens = pattern.trim().split_whitespace();
    if tokens.next()? != "for" {
        return None;
    }
    let item = lane_meta(tokens.next()?)?;
    if tokens.next()? != "in" {
        return None;
    }
    let collection = lane_meta(tokens.next()?)?;
    let mut where_clause = None;
    match tokens.next()? {
        "where" => {
            where_clause = Some(lane_meta(tokens.next()?)?);
            if tokens.next()? != "{" {
                return None;
            }
        }
        "{" => {}
        _ => return None,
    }
    let body = lane_meta(tokens.next()?)?;
    // The spelling's closing `}` is the last token.
    if tokens.next() != Some("}") || tokens.next().is_some() {
        return None;
    }
    Some(SwiftForTemplate {
        item,
        collection,
        where_clause,
        body,
    })
}

fn walk_swift_for(
    node: Node,
    source: &str,
    pattern: &str,
    template: &SwiftForTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "for_statement" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = swift_for_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_swift_for(child, source, pattern, template, out);
    }
}

fn swift_for_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &SwiftForTemplate,
) -> Option<PatternMatch> {
    let item = node.child_by_field_name("item")?;
    let collection = node.child_by_field_name("collection")?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    template
        .item
        .bind(&mut captures, node_text(&item, source)?.trim())?;
    template
        .collection
        .bind(&mut captures, node_text(&collection, source)?.trim())?;
    // The `where` clause presence must agree on BOTH sides (R2 cells).
    let mut cursor = node.walk();
    let where_node = node
        .children(&mut cursor)
        .find(|c| c.kind() == "where_clause");
    match (&template.where_clause, &where_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    if let (Some(want), Some(where_node)) = (&template.where_clause, where_node) {
        // The condition is the where_clause's named child EXCLUDING the
        // `where` keyword token the grammar surfaces as a named child
        // (F_sw_for_where W=`x > 1`).
        let mut wcursor = where_node.walk();
        let condition = where_node
            .children(&mut wcursor)
            .filter(|c| c.is_named())
            .find(|c| node_text(c, source) != Some("where"))?;
        want.bind(&mut captures, node_text(&condition, source)?.trim())?;
    }
    // Body: the LAST named child; B = the inner text trim_start ONLY — sg
    // KEEPS the trailing whitespace (B=`g(x)\n    `, F_sw_for_where).
    // tree-sitter-swift exposes the body as the brace-less statements
    // group whose span IS sg's B text; a braced child (defensive spelling)
    // unwraps first.
    let mut cursor = node.walk();
    let body = node
        .children(&mut cursor)
        .filter(|c| c.is_named())
        .last()?;
    let body_text = node_text(&body, source)?;
    let inner = if let Some(brace_inner) = body_text.trim_start().strip_prefix('{') {
        brace_inner.strip_suffix('}')?.trim_start()
    } else {
        body_text
    };
    template.body.bind(&mut captures, inner)?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

struct RsLetElseTemplate {
    pattern_slot: LaneMeta,
    value: LaneMeta,
    body: LaneMeta,
}

fn rs_let_else_template(pattern: &str) -> Option<RsLetElseTemplate> {
    let mut tokens = pattern.trim().split_whitespace();
    if tokens.next()? != "let" {
        return None;
    }
    let pattern_slot = tokens.next()?;
    if tokens.next()? != "=" {
        return None;
    }
    let value = tokens.next()?;
    if tokens.next()? != "else" {
        return None;
    }
    if tokens.next()? != "{" {
        return None;
    }
    let body = tokens.next()?;
    // The spelling ends `};`.
    if tokens.next()? != "};" || tokens.next().is_some() {
        return None;
    }
    Some(RsLetElseTemplate {
        pattern_slot: lane_meta(pattern_slot)?,
        value: lane_meta(value)?,
        body: lane_meta(body)?,
    })
}

fn walk_rs_let_else(
    node: Node,
    source: &str,
    pattern: &str,
    template: &RsLetElseTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "let_declaration" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = rs_let_else_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rs_let_else(child, source, pattern, template, out);
    }
}

fn rs_let_else_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &RsLetElseTemplate,
) -> Option<PatternMatch> {
    let pattern_node = node.child_by_field_name("pattern")?;
    let value = node.child_by_field_name("value")?;
    // A no-else candidate refuses (R2_rs_letelse_noelse rc1 `[]`).
    let alternative = node.child_by_field_name("alternative")?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    template
        .pattern_slot
        .bind(&mut captures, node_text(&pattern_node, source)?.trim())?;
    template
        .value
        .bind(&mut captures, node_text(&value, source)?.trim())?;
    // B = the ONE-statement else body's text (R2_rs_letelse_2stmt rc1).
    let mut body_cursor = alternative.walk();
    let stmts: Vec<Node> = alternative
        .children(&mut body_cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()))
        .collect();
    let [only] = stmts.as_slice() else {
        return None;
    };
    template
        .body
        .bind(&mut captures, node_text(only, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

struct CGotoTemplate {
    label: LaneMeta,
}

fn c_goto_template(pattern: &str) -> Option<CGotoTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("goto")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let label = lane_meta(rest.trim_start().strip_suffix(';')?.trim())?;
    Some(CGotoTemplate { label })
}

fn walk_c_goto(
    node: Node,
    source: &str,
    pattern: &str,
    template: &CGotoTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "goto_statement" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = c_goto_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_c_goto(child, source, pattern, template, out);
    }
}

fn c_goto_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &CGotoTemplate,
) -> Option<PatternMatch> {
    let mut cursor = node.walk();
    let label = node
        .children(&mut cursor)
        .find(|c| c.kind() == "statement_identifier")?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    template
        .label
        .bind(&mut captures, node_text(&label, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// PASS 140: go's `$`-carrying `;`-ful import spelling is sg
/// ACCEPTED-EMPTY (E_go_importblock rc1 `[]`) — census-answerable, the
/// walk's empty IS the sg agreement (the `goto $L;` twin rides
/// [`sg_goto_semi_pattern`]).
fn sg_go_import_semi_pattern(pattern: &str) -> bool {
    let p = pattern.trim();
    let Some(rest) = p.strip_prefix("import") else {
        return false;
    };
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let Some(target) = rest.trim_start().strip_suffix(';') else {
        return false;
    };
    let target = target.trim();
    !target.is_empty()
        && !target.contains(char::is_whitespace)
        && capture_name(target).is_some()
}

/// PASS 144 (143A-F8, grid G*): the go SEMI-LESS `goto <label>` spelling —
/// sg binds every go goto_statement's label (G1 n1 L=end, G5 n2, G2 tail
/// comment irrelevant) where the `;`-ful twin is the registered
/// accepted-empty envelope (§49/140A-F6 sg rc8/su-empty) and the C grammar
/// is the opposite (its semi-ful form binds, the f140f lane). `$`-carrying
/// labels only: the concrete `goto end` spelling keeps the literal lane's
/// faces.
struct GoGotoBareTemplate {
    label: LaneMeta,
}

fn go_goto_bare_template(pattern: &str) -> Option<GoGotoBareTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("goto")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let label = rest.trim_start();
    if label.is_empty() || label.contains(char::is_whitespace) || label.ends_with(';') {
        return None;
    }
    Some(GoGotoBareTemplate { label: lane_meta(label)? })
}

fn walk_go_goto_bare(
    node: Node,
    source: &str,
    pattern: &str,
    template: &GoGotoBareTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "goto_statement" && node.is_named() && !is_in_comment_or_string(&node) {
        let mut cursor = node.walk();
        let label = node.children(&mut cursor).find(|c| c.is_named());
        if let Some(hit) = label.and_then(|label| {
            let mut captures = BTreeMap::new();
            if let Some(text) = node_text(&node, source) {
                captures.insert("MATCH".to_string(), text.to_string());
            }
            template
                .label
                .bind(&mut captures, node_text(&label, source)?.trim())?;
            let (line_start, line_end) = node_lines(&node, source);
            let excerpt = excerpt_for_node(&node, source, pattern);
            Some(PatternMatch {
                line_start,
                line_end,
                byte_start: node.start_byte(),
                byte_end: node.end_byte(),
                excerpt,
                captures,
            })
        }) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_go_goto_bare(child, source, pattern, template, out);
    }
}

/// PASS 140: the remaining statement-root lanes, language-scoped exactly as
/// gridded (c `goto $L;` BINDS while go answers the same spelling [] — the
/// H_go/F_c cell pair).
fn match_statement_root_140(lang: Language, source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    match lang {
        Language::Kotlin => {
            if let Some(template) = kt_typealias_template(pattern) {
                let tree = parse_source(lang, source).ok()?;
                let mut out = Vec::new();
                walk_kt_typealias(tree.root_node(), source, pattern, &template, &mut out);
                return Some(out);
            }
            let template = kt_for_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_kt_for(tree.root_node(), source, pattern, &template, &mut out);
            Some(out)
        }
        Language::Swift => {
            let template = swift_for_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_swift_for(tree.root_node(), source, pattern, &template, &mut out);
            Some(out)
        }
        Language::Rust => {
            let template = rs_let_else_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_rs_let_else(tree.root_node(), source, pattern, &template, &mut out);
            Some(out)
        }
        Language::C => {
            let template = c_goto_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_c_goto(tree.root_node(), source, pattern, &template, &mut out);
            Some(out)
        }
        Language::Go => {
            // PASS 144 (143A-F8): the semi-less `goto $L` family binds per
            // site (grid G1/G5); the `;`-ful twin keeps its accepted-empty
            // envelope via sg_goto_semi_pattern and never reaches here.
            let template = go_goto_bare_template(pattern)?;
            let tree = parse_source(lang, source).ok()?;
            let mut out = Vec::new();
            walk_go_goto_bare(tree.root_node(), source, pattern, &template, &mut out);
            Some(out)
        }
        _ => None,
    }
}

// ===========================================================================
// PASS 142 (142A-F3 + 142A-F5, grids E*/H*/J*/K*): the statement/decl-root
// lanes the r74 red-team found census-loud where sg answers —
//   rs `mod $N { $B }` (single-statement-exact body) / `extern crate $N;`
//   go `type $N $T` (T = the whole type tail)
//   rb `module $N\n  $B\nend` (B = the trimmed body)
//   ts `declare module $N { $B }` (single-statement-exact) / `declare const $X: $T;`
//   py `async def $N($$P):\n    $$B` (P param-exact, B = the whole body block)
//   java `enum <name> { <body> }` (single-member-exact; empty-body pattern
//        binds empty candidates)
//   cs `record <name>(<P>);[ { $B }]` / `struct <name> { $B }`
// Each lane's own parse is the admission (the 137A-F2 discipline); every
// law cell is oracle-gridded (grid1-grid6 receipts). Modifier doctrine
// (gridded at G4/G6 for cs, G3 for java, extended to the rs visibility
// child by the same sg child-alignment mechanism): a candidate carrying a
// modifier/visibility child the pattern lacks REFUSES.
// ===========================================================================

#[derive(Debug, Clone)]
enum Root142Template {
    RsMod { name: LaneName, body: LaneMeta },
    RsExternCrate { name: LaneName },
    GoType { name: LaneName, ty: LaneName },
    RbModule { name: LaneName, body: LaneMeta },
    TsDeclareModule { name: LaneName, body: LaneMeta },
    TsDeclareConst { name: LaneName, ty: LaneName },
    PyAsyncDef { name: LaneName, param: LaneMeta, body: LaneMeta },
    JaEnum { name: LaneName, body: JaEnumBody },
    CsRecord { name: LaneName, param: LaneMeta, body: Option<LaneMeta> },
    CsStruct { name: LaneName, body: LaneMeta },
    /// PASS 146 (145A-F1, grid a*): cs `goto $L;` / semi-less `goto $L` —
    /// sg binds PER SITE across the gap-junk class and comments (a1-a3),
    /// two sites n2 (a6). The registered c goto lane is c-scoped; no cs
    /// goto row existed. Label is META-only: the concrete `goto end;`
    /// spelling keeps its literal-lane route (a5 n1/n1 pre-existing).
    CsGoto { label: LaneMeta },
    /// PASS 146 (145A-F2, grid b*): cs `new $T($A)[;]` — sg binds the ONE-
    /// argument object-creation STATEMENT (b1/b2/b3/b5) and refuses nested
    /// (b4), zero-arg (b6), and two-arg (b7) faces.
    CsNew { ty: LaneName, arg: LaneMeta },
    /// PASS 146 (145A-F4, grid d*): cs switch-STATEMENT meta body — both
    /// spellings bind ONE section (d1/d2/d6); empty (d4) and multi-section
    /// (d5) refuse; the switch-EXPRESSION row (d7) never crosses (kind-exact
    /// walk on `switch_statement`).
    CsSwitch { subject: Option<LaneName>, body: LaneMeta },
    /// PASS 146 (145A-F3, grid c*): php declaration-kind meta bodies —
    /// function/class/trait/interface with a `PhpNamespaceBody` slot (Meta
    /// binds ONE member c1/c4/c7/c8/c9; Mixed exact-order binds c10);
    /// multi-member/empty refuse (c2/c5/c6/c13). The served function-`$B`
    /// face keeps its decl-lane route, so the function body DEMANDS `$$`
    /// (the E9 discipline); `$$$B` stays ungridded-out (template refuses).
    PhpDecl { kind: PhpDeclKind, name: LaneName, body: PhpNamespaceBody },
    /// PASS 146 (145A-F5, grid e*): go plain `switch [<X>] { <B> }` — ONE
    /// clause binds (e1/e2), 2-clause/empty refuse (e3/e4); the
    /// type-switch spelling is a distinct kind (e14 n0 both engines).
    GoSwitch { subject: Option<LaneName>, body: LaneMeta },
    /// PASS 146 (145A-F6, grid e10-e13): go `if <C> { <B> }` — B binds the
    /// WHOLE body text (e10 single, e11 multi); the empty body refuses
    /// (e12). The single-`$` body keeps its served general-lane route, so
    /// the body DEMANDS `$$` (the E9 discipline).
    GoIf { cond: LaneName, body: LaneMeta },
    /// PASS 146 (145A-F5, grid e5/e6): go `package <N>` — binds through a
    /// comment gap (sg n1; the package root was never admitted).
    GoPackage { name: LaneMeta },
    /// PASS 146 (145A-F5, grid e8/e9): go SEMI-LESS `import <X>` — X binds
    /// the single path (`"fmt"`) or the whole group text; the `;`-ful
    /// spelling keeps its registered accepted-empty envelope.
    GoImport { target: LaneMeta },
    /// PASS 146 (145A-F7, grid f*): js `$$` arm bodies — see
    /// [`JsIfTemplate`].
    JsElseIf(JsIfTemplate),
}

/// PASS 146: the php declaration kind of a [`Root142Template::PhpDecl`] —
/// the walk keys the candidate node kind and the Mixed-doc wrapper on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhpDeclKind {
    Function,
    Class,
    Trait,
    Interface,
}

impl PhpDeclKind {
    fn keyword(self) -> &'static str {
        match self {
            PhpDeclKind::Function => "function",
            PhpDeclKind::Class => "class",
            PhpDeclKind::Trait => "trait",
            PhpDeclKind::Interface => "interface",
        }
    }

    fn root_kind(self) -> &'static str {
        match self {
            PhpDeclKind::Function => "function_definition",
            PhpDeclKind::Class => "class_declaration",
            PhpDeclKind::Trait => "trait_declaration",
            PhpDeclKind::Interface => "interface_declaration",
        }
    }
}

#[derive(Debug, Clone)]
enum JaEnumBody {
    Meta(LaneMeta),
    Literal(String),
    Empty,
}

/// One identifier token (or canonical meta) as a lane name slot.
fn root142_name_slot(token: &str) -> Option<LaneName> {
    if let Some(meta) = lane_meta(token) {
        return Some(LaneName::Meta(meta));
    }
    is_pattern_ident(token).then(|| LaneName::Literal(token.to_string()))
}

fn root142_meta_slot(token: &str) -> Option<LaneMeta> {
    lane_meta(token)
}

fn root142_trivia(text: &str) -> &str {
    text.trim()
}

fn statement_root_142_template(lang: Language, pattern: &str) -> Option<Root142Template> {
    let p = root142_trivia(pattern);
    match lang {
        Language::Rust => {
            // `mod <name> { $B }` — the single-statement body law lives in
            // the walk (E2: 2-statement bodies refuse).
            if let Some(rest) = p.strip_prefix("mod ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let inner = rest.trim_start().strip_prefix('{')?;
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                let body = root142_meta_slot(inner[..close].trim())?;
                return Some(Root142Template::RsMod {
                    name: root142_name_slot(name_tok)?,
                    body,
                });
            }
            // `extern crate <name>;`
            let rest = p.strip_prefix("extern crate ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            if !root142_trivia(rest).strip_suffix(';')?.is_empty() {
                return None;
            }
            Some(Root142Template::RsExternCrate {
                name: root142_name_slot(name_tok)?,
            })
        }
        Language::Go => {
            // `type <name> <T>` — T is the WHOLE type tail (E6 struct blocks);
            // the `=`-alias spelling keeps the kt/general routes (ungridded
            // here on purpose), and the `;`-ful spelling is sg RC8 (E13).
            if let Some(rest) = p.strip_prefix("type ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let ty = rest.trim();
                if ty.is_empty() || ty.starts_with('=') || ty.contains(';') {
                    return None;
                }
                if ty.contains(char::is_whitespace) {
                    // A multi-token tail never parses in go without a brace
                    // group; the brace-group tail is ungridded and keeps its
                    // loud route.
                    return None;
                }
                return Some(Root142Template::GoType {
                    name: root142_name_slot(name_tok)?,
                    ty: root142_name_slot(ty)?,
                });
            }
            // PASS 146 (145A-F5/F6, grids e*): the go root siblings of the
            // 144-fixed goto-bare face. Each was census-loud pre-146.
            if let Some(rest) = p.strip_prefix("switch ") {
                // `switch [<X>] { <B> }` — the global `{` form has no subject.
                let (subject, brace_section) = if let Some(inner) = rest.strip_prefix('{') {
                    (None, inner)
                } else {
                    let (subj_tok, after) = root142_split_token(rest)?;
                    let brace_at = after.find('{')?;
                    if !after[..brace_at].trim().is_empty() {
                        return None;
                    }
                    (Some(root142_name_slot(subj_tok)?), &after[brace_at + 1..])
                };
                let close = balanced_brace_close(brace_section)?;
                if !brace_section[close + 1..].trim().is_empty() {
                    return None;
                }
                // Body spelling: `$B` (e2) and `$$B` (e1) both bind the ONE
                // clause's raw text; `$$$B` is ungridded and keeps its prior
                // class.
                let body_tok = brace_section[..close].trim();
                if body_tok.starts_with("$$$") {
                    return None;
                }
                let body = root142_meta_slot(body_tok)?;
                return Some(Root142Template::GoSwitch { subject, body });
            }
            if let Some(rest) = p.strip_prefix("if ") {
                // `if <C> { <B> }` — C is one token slot (e13 `x` literal,
                // e10 `$C` meta); B binds the WHOLE body and demands `$$`.
                let brace_at = rest.find('{')?;
                let cond_tok = rest[..brace_at].trim();
                if cond_tok.contains(char::is_whitespace) {
                    return None;
                }
                let cond = root142_name_slot(cond_tok)?;
                let inner = &rest[brace_at + 1..];
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                let body_tok = inner[..close].trim();
                if !body_tok.starts_with("$$") || body_tok.starts_with("$$$") {
                    return None;
                }
                let body = root142_meta_slot(body_tok)?;
                return Some(Root142Template::GoIf { cond, body });
            }
            if let Some(rest) = p.strip_prefix("package ") {
                // `package <N>` — meta name, no semi (e5/e6).
                let tok = rest.trim();
                if tok.contains(char::is_whitespace) || tok.ends_with(';') {
                    return None;
                }
                let name = lane_meta(tok)?;
                if name.multi {
                    return None;
                }
                return Some(Root142Template::GoPackage { name });
            }
            if let Some(rest) = p.strip_prefix("import ") {
                // SEMI-LESS `import <X>` (e8/e9); the `;`-ful spelling keeps
                // its registered accepted-empty envelope upstream.
                let tok = rest.trim();
                if tok.contains(char::is_whitespace) || tok.ends_with(';') {
                    return None;
                }
                let target = lane_meta(tok)?;
                if target.multi {
                    return None;
                }
                return Some(Root142Template::GoImport { target });
            }
            None
        }
        Language::Ruby => {
            // `module <Name>\n  $B\nend` — the body slot is the whole
            // trimmed middle; only the canonical-meta spelling is gridded.
            let rest = p.strip_prefix("module ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            let mid = rest.trim_end().strip_suffix("end")?;
            if !mid.trim_end().is_empty() && !mid.ends_with(|c: char| c.is_whitespace()) {
                // `end` must be its own token (an `endX` tail is a name).
                return None;
            }
            Some(Root142Template::RbModule {
                name: root142_name_slot(name_tok)?,
                body: root142_meta_slot(mid.trim())?,
            })
        }
        Language::TypeScript => {
            if let Some(rest) = p.strip_prefix("declare module ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let inner = rest.trim_start().strip_prefix('{')?;
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                let body = root142_meta_slot(inner[..close].trim())?;
                return Some(Root142Template::TsDeclareModule {
                    name: root142_name_slot(name_tok).or_else(|| {
                        // The quoted ambient name (`"m"`) is a literal slot.
                        let tok = root142_trivia(name_tok);
                        (!tok.is_empty() && !tok.contains('$') && !tok.contains(char::is_whitespace))
                            .then(|| LaneName::Literal(tok.to_string()))
                    })?,
                    body,
                });
            }
            // `declare const <X>: <T>;` — the kind token is part of the FACE
            // (H23: `declare let` refuses).
            let rest = p.strip_prefix("declare const ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            let ty = root142_trivia(rest).strip_suffix(';')?;
            let ty = ty.strip_prefix(':').unwrap_or(ty).trim();
            if ty.is_empty() || ty.contains(char::is_whitespace) {
                return None;
            }
            Some(Root142Template::TsDeclareConst {
                name: root142_name_slot(name_tok)?,
                ty: root142_name_slot(ty)?,
            })
        }
        Language::Python => py_async_def_template(pattern).map(|t| match t {
            PyAsyncDefTemplate { name, param, body } => Root142Template::PyAsyncDef { name, param, body },
        }),
        Language::Java => {
            // `enum <name> { <body> }` — the body is one canonical meta, one
            // literal token, or EMPTY (K4: the empty pattern binds empty
            // candidates). Name meta-or-literal.
            let rest = p.strip_prefix("enum ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            let inner = rest.trim_start().strip_prefix('{')?;
            let close = balanced_brace_close(inner)?;
            if !inner[close + 1..].trim().is_empty() {
                return None;
            }
            let body_text = inner[..close].trim();
            let body = if body_text.is_empty() {
                JaEnumBody::Empty
            } else if let Some(meta) = root142_meta_slot(body_text) {
                JaEnumBody::Meta(meta)
            } else if is_pattern_ident(body_text) {
                JaEnumBody::Literal(body_text.to_string())
            } else {
                return None;
            };
            Some(Root142Template::JaEnum {
                name: root142_name_slot(name_tok)?,
                body,
            })
        }
        Language::CSharp => {
            if let Some(rest) = p.strip_prefix("record ") {
                // `record <name>(<P>);` optionally ` { $B }`.
                let (name_tok, rest) = root142_split_token(rest)?;
                let params_inner = rest.trim_start().strip_prefix('(')?;
                let close = balanced_paren_close(params_inner)?;
                let after = &params_inner[close + 1..];
                let param = root142_meta_slot(params_inner[..close].trim())?;
                let after_trim = after.trim();
                if after_trim.is_empty() || after_trim == ";" {
                    return Some(Root142Template::CsRecord {
                        name: root142_name_slot(name_tok)?,
                        param,
                        body: None,
                    });
                }
                let brace_at = after.find('{')?;
                let pre_brace = after[..brace_at].trim();
                if !pre_brace.is_empty() && pre_brace != ";" {
                    return None;
                }
                let inner = &after[brace_at + 1..];
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty()
                    && inner[close + 1..].trim() != ";"
                {
                    return None;
                }
                return Some(Root142Template::CsRecord {
                    name: root142_name_slot(name_tok)?,
                    param,
                    body: root142_meta_slot(inner[..close].trim()).map(Some)?,
                });
            }
            // `struct <name> { $B }`.
            if let Some(rest) = p.strip_prefix("struct ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let inner = rest.trim_start().strip_prefix('{')?;
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                return Some(Root142Template::CsStruct {
                    name: root142_name_slot(name_tok)?,
                    body: root142_meta_slot(inner[..close].trim())?,
                });
            }
            if p.split_whitespace().next() == Some("goto") {
                // PASS 146 (145A-F1, grid a*): `goto <L>[;]` — meta label
                // only; the concrete-label spelling keeps its literal route
                // (a5).
                let rest = p[4..].trim();
                let rest = rest.strip_suffix(';').unwrap_or(rest).trim();
                if rest.is_empty() || rest.contains(char::is_whitespace) {
                    return None;
                }
                let label = lane_meta(rest)?;
                if label.multi {
                    return None;
                }
                return Some(Root142Template::CsGoto { label });
            }
            if p.split_whitespace().next() == Some("new") {
                // PASS 146 (145A-F2, grid b*): `new <T>(<A>)[;]` — T meta or
                // literal ident; A one single-meta argument (b6/b7 refuse).
                let rest = p[3..].trim_start();
                let (ty_tok, rest) = root142_split_token(rest)?;
                let params = rest.trim_start().strip_prefix('(')?;
                let close = balanced_paren_close(params)?;
                let arg = params[..close].trim();
                let mut after = params[close + 1..].trim();
                if let Some(stripped) = after.strip_suffix(';') {
                    after = stripped.trim_end();
                }
                if !after.is_empty() {
                    return None;
                }
                let arg = lane_meta(arg)?;
                if arg.multi {
                    return None;
                }
                return Some(Root142Template::CsNew {
                    ty: root142_name_slot(ty_tok)?,
                    arg,
                });
            }
            if p.split_whitespace().next() == Some("switch") {
                // PASS 146 (145A-F4, grid d*): `switch (<X>) { <B> }` — the
                // subject is a single meta-or-literal token slot (d6 `x`);
                // B binds ONE section (both spellings, d1/d2).
                let rest = p[6..].trim_start();
                let paren_inner = rest.strip_prefix('(')?;
                let close = balanced_paren_close(paren_inner)?;
                let subject = root142_name_slot(paren_inner[..close].trim())?;
                let inner = paren_inner[close + 1..].trim_start().strip_prefix('{')?;
                let close_b = balanced_brace_close(inner)?;
                if !inner[close_b + 1..].trim().is_empty() {
                    return None;
                }
                let body = root142_meta_slot(inner[..close_b].trim())?;
                if body.multi {
                    return None;
                }
                return Some(Root142Template::CsSwitch {
                    subject: Some(subject),
                    body,
                });
            }
            None
        }
        Language::JavaScript => {
            // PASS 146 (145A-F7, grid f*): the js `$$`-arm faces — see
            // [`js_if_meta_template`]. Every admitted shape was census-loud
            // pre-146 (f1/f2/f3/f4/f5/f6 rc2); the served single-`$` faces
            // keep their routes through the `$$` demand.
            js_if_meta_template(p).map(Root142Template::JsElseIf)
        }
        Language::Php => {
            // PASS 146 (145A-F3, grid c*): the declaration-kind meta-body
            // faces. The function body demands `$$` (the served `$B` face
            // keeps its decl-lane route); class/trait/interface admit both
            // spellings (c7/c4) — all were census-loud pre-146.
            php_decl_block_template(p)
        }
        _ => None,
    }
}

/// PASS 146 (145A-F3, grid c*): the php declaration-kind template —
/// `function <N>() { <B> }` / `<class|trait|interface> <N> { <B> }`. B is a
/// [`PhpNamespaceBody`] (Meta ONE-member or Mixed exact-order — the
/// 143A-F13 law at the declaration kinds; c10). `$$$B` stays refused
/// (ungridded); the function body demands `$$` (c3's served `$B` face keeps
/// its decl-lane route).
fn php_decl_block_template(pattern: &str) -> Option<Root142Template> {
    let p = pattern.trim();
    let (kind, rest) = if let Some(rest) = p.strip_prefix("function ") {
        (PhpDeclKind::Function, rest)
    } else if let Some(rest) = p.strip_prefix("class ") {
        (PhpDeclKind::Class, rest)
    } else if let Some(rest) = p.strip_prefix("trait ") {
        (PhpDeclKind::Trait, rest)
    } else if let Some(rest) = p.strip_prefix("interface ") {
        (PhpDeclKind::Interface, rest)
    } else {
        return None;
    };
    let (name_tok, rest) = root142_split_token(rest)?;
    if kind == PhpDeclKind::Function {
        // The parameter list is part of the probed face: empty `()` (c1).
        let params = rest.trim_start();
        let inner = params.strip_prefix('(')?;
        let close = balanced_paren_close(inner)?;
        if !inner[..close].trim().is_empty() {
            return None;
        }
        let _ = params;
        let rest = inner[close + 1..].trim_start();
        let inner = rest.strip_prefix('{')?;
        let close_b = balanced_brace_close(inner)?;
        if !inner[close_b + 1..].trim().is_empty() {
            return None;
        }
        let body = php_decl_body_slot(inner[..close_b].trim(), true)?;
        return Some(Root142Template::PhpDecl {
            kind,
            name: root142_name_slot(name_tok)?,
            body,
        });
    }
    let inner = rest.trim_start().strip_prefix('{')?;
    let close = balanced_brace_close(inner)?;
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    let body = php_decl_body_slot(inner[..close].trim(), false)?;
    Some(Root142Template::PhpDecl {
        kind,
        name: root142_name_slot(name_tok)?,
        body,
    })
}

/// The php declaration body slot: canonical meta (`$B`/`$$B`) or the Mixed
/// exact-order face; `$$$B` refuses (ungridded). `demand_dollar_dollar`
/// scopes the served `$B` face out (function bodies).
fn php_decl_body_slot(section: &str, demand_dollar_dollar: bool) -> Option<PhpNamespaceBody> {
    let body = if let Some(meta) = lane_meta(section) {
        if meta.multi {
            return None;
        }
        if demand_dollar_dollar && !section.trim().starts_with("$$") {
            return None;
        }
        PhpNamespaceBody::Meta(meta)
    } else if let Some((prefix, tail)) = php_mixed_namespace_body(section) {
        PhpNamespaceBody::Mixed { prefix, tail }
    } else {
        return None;
    };
    Some(body)
}

/// PASS 146 (145A-F7, grid f*): the js `$$`-arm template shapes —
/// `if (<A>) { <$$B> } [else { <$$D> } | else if (<C>) { <$$D> }]`. All
/// admitted spellings were census-loud pre-146 (f1/f3/f5/f6 rc2); the
/// served single-`$` faces keep their routes through the `$$` demand. The
/// walk emits PER LEVEL on else-if chains (f4 n2), so the else-if tail
/// beyond D is deliberately not consumed (a trailing chain spelling
/// refuses here).
#[derive(Debug, Clone)]
struct JsIfTemplate {
    cond: LaneMeta,
    body: LaneMeta,
    tail: JsTail,
}

#[derive(Debug, Clone)]
enum JsTail {
    None,
    Block(LaneMeta),
    ElseIf { cond: LaneMeta, body: LaneMeta },
}

fn js_if_meta_template(pattern: &str) -> Option<JsIfTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("if ")?;
    let paren_inner = rest.trim_start().strip_prefix('(')?;
    let close = balanced_paren_close(paren_inner)?;
    let cond = js_cond_slot(paren_inner[..close].trim())?;
    let rest = paren_inner[close + 1..].trim_start();
    let inner = rest.strip_prefix('{')?;
    let close_b = balanced_brace_close(inner)?;
    let body = js_arm_slot(inner[..close_b].trim())?;
    let rest = inner[close_b + 1..].trim();
    let tail = if rest.is_empty() {
        JsTail::None
    } else if let Some(rest) = rest.strip_prefix("else if ") {
        let paren_inner = rest.trim_start().strip_prefix('(')?;
        let close = balanced_paren_close(paren_inner)?;
        let cond = js_cond_slot(paren_inner[..close].trim())?;
        let rest = paren_inner[close + 1..].trim_start();
        let inner = rest.strip_prefix('{')?;
        let close_b = balanced_brace_close(inner)?;
        let body = js_arm_slot(inner[..close_b].trim())?;
        if !inner[close_b + 1..].trim().is_empty() {
            return None;
        }
        JsTail::ElseIf { cond, body }
    } else if let Some(rest) = rest.strip_prefix("else ") {
        let inner = rest.trim_start().strip_prefix('{')?;
        let close_b = balanced_brace_close(inner)?;
        let body = js_arm_slot(inner[..close_b].trim())?;
        if !inner[close_b + 1..].trim().is_empty() {
            return None;
        }
        JsTail::Block(body)
    } else {
        return None;
    };
    Some(JsIfTemplate { cond, body, tail })
}

/// A js arm slot: one canonical meta, `$$`-demanded (the E9 discipline —
/// the served single-`$` faces keep their general-lane route). Condition
/// slots use [`js_cond_slot`] instead: `$A` is the receipted spelling (f1).
fn js_arm_slot(token: &str) -> Option<LaneMeta> {
    if !token.starts_with("$$") || token.starts_with("$$$") {
        return None;
    }
    let meta = lane_meta(token)?;
    if meta.multi {
        return None;
    }
    Some(meta)
}

/// A js condition slot: one non-multi canonical meta.
fn js_cond_slot(token: &str) -> Option<LaneMeta> {
    let meta = lane_meta(token)?;
    if meta.multi {
        return None;
    }
    Some(meta)
}

/// The `async def $N($$P):\n    $$B` slot parse — the `$$` UNIVERSAL
/// spellings are demanded (the single-`$` face keeps its existing answering
/// route, E9). P is param-exact (gridded H16/E8c), B the whole body block.
struct PyAsyncDefTemplate {
    name: LaneName,
    param: LaneMeta,
    body: LaneMeta,
}

fn py_async_def_template(pattern: &str) -> Option<PyAsyncDefTemplate> {
    let rest = root142_trivia(pattern).strip_prefix("async def ")?;
    let (name_tok, rest) = root142_split_token(rest)?;
    let params_inner = rest.trim_start().strip_prefix('(')?;
    let close = balanced_paren_close(params_inner)?;
    let param_tok = params_inner[..close].trim();
    // Demand the `$$` universal spelling.
    if !param_tok.starts_with("$$") || param_tok.starts_with("$$$") {
        return None;
    }
    let param = root142_meta_slot(param_tok)?;
    let after = params_inner[close + 1..].trim();
    let body_tok = after.strip_prefix(':')?.trim();
    if !body_tok.starts_with("$$") || body_tok.starts_with("$$$") {
        return None;
    }
    Some(PyAsyncDefTemplate {
        name: root142_name_slot(name_tok)?,
        param,
        body: root142_meta_slot(body_tok)?,
    })
}

/// Split the leading identifier/meta token off `text`; the cut is the
/// FIRST structural delimiter (`ws { ( ; :`) — `async def $N(...)` cuts at
/// the `(`, `declare const y: number;` at the `:`, `mod $N {` at the ws.
fn root142_split_token(text: &str) -> Option<(&str, &str)> {
    let text = root142_trivia(text);
    let end = text
        .find([' ', '\t', '\n', '\r', '{', '(', ';', ':'])
        .unwrap_or(text.len());
    let token = &text[..end];
    if token.is_empty() {
        return None;
    }
    Some((token, &text[end..]))
}

fn named_non_trivia_children<'tree>(node: &Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect()
}

fn has_child_kind<'tree>(node: &Node<'tree>, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == kind);
    found
}

fn match_statement_root_142(lang: Language, source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = statement_root_142_template(lang, pattern)?;
    let tree = parse_source(lang, source).ok()?;
    let mut out = Vec::new();
    walk_root_142(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

fn walk_root_142(node: Node, source: &str, pattern: &str, template: &Root142Template, out: &mut Vec<PatternMatch>) {
    if node.is_named() && !is_in_comment_or_string(&node) {
        let hit = match template {
            Root142Template::RsMod { .. } if node.kind() == "mod_item" => {
                root142_block_member_match(&node, source, pattern, "name", "body", template)
            }
            Root142Template::RsExternCrate { .. } if node.kind() == "extern_crate_declaration" => {
                root142_field_match(&node, source, pattern, "name", None, template)
            }
            Root142Template::GoType { .. } if node.kind() == "type_spec" => {
                // PASS 144 (143A-F6, grid T*): sg refuses a comment or
                // gap-junk run in the `type`-keyword→name position (T1
                // comment / T6 FEFF / T7 U+2028 / T8 A0, all rc1) while the
                // name-internal (T2), pre-tail (T3), and lead/trail (T5)
                // comment positions bind. Gate: when the parent declaration
                // leads with the anonymous `type` token, the gap bytes
                // between that token and the spec must be ASCII whitespace
                // only (comment bytes and junk are outside that class; a
                // grouped `type ( … )` declaration does not lead with the
                // token and keeps its prior route).
                let gap_ascii_clean = node.parent().map_or(true, |decl| {
                    let mut kw = decl.walk();
                    decl.children(&mut kw)
                        .next()
                        .is_some_and(|first| !first.is_named() && first.kind() == "type")
                        && source[decl.start_byte() + "type".len()..node.start_byte()]
                            .bytes()
                            .all(|b| b.is_ascii_whitespace())
                });
                if gap_ascii_clean {
                    root142_field_match(&node, source, pattern, "name", Some("type"), template)
                } else {
                    None
                }
            }
            Root142Template::RbModule { .. } if node.kind() == "module" => {
                root142_rb_module_match(&node, source, pattern, template)
            }
            Root142Template::TsDeclareModule { .. }
                if node.kind() == "ambient_declaration" =>
            {
                root142_ts_module_match(&node, source, pattern, template)
            }
            Root142Template::TsDeclareConst { .. }
                if node.kind() == "ambient_declaration" =>
            {
                root142_ts_const_match(&node, source, pattern, template)
            }
            Root142Template::PyAsyncDef { .. }
                if node.kind() == "function_definition" =>
            {
                root142_py_async_match(&node, source, pattern, template)
            }
            Root142Template::JaEnum { .. } if node.kind() == "enum_declaration" => {
                root142_ja_enum_match(&node, source, pattern, template)
            }
            Root142Template::CsRecord { .. } if node.kind() == "record_declaration" => {
                root142_cs_record_match(&node, source, pattern, template)
            }
            Root142Template::CsStruct { .. } if node.kind() == "struct_declaration" => {
                root142_cs_struct_match(&node, source, pattern, template)
            }
            // PASS 146 arms (grids a*/b*/c*/d*/e*/f*): the statement-root
            // siblings — each walk is kind-exact and binds its slots
            // sg-exactly per the grid receipts.
            Root142Template::CsGoto { label } if node.kind() == "goto_statement" => {
                root146_goto_match(&node, source, pattern, label)
            }
            Root142Template::CsNew { ty, arg }
                if node.kind() == "expression_statement" =>
            {
                root146_cs_new_match(&node, source, pattern, ty, arg)
            }
            Root142Template::CsSwitch { subject, body }
                if node.kind() == "switch_statement" =>
            {
                root146_cs_switch_match(&node, source, pattern, subject, body)
            }
            Root142Template::PhpDecl { kind, name, body }
                if node.kind() == kind.root_kind() =>
            {
                root146_php_decl_match(&node, source, pattern, *kind, name, body)
            }
            Root142Template::GoSwitch { subject, body }
                if node.kind() == "expression_switch_statement" =>
            {
                root146_go_switch_match(&node, source, pattern, subject, body)
            }
            Root142Template::GoIf { cond, body }
                if node.kind() == "if_statement" =>
            {
                root146_go_if_match(&node, source, pattern, cond, body)
            }
            Root142Template::GoPackage { name }
                if node.kind() == "package_clause" =>
            {
                root146_go_package_match(&node, source, pattern, name)
            }
            Root142Template::GoImport { target }
                if node.kind() == "import_declaration" =>
            {
                root146_go_import_match(&node, source, pattern, target)
            }
            Root142Template::JsElseIf(tpl) if node.kind() == "if_statement" => {
                root146_js_if_match(&node, source, pattern, tpl)
            }
            _ => None,
        };
        if let Some(hit) = hit {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_root_142(child, source, pattern, template, out);
    }
}

/// Shared hit constructor for the PASS 146 root-142 arms.
fn root146_hit(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: BTreeMap<String, String>,
) -> Option<PatternMatch> {
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

fn root146_named_children<'tree>(node: &Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()) && !child.is_extra())
        .collect()
}

/// PASS 146 (145A-F1, grid a*): cs `goto_statement` — the label is the first
/// named child; the candidate gap (comment/U+2028 junk) is sg-transparent
/// (a2/a3 bind; the subject parse recovers the junk outside the label).
fn root146_goto_match(
    node: &Node,
    source: &str,
    pattern: &str,
    label: &LaneMeta,
) -> Option<PatternMatch> {
    let label_node = root146_named_children(node).first().copied()?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(&label_node, source)?;
    label.bind(&mut captures, text.trim())?;
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F2, grid b*): cs `new <T>(<A>)[;]` at the
/// expression_statement root — exactly ONE argument (b6/b7 refuse); the
/// nested face refuses naturally (b4: no object-creation child).
fn root146_cs_new_match(
    node: &Node,
    source: &str,
    pattern: &str,
    ty: &LaneName,
    arg: &LaneMeta,
) -> Option<PatternMatch> {
    let children = root146_named_children(node);
    let [creation] = children.as_slice() else {
        return None;
    };
    if creation.kind() != "object_creation_expression" {
        return None;
    }
    let creation_children = root146_named_children(creation);
    let (type_node, arg_list) = match creation_children.as_slice() {
        [type_node, arg_list] if arg_list.kind() == "argument_list" => (type_node, arg_list),
        _ => return None,
    };
    let args = root146_named_children(arg_list);
    let [only_arg] = args.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let type_text = node_text(type_node, source)?;
    ty.bind_or_match(&mut captures, type_text.trim())?;
    let arg_text = node_text(only_arg, source)?;
    arg.bind(&mut captures, arg_text.trim())?;
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F4, grid d*): cs switch_statement — the subject slot binds
/// between the parens (d6 literal) and the body must carry EXACTLY ONE
/// switch_section (d1/d2; d4/d5 refuse). Kind-exact: the switch_expression
/// row never crosses (d7).
fn root146_cs_switch_match(
    node: &Node,
    source: &str,
    pattern: &str,
    subject: &Option<LaneName>,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let body_node = named.last()?;
    if body_node.kind() != "switch_body" {
        return None;
    }
    let subject_node = if named.len() == 2 { Some(&named[0]) } else { None };
    match (subject, subject_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let sections = root146_named_children(body_node);
    let [section] = sections.as_slice() else {
        return None;
    };
    if section.kind() != "switch_section" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(slot), Some(subject_node)) = (subject, subject_node) {
        let text = node_text(subject_node, source)?;
        slot.bind_or_match(&mut captures, text.trim())?;
    }
    let text = node_text(section, source)?;
    body.bind(&mut captures, text)?;
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F3, grid c*): php declaration kinds — the ONE-member meta
/// law (c1/c4/c7/c8/c9) and the Mixed exact-order face (c10), modeled on the
/// registered namespace lane; the Mixed-doc wrapper uses the SAME
/// declaration kind so member-kind alignment holds (class property/const
/// prefixes).
fn root146_php_decl_match(
    node: &Node,
    source: &str,
    pattern: &str,
    kind: PhpDeclKind,
    name: &LaneName,
    body: &PhpNamespaceBody,
) -> Option<PatternMatch> {
    let name_node = node.child_by_field_name("name")?;
    let body_node = node.child_by_field_name("body")?;
    let members = root146_named_children(&body_node);
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_text = node_text(&name_node, source)?;
    name.bind_or_match(&mut captures, name_text.trim())?;
    match body {
        PhpNamespaceBody::Meta(body_meta) => {
            let [only] = members.as_slice() else {
                return None;
            };
            let text = node_text(only, source)?;
            body_meta.bind(&mut captures, text)?;
        }
        PhpNamespaceBody::Mixed { prefix, tail } => {
            let doc = format!(
                "<?php\n{kw} __Px {{ {prefix} }}\n",
                kw = kind.keyword()
            );
            let tpl_tree = parse_source(Language::Php, &doc).ok()?;
            if tpl_tree.root_node().has_error() {
                return None;
            }
            let prefix_members: Vec<Node> = tpl_tree
                .root_node()
                .children(&mut tpl_tree.root_node().walk())
                .find(|n| n.kind() == kind.root_kind())
                .and_then(|decl| decl.child_by_field_name("body"))
                .map(|body| root146_named_children(&body))
                .unwrap_or_default();
            if prefix_members.is_empty() || members.len() != prefix_members.len() + 1 {
                return None;
            }
            let normalize = |node: &Node, src: &str| -> Option<String> {
                Some(strip_comment_spans(node_text(node, src)?).split_whitespace().collect())
            };
            for (p_stmt, c_stmt) in prefix_members.iter().zip(members.iter()) {
                if normalize(p_stmt, &doc) != normalize(c_stmt, source) {
                    return None;
                }
            }
            let tail_text = node_text(&members[members.len() - 1], source)?.trim();
            tail.bind(&mut captures, tail_text)?;
        }
    }
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F5, grid e*): go expression_switch_statement — optional
/// subject slot (demarcated: a global pattern refuses a subject-bearing
/// candidate and vice versa) and EXACTLY ONE case clause (e1/e2 bind; e3/e4
/// refuse). The clause text binds RAW (e1 capture keeps the trailing
/// newline). Type-switch candidates are a distinct kind (e14).
fn root146_go_switch_match(
    node: &Node,
    source: &str,
    pattern: &str,
    subject: &Option<LaneName>,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let is_clause = |n: &Node| n.kind().ends_with("_case") || n.kind() == "default_case";
    // tree-sitter-go wraps the clauses in a `switch_body` child; the
    // subject-less face (e2 `switch { $B }`) must not mistake that wrapper
    // for the subject.
    let body_wrapper = named.iter().find(|n| n.kind() == "switch_body").copied();
    let clauses: Vec<Node> = match &body_wrapper {
        Some(wrapper) => root146_named_children(wrapper)
            .into_iter()
            .filter(|n| is_clause(n))
            .collect(),
        None => named.iter().filter(|n| is_clause(n)).copied().collect(),
    };
    let subject_node = named
        .iter()
        .find(|n| !is_clause(n) && n.kind() != "switch_body");
    match (subject, subject_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let [clause] = clauses.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(slot), Some(subject_node)) = (subject, subject_node) {
        let text = node_text(subject_node, source)?;
        slot.bind_or_match(&mut captures, text.trim())?;
    }
    let text = node_text(clause, source)?;
    body.bind(&mut captures, text)?;
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F6, grid e10-e13): go if_statement — C binds the raw
/// condition bytes between the `if` keyword and the body; B binds the WHOLE
/// body inner text (e10/e11) and refuses the empty body (e12).
fn root146_go_if_match(
    node: &Node,
    source: &str,
    pattern: &str,
    cond: &LaneName,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let if_kw = children.first()?;
    if if_kw.kind() != "if" {
        return None;
    }
    let block = children
        .iter()
        .find(|child| child.is_named() && child.kind() == "block")?;
    let cond_node = children.iter().find(|child| child.is_named())?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let cond_text = source[if_kw.end_byte()..block.start_byte()].trim();
    if cond_text.is_empty() {
        return None;
    }
    let _ = cond_node;
    cond.bind_or_match(&mut captures, cond_text)?;
    let inner = &source[block.start_byte() + 1..block.end_byte().saturating_sub(1)];
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    body.bind(&mut captures, inner)?;
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F5, grid e5/e6): go package_clause — the name binds
/// through a comment gap (the comment is an extra outside the identifier).
fn root146_go_package_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [name_node] = named.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(name_node, source)?;
    name.bind(&mut captures, text.trim())?;
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F5, grid e8/e9): go SEMI-LESS import_declaration — the
/// single path or the WHOLE group text binds (X raw).
fn root146_go_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    target: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [spec] = named.as_slice() else {
        return None;
    };
    if spec.kind() != "import_spec" && spec.kind() != "import_spec_list" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(spec, source)?;
    target.bind(&mut captures, text)?;
    root146_hit(node, source, pattern, captures)
}

/// PASS 146 (145A-F7, grid f*): js if_statement arms — ONE-statement bodies
/// (f2/f3 refuse), the else-if tail binds the INNER if's head only and the
/// walk's own recursion emits per level (f4 n2); the plain-else and no-else
/// tails demarcate (f5/f6).
fn root146_js_if_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JsIfTemplate,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [cond_node, body_node, tail_node] = named.as_slice() else {
        let [cond_node, body_node] = named.as_slice() else {
            return None;
        };
        return root146_js_if_bind(
            node,
            source,
            pattern,
            template,
            cond_node,
            body_node,
            None,
        );
    };
    root146_js_if_bind(node, source, pattern, template, cond_node, body_node, Some(tail_node))
}

fn root146_js_if_bind(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JsIfTemplate,
    cond_node: &Node,
    body_node: &Node,
    tail_node: Option<&Node>,
) -> Option<PatternMatch> {
    if cond_node.kind() != "parenthesized_expression" || body_node.kind() != "statement_block" {
        return None;
    }
    let cond_inner = root146_named_children(cond_node);
    let [cond_expr] = cond_inner.as_slice() else {
        return None;
    };
    let body_stmts = root146_named_children(body_node);
    let [body_stmt] = body_stmts.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let cond_text = node_text(cond_expr, source)?;
    template.cond.bind(&mut captures, cond_text.trim())?;
    let body_text = node_text(body_stmt, source)?;
    template.body.bind(&mut captures, body_text)?;
    match &template.tail {
        JsTail::None => {
            if tail_node.is_some() {
                return None;
            }
        }
        JsTail::Block(meta) => {
            let tail_node = tail_node?;
            if tail_node.kind() != "else_clause" {
                return None;
            }
            let else_body = root146_named_children(tail_node);
            let [else_block] = else_body.as_slice() else {
                return None;
            };
            if else_block.kind() != "statement_block" {
                return None;
            }
            let stmts = root146_named_children(else_block);
            let [stmt] = stmts.as_slice() else {
                return None;
            };
            let text = node_text(stmt, source)?;
            meta.bind(&mut captures, text)?;
        }
        JsTail::ElseIf { cond, body } => {
            let tail_node = tail_node?;
            if tail_node.kind() != "else_clause" {
                return None;
            }
            let inner = root146_named_children(tail_node);
            let [inner_if] = inner.as_slice() else {
                return None;
            };
            if inner_if.kind() != "if_statement" {
                return None;
            }
            let inner_named = root146_named_children(inner_if);
            // The inner if may itself carry a FURTHER else-if tail (f4's
            // outer level, sg n2 per level): only cond+body feed this
            // level's C/D binds; the walk emits the deeper levels at their
            // own if nodes.
            if inner_named.len() < 2 || inner_named.len() > 3 {
                return None;
            }
            let (inner_cond, inner_body) = (&inner_named[0], &inner_named[1]);
            if inner_cond.kind() != "parenthesized_expression"
                || inner_body.kind() != "statement_block"
            {
                return None;
            }
            let inner_cond_expr = root146_named_children(inner_cond);
            let [inner_cond_expr] = inner_cond_expr.as_slice() else {
                return None;
            };
            let inner_stmts = root146_named_children(inner_body);
            let [inner_stmt] = inner_stmts.as_slice() else {
                return None;
            };
            let cond_text = node_text(inner_cond_expr, source)?;
            cond.bind(&mut captures, cond_text.trim())?;
            let body_text = node_text(inner_stmt, source)?;
            body.bind(&mut captures, body_text)?;
        }
    }
    root146_hit(node, source, pattern, captures)
}

/// Shared slot binding: a name field (LaneName) and an optional second
/// field (LaneName against the field node's text).
fn root142_field_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_field: &str,
    second_field: Option<&str>,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let (name, second) = match template {
        Root142Template::RsExternCrate { name } => (name, None),
        Root142Template::GoType { name, ty } => (name, Some(ty)),
        _ => return None,
    };
    let _ = second_field;
    // Modifier doctrine: a visibility/modifier child the pattern lacks
    // breaks the sg child alignment (gridded G3/G4/G6 class).
    if has_child_kind(node, "visibility_modifier") || has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name(name_field)?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    if let Some(ty) = second {
        let ty_node = node.child_by_field_name("type")?;
        ty.bind_or_match(&mut captures, node_text(&ty_node, source)?.trim())?;
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// `module <Name>\n  $B\nend` — the name field binds N; the body field
/// (the `body_statement`) binds B as the WHOLE trimmed middle (E3/E3b).
fn root142_rb_module_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::RbModule { name, body } = template else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    body.bind(&mut captures, node_text(&body_node, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// The `{ $B }` block law: the body field's named non-trivia children are
/// SINGLE-EXACT — exactly one member binds its text (E2/K1/K3); 0/≥2
/// refuse. The modifier doctrine applies to the enclosing node.
fn root142_block_member_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_field: &str,
    body_field: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::RsMod { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "visibility_modifier") || has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name(name_field)?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name(body_field)?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// `declare module <name> { $B }` — the ambient_declaration wraps a module
/// node (name field carries the QUOTES, E10); the statement block is
/// single-statement-exact (E10b).
fn root142_ts_module_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::TsDeclareModule { name, body } = template else {
        return None;
    };
    let module_node = node.named_child(0)?;
    // `declare module "x"` / `declare module x` / `namespace` — the
    // ambient child is the ts `module` or `internal_module` node (both
    // carry name!/body? fields).
    if module_node.kind() != "module" && module_node.kind() != "internal_module" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = module_node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = module_node.child_by_field_name("body")?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// `declare const <X>: <T>;` — the lexical_declaration's kind token must be
/// `const` (H23); X = the declarator name, T = the annotation text minus
/// the `:` (E11: T='number').
fn root142_ts_const_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::TsDeclareConst { name, ty } = template else {
        return None;
    };
    let decl = node.named_child(0)?;
    if decl.kind() != "lexical_declaration" {
        return None;
    }
    let mut cursor = decl.walk();
    let kind_tok = decl
        .child_by_field_name("kind")
        .or_else(|| decl.children(&mut cursor).find(|c| !c.is_named()))?;
    if node_text(&kind_tok, source)?.trim() != "const" {
        return None;
    }
    let declarators = named_non_trivia_children(&decl);
    let Some(declarator) = declarators.first() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = declarator.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let ty_node = declarator.child_by_field_name("type")?;
    let ty_text = node_text(&ty_node, source)?.trim();
    let ty_text = ty_text.strip_prefix(':').unwrap_or(ty_text).trim();
    ty.bind_or_match(&mut captures, ty_text)?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// `async def <N>($$P):\n    $$B` — the function_definition must carry the
/// `async` token (sync faces keep their existing routes); P is
/// PARAM-EXACT; B = the whole body block text trimmed (E8b).
fn root142_py_async_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::PyAsyncDef { name, param, body } = template else {
        return None;
    };
    let mut cursor = node.walk();
    let is_async = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && node_text(&c, source).is_some_and(|t| t == "async"));
    if !is_async {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let params_node = node.child_by_field_name("parameters")?;
    let params = named_non_trivia_children(&params_node);
    let [only] = params.as_slice() else {
        return None;
    };
    param.bind(&mut captures, node_text(only, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    body.bind(&mut captures, node_text(&body_node, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// `enum <name> { <body> }` — members are the enum_constant children plus
/// the declarations inside `enum_body_declarations`; the body slot is
/// SINGLE-EXACT (K3/G3), the literal slot byte-matches (H3), and the EMPTY
/// slot binds 0-member candidates (K4). Modifier candidates refuse (G3).
fn root142_ja_enum_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::JaEnum { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifiers") {
        return None;
    }
    let body_node = node.child_by_field_name("body")?;
    let mut members: Vec<Node> = named_non_trivia_children(&body_node)
        .into_iter()
        .flat_map(|child| {
            if child.kind() == "enum_body_declarations" {
                named_non_trivia_children(&child)
            } else {
                vec![child]
            }
        })
        .collect();
    if matches!(body, JaEnumBody::Meta(_)) {
        // The `{`/`}` around an EMPTY member list can surface as an
        // anonymous-token-only body; nothing to filter — 0 members is 0.
        members.retain(|m| m.kind() != "enum_body_declarations");
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    match body {
        JaEnumBody::Empty => {
            if !members.is_empty() {
                return None;
            }
        }
        JaEnumBody::Meta(meta) => {
            let [only] = members.as_slice() else {
                return None;
            };
            meta.bind(&mut captures, node_text(only, source)?.trim())?;
        }
        JaEnumBody::Literal(literal) => {
            let [only] = members.as_slice() else {
                return None;
            };
            if node_text(only, source)?.trim() != literal {
                return None;
            }
        }
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// `record <name>(<P>);[ { $B }]` — the parameter slot is PARAM-EXACT
/// (K2/K7), the optional body single-member-exact (J6); modifier
/// candidates refuse (G4).
fn root142_cs_record_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::CsRecord { name, param, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    // NOTE: the cs grammar gives record_declaration NO parameter_list
    // FIELD (only name/body are fields) — the list is a fieldless child.
    let params_node = named_non_trivia_children(node)
        .into_iter()
        .find(|c| c.kind() == "parameter_list")?;
    let params: Vec<Node> = named_non_trivia_children(&params_node)
        .into_iter()
        .filter(|c| c.kind() == "parameter")
        .collect();
    let [only] = params.as_slice() else {
        return None;
    };
    param.bind(&mut captures, node_text(only, source)?.trim())?;
    let body_node = node.child_by_field_name("body");
    match (body, body_node) {
        (None, None) => {}
        (None, Some(_)) => return None,
        (Some(_), None) => return None,
        (Some(slot), Some(body_node)) => {
            let members = named_non_trivia_children(&body_node);
            let [only] = members.as_slice() else {
                return None;
            };
            slot.bind(&mut captures, node_text(only, source)?.trim())?;
        }
    }
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// `struct <name> { $B }` — the body is SINGLE- MEMBER-EXACT (K1); empty
/// bodies refuse (J9); modifier candidates refuse (G6).
fn root142_cs_struct_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::CsStruct { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    Some(PatternMatch {
        line_start,
        line_end,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        excerpt,
        captures,
    })
}

/// PASS 142 (142A-F1 pattern-side + 142A-F5 dotted-meta, grids A21/A22/
/// A23/I1-I4/I8 + C8/C13 + G1/G2/G8/G9/I6): directive-family PATTERN
/// spellings sg ACCEPTS but binds NOTHING on (rc1 `[]` — census answerable,
/// the walk's empty is the agreement, never loud):
///   * a comment span (or, in cs, a Rust-whitespace char outside the sg
///     cs trivia class) inside the demarcation zone of a family pattern;
///   * a plain-face cs using pattern whose target is a dotted/qualified
///     META path (`$A.B`, `static $N.M`, `global::$N`).
fn directive_pattern_sg_accepts_empty(lang: Language, pattern: &str) -> bool {
    // The META-face class only: a `$`-less (literal) comment face keeps its
    // own answering route (sg binds comment-transparent literals).
    if !pattern.contains('$') {
        return false;
    }
    if !matches!(lang, Language::CSharp | Language::Php | Language::Java) {
        return false;
    }
    let Some(semi_at) = pattern.rfind(';') else {
        return false;
    };
    // A non-empty tail after the last `;` is a different face (comment
    // tails keep their own census class).
    if !pattern[semi_at + 1..].trim().is_empty() {
        return false;
    }
    let zone = &pattern[..=semi_at];
    let stripped = strip_comment_spans(zone);
    let fixed = if lang == Language::CSharp {
        stripped
            .chars()
            .map(|c| if !is_sg_cs_trivia(c) && c.is_whitespace() { ' ' } else { c })
            .collect::<String>()
    } else {
        stripped
    };
    fixed != zone && directive_template(&fixed).is_some()
}

/// The plain-face cs using dotted/qualified-META path family — every
/// segment a canonical meta or an identifier, >= 2 segments, >= 1 meta
/// (gridded G1/G2/G8/G9/I6: sg binds NOTHING on any candidate).
fn cs_using_plainface_meta_path(pattern: &str) -> bool {
    let p = pattern.trim();
    let mut rest = p;
    if let Some(after) = rest.strip_prefix("global") {
        if after.starts_with(is_sg_cs_trivia) {
            rest = after.trim_start_matches(is_sg_cs_trivia);
        }
    }
    let Some(after) = rest.strip_prefix("using") else {
        return false;
    };
    if !after.starts_with(is_sg_cs_trivia) {
        return false;
    }
    let mut rest = after.trim_start_matches(is_sg_cs_trivia);
    loop {
        let mut consumed = false;
        for keyword in [CsUsingKeyword::Static, CsUsingKeyword::Unsafe] {
            if let Some(seg) = rest.strip_prefix(keyword.spell()) {
                if seg.starts_with(is_sg_cs_trivia) {
                    rest = seg.trim_start_matches(is_sg_cs_trivia);
                    consumed = true;
                    break;
                }
            }
        }
        if !consumed {
            break;
        }
    }
    let Some(inner) = rest.strip_suffix(';') else {
        return false;
    };
    let inner = inner.trim_matches(is_sg_cs_trivia);
    if inner.is_empty() || top_level_equals(inner).is_some() {
        return false;
    }
    let segments: Vec<&str> = inner.split("::").flat_map(|s| s.split('.')).collect();
    segments.len() >= 2
        && segments.iter().any(|s| lane_meta(s).is_some())
        && segments
            .iter()
            .all(|s| lane_meta(s).is_some() || is_pattern_ident(s))
}

/// The language-free ingress unions — the PASS 141 kt/swift precedent
/// (these faces must reach the per-language census, not the query-level
/// rc2).
fn statement_root_142_any_language(pattern: &str) -> bool {
    [
        Language::Rust,
        Language::Go,
        Language::Ruby,
        Language::TypeScript,
        Language::Python,
        Language::Java,
        Language::CSharp,
        // PASS 146 (145A-F3/F7): the php declaration-kind meta bodies and
        // the js `$$`-arm faces join the language-free union (the 141/142
        // precedent).
        Language::JavaScript,
        Language::Php,
    ]
    .iter()
    .any(|&lang| statement_root_142_template(lang, pattern).is_some())
}

fn directive_accepted_empty_any_language(pattern: &str) -> bool {
    [
        Language::CSharp,
        Language::Php,
        Language::Java,
    ]
    .iter()
    .any(|&lang| {
        directive_pattern_sg_accepts_empty(lang, pattern)
            || (lang == Language::CSharp && cs_using_plainface_meta_path(pattern))
    })
}

fn walk_kt_typealias(
    node: Node,
    source: &str,
    pattern: &str,
    template: &KtTypealiasTemplate,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == "type_alias" && node.is_named() && !is_in_comment_or_string(&node) {
        if let Some(hit) = kt_typealias_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kt_typealias(child, source, pattern, template, out);
    }
}

// ===========================================================================
// PASS 137 (137A-F2/F12 + 137B-F4): the csharp statement-head lane —
// `fixed (R) { B }`, `checked { B }`, `unchecked { B }`, `unsafe { B }`.
//
// sg 0.45.2 receipts (oracle /tmp/phase137A varprobe + lockprobe re-probes,
// grid137a B3/D4): the four heads are RESERVED in csharp, so a template
// cannot parse at a bare top level and the general lane's context wrap
// roots at kinds the root gate refuses (`fixed_statement` et al. are not
// general roots) — the faces starved (grid: `fixed ($D) { *p = 'x'; }` sg
// n1 D=`char* p = s` vs subject rc2; `checked { int v = a + b; }` sg n1 vs
// subject n0). Their body metas also bind NON-expression statements
// (`checked { $B }` binds B=`int v = a + b;` — a local_declaration), which
// the general lane's expression-statement template shape can never align.
//
// The probed sg law this lane encodes:
//   * paren heads (fixed) with a META resource bind the whole resource text
//     (`fixed ($D)` → D=`char* p = s`);
//   * a META BODY binds only under checked/unchecked/unsafe, and only a
//     ONE-statement candidate body (`checked { $B }` → B=`int v = a + b;`);
//     under `fixed` sg binds NOTHING — every candidate answers valid-empty
//     (grid fxB_multi rc1 [] with the candidate present);
//   * concrete bodies align statement-wise with meta unification
//     (`unchecked { int v = a + $C; }` → C=`1`);
//   * PASS 139 (139A-F1, grid139 A): NESTED head compositions
//     (`fixed ($D) { checked { $B } }`, `unsafe { checked { $B } }`,
//     `lock ($L) { checked { x = $V; } }`, any depth/order) BIND one match
//     at the OUTERMOST statement — `$D`/`$B`/`$V` each bind their own
//     level (resource text / innermost body statement / inner expression);
//     the fixed/lock/using binds-nothing law holds at every level but ONLY
//     for the BARE-meta body (nested-head bodies under those heads bind).
//     One-statement law per pattern body slot at every level.
//   * lock/using META bodies are sg-ACCEPTED bind-nothing faces (f137b) —
//     census-answerable, walk-empty (silent ok:true-0 parity, replacing the
//     pass-135 loud class); their concrete bodies stay on the general lane.
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CsStatementHead {
    Fixed,
    Checked,
    Unchecked,
    Unsafe,
    Lock,
    Using,
}

impl CsStatementHead {
    fn root_kind(self) -> &'static str {
        match self {
            CsStatementHead::Fixed => "fixed_statement",
            // tree-sitter-c-sharp has NO unchecked_statement — both spellings
            // root as `checked_statement` (the grammar's single
            // checked/unchecked kind); the anonymous HEAD KEYWORD token is
            // the discriminator and sg's structural match compares it, so
            // csharp_statement_match verifies the keyword text.
            CsStatementHead::Checked | CsStatementHead::Unchecked => "checked_statement",
            CsStatementHead::Unsafe => "unsafe_statement",
            // tree-sitter-c-sharp spells the lock head `lock_statement`
            // (node-types; the general lane's own force-empty check reads
            // the parsed root kind "lock_statement"). The 137 "locked_
            // statement" spelling was a guess never walk-exercised — the
            // lock META-body face was force-empty pre-139; PASS 139's
            // nested/concrete-meta lock faces walk this root for the first
            // time (grid139 A08/A24/A29 receipts).
            CsStatementHead::Lock => "lock_statement",
            CsStatementHead::Using => "using_statement",
        }
    }
    fn spell(self) -> &'static str {
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
enum CsResource {
    Meta(String),
    Literal(String),
}

#[derive(Debug, Clone)]
enum CsBody {
    /// `{ $B }` — binds the candidate's single body statement (checked/
    /// unchecked/unsafe); under `fixed`/`lock`/`using` the bare-meta face
    /// answers valid-empty (PASS 137/139: at ANY nesting depth).
    Meta(String),
    /// PASS 139 (139A-F1, grid139 A): the body section is itself a
    /// head-statement spelling (`fixed ($D) { checked { $B } }`) — sg binds
    /// one match at the OUTERMOST statement, every level's resource/meta
    /// binding its own capture (`$B` = the INNERMOST body's single
    /// statement). The accepted-empty law of `fixed`/`lock`/`using` holds
    /// only for a BARE-meta body at that level (grid A05/A07/A19/A20/A22
    /// sg rc1 `[]`); a nested-head body under those heads BINDS (grid
    /// A01/A02/A08/A09/A24). Served by recursing
    /// [`csharp_statement_bind`] into the candidate's single body
    /// statement — captures merge upward, the match emits at the root.
    Nested {
        inner: Box<CsStatementTemplate>,
        /// PASS 146 (145A-F10, grid i*): the PATTERN-side seam — bytes
        /// between the enclosing `{` and the nested head in the QUERY must
        /// be sg-class trivia; an outsider char (U+2028, i1) refuses where
        /// sg answers valid-empty. Symmetric with the candidate-side byte
        /// check in [`csharp_statement_bind`].
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
struct CsStatementTemplate {
    head: CsStatementHead,
    resource: Option<CsResource>,
    body: CsBody,
}

/// The PASS 137 csharp statement-template parser. Spelling-level admission
/// only: head keyword, balanced resource parens (fixed), balanced braces,
/// and the body grammar (bare meta / meta-free-of-noncanonical-substitution
/// statement text). The per-language truth is the walk.
fn csharp_statement_template(pattern: &str) -> Option<CsStatementTemplate> {
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
        // PASS 139 (139A-F1, grid139 A): a nested statement-head composition
        // — `fixed ($D) { checked { $B } }` — binds sg-exactly at the
        // nested level too (A01/A08/A24 sg n1), so the body recurses into
        // its own template instead of refusing or placeholder-hijacking.
        // BOUNDEDNESS-OF-RECORD (141B-F3): this recursion is
        // PATTERN-controlled — one frame per nested head-brace group in
        // the QUERY string, depth bounded only by the pattern's own brace
        // nesting. No depth bound is enforced: any bound is a refusal
        // sg-exactness claim that would need its own oracle grid first
        // (a wrong bound trades a remote-DoS surface for silent
        // under-serve). Hostile adversarial query strings remain a
        // documented exposure of this lane.
        // PASS 146 (145A-F10, grid i*): capture the pattern-side seam
        // class — the lead bytes before the section are sg-trivia-clean or
        // the bind refuses (i1 U+2028 seam, sg valid-empty).
        let section_raw = &inner[..close];
        let lead = &section_raw[..section_raw.len() - section_raw.trim_start().len()];
        let seam_clean = lead.chars().all(is_sg_cs_trivia);
        CsBody::Nested {
            inner: Box::new(nested),
            seam_clean,
        }
    } else {
        // PASS 139 (grid139 A12/A17): a concrete Template body parses for
        // fixed/checked/unchecked/unsafe heads and binds sg-exactly in the
        // dedicated lane. lock/using keep their 137 contract: a
        // placeholders-EMPTY (fully concrete) body stays refused at parse —
        // those spellings keep the general-lane route established in PASS
        // 137 (the dispatch above serves lock/using via the general
        // template); a lock/using template WITH placeholders is new-lane
        // territory and binds.
        let (substituted, placeholders, _) = substitute_general_metavariables(section)?;
        if placeholders.is_empty()
            && matches!(head, CsStatementHead::Lock | CsStatementHead::Using)
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
fn match_csharp_statement(source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
    let template = csharp_statement_template(pattern)?;
    // sg's registered meta-body law for `fixed`/`lock`/`using` (137A-F2/
    // 137B-F4, f137b) — PASS 139 grid extension: the law holds at EVERY
    // nesting level (grid A05/A07/A19/A20/A22: `unchecked { fixed ($D) {
    // $B } }`, `unsafe { lock ($L) { $B } }`, `fixed ($D1) { fixed ($D2) {
    // $B } }` all rc1 `[]`), and ONLY for the bare-meta body — a
    // nested-head body under those heads BINDS (A01/A08/A24 sg n1). The
    // walk's empty IS the sg agreement for the binds-nothing family.
    if cs_template_binds_nothing(&template) {
        return Some(Vec::new());
    }
    let tree = parse_source(Language::CSharp, source).ok()?;
    let mut out = Vec::new();
    walk_csharp_statement(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

/// PASS 139: true when any level of the head composition is a
/// `fixed`/`lock`/`using` head with a BARE-meta (`$B`) body — the sg
/// accepted-and-binds-nothing class (grid139 A). Nested-head bodies do NOT
/// trigger it (they bind).
fn cs_template_binds_nothing(template: &CsStatementTemplate) -> bool {
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

fn walk_csharp_statement(
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

fn csharp_statement_match(
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

/// PASS 139 refactor of the 137 matcher: the per-candidate BIND half —
/// head-keyword token check, resource bind, body bind — mutating one shared
/// capture map so the Nested arm can recurse into the candidate's single
/// body statement and merge every level's captures (sg emits ONE match at
/// the OUTERMOST statement; grid A01 metaVariables: `$D`=outer resource,
/// `$B`=innermost body statement).
fn csharp_statement_bind(
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
    // root kind; sg's structural match compares the anonymous keyword).
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
    let body = named.iter().copied().find(|c| BLOCK_KINDS.contains(&c.kind()))?;
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
            // sg law: `{ $B }` binds a ONE-statement body's statement text
            // (`checked { $B }` → B=`int v = a + b;`).
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
            // PASS 146 (145A-F10, grid i*): the seam gate is symmetrical —
            // a pattern-side outsider char refuses (i1, sg valid-empty)
            // exactly as the candidate-side check below refuses (i2).
            if !seam_clean {
                return None;
            }
            // PASS 139 (grid139 A): the pattern body is itself a head
            // statement — the candidate body must be exactly ONE statement
            // of the nested head's root kind; the bind recurses (head token
            // check, resource bind, body bind) and captures merge upward.
            // PASS 140 (140A-F1, grids A_v_inner/A_v_block/A_3lvl_c1/
            // A_3lvl_c2): sg's nested-head seam is TRIVIA-TIGHT — a comment
            // between the enclosing body's `{` and the nested-head statement
            // refuses the match (rc1 `[]`), while comments before the outer
            // head, inside the innermost body, and after the inner close
            // keep binding (A_v_outer/A_v_leaf/A_v_after/A_meta_inner n1).
            // Byte-tight check: only whitespace may sit between the body's
            // open brace and the nested statement.
            let [only] = stmts.as_slice() else {
                return None;
            };
            // PASS 141 (141A-F4 + 141B-F1, grid D_*): the seam trivia class
            // of record — sg's cs grammar extras are /[\s\u00A0\uFEFF\u3000]+/
            // where that `\s` is ASCII-scoped ([\t\n\v\f\r ]), so the class
            // is ASCII whitespace (incl. vertical tab U+000B, which Rust's
            // `is_ascii_whitespace` excludes) PLUS the explicit members
            // U+00A0 (NBSP), U+FEFF, U+3000 (D_d_vtab/D_d_nbsp/D_d_u3000/
            // D_d_feff sg n1) while U+0085/U+2028/U+202F REFUSE (D_d_nel/
            // D_d_u2028/D_d_u202f rc1 `[]`). Comments still refuse (any
            // comment byte is outside the class); CRLF/formfeed controls
            // keep binding.
            let seam = &source[body.start_byte() + 1..only.start_byte()];
            if seam.chars().any(|c| {
                !(matches!(
                    c,
                    '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' '
                        | '\u{00A0}' | '\u{FEFF}' | '\u{3000}'
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
            let doc = format!(
                "class __AsgrepCtx {{ void M() {{ {substituted} }} }}"
            );
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

/// PASS 137 (D3 grid): the `return (META)` family lane (js/ts). sg 0.45.2
/// answers `return ($X)` and `return($X)` on `return (1);` binding X=`1` —
/// the parenthesized operand's inner text; the operand-less `return;` and
/// paren-free operand candidates never align (structural). The Call
/// classification of the spelling (callee `return`) never fires — a source
/// can never spell a callee `return` — so this lane serves the family
/// sg-exactly and every other return face keeps its routes.
fn match_return_paren_meta(lang: Language, source: &str, pattern: &str) -> Option<Vec<PatternMatch>> {
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

fn walk_return_paren_meta(
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
    // PASS 135 (134B-F1/F2, grids /tmp/phase135/cells + the Z-grid re-probed
    // fresh 2026-09-11): the escaped statement-head spellings OUTSIDE their
    // keyword grammars keep the F66a-8 plain-identifier doctrine — sg
    // answers the identifier faces (`go` js/ts/py/java/c/rb/kt n2; `defer`
    // js/py/rs n2; `await` js idents n2, c n2, rs ident n1) and NEVER the
    // general lane's childless template, which silently answers nothing for
    // them (the 133 return-class intercept mechanism: grid js `go` sg n2 vs
    // subject n0). `go`/`defer` ride the full literal lane — the 133
    // keyword-token-leaf route (swift's `defer` token leaf n1 agrees);
    // `await` takes the identifier-faces-ONLY route because sg never
    // answers the token inside an await_expression (js Z-grid: idents n2,
    // the `await g()` token unanswered), and python's bare `await` is the
    // receipted sg REFUSAL (rc1 valid-empty — the literal lane would
    // over-answer the token inside `await g()`). TypeScript keeps its
    // await_expression arm below (sg's ts rows sit at the await tokens; the
    // arm's statement spans agree on line and count — the pass-65 excerpt
    // genus).
    if matches!(keyword, "go" | "defer") && lang != Language::Go {
        return Some(match_literal_pattern(lang, source, pattern).unwrap_or_default());
    }
    if keyword == "await" {
        // PASS 137 (137A-F11, §45.11 receipt REFUTED): the registered claim
        // that sg 0.45.2 rc1s the py bare `await` face (valid-empty) is
        // refuted — grid137 F/reg_py_bare_await (2026-09-11): sg ANSWERS n1
        // on the bare-`await` fixture while the subject answered n0. The
        // literal-lane keyword-token route (the F66a-8/133 doctrine for
        // un-armed escaped heads) is the corrected serving path.
        // PASS 138 (F-137E-1): the route is SHARPENED to the sg-answered
        // shape. sg's bare `await` pattern binds only the bare keyword
        // subtree: a bare `await` token error-recovers to a plain
        // `identifier` (grid138 parse probe /tmp/phase138/pyparse_probe),
        // while `await <expr>` yields an `await`-kind node with a named
        // operand child the identifier-shaped pattern cannot match — the
        // operand-bearing faces (`await g()` statement / assignment /
        // comprehension / paren, top-level included) are sg rc1 `[]` (the
        // 136E registered both-empty envelope; the F-137E-1 cell). The 137
        // route over-served the keyword token on those rows. Refuse every
        // hit whose span sits inside an operand-bearing `await` node; the
        // operand-free statements keep the §46-refuted binding (grid138
        // S1/S6/S7/S8/S10/S11/S13/S16 + E1/E2: bare, `;`-terminated,
        // `await ;`, trailing comment, `await; g()` where `;` ends the
        // statement, one row per token, `await = 5` ident recovery). The
        // `;`-ful PATTERN spelling is sg valid-empty on every probed
        // fixture (19/19 grid138 rc1 `[]`, the f137j accepted-empty class)
        // while the walk served it through this same lane.
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
        // PASS 133 (F-132E class sweep): ruby names the bare/control node
        // `break` (both the bare and command spellings share the name) — the
        // statement/expression kinds below do not exist in tree-sitter-ruby,
        // so the arm SILENCED the sg-answered face (grid rb:break sg n1,
        // subject n0; oracle /tmp/phase133/live/grid.json) instead of
        // falling through to the literal lane like arm-less `next` did.
        // `break` as a kind name exists in no other indexed grammar.
        "break" => &["break_statement", "break_expression", "break"],
        "continue" => &["continue_statement", "continue_expression"],
        "throw" => &["throw_statement"],
        "yield" => &["yield", "yield_statement"],
        "raise" => &["raise_statement"],
        // PASS 131 (130A-F5, f131d): the js/ts debugger statement root —
        // sg answers the debugger_statement for both the `;`-ful and bare
        // spellings (oracle dbg_js / dbg_ts). SCOPED to the two grammars
        // where `debugger` is a keyword: elsewhere the token is a plain
        // identifier the literal lane must keep serving (the F66a-8 ruby
        // `raise` doctrine).
        "debugger" if matches!(lang, Language::JavaScript | Language::TypeScript) => {
            &["debugger_statement"]
        }
        // PASS 133 (F-132E class sweep, 52-cell grid
        // /tmp/phase133/live/grid.json): the bare `return` family is the
        // one class the walk UNDER-answers without a kinds arm — the bare
        // spelling is a STATEMENT_HEAD_KEYWORD, so the general lane's
        // childless-template face intercepts below this lane and answers
        // only the operand-less form (grid js/ts:return sg n3 vs subject
        // n1; rs n2 vs 1; go n3 vs 2; rb n2 vs 1), while sg answers every
        // statement of the family. Scoped to the grammars the grid pins
        // (an arm whose kinds a grammar lacks SILENCES the face — the
        // F66a-8 ruby-raise doctrine): js/ts/go statements, rust
        // return_expression (rust has no return_statement; sg's rs rows
        // sit at those spans), ruby `return` (sym_return/sym_return_command
        // share the name; the outermost-first kind walk answers one row
        // per site). java/c/cpp/csharp `return` keep their agreeing
        // general-lane face untouched.
        //
        // The OTHER pass-133 escapes (import/pass/global/del/assert/use/
        // fallthrough/goto/redo/retry) need NO arm here: without one they
        // fall to the literal lane, which answers the keyword-token leaf —
        // byte-identical to sg's row — and mutants M-133c proved the
        // f133b/f133c pins hold with arms deleted. RB `break` is the
        // opposite shape: ruby names the node `break`, the pre-existing
        // statement/expression kinds exist nowhere in tree-sitter-ruby,
        // and the arm SHORT-CIRCUITED with an empty set where sg answers
        // n1 (grid rb:break) — appending the kind restores the fall
        // through-to-literal behavior arm-less `next` always had.
        // PASS 135 (134A-F2, grids /tmp/phase135/cells gridBCG): sg's
        // empty-operand discipline on the `;`-ful js/ts spelling —
        // `return;` answers ONLY the operand-less return_statement (grid
        // B_js_return;: sg n1 vs subject n3; B_ts_return;: n1 vs n2), while
        // the bare spelling keeps answering the whole family and rust keeps
        // its all-returns `;`-ful answer (B_rs_return;: sg n2 — the `;` is
        // not an operand marker there; go/rb/py `return;` spellings rc8 at
        // sg's own gate and never reach this lane). go/py join the scoped
        // statement arm and go's `defer`/`go` + ts `await` join here too
        // (grids G_go_defer/G_go_go/X_ts_await_bare_src: sg answers the
        // statement kind; js `await` stays arm-less — sg rc1 valid-empty,
        // grid G_js_await, both engines answer nothing).
        "return" if matches!(lang, Language::JavaScript | Language::TypeScript | Language::Go) => {
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
    // PASS 135 (134A-F2): the `;`-ful js/ts return spelling demands an
    // operand-less candidate (sg's empty-operand discipline, grid
    // B_js_return;). A trivia-carrying empty return
    // (`return /* c */;`) stays operand-less — comments are trivia, grid
    // Z_js_return_semi: sg answers both empty spellings.
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

/// PASS 138 (F-137E-1), PASS 139 span refinement (139B-F2, grid139 H):
/// drop every py bare-`await` hit whose span sits on the `await` KEYWORD
/// TOKEN of an operand-bearing `await` (await_expression) node — the
/// literal-lane row overlaps exactly that token, never the whole node. The
/// 138 whole-node containment also dropped a genuinely sg-answerable row:
/// a bare `await` token that error-recovers to an identifier INSIDE an
/// operand-bearing await's operand (`x = await (await)` — sg n1 on the
/// inner identifier, grid139 H01/H02) is span-disjoint from the outer
/// keyword token and now survives. sg's bare pattern matches only the
/// identifier-recovered bare keyword subtree, so operand rows never answer
/// (grid138 S2/S3/S4/S5/S9/S12/S15, parse probe
/// /tmp/phase138/pyparse_probe); hits outside collected tokens keep the
/// literal-lane bytes.
fn retain_py_await_operand_free(source: &str, hits: &mut Vec<PatternMatch>) {
    if hits.is_empty() {
        return;
    }
    let Ok(tree) = parse_source(Language::Python, source) else {
        return;
    };
    let mut operand_spans: Vec<(usize, usize)> = Vec::new();
    collect_py_operand_await_spans(tree.root_node(), &mut operand_spans);
    if operand_spans.is_empty() {
        return;
    }
    hits.retain(|m| {
        !operand_spans
            .iter()
            .any(|&(start, end)| m.byte_start >= start && m.byte_end <= end)
    });
}

/// The KEYWORD-TOKEN span of each operand-bearing `await` node — the byte
/// range of the anonymous `await` token (the node's first non-named
/// child), not the whole-node span (the PASS 139 refinement above).
fn collect_py_operand_await_spans(node: Node, spans: &mut Vec<(usize, usize)>) {
    if node.kind() == "await" && node.named_child_count() > 0 {
        let mut cursor = node.walk();
        let token = node
            .children(&mut cursor)
            .find(|child| !child.is_named())
            .map(|child| (child.start_byte(), child.end_byte()));
        if let Some(span) = token {
            spans.push(span);
        } else {
            // No anonymous child (defensive): fall back to the node start
            // — still never swallows a sibling-disjoint identifier row.
            spans.push((node.start_byte(), node.start_byte()));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_py_operand_await_spans(child, spans);
    }
}

/// [`collect_kind_matches`] with an optional operand-less candidate filter
/// (PASS 135): when `require_operand_less`, only kind nodes with no
/// non-trivia named children answer — the `;`-ful `return;` face's
/// empty-operand discipline.
fn collect_kind_matches_filtered(
    lang: Language,
    source: &str,
    pattern: &str,
    kinds: &[&str],
    require_operand_less: bool,
) -> Vec<PatternMatch> {
    if !require_operand_less {
        return collect_kind_matches(lang, source, pattern, kinds);
    }
    let Ok(tree) = parse_source(lang, source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk_kind_list_operand_less(tree.root_node(), source, pattern, kinds, &mut seen, &mut out);
    out
}

fn walk_kind_list_operand_less(
    node: Node,
    source: &str,
    pattern: &str,
    kinds: &[&str],
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if kinds.contains(&node.kind()) && !is_in_comment_or_string(&node) {
        let mut cursor = node.walk();
        let operand_less = node
            .children(&mut cursor)
            .all(|child| child.kind().contains("comment") || !child.is_named());
        if operand_less {
            walk_kind_list(node, source, pattern, kinds, seen, out);
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kind_list_operand_less(child, source, pattern, kinds, seen, out);
    }
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
        had_semi: false,
        root_kind: String::new(),
        force_empty: false,
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
        NativeKind::Class {
            keyword,
            name,
            body,
        } => {
            // PASS 131 (130A-F8, f131f): the interface member-count template
            // rides the existing body-filter machinery — the ts/java
            // `body` field of an interface_declaration IS the interface_body,
            // whose named non-trivia children are the members
            // (`function_body_matches` counts them). Scoped to the receipted
            // grammars AND the interface keyword; other languages never pass
            // the answerability gate, and the walk stays silent-empty for
            // them as a defense. PASS 135 (f135b): sg's refusal doctrine on
            // this lane is CANDIDATE-scoped — extends/base clauses,
            // non-empty modifier lists, and trivia in the name→body gap
            // refuse (grids D_ts_ext1/ext2, D_java_public, D_java_triv,
            // X_cs_iface_ext/pub, Y_cs_iface_triv: sg rc1 n0) — consulted
            // per candidate inside [`run_queries`]. PASS 135 (134A-F7):
            // the ts `type-alias` object face rides the same machinery with
            // the type_alias_declaration's `value` object-type as the body.
            let body_filter = if matches!(*keyword, "interface" | "type-alias" | "class")
                && match (*keyword, lang) {
                    ("interface", Language::TypeScript | Language::Java | Language::CSharp) => true,
                    ("type-alias", Language::TypeScript) => true,
                    // PASS 139 (139A-F5, grid139 E): the java class
                    // member-count face — sg binds single-member java
                    // classes (E01/E02/E08 n1), refuses empty/multi-member/
                    // extends (E03/E04/E07 rc1 `[]`).
                    ("class", Language::Java) => true,
                    _ => false,
                }
            {
                body.as_ref()
            } else {
                None
            };
            run_queries(
                &language,
                lang,
                tree.root_node(),
                source,
                class_queries_for(lang, keyword),
                declaration_modifiers,
                name.as_deref(),
                Some((lang, *keyword)),
                body_filter,
                None,
                pattern,
                &mut out,
            )?;
        }
        NativeKind::Call { path, arg_slots } => {
            walk_calls(
                tree.root_node(),
                source,
                pattern,
                path,
                arguments.as_ref(),
                arg_slots.as_deref(),
                lang,
                &mut out,
            );
        }
        NativeKind::MemberCall {
            path,
            arg_slots,
            require_continuation,
            nullsafe,
        } => {
            walk_member_calls(
                tree.root_node(),
                source,
                pattern,
                path,
                arguments.as_ref(),
                arg_slots.as_deref(),
                *require_continuation,
                *nullsafe,
                &mut out,
            );
        }
        NativeKind::MemberCallChain {
            segments,
            nullsafe_flags,
        } => {
            walk_member_call_chains(
                tree.root_node(),
                source,
                pattern,
                segments,
                nullsafe_flags,
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
            // PASS 115 (114A-F1/F2): a property-segment `?.` chain outside the
            // optional machinery's registered grammar family keeps the general
            // structural lane it rode pre-115 (byte-identical walk) — the
            // leaf-decomposition contract refuses kotlin/swift/rust connector
            // shapes, and silently emptying their sg-answering clean faces is
            // not an option. Inside the family (ts/js) the gated
            // `walk_optional_call_chains` is the whole fix.
            // PASS 117 (116A-F1): the preserved general arm is no longer
            // UNGATED — kotlin/swift junction-comment and mid-chain/receiver
            // trivia faces over-answered where sg refuses (first-hand grid
            // 2026-09-08). The gated wrapper below keeps every clean face's
            // answer and refuses exactly the commented faces, with the ONE
            // junction rule (every consumed call level) plus the member-link
            // trivia veto applied to the CANDIDATE nodes the general lane
            // matched.
            let property_face = segments.iter().skip(1).any(|segment| {
                segment.args.is_none() && segment.arg_slots.is_none()
            });
            if property_face && !matches!(lang, Language::TypeScript | Language::JavaScript) {
                return Ok(match_structural_chain_gated(lang, source, pattern));
            }
            walk_optional_call_chains(
                lang,
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
                lang,
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
        NativeKind::If {
            cond,
            body,
            body_braced,
            alternative,
        } => {
            walk_ifs(
                lang,
                tree.root_node(),
                source,
                pattern,
                cond.as_deref(),
                body.as_ref(),
                *body_braced,
                alternative.as_ref(),
                &mut out,
            );
        }
        NativeKind::Assignment {
            target,
            op,
            op_class,
            rhs_expr,
            value,
            value_multi,
        } => {
            // PASS 83a (FB-82a-02): the general RHS pattern tree must
            // outlive the walk (classified faces always parse —
            // `validate_php_rhs_expr` — so a None here only means the
            // bare-meta path). The rhs PATTERN text rides along: pattern
            // node offsets index the pattern source, not the candidate.
            let rhs_tree = rhs_expr.as_ref().and_then(|rhs| parse_php_rhs_tree(rhs));
            // The pattern source is the template DOC (`<?php $V + 1;`) —
            // pattern node offsets are doc-relative.
            let rhs_root = rhs_tree
                .as_ref()
                .map(|parsed| (parsed.tree.root_node(), parsed.doc.as_str()));
            // PASS 127 (125A-F5, f127a): a META assignment target (`$X`,
            // `$o->$A`) parses its own php doc so the walker binds it
            // structurally (the whole-meta spelling binds the candidate LHS
            // text; a meta link binds the link node text) instead of the
            // verbatim LHS text equality literal targets use.
            // PASS 131 (130A-F2, f131a): so does a STATIC target carrying a
            // single-canonical-meta subscript index (`C::$s[$K]` binds K to
            // the whole candidate index text, oracle a_msub/a2_callsub).
            // PASS 135 (134A-F6, f135d): so does EVERY other static target —
            // sg is layout-insensitive (`C::$s = $V` answers the padded
            // candidate `C :: $s = 5`, gridE E_php_padded2), so the byte
            // compare is replaced by the structural LHS unification for the
            // whole literal-static family. Dynamic-class heads
            // ([`php_static_target_dynamic_head`]) stay on the walker's
            // dedicated scope-text branch below (their sg binding is the
            // whole candidate scope TEXT, not a node unification).
            let lhs_tree = if php_assignment_target_is_meta(target)
                || php_static_target_has_meta_index(target)
                || (is_php_static_scope_target(target)
                    && php_static_target_dynamic_head(target).is_none())
            {
                parse_php_rhs_tree(target)
            } else {
                None
            };
            let lhs_root = lhs_tree
                .as_ref()
                .map(|parsed| (parsed.tree.root_node(), parsed.doc.as_str()));
            // F-r40-3 (r41): the pattern's RHS-head comment sequence,
            // derived once for the whole walk.
            let head_comments = php_assignment_head_comments(pattern, target, op);
            walk_php_assignments(
                tree.root_node(),
                source,
                pattern,
                target,
                op,
                *op_class,
                rhs_root,
                lhs_root,
                value,
                *value_multi,
                &head_comments,
                &mut out,
            );
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
            // PASS 137 (137A-F5): for JAVA INTERFACE candidates the prefix
            // demand is delegated to [`interface_candidate_refused`]'s
            // subsequence+annotation arm — sg 0.45.2 binds non-prefix
            // candidates (`public interface $N { $B }` answers `abstract
            // public interface K` n1, grid137 B-lane), which this
            // leading-text gate would refuse before the comparative arm ran.
            // PASS 139 (139A-F5, grid139 E02): JAVA CLASS member-count
            // candidates join — sg hops pattern keywords over candidate
            // modifier keywords on classes too (`public abstract class $N
            // { $B }` answers `public static abstract class A` n1). The
            // class extension is scoped to the member-count faces the PASS
            // 139 admission created (body_filter.is_some()), so every
            // pre-existing class face keeps its exact pre-139 gate.
            // Every other lane keeps the strict prefix gate.
            let java_interface_comparative = matches!(
                class_filter,
                Some((Language::Java, "interface"))
            ) && declaration_modifiers.is_some()
                || matches!(class_filter, Some((Language::Java, "class")))
                    && declaration_modifiers.is_some()
                    && body_filter.is_some();
            if !java_interface_comparative
                && !declaration_modifiers_match(&node, source, declaration_modifiers)
            {
                continue;
            }
            if let Some((lang, keyword)) = class_filter {
                if !class_keyword_matches(lang, &node, source, keyword) {
                    continue;
                }
                // PASS 135 (134A-F4, f135b): sg's per-candidate refusals on
                // the interface member-count lane — extends/base heritage
                // clauses, non-empty modifier lists, and trivia inside the
                // name→body gap all refuse (grids D_ts_ext1/ext2,
                // D_java_public, D_java_triv, X_cs_iface_ext/pub,
                // Y_cs_iface_triv: sg rc1 n0; trivia BEFORE the name keeps
                // answering, D_ts_triv_before_name sg n1).
                // PASS 136 (F-135E-1, f136a): the modifier refusal is
                // PATTERN-COMPARATIVE, not blanket — sg 0.45.2 binds the
                // modifier-MATCHING candidate (`public interface $N { $B }`
                // answers `public interface K` n1, N/B, csharp AND java —
                // 135E grids). [`interface_candidate_refused`] receives the
                // pattern-side modifier text and adjudicates per grammar.
                // PASS 139 (grid139 E07): java class candidates consult the
                // same scan on the member-count faces — the heritage clause
                // refuses (`public abstract class $N { $B }` ×
                // `class A extends B { … }` sg rc1 `[]`).
                if matches!(keyword, "interface")
                    || (matches!(keyword, "class")
                        && lang == Language::Java
                        && body_filter.is_some())
                {
                    if interface_candidate_refused(lang, &node, source, declaration_modifiers) {
                        continue;
                    }
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

/// PASS 135 (134A-F4): the per-candidate interface refusals sg applies to
/// the member-count lane — a heritage clause (`extends_type_clause` ts /
/// `extends_interfaces` java / `base_list` csharp) or a comment child fully
/// inside the name→body gap (`interface I /* c */ {`) each refuse the
/// candidate. Trivia BEFORE the name attaches outside the gap and keeps
/// answering (sg n1).
/// PASS 136 (F-135E-1): the modifier refusal is SEQUENCE-COMPARATIVE — the
/// 135 blanket `any modifier refuses` posture under-answered the
/// modifier-carrying PATTERN faces (`public interface $N { $B }` sg-binds
/// `public interface K` n1, csharp AND java). Probed semantics (135E grids +
/// /tmp/phase136/sgprobe):
///   * PATTERN carries NO modifiers: any candidate modifier list refuses
///     (the F4 posture, unchanged — f135b pins);
///   * csharp: the leaf `modifier` sequence must byte-equal the pattern
///     sequence EXACTLY (`public` refuses `public sealed`, `public sealed`
///     answers `public sealed`, sg probed both);
///   * TypeScript: unchanged — any modifier-carrying candidate refuses (the
///     ts export face binds through the wrapper-parent arm, f136a/ts grid).
/// Heritage/gap-trivia refusals fire regardless of modifiers (sg rc1 on
/// `public interface K : J` and the trivia twins, probed 136).
/// PASS 137 (137A-F5 — CORRECTS the 136 java semantics of record, appended
/// to CNR §45 + HYP): the 136 java arm ("pattern sequence byte-equals the
/// candidate's LEADING tokens") was an INCOMPLETE reading — grid137 B-lane
/// probes sg 0.45.2 answering non-prefix candidates: `public interface $N {
/// $B }` BINDS `abstract public interface K` / `static public interface K`
/// n1, `public abstract interface $N { $B }` BINDS `static public abstract`
/// and the newline-split `public\nabstract` list n1 (trivia-insensitive).
/// The grid-fitted java model (every probed row reproduced — javaprobe +
/// pinprobe, 2026-09-11):
///   * KEYWORDS: greedy scan hop over non-matching candidate KEYWORDS —
///     unanchored order-preserving subsequence (`public` ⊆ `abstract
///     public` binds; `public abstract` ⊄ `abstract public` refuses, order
///     still matters; `final` ⊆ `static final public` binds);
///   * ANNOTATIONS (`@…` children of the `modifiers` node): POSITIONAL and
///     never skippable — one must sit exactly at the scan cursor
///     (`@SafeVarargs` binds `@SafeVarargs public`, refuses `@Deprecated
///     @SafeVarargs public`; `@Deprecated` binds `@Deprecated public`,
///     refuses `public @Deprecated`), and a sitting annotation BLOCKS a
///     keyword's passage (`public` refuses `@Deprecated public`).
/// PASS 137 (137B-F1): the 136 debug_assert keyed the function to
/// TypeScript|Java|CSharp, but the library entry `match_pattern` dispatches
/// Class-classified patterns WITHOUT the core answerability gate, so e.g.
/// `match_pattern(Language::Kotlin, …, "interface $N { $B }")` reached the
/// assert and PANICKED debug builds (release silently applied the ts
/// blanket arm). The assert is replaced by the explicit receipted-language
/// guard: non-TS/Java/CSharp callers take the ts blanket posture (refuse
/// modifier-carrying candidates), the documented pre-137 release behavior.
fn interface_candidate_refused(
    lang: Language,
    node: &Node,
    source: &str,
    pattern_modifiers: Option<&str>,
) -> bool {
    let receipted = matches!(
        lang,
        Language::TypeScript | Language::Java | Language::CSharp
    );
    let name = node.child_by_field_name("name");
    let body = node.child_by_field_name("body");
    let mut cursor = node.walk();
    let mut candidate_modifiers: Vec<&str> = Vec::new();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // PASS 139 (grid139 E07): java class heritage kinds join the
            // refuse list — `superclass`/`interfaces` appear only on java
            // class_declaration candidates (java interfaces carry
            // `extends_interfaces`), and sg refuses an extends-carrying
            // class candidate (`public abstract class $N { $B }` ×
            // `class A extends B { … }` rc1 `[]`).
            "extends_type_clause" | "extends_interfaces" | "base_list" | "superclass"
            | "interfaces" => return true,
            // java wraps its modifier list in ONE `modifiers` node; csharp
            // spells each modifier as its own leaf `modifier` child
            // (tree-sitter-c-sharp has no wrapper — node-types:
            // interface_declaration children carry `modifier` singles).
            "modifiers" => {
                let mut modifier_cursor = child.walk();
                for modifier in child.children(&mut modifier_cursor) {
                    if let Some(text) = node_text(&modifier, source) {
                        candidate_modifiers.push(text.trim());
                    }
                }
            }
            "modifier" => {
                if let Some(text) = node_text(&child, source) {
                    candidate_modifiers.push(text.trim());
                }
            }
            kind if kind.contains("comment") => {
                if let (Some(name), Some(body)) = (name, body) {
                    if child.start_byte() >= name.end_byte() && child.end_byte() <= body.start_byte()
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    let modifier_sequence_refuses = |wanted: &str| -> bool {
        let want: Vec<&str> = wanted.split_whitespace().collect();
        // PASS 137 (137B-F1): non-receipted languages (reachable through the
        // library `match_pattern` entry, which has no answerability gate)
        // keep the ts blanket posture — the pre-137 release behavior.
        if !receipted {
            return !candidate_modifiers.is_empty();
        }
        match lang {
            Language::Java => {
                // PASS 137 (137A-F5, grid137a D1 + javaprobe/pinprobe): the
                // grid-fitted model — the pattern's modifier+annotation
                // tokens match the candidate's combined token list under a
                // greedy scan where KEYWORDS may hop over non-matching
                // candidate KEYWORDS (unanchored subsequence: `public`
                // binds `abstract public`, `static public`,
                // `static final public` — javaprobe n1 each; `public
                // abstract` binds `public static abstract` — pinprobe n1 —
                // and refuses `abstract public` by order) while ANNOTATIONS
                // are POSITIONAL and never skippable: they must sit exactly
                // at the scan cursor (`@SafeVarargs` binds `@SafeVarargs
                // public` but refuses `@Deprecated @SafeVarargs public` —
                // javaprobe; `@Deprecated` binds `@Deprecated public` but
                // refuses `public @Deprecated` — pinprobe). Keyword passage
                // is BLOCKED by a sitting annotation (`public` refuses
                // `@Deprecated public` — pinprobe). An exhausted candidate
                // list refuses (`public` × `private`).
                if want.is_empty() {
                    return !candidate_modifiers.is_empty();
                }
                let is_anno = |token: &str| token.starts_with('@');
                let mut cursor = 0usize;
                for token in &want {
                    if is_anno(token) {
                        match candidate_modifiers.get(cursor) {
                            Some(cand) if *cand == *token => cursor += 1,
                            _ => return true,
                        }
                    } else {
                        loop {
                            match candidate_modifiers.get(cursor) {
                                None => return true,
                                Some(cand) if is_anno(cand) => return true,
                                Some(cand) if *cand == *token => {
                                    cursor += 1;
                                    break;
                                }
                                Some(_) => cursor += 1,
                            }
                        }
                    }
                }
                false
            }
            Language::CSharp =>
            // csharp leaf modifiers must byte-equal the pattern sequence
            // exactly (`public` refuses `public sealed`).
            {
                candidate_modifiers != want
            }
            // TypeScript keeps the PRE-FIX blanket posture byte-for-byte:
            // refuse any candidate carrying modifier children, regardless
            // of the pattern side — the ts `export interface` face binds
            // through declaration_modifiers_match's wrapper-parent arm,
            // never through this scan (the f136 grid's ts_exppat guard
            // flipped when the exact-sequence arm leaked onto ts).
            _ => !candidate_modifiers.is_empty(),
        }
    };
    match pattern_modifiers {
        None => !candidate_modifiers.is_empty(),
        Some(wanted) => modifier_sequence_refuses(wanted),
    }
}

/// The kotlin enum/interface/class head sniffer: the declaration keyword is
/// read from the candidate's modifier wrapper (`modifiers`/`class_modifier`
/// children recurse — kotlin folds the `enum`/`interface` head tokens in
/// there, so a plain first-child byte-read of the declaration head misses
/// them); anything else defaults to `"class"`. The JAVA interface/class
/// modifier-matching doctrine this function was historically mislabeled
/// with (the 137A-F5 subsequence note) lives on
/// [`interface_candidate_refused`]'s Java arm.
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
    // Fieldless grammars (MoonBit apply/dot-apply calls) expose the container
    // as a direct named child whose kind equals the field name.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if fields.contains(&child.kind()) {
            return Some(child);
        }
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
            // PASS 102 (F-101A-1): tree-sitter recovers an error-glued
            // argument (`q(µµµ$A)` in py/go/swift/rust/kotlin) as
            // `identifier µµµ` + an ERROR child carrying the `$A` tail, the
            // same extra-marked recovery the r50 `general_eq` alignment
            // filter models for binary position. sg's Smart strictness
            // skips candidate extras in ARGUMENT position too
            // (match_tree/strictness.rs `match_terminal` → SkipCandidate for
            // a kind-mismatched extra; `should_skip_cand_for_metavar` under
            // a meta goal), so the row presents ONE argument — the
            // identifier fragment the meta binds
            // (`metaVariables.single.A_ = "µµµ"`, m1 oracle). Counting the
            // extra as an arity slot made `q(µA_)` a 2-against-1 mismatch
            // and the row went silently unanswered.
            .filter(|child| !child.is_extra())
            .collect(),
    )
}

/// PASS 102 (F-101A-3): true when the candidate call presents its argument
/// list INSIDE a `(`/`)` token pair. Call-lane patterns always spell the
/// parens ([`classify_native`] requires them) and sg's child alignment
/// matches those anonymous tokens one-for-one — ruby's paren-less command
/// call (`q x`, `q µµµ$A`) carries a bare argument list with no paren
/// tokens, so sg's `(`-vs-argument_list kind mismatch answers [] there
/// (m3 oracle: subject over-answered both paren-less shapes pre-fix). The
/// tokens may live inside the argument container (py/go/rust/swift/kotlin/
/// ruby grammars nest them) or as direct call children (grammars that split
/// them); a container with neither shape is a command call.
fn candidate_call_parenthesized(node: &Node) -> bool {
    let Some(container) = argument_container(node, &["arguments"]) else {
        return false;
    };
    let mut cursor = container.walk();
    let container_children: Vec<Node> = container.children(&mut cursor).collect();
    if container_children.first().is_some_and(|child| child.kind() == "(")
        && container_children.last().is_some_and(|child| child.kind() == ")")
    {
        return true;
    }
    let mut cursor = node.walk();
    let node_children: Vec<Node> = node.children(&mut cursor).collect();
    node_children.iter().any(|child| child.kind() == "(")
        && node_children.iter().any(|child| child.kind() == ")")
}

#[allow(clippy::too_many_arguments)]
fn walk_calls(
    node: Node,
    source: &str,
    pattern: &str,
    path: &[Option<String>],
    arguments: Option<&ArgumentTemplate>,
    arg_slots: Option<&[ArgSlot]>,
    lang: Language,
    out: &mut Vec<PatternMatch>,
) {
    // PASS 75a (F74a-2): the php member-call kinds are the dedicated
    // MemberCall/OptionalCall lanes' candidates — a plain path (dot- or
    // name-spelled) never answers a `->`/`?->` call site (sg connector
    // token-exactness, the registered pass-22/64-7 semantics).
    // MED-1 (84c, r35): when a rest slot owns the argument list, the slots
    // decide the arity — the text-derived Exactly(n) vetoed rest-expanded
    // candidates (3-arg sites) in the pre-filter BEFORE the slots ran.
    let arguments = match arg_slots {
        Some(slots) if slots.iter().any(|slot| matches!(slot, ArgSlot::Rest(_))) => None,
        _ => arguments,
    };
    let callee = if matches!(
        node.kind(),
        "member_call_expression" | "nullsafe_member_call_expression"
    ) {
        None
    } else {
        call_match_path(&node, source, path)
            // PASS 102 (F-101A-3): the pattern's paren tokens must exist on
            // the candidate — a paren-less command call is a different node
            // shape sg never matches with a paren-spelled pattern (m3).
            .filter(|_| candidate_call_parenthesized(&node))
            .filter(|_| arguments_match(&node, arguments, &["arguments"]))
            // PASS 122 (F2, live-grid CORRECTED): swift's member-link trivia
            // rule is PER-CANDIDATE — a callee link whose OWN text carries a
            // comment before a later `navigation_suffix` is link-STRUCTURAL
            // and sg refuses that candidate (`a /*c*/ .b(1)` × `a.b($X)`;
            // oracle grid /tmp/phase122/f2). A trivia-free INNER link of a
            // longer chain still answers (`a.b(1) /*c*/ .c(2)` × `a.b($X)`
            // sg n1) — the earlier ancestor-chain form over-refused those
            // and was removed (f122b inner-link pins). Callee-internal
            // comments keep the PASS 111 transparency contract.
            .filter(|_| {
                !(lang == Language::Swift
                    && call_field_node(&node)
                        .is_some_and(|callee| swift_member_link_structural(&callee)))
            })
            // PASS 124 (F2, f124b): the kt DOTTED call spelling (`a.b($X)`,
            // `$A.b($X)`, `$`-less `a.b(1)`) rides this plain-call lane, which
            // had NO link-structural consult — receiver-link trivia
            // (`a /*c*/ .b(1)`) over-answered where sg refuses. Consult the
            // union doctrine (kt `?.` faces keep their decomposer/optional
            // -lane consults; dotted links need the wrapped rule).
            .filter(|_| {
                !(lang == Language::Kotlin
                    && call_field_node(&node)
                        .is_some_and(|callee| member_link_trivia_structural(&callee)))
            })
            // PASS 144 (143A-F2, grid J*): the cs junction gate — sg refuses
            // a comment/U+2028/U+2029 run in the callee→`(` gap (J1/J2/J7/
            // J8 rc1) while the FEFF/NBSP junctions bind (J3/J4) and
            // comment-inside-args stays outside the junction (J5). The
            // pre-fix plain-call path never consumed the junction consult
            // for csharp.
            .filter(|_| {
                !(lang == Language::CSharp && !cs_call_junction_trivia_free(&node, source))
            })
            // PASS 105 (FB-104A-1): swift's arithmetic-binary rows never
            // present a matchable call (m2 oracle).
            .filter(|_| !swift_compound_callee_call(lang, &node))
    };
    if let Some(callee) = callee {
        match arg_slots {
            None => push_match(&node, source, pattern, Some(&callee.join(".")), out),
            Some(slots) => {
                // PASS 113 (112B-F1): the slots arm answers through
                // `push_match_with_captures` DIRECTLY — `capture_call_path`
                // (and the PASS 111 junction gate inside it) never runs, so
                // consult the same rule here before answering. Mixed
                // rest-slot patterns classify native through
                // `parse_call_arg_slots` (the `classify_native` argument-slot
                // arm) and the
                // arity admits junction-extra candidates (`a?.(1, 2)` under
                // `a($A, $$$B)`; `a?.(1)` / `a /*c*/ (1)` under
                // `a($$$A, $B)` / `a($$$A, $$$B)`; ts `a<number>(...)`
                // twins — oracle grid 2026-09-08, all sg-[]). The arity
                // semantics themselves are untouched: this only vetoes the
                // junction-extra candidates.
                if let Some(nodes) = argument_nodes(&node, &["arguments"])
                    .filter(|_| call_junction_exact(&node))
                {
                    let mut captures = BTreeMap::new();
                    if let Some(text) = node_text(&node, source) {
                        captures.insert("MATCH".to_string(), text.to_string());
                    }
                    if call_arg_slots_match(slots, &nodes, source, &mut captures).is_some() {
                        push_match_with_captures(&node, source, pattern, captures, out);
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls(child, source, pattern, path, arguments, arg_slots, lang, out);
    }
}

/// PASS 105 (FB-104A-1): true when a swift call candidate's CALLEE is a
/// folded compound. This workspace's swift grammar folds `LHS <binop> q`
/// into the compound and hangs the `call_suffix` off the WHOLE call (parse
/// dump: `1 + q(x)` = `call_expression[additive_expression "1 + q",
/// call_suffix "(x)"]`) — there is NO standalone inner call node, and sg
/// 0.45.2 answers [] for clean call patterns on every arithmetic-binary row
/// (m2 oracle: int/float/string LHS x +,-,*,/, INCLUDING the clean rows
/// `1 + q(1)`; 80 fail-open subject cells).
/// PASS 107 (FB-106A-3): the same fold swallows PREFIX-unary compounds —
/// `-q(1)` / `!q(1)` / `&q(1)` parse
/// `call_expression[prefix_expression "-q", call_suffix "(1)"]` (0.7.3
/// tree-dump, pass107), and the r55 scope (additive/multiplicative children
/// only) let those rows over-answer where sg answers [] (m2_swiftprefix, 19
/// cells). The gate keys on the FOLDED SHAPE — a direct compound-callee
/// child kind — never on operator text. `try q(1)` wraps a REAL
/// `call_expression` inside `try_expression` and keeps answering (sg H),
/// equality keeps its nested call child, and a call on the LEFT operand IS
/// the matchable call (`q(x) - 1` parses
/// `additive_expression[call, op, rhs]`) — none fires this gate (m2
/// controls H). Pre-fix the plain-call path resolved the compound callee
/// through `last_identifier_in_chain`, whose first-identifier recursion
/// answers `q` for `1 + q` (leading literal) but `x` for `x + q` (leading
/// identifier) — the exact lit/ident asymmetry in the m2 subject column.
/// A compound callee can never be spelled by a plain identifier path (those
/// faces are general-lane compound alignment), so the candidate is refused
/// outright. Scope is the probed compound kinds only; bitshift/
/// nil-coalescing compounds have no probed contract and keep today's
/// behavior.
fn swift_compound_callee_call(lang: Language, node: &Node) -> bool {
    if lang != Language::Swift || node.kind() != "call_expression" {
        return false;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    children.iter().any(|child| {
        matches!(
            child.kind(),
            "additive_expression"
                | "multiplicative_expression"
                | "prefix_expression"
        )
    })
}

/// PASS 75a (F74a-2): match the php plain `->` member-call spelling against
/// `member_call_expression` candidates only. 86a-L5 (r37): a pattern that
/// SPELLS the nullsafe `?->` connector (`nullsafe: true`) switches the
/// candidates to `nullsafe_member_call_expression` — token-exact in both
/// directions (a plain spelling never answers a nullsafe site, probed sg
/// `$o->m($A)` [] on the `?->` corpus, and vice versa); shapes the carve
/// refuses keep the dedicated optional lane. The candidate callee decomposes
/// through `call_callee`'s member arm into the full object->name chain; the
/// empty synthetic shape vetoes unresolvable receivers like sg's empty
/// answers.
/// PASS 83a (FB-82a-04): when the pattern's argument list mixes literal
/// tokens with metas, the per-position slots decide the argument contract
/// and the captures (the generic pattern-text capture path would mis-bind
/// a meta across a literal position).
#[allow(clippy::too_many_arguments)]
fn walk_member_calls(
    node: Node,
    source: &str,
    pattern: &str,
    path: &[Option<String>],
    arguments: Option<&ArgumentTemplate>,
    arg_slots: Option<&[ArgSlot]>,
    require_continuation: bool,
    nullsafe: bool,
    out: &mut Vec<PatternMatch>,
) {
    // MED-1 (84c, r35): when a rest slot owns the argument list, the slots
    // decide the arity — the text-derived Exactly(n) vetoed rest-expanded
    // candidates (3-arg sites) in the pre-filter BEFORE the slots ran,
    // while the chained spelling (segment.args = Any) answered the same
    // sources.
    let arguments = match arg_slots {
        Some(slots) if slots.iter().any(|slot| matches!(slot, ArgSlot::Rest(_))) => None,
        _ => arguments,
    };
    // MED-1 (84c, r35): a `;`-terminated flat pattern is sg's
    // statement-level spelling — it binds statement-rooted member calls
    // only (probed: `$w->q9(1, $$$A);` answers {2,3,8} on the flat/chain
    // fixture, excluding the chained lines' inner q9 subnodes), while the
    // `;`-less spelling answers embedded faces too ({2,3,5,8}). The same
    // statement discipline the assignment lane registered (FB-82a-03).
    let pattern_is_semi = pattern.trim().ends_with(';');
    // PASS 83a (FB-82a-05): the dangling-arrow repair answers only
    // chain-PREFIX sites — sg's repaired pattern never answers the
    // standalone statement spelling (probed []), so the candidate
    // member-call node must be continued by a member-access/call link.
    let continuation_ok = !require_continuation
        || node.parent().is_some_and(|parent| {
            matches!(
                parent.kind(),
                "member_access_expression"
                    | "member_call_expression"
                    | "nullsafe_member_access_expression"
                    | "nullsafe_member_call_expression"
            )
        });
    let statement_ok = !pattern_is_semi
        || node
            .parent()
            .is_some_and(|parent| parent.kind() == "expression_statement");
    // 86a-L5 (r37): the candidate kind carries the PATTERN's connector
    // spelling — token-exact in both directions.
    let kind_ok = if nullsafe {
        node.kind() == "nullsafe_member_call_expression"
    } else {
        node.kind() == "member_call_expression"
    };
    if kind_ok && continuation_ok && statement_ok && !is_in_comment_or_string(&node) {
        let matched = call_callee(&node, source)
            .filter(|(segments, _)| !segments.is_empty())
            .filter(|(segments, _)| path_matches(segments, path))
            .filter(|_| arguments_match(&node, arguments, &["arguments"]));
        if matched.is_some() {
            match arg_slots {
                None => push_match(&node, source, pattern, None, out),
                Some(slots) => {
                    if let Some(nodes) = argument_nodes(&node, &["arguments"]) {
                        let mut captures = BTreeMap::new();
                        if let Some(text) = node_text(&node, source) {
                            captures.insert("MATCH".to_string(), text.to_string());
                        }
                        // PASS 135 (134A-F5, f135d): the slot faces build
                        // captures HERE (the generic push path never runs for
                        // them), so a callee-path metavariable stayed unbound
                        // where sg binds it per site (`C::$s->$M();` sg n1
                        // M=`m`, grid /tmp/phase135/cells gridE). Bind the
                        // EQUAL-LENGTH callee path segment-wise; a conflict is
                        // sg's unification veto.
                        if bind_slot_face_callee_metas(&node, source, pattern, &mut captures)
                            .is_some()
                            && arg_slots_match(slots, &nodes, source, &mut captures).is_some()
                        {
                            push_match_with_captures(&node, source, pattern, captures, out);
                        }
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_member_calls(
            child,
            source,
            pattern,
            path,
            arguments,
            arg_slots,
            require_continuation,
            nullsafe,
            out,
        );
    }
}

/// PASS 135 (134A-F5, f135d): callee-path metavariable binding for the SLOT
/// faces of the flat member-call lane. The slot arms build their capture map
/// in-walk (the generic `push_match` → `captures_for_node` path never runs
/// for them), so `C::$s->$M();` matched both sites with `M` unbound where sg
/// binds the member-name meta per site (grid /tmp/phase135/cells gridE:
/// `C::$s->$M();` sg n1 M=`m`; `self::$s->$M();` likewise). Literal segments
/// already byte-matched through `path_matches`, so only meta segments bind —
/// the same unification [`capture_call_path`] applies to the non-slot faces.
/// PASS 137 (137B-F2): the nullsafe `?->` spelling now joins the PASS 73
/// normalization FIRST (mirroring the classifier's :3291 order) — a
/// nullsafe slot face with a META receiver (`$O?->$M($A);` sg n1, O=`$a`,
/// M=`b`, A=`1`) used to normalize `?` INTO the receiver segment
/// (`$O?.$M` → segment `$O?`), whose capture name is None, silently skipping
/// the receiver binding.
/// PASS 137 (137B-F3): the length-mismatch arm is sg-EXACT now, not
/// "emit-with-no-binding": `path_matches` admits longer candidate chains
/// when the pattern's FIRST segment is a meta (the pass-22 absorption
/// contract), and sg binds that meta to the absorbed head
/// (`$A->b($X);` × `$x->y->b(1);` sg n1 A=`$x->y` — grid137 E-lane,
/// including the 3-deep `$x->y->z` head). Bind the head the same way
/// [`capture_call_path`]'s absorption arm does, then align the tails
/// segment-wise; a `bind_capture` conflict vetoes. Length mismatch WITHOUT
/// a leading meta never reaches here by contract (`path_matches` refuses
/// it); the arm keeps the no-binding return for that unreachable shape.
fn bind_slot_face_callee_metas(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    let open = pattern.find('(')?;
    let callee = pattern[..open].trim();
    // PASS 73 (F72a-1) normalization: `::` and `->` both spell one dotted
    // segment chain, mirroring `parse_call_path`'s decomposition. The
    // nullsafe `?->` is deliberately NOT normalized (137B-F2 refuted by
    // oracle probe 2026-09-11: `$O->c1($U)` × `$o?->c1($u);` is sg rc1 []
    // AND `$O?->c1($U)` × `$o->c1($u);` is sg rc1 [] — both directions
    // token-exact, the registered PASS 79/FB-80a-01 doctrine); a `?` here
    // would glue onto the receiver segment and refuse, which IS sg's
    // behavior for the cross spellings.
    let normalized = callee.replace("::", ".").replace("->", ".");
    if normalized.is_empty() {
        return Some(());
    }
    let pattern_segments: Vec<&str> = normalized.split('.').collect();
    let (actual, _) = call_callee(node, source)?;
    if actual.len() != pattern_segments.len() {
        // PASS 137 (137B-F3): leading-meta absorption — bind the pattern's
        // first segment to the whole candidate head, then align the tails.
        // This is the php flat member-call slot lane, so the head text
        // carries the `->` connector sg's capture reports
        // (`$A->b($X);` × `$x->y->b(1);` → A=`$x->y`, grid137 E-lane; the
        // dot-joined `capture_call_path` arm serves the DOT-connector
        // grammars where the dedicated php lane is never consulted).
        if actual.len() > pattern_segments.len()
            && capture_name(pattern_segments[0]).is_some()
        {
            let head_len = actual.len() - (pattern_segments.len() - 1);
            let head = actual[..head_len].join("->");
            let mut absorption = captures.clone();
            if bind_capture(&mut absorption, capture_name(pattern_segments[0])?, &head).is_none() {
                return None;
            }
            for (want, have) in pattern_segments[1..].iter().zip(actual[head_len..].iter()) {
                if let Some(variable) = capture_name(want) {
                    if bind_capture(&mut absorption, variable, have).is_none() {
                        return None;
                    }
                }
            }
            *captures = absorption;
        }
        return Some(());
    }
    for (want, have) in pattern_segments.iter().zip(actual.iter()) {
        if let Some(variable) = capture_name(want) {
            bind_capture(captures, variable, have)?;
        }
    }
    Some(())
}

/// PASS 77b (F76-1): match php `->` member-call CHAIN templates against
/// php member-call candidates (the [`NativeKind::MemberCallChain`]
/// lane). Every chain node in the tree is visited, so a pattern answers the
/// outermost chain whose per-segment shape matches exactly AND the inner
/// prefix subnodes whose own depth equals the pattern's — the same
/// prefix-subnode contract the pass-75 flat lane already shows on chains
/// (`$obj->m($A)` answers the head of `$obj->m($w)->n($u)`).
/// PASS 79 (F78-2): both member-call spellings are candidates — the
/// candidate node kind carries the LAST link's connector and the
/// decomposition carries every link's, so the per-link flag comparison
/// keeps `->` and `?->` token-exact in both directions.
/// PASS 81a (FB-80a-01): when the PATTERN ends in a property link, the
/// member-access spellings are candidates too (sg answers the property-tail
/// node; a call-tail pattern still never matches one because the exact-depth
/// unification vetoes the args mismatch).
fn walk_member_call_chains(
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    nullsafe_flags: &[bool],
    out: &mut Vec<PatternMatch>,
) {
    let pattern_ends_property =
        segments.len() >= 2 && segments.last().is_some_and(|segment| segment.args.is_none());
    // PASS 94b (FB-93A-5): a `;`-terminated FLAT PROPERTY pattern is sg's
    // statement-rooted spelling — it answers the property-access statement,
    // never an embedded access node (`$this->$P;` answers the
    // `$this->prop;` statement but not the `$this->prop = 1;` assignment's
    // inner `$this->prop` — probed matrix M {4} not {9}). Registered chain
    // faces keep the embedded answering contract: property-TAIL faces with
    // a call segment (FB-80a-01) and every semi-less spelling.
    let pattern_is_semi = pattern.trim().ends_with(';');
    let flat_property = segments.len() == 2 && segments.iter().all(|s| s.args.is_none());
    let statement_ok = !(pattern_is_semi && flat_property)
        || node
            .parent()
            .is_some_and(|parent| parent.kind() == "expression_statement");
    let candidate_kind = node.kind();
    let is_candidate = if pattern_ends_property {
        matches!(
            candidate_kind,
            "member_access_expression"
                | "nullsafe_member_access_expression"
                | "member_call_expression"
                | "nullsafe_member_call_expression"
        )
    } else {
        matches!(
            candidate_kind,
            "member_call_expression" | "nullsafe_member_call_expression"
        )
    };
    if is_candidate && statement_ok && !is_in_comment_or_string(&node) {
        if let Some(captures) = member_chain_matches(&node, source, segments, nullsafe_flags) {
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
        walk_member_call_chains(child, source, pattern, segments, nullsafe_flags, out);
    }
}

/// PASS 81a (FB-80a-03); PASS 83a (FB-82a-02, -03, -06): match the widened
/// php operand lane. Candidates are the operator class's node kind
/// (`assignment_expression` / `augmented_assignment_expression` /
/// `binary_expression`) whose LEFT text equals the pattern's literal target
/// byte-exactly and whose operator token equals the pattern's (binary
/// candidates of every operator share one node kind). The RHS binds either
/// through the bare-meta capture path or the structural expression matcher
/// (`php_rhs_expr_matches`). Statement discipline: a `;`-terminated pattern
/// binds ONLY candidates rooted at an `expression_statement` (sg's
/// `;`-rooted answer node — condition/argument embedded assignments stay
/// silent), and the match span is that statement INCLUDING the `;` (sg -U
/// consumes it); a `;`-less pattern also answers embedded nodes and keeps
/// the assignment-node span (`;` preserved — the registered byte-parity
/// control).
#[allow(clippy::too_many_arguments)]
fn walk_php_assignments(
    node: Node,
    source: &str,
    pattern: &str,
    target: &str,
    op: &str,
    op_class: PhpBinaryClass,
    rhs_root: Option<(Node, &str)>,
    lhs_root: Option<(Node, &str)>,
    value: &str,
    value_multi: bool,
    head_comments: &[String],
    out: &mut Vec<PatternMatch>,
) {
    let pattern_is_semi = pattern.trim().ends_with(';');
    let candidate_kind = match op_class {
        PhpBinaryClass::Assign => "assignment_expression",
        PhpBinaryClass::Augmented => "augmented_assignment_expression",
        PhpBinaryClass::Binary => "binary_expression",
    };
    if node.kind() == candidate_kind && !is_in_comment_or_string(&node) {
        // FB-84a-03 (r34): sg aligns the matched operator node's own
        // children strictly — a comment sitting directly inside the
        // assignment/augmented/binary candidate (`$a = /* h */ $v + 1;`
        // probed sg [] for the comment-FREE pattern) breaks the alignment
        // and the face is refused. Comments BETWEEN the RHS operands sit
        // inside the RHS sub-expression, where the structural matcher stays
        // transparent.
        // F-r40-3 (r41, 90A-F5): when the PATTERN ITSELF carries RHS-head
        // block comments, sg's CST aligns them positionally at the operator
        // node — the face ANSWERS exactly when the candidate's direct
        // comment children equal the pattern's sequence byte-for-byte
        // (probed `$a = /* h */ $V + 1;` {3}, `$a = /* nope */ $V + 1;` []
        // ATTACHED 2026-09-08). The comment-free contract is unchanged.
        // FB-93A-4d (r44): a BARE-META RHS face swallows the candidate's
        // direct comment children — sg 0.45.2 answers `$a = $V;` on every
        // `$a`-targeted line including the head-comment spellings
        // (probes_run1.jsonl matrix C: {2,3,4,8,9,10}, probed 2026-09-08
        // ATTACHED), because the pattern's metavariable child consumes the
        // next child regardless of comment nodes. Expression-RHS faces keep
        // the refusal (the pattern's structural child must align with the
        // candidate's: `$a = $V + 1;` stays {10}).
        let head_comment_ok = if !head_comments.is_empty() {
            let mut child_cursor = node.walk();
            let cand_comments: Vec<String> = node
                .children(&mut child_cursor)
                .filter(|child| child.kind().contains("comment"))
                .filter_map(|child| node_text(&child, source).map(str::to_string))
                .collect();
            cand_comments == head_comments
        } else if rhs_root.is_none() {
            true
        } else {
            let mut child_cursor = node.walk();
            let any_comment = node
                .children(&mut child_cursor)
                .any(|child| child.kind().contains("comment"));
            !any_comment
        };
        let op_token_ok = head_comment_ok
            && match op_class {
            PhpBinaryClass::Assign => true,
            PhpBinaryClass::Augmented | PhpBinaryClass::Binary => {
                // The operator is an anonymous child token; text-equal.
                let mut cursor = node.walk();
                let mut children = node.children(&mut cursor);
                children.any(|child| {
                    !child.is_named()
                        && node_text(&child, source).is_some_and(|text| text == op)
                })
            }
        };
        // FB-82a-03: `;`-terminated patterns root at expression statements.
        let statement = node
            .parent()
            .filter(|parent| parent.kind() == "expression_statement");
        if op_token_ok && (!pattern_is_semi || statement.is_some()) {
            if let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                // PASS 127 (125A-F5, f127a): a meta target (`$X = $Y`,
                // `$o->$A = $Y`) binds through the RHS expression machinery
                // — the `variable_name` pattern arm binds the candidate LHS
                // text, and a meta LINK binds the candidate link node text
                // (`a` literal / `$a` dynamic) — instead of the verbatim
                // text equality the literal targets keep.
                let mut captures = BTreeMap::new();
                // PASS 135 (134A-F5/F6, f135d; REVISED f135f, grid
                // /tmp/phase135/iso2/f.php): a dynamic-class static head
                // (`$C::$s` / `$$C::$s`, `=`-only) binds the WHOLE candidate
                // scope text — sg answers variable heads dollar-included
                // (row 5, C=`$c`) AND concrete identifier/qualified heads
                // (rows 2/4 C=`C`, row 3 C=`Foo\Bar`). The former
                // `starts_with('$')` restriction answered variable heads
                // only (gridE's variable-headed fixture read as "literal
                // class unprobed"); the fresh grid refutes that premise. The
                // admitted scope kinds are the probed ones (name,
                // qualified_name, variable_name) — anything else
                // (call/member scopes) stays fail-closed. The prop is
                // literal text equality (row 6 `$name` never answers a `$s`
                // pattern).
                let target_ok = if let Some(meta_name) = php_static_target_dynamic_head(target) {
                    let scope = left.child_by_field_name("scope");
                    let prop = left.child_by_field_name("name");
                    let scope_ok = left.kind() == "scoped_property_access_expression"
                        && scope.is_some_and(|scope| {
                            matches!(scope.kind(), "name" | "qualified_name" | "variable_name")
                        });
                    let prop_ok = prop
                        .and_then(|prop| node_text(&prop, source))
                        .is_some_and(|have| {
                            have == target
                                .split_once("::")
                                .map(|(_, want)| want.trim())
                                .unwrap_or_default()
                        });
                    scope_ok
                        && prop_ok
                        && scope
                            .and_then(|scope| node_text(&scope, source))
                            .is_some_and(|text| {
                                bind_capture(&mut captures, meta_name, text).is_some()
                            })
                } else if let Some((lhs_pat, lhs_doc)) = lhs_root {
                    let lhs_expr = unwrap_pattern_expression(lhs_pat);
                    php_rhs_expr_matches(lhs_expr, left, source, lhs_doc, &mut captures)
                } else {
                    node_text(&left, source).map(str::to_string).as_deref() == Some(target)
                };
                if target_ok {
                    // FB-82a-06: the `;`-terminated pattern's match node is
                    // the `;`-rooted statement (span consumes the `;`).
                    let span_node = if pattern_is_semi {
                        statement.unwrap_or(node)
                    } else {
                        node
                    };
                    let rhs_bound = if let Some((rhs_pattern, rhs_source)) = rhs_root {
                        // General expression RHS: structural match with meta
                        // binding (rhs_expr faces never carry the bare-meta
                        // binding). The pattern root is unwrapped to the
                        // expression node and text-compared against its own
                        // pattern source.
                        let rhs_expr_node = unwrap_pattern_expression(rhs_pattern);
                        php_rhs_expr_matches(
                            rhs_expr_node,
                            right,
                            source,
                            rhs_source,
                            &mut captures,
                        )
                    } else if let Some(right_text) = node_text(&right, source).map(str::to_string)
                    {
                        bind_capture_kind(&mut captures, value, &right_text, value_multi).is_some()
                    } else {
                        false
                    };
                    if rhs_bound {
                        if let Some(text) = node_text(&span_node, source) {
                            captures.insert("MATCH".to_string(), text.to_string());
                        }
                        let (line_start, line_end) = node_lines(&span_node, source);
                        out.push(PatternMatch {
                            line_start,
                            line_end,
                            byte_start: span_node.start_byte(),
                            byte_end: span_node.end_byte(),
                            excerpt: excerpt_for_node(&span_node, source, pattern),
                            captures,
                        });
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_assignments(
            child,
            source,
            pattern,
            target,
            op,
            op_class,
            rhs_root,
            lhs_root,
            value,
            value_multi,
            head_comments,
            out,
        );
    }
}

/// 88a-M2 (r39): walk a classified bare php binary-expression META template
/// ([`classify_php_operand_template`]). Candidates are nodes of the
/// pattern's OWN comparison root kind — the binary/paren expression
/// (no-semi: sg binds at expression level, nested binaries included) or
/// the wrapping `expression_statement` (semi: sg's statement-rooted `;`
/// discipline, so only bare `expr;` statements answer) — unified through
/// [`php_rhs_expr_matches`] (the r33 expression machinery: metavariable
/// binds with the same-name veto, text-exact leaves/tokens, transparent
/// candidate comments, text-exact pattern comments).
fn walk_php_operand_template(
    tree: &tree_sitter::Tree,
    source: &str,
    pattern: &str,
    tpl: &PhpOperandTemplate,
    out: &mut Vec<PatternMatch>,
) {
    let Some(pat_root) = tpl.comparison_root() else {
        return;
    };
    let doc = tpl.parsed.doc.as_str();
    // P7-style hash-set dedup: nested self-similar binaries can emit the
    // same node twice through distinct binds; insertion order is preserved.
    let mut seen = std::collections::HashSet::new();
    walk_php_operand_node(
        tree.root_node(),
        source,
        pattern,
        pat_root,
        doc,
        &mut seen,
        out,
    );
}

fn walk_php_operand_node<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    pat_root: Node<'_>,
    doc: &str,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == pat_root.kind() && !is_in_comment_or_string(&node) {
        let mut captures = BTreeMap::new();
        if php_rhs_expr_matches(pat_root, node, source, doc, &mut captures) {
            let byte_start = node.start_byte();
            let byte_end = node.end_byte();
            if seen.insert((byte_start, byte_end)) {
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
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_operand_node(child, source, pattern, pat_root, doc, seen, out);
    }
}
/// PASS 83a (FB-82a-02): structural match of a general RHS expression
/// pattern against the candidate's right-hand node. A `variable_name`
/// pattern leaf is either a canonical meta (`$V`/`$$V` single namespace,
/// `$$$V` multi — binds the candidate node text, same-name veto via
/// `bind_capture_kind`) or literal variable text (byte-exact). A `$$V`
/// dynamic-variable pattern token wildcards plain AND dynamic candidate
/// variables (sg answers `$alpha = $$V + 1;` on both `$v + 1` and
/// `$$v + 1`). Any other node requires the SAME kind with aligned
/// children: childless leaves (integers, names, string content) compare
/// text-exactly, anonymous tokens (operators, punctuation) compare
/// text-exactly, named children recurse pairwise. `pat`'s text is read
/// from `pat_source` (pattern node offsets index the pattern text, not
/// the candidate source). Mirrors sg's pattern-node matching for these
/// expression shapes.
fn php_rhs_expr_matches(
    pat: Node,
    cand: Node,
    source: &str,
    pat_source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    if pat.kind() == "variable_name" {
        if let Some(text) = node_text(&pat, pat_source) {
            if let Some(name) = capture_name(text) {
                let multi = text.starts_with("$$$");
                if let Some(cand_text) = node_text(&cand, source) {
                    return bind_capture_kind(captures, name, &cand_text, multi).is_some();
                }
                return false;
            }
        }
        return match (node_text(&pat, pat_source), node_text(&cand, source)) {
            (Some(want), Some(have)) => want == have,
            _ => false,
        };
    }
    // A `$$V` dynamic-variable pattern token is a canonical wildcard over
    // BOTH variable spellings (plain and dynamic); candidate nodes of any
    // other kind refuse (fail-closed — only the probed faces admit).
    if pat.kind() == "dynamic_variable_name" {
        if let Some(text) = node_text(&pat, pat_source) {
            if let Some(name) = text.strip_prefix("$$").filter(|n| is_metavar_name(n)) {
                if matches!(
                    cand.kind(),
                    "variable_name" | "dynamic_variable_name"
                ) {
                    if let Some(cand_text) = node_text(&cand, source) {
                        return bind_capture_kind(captures, name, &cand_text, false).is_some();
                    }
                }
                return false;
            }
        }
    }
    // FB-84a-04 (r34): an argument list carrying a `$$$NAME` rest slot is
    // sg's rest-metavariable contract — bind the remaining arguments'
    // source bytes (whole-list incl. empty, leading, mid, or trailing)
    // instead of the strict child zip, which cannot decompose the raw
    // `$$$` text (probed sg: `$b = f($v, $$$A);` answers
    // f($v,1)/f($v,1,2)/f($v,$u)/f($v,1,2,3) and refuses the zero-argument
    // trailing face; `$b = f($$$A);` answers every arity incl. empty).
    // Lists without a rest keep the strict structural path byte-compatible.
    if pat.kind() == "arguments" {
        if let Some(text) = node_text(&pat, pat_source) {
            if text.contains("$$$") {
                let (Some(open), Some(close)) = (text.find('('), text.rfind(')')) else {
                    return false;
                };
                if close <= open {
                    return false;
                }
                let inner = &text[open + 1..close];
                let Some(slots) = parse_rhs_arg_slots(inner) else {
                    return false;
                };
                if cand.kind() != "arguments" {
                    return false;
                }
                let mut cand_cursor = cand.walk();
                let nodes: Vec<_> = cand
                    .named_children(&mut cand_cursor)
                    .filter(|child| !is_trivia_kind(child.kind()))
                    .collect();
                return arg_slots_match(&slots, &nodes, source, captures).is_some();
            }
        }
    }
    // Childless leaves are literal tokens — byte-exact text compare (the
    // zero-children recursion below would otherwise admit ANY same-kind
    // leaf, e.g. `1` matching `2`).
    if pat.child_count() == 0 {
        return match (node_text(&pat, pat_source), node_text(&cand, source)) {
            (Some(want), Some(have)) => want == have,
            _ => false,
        };
    }
    if pat.kind() != cand.kind() {
        return false;
    }
    // FB-84a-03 (r34): sg aligns expression children with CANDIDATE comment
    // children transparent (probed: `$a = $V + 1;` answers
    // `$a = $v /* = */ + 1;` and the paren spellings), while a PATTERN-side
    // comment must find its text-exact counterpart (probed: the commented
    // paren pattern answers only the self line — a different comment text
    // does not match). Walk the pattern children against the candidate
    // stream with that asymmetry.
    let pat_children: Vec<_> = {
        let mut cursor = pat.walk();
        pat.children(&mut cursor).collect()
    };
    let cand_children: Vec<_> = {
        let mut cursor = cand.walk();
        cand.children(&mut cursor).collect()
    };
    let mut cand_index = 0usize;
    for pat_child in &pat_children {
        if pat_child.kind().contains("comment") {
            let Some(want) = node_text(pat_child, pat_source) else {
                return false;
            };
            let mut matched = false;
            while cand_index < cand_children.len() {
                let cand_child = &cand_children[cand_index];
                cand_index += 1;
                if cand_child.kind().contains("comment")
                    && node_text(cand_child, source).is_some_and(|text| text == want)
                {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return false;
            }
            continue;
        }
        while cand_index < cand_children.len() && cand_children[cand_index].kind().contains("comment")
        {
            cand_index += 1;
        }
        let Some(cand_child) = cand_children.get(cand_index) else {
            return false;
        };
        cand_index += 1;
        if pat_child.is_named() {
            if !cand_child.is_named()
                || !php_rhs_expr_matches(*pat_child, *cand_child, source, pat_source, captures)
            {
                return false;
            }
        } else if cand_child.is_named() {
            return false;
        } else {
            match (node_text(pat_child, pat_source), node_text(cand_child, source)) {
                (Some(want), Some(have)) if want == have => {}
                _ => return false,
            }
        }
    }
    // Trailing candidate comments are transparent; any other leftover
    // candidate child breaks the alignment.
    while cand_index < cand_children.len() && cand_children[cand_index].kind().contains("comment") {
        cand_index += 1;
    }
    cand_index == cand_children.len()
}

/// PASS 83a (FB-82a-02): the parsed RHS pattern's root sits under `program`
/// (and possibly `expression_statement` — its trailing `;` is an anonymous
/// child) wrappers — descend through single-NAMED-child wrappers to the
/// expression node the structural matcher compares against the candidate.
/// A multi-statement pattern stops the descent and refuses (kind mismatch —
/// fail-closed).
fn unwrap_pattern_expression(mut node: Node) -> Node {
    while matches!(node.kind(), "program" | "expression_statement") {
        let mut cursor = node.walk();
        let named: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|child| child.kind() != "php_tag")
            .collect();
        match named.as_slice() {
            [only] => node = *only,
            _ => break,
        }
    }
    node
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
/// separate candidates the walk visits, never an absorb here), every link's
/// nullsafe flag must equal the pattern's (PASS 79 F78-2 token exactness),
/// every segment name unifies through `bind_capture` (the same-name veto),
/// a pattern property segment must land on a property link and every
/// pattern call segment on a REAL call link with matching arity (a property
/// access in the receiver chain is not a call: `$a->b->c($u)` has no `b()`
/// link, so `$a->b()->c($A)` must not answer it, and `$a->b()->c($A)` has
/// no `->b` property link, so `$a->b->c($A)` must not answer it either),
/// and argument metavars bind like the flat lane (`$A`/`$$A` → single
/// namespace key `A`; `$$$A` → multi). A receiver (argument-free) pattern
/// segment carries no argument contract, mirroring [`chain_matches`].
fn member_chain_matches(
    node: &Node,
    source: &str,
    segments: &[CallChainSegment],
    nullsafe_flags: &[bool],
) -> Option<BTreeMap<String, String>> {
    let mut actual = Vec::new();
    let mut actual_flags = Vec::new();
    member_chain_segments(node, source, &mut actual, &mut actual_flags)?;
    if actual.len() != segments.len() {
        return None;
    }
    let mut captures = BTreeMap::new();
    for (index, (segment, candidate)) in segments.iter().zip(actual.iter()).enumerate() {
        if actual_flags[index] != nullsafe_flags[index] {
            return None;
        }
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want != &candidate.text => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, &candidate.text)?,
            (None, None) => return None,
        }
        if segment.args.is_none() && index > 0 && candidate.args.is_some() {
            // A pattern property link never folds onto a candidate call
            // position (F78-1 cross probe: sg keeps the faces disjoint).
            return None;
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
            // PASS 81a (FB-80a-05): a `$A, $B` list binds every name to its
            // positional candidate argument — the flat lane's
            // `capture_arguments` contract, without which the codemod
            // rewrite falsely bails "unbound metavariable" on names sg
            // substitutes (arity above already pinned the equal lengths).
            // PASS 83a (FB-82a-04): mixed literal/meta lists match through
            // per-position slots instead (literals byte-exact, metas bind,
            // rest binds the remaining source bytes).
            if let Some(slots) = &segment.arg_slots {
                arg_slots_match(slots, nodes, source, &mut captures)?;
            } else if let Some(names) = &segment.arg_metas {
                for (name, argument) in names.iter().zip(nodes.iter()) {
                    if let Some(text) = node_text(argument, source) {
                        bind_capture(&mut captures, name, text)?;
                    }
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
/// `scope::name` texts (the call keeping its argument list). The parallel
/// `flags` vector records, per pushed segment, whether the connector INTO it
/// is the nullsafe `?->` (PASS 79 F78-2: `nullsafe_member_call_expression`
/// and `nullsafe_member_access_expression` carry the named connector kind;
/// plain links push false). Unresolvable receivers (exotic nodes) veto the
/// whole decomposition — sg keeps such faces empty, never an over-match.
fn member_chain_segments<'a>(
    node: &Node<'a>,
    source: &'a str,
    segs: &mut Vec<MemberChainSegment<'a>>,
    flags: &mut Vec<bool>,
) -> Option<()> {
    match node.kind() {
        kind @ ("member_call_expression" | "nullsafe_member_call_expression") => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs, flags)?;
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(&text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            flags.push(kind == "nullsafe_member_call_expression");
            Some(())
        }
        kind @ ("member_access_expression" | "nullsafe_member_access_expression") => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            member_chain_segments(&object, source, segs, flags)?;
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args: None,
                args_content: None,
            });
            flags.push(kind == "nullsafe_member_access_expression");
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
            flags.push(false);
            let args = argument_nodes(node, &["arguments"]);
            let args_content = argument_container(node, &["arguments"])
                .and_then(|container| node_text(&container, source))
                .map(|text| strip_container(&text).to_string());
            segs.push(MemberChainSegment {
                text: node_text(&name, source)?.to_string(),
                args,
                args_content,
            });
            flags.push(false);
            Some(())
        }
        _ => {
            // PASS 83a (FB-82a-01): a dynamic-variable link node (`->$$dyn`)
            // pushes its FULL text including the dollars — the classifier's
            // property literal (`$$dyn`) compares it byte-exactly.
            if is_ident_kind(node.kind())
                || KEYWORD_RECEIVER_KINDS.contains(&node.kind())
                || node.kind() == "variable_name"
                || node.kind() == "dynamic_variable_name"
            {
                segs.push(MemberChainSegment {
                    text: node_text(node, source)?.to_string(),
                    args: None,
                    args_content: None,
                });
                flags.push(false);
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
    lang: Language,
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
        lang,
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
            lang,
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
    lang: Language,
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
    // PASS 113 (112A-F2): the optional lanes answer through this function and
    // never reach `capture_call_path` (the `simple_call` guard excludes
    // `?.`-spelled patterns from its PASS 111 gate), so the SAME junction rule
    // must be consulted here: a comment extra between the callee and the
    // argument list (`a?.b /*c*/ (1)`) breaks sg's exact-children match for
    // `?.`-spelled patterns exactly as for plain ones — oracle grid 2026-09-08
    // (js+ts: `a?.b($X)`, `$A?.b($X)`, `a?.b($A)`, and `a?.b($$$A)` at both
    // arities all answer []).
    if !call_junction_exact(node) {
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
    let (segments, connectors) = optional_chain_decompose(lang, &field)?;
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
fn optional_chain_decompose<'a>(
    lang: Language,
    node: &Node<'a>,
) -> Option<(Vec<Node<'a>>, Vec<bool>)> {
    if !is_member_expr_kind(node.kind()) {
        return Some((vec![*node], Vec::new()));
    }
    // PASS 120 (119B-F1): kotlin's flat member links carry comment children
    // (`member_link_parts` skips them to fill its slots), so the 2-link
    // lane answered link-STRUCTURAL trivia faces sg 0.45.2 refuses. Apply
    // the ONE position-scoped doctrine the kt general arm already uses —
    // the same predicate `chain_candidate_exact` consults — so both kt
    // optional lanes hold it too (js/ts keep their PASS 113 transparency
    // contracts; swift never decomposes here).
    if kt_link_structural(lang, node) {
        return None;
    }
    let link = member_link_parts(lang, node)?;
    if !is_ident_kind(link.leaf.kind()) && !KEYWORD_RECEIVER_KINDS.contains(&link.leaf.kind()) {
        return None;
    }
    let (mut segments, mut connectors) = optional_chain_decompose(lang, &link.base)?;
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
    lang: Language,
    node: &Node<'a>,
) -> Option<(Vec<Node<'a>>, Vec<bool>, Vec<usize>)> {
    if is_call_kind(node.kind()) {
        let field = call_field_node(node)?;
        let mut decomp = optional_call_chain_decompose(lang, &field)?;
        if let Some(end) = decomp.2.last_mut() {
            *end = node.end_byte();
        }
        return Some(decomp);
    }
    if !is_member_expr_kind(node.kind()) {
        return Some((vec![*node], Vec::new(), vec![node.end_byte()]));
    }
    // PASS 120 (119B-F1): same kotlin candidate-link veto as
    // [`optional_chain_decompose`] — the all-call lane admitted by 118-H2
    // decomposes trivia children away and answered the trivia×all-call
    // intersection (`a?.b(1) /*c*/ ?.c(2)`, `a /*c*/ ?.b(1)?.c(2)`, 2-link
    // receiver twin) where sg refuses. Callee-internal trivia (only named
    // siblings after it) keeps answering — the position scope is the ONE
    // registered sg rule (CNR §39.13).
    if kt_link_structural(lang, node) {
        return None;
    }
    let link = member_link_parts(lang, node)?;
    if !is_ident_kind(link.leaf.kind()) && !KEYWORD_RECEIVER_KINDS.contains(&link.leaf.kind()) {
        return None;
    }
    let (mut segments, mut connectors, mut ends) = optional_call_chain_decompose(lang, &link.base)?;
    segments.push(link.leaf);
    connectors.push(link.optional);
    ends.push(node.end_byte());
    Some((segments, connectors, ends))
}

/// PASS 117 (116A-F2): enforce ONE call segment's argument contract.
/// Mixed-rest slot lists ride the registered plain-call rest-slot semantics
/// (`call_arg_slots_match`, PASS 105: trailing rest + single ⇒ n ≥ 2, k ≥ 2
/// rests ⇒ n ≥ k-1, non-trailing rest ⇒ n == 1 — first-hand sg grid
/// 2026-09-08 shows the `?.`-chain faces answer at exactly those arities).
/// Plain templates keep `arguments_match` + the args-capture bind,
/// byte-identical.
fn chain_segment_args_contract(
    node: &Node,
    segment: &CallChainSegment,
    source: &str,
    captures: &mut BTreeMap<String, String>,
) -> Option<()> {
    if let Some(slots) = &segment.arg_slots {
        let nodes = argument_nodes(node, &["arguments"])?;
        return call_arg_slots_match(slots, &nodes, source, captures);
    }
    if !arguments_match(node, segment.args.as_ref(), &["arguments"]) {
        return None;
    }
    if let Some((name, multi)) = &segment.args_capture {
        if let Some(text) = capture_arguments_text(node, source) {
            bind_capture_kind(captures, name, strip_container(&text), *multi)?;
        }
    }
    Some(())
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
    lang: Language,
    node: &Node,
    source: &str,
    _pattern: &str,
    segments: &[CallChainSegment],
    optional_flags: &[bool],
) -> Option<BTreeMap<String, String>> {
    if !is_call_kind(node.kind()) || is_in_comment_or_string(node) {
        return None;
    }
    // PASS 113 (112A-F2): the N-segment optional chain lane answers here and
    // never reaches `capture_call_path`'s PASS 111 gate — consult the SAME
    // junction rule (see the twin hunk in `optional_call_matches`): a comment
    // extra in the callee→arguments gap refuses the candidate for
    // `?.`-spelled patterns too, while trivia INSIDE the member-chain callee
    // (`a?.b /*c*/ .c(1)`) sits outside the gap and keeps answering.
    if !call_junction_exact(node) {
        return None;
    }
    let field = call_field_node(node)?;
    let (segs, connectors, ends) = optional_call_chain_decompose(lang, &field)?;
    let head = &segments[0];
    let links = &segments[1..];
    if segs.len() < links.len() + 1 || connectors.len() + 1 != segs.len() {
        return None;
    }
    // Alignment: the pattern's segments pair with the candidate's LAST
    // segments; the head folds everything before the first aligned segment.
    let absorb = segs.len() - (links.len() + 1);
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
    for (segment, seg_node) in links.iter().zip(segs[absorb + 1..].iter()) {
        let text = node_text(seg_node, source)?;
        match (&segment.literal, &segment.capture) {
            (Some(want), _) if want != text => return None,
            (Some(_), _) => {}
            (None, Some(name)) => bind_capture(&mut captures, name, text)?,
            (None, None) => return None,
        }
    }
    // Alignment of segment SHAPES + per-position argument contracts: every
    // non-terminal link consumes one receiver hop, landing on the candidate
    // expression that ends at that link. A pattern CALL segment requires a
    // call there (arity/args-capture checked on it); a pattern PROPERTY
    // segment (PASS 115, 114A-F1) requires a PLAIN member link — a call in
    // the candidate breaks sg's exact-children match exactly as a comment
    // does (`a?.b(1)?.c(2)` never answers `a?.b?.c($X)`).
    // Arguments: the candidate node carries the LAST link's list; every
    // earlier CALL link consumes one receiver link along the walk.
    let last = links.last()?;
    let segment_is_call = |segment: &CallChainSegment| {
        segment.args.is_some() || segment.args_capture.is_some() || segment.arg_slots.is_some()
    };
    // ONE hop inward: a call node hops its callee's object; a member node
    // (the landing shape of a PROPERTY link) hops its own object. Mixed
    // call/property chains alternate the two (PASS 115).
    let mut receiver = *node;
    for segment in links[..links.len() - 1].iter().rev() {
        receiver = if is_call_kind(receiver.kind()) {
            chain_receiver(&receiver)?
        } else {
            member_receiver(&receiver)?
        };
        if segment_is_call(segment) {
            if !is_call_kind(receiver.kind()) {
                return None;
            }
            // PASS 117 (116B-F2/CCB-2): EVERY consumed call level obeys the
            // ONE junction rule — a comment extra in a mid-link callee→
            // arguments gap refuses the candidate (`a?.b?.c($X)?.d($Y)` never
            // answers `a?.b?.c /*c*/ (1)?.d(2)`, js+ts+kt first-hand sg grid
            // 2026-09-08; the terminal consult at the top covered only the
            // outer call).
            if !call_junction_exact(&receiver) {
                return None;
            }
            chain_segment_args_contract(&receiver, segment, source, &mut captures)?;
        } else if is_call_kind(receiver.kind()) {
            return None;
        }
    }
    // PASS 69a (F-68c-1): a CALL head carries its own argument contract —
    // reachable as one more receiver link when nothing was absorbed (the
    // head segment IS the chain root then). Without this block the head's
    // arity/args-capture template was never consulted (`fetch(1)?…`
    // over-answered the arity-mismatched faces). PASS 117: the head call is
    // a consumed call level too — same junction consult, same slot-aware
    // contract (`a(0)?.b?.c($X)` refuses `a /*c*/ (0)?.b?.c(1)`, sg []).
    if absorb == 0 && segment_is_call(head) {
        receiver = if is_call_kind(receiver.kind()) {
            chain_receiver(&receiver)?
        } else {
            member_receiver(&receiver)?
        };
        if !is_call_kind(receiver.kind()) {
            return None;
        }
        if !call_junction_exact(&receiver) {
            return None;
        }
        chain_segment_args_contract(&receiver, head, source, &mut captures)?;
    }
    chain_segment_args_contract(node, last, source, &mut captures)?;
    Some(captures)
}

/// PASS 67a (F66a-3): walker for the N-segment optional chain — one row per
/// matching call node (nested chains emit inner+outer rows like sg), the
/// same emission shape as [`walk_optional_calls`].
fn walk_optional_call_chains(
    lang: Language,
    node: Node,
    source: &str,
    pattern: &str,
    segments: &[CallChainSegment],
    optional_flags: &[bool],
    out: &mut Vec<PatternMatch>,
) {
    if let Some(captures) =
        optional_chain_matches(lang, &node, source, pattern, segments, optional_flags)
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
        walk_optional_call_chains(lang, child, source, pattern, segments, optional_flags, out);
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
    // PASS 122 (F2): the swift link-STRUCTURAL trivia veto on chain
    // candidates — same predicate and grid as the plain-call consult above
    // (receiver-side/mid-link comment before a later `navigation_suffix`
    // refuses; callee-internal stays transparent).
    if lang == Language::Swift
        && call_field_node(node).is_some_and(|callee| swift_member_link_structural(&callee))
    {
        return None;
    }
    // PASS 129 (128A-F5, f129b): the ONE junction rule at EVERY consumed
    // call level of the dotted chain — the head call here, every receiver
    // hop in the loop below — for the js/ts grammars whose sg doctrine is
    // grid-proven (oracle /tmp/phase129/cells/f5b/f5c: `a.b(1).c /*j*/ (2)`
    // and `a.b /*j*/ (1).c(2)` are both sg [] while the comment-transparent
    // link positions bind). Comments INSIDE the member-chain callee and
    // INSIDE argument lists never fire this gate (§39.4/§39.6 scope).
    if matches!(lang, Language::JavaScript | Language::TypeScript)
        && !call_junction_exact(node)
    {
        return None;
    }
    let path = chain_callee_segments(lang, node, source)?;
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
        // PASS 129 (128A-F5, f129b): the interior link's own callee→`(`
        // junction — sg applies the exact-children rule per consumed call
        // level, not just the matched head.
        if matches!(lang, Language::JavaScript | Language::TypeScript)
            && !call_junction_exact(&receiver)
        {
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
        // PASS 81a (FB-80a-05): positional `$A, $B` binding, same contract
        // as the member-chain lane (sg: `r.m1($A, $B).m2()` binds A=1, B=2).
        if let Some(names) = &segment.arg_metas {
            if let Some(nodes) = argument_nodes(&receiver, &["arguments"]) {
                for (name, argument) in names.iter().zip(nodes.iter()) {
                    if let Some(text) = node_text(argument, source) {
                        bind_capture(&mut captures, name, text)?;
                    }
                }
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
    // PASS 81a (FB-80a-05): the tail call carries the positional contract.
    if let Some(names) = &last.arg_metas {
        if let Some(nodes) = argument_nodes(node, &["arguments"]) {
            for (name, argument) in names.iter().zip(nodes.iter()) {
                if let Some(text) = node_text(argument, source) {
                    bind_capture(&mut captures, name, text)?;
                }
            }
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
fn chain_callee_segments(
    lang: Language,
    node: &Node,
    source: &str,
) -> Option<Vec<ChainPathSegment>> {
    if node.kind() == "call" {
        if let Some(segs) = ruby_receiver_callee(node, source) {
            return Some(
                segs.into_iter()
                    .map(|text| ChainPathSegment {
                        text,
                        start: 0,
                        end: None,
                        optional: false,
                    })
                    .collect(),
            );
        }
    }
    let mut segs = Vec::new();
    chain_collect_segments(lang, node, source, &mut segs)?;
    Some(segs)
}

/// Recursive dotted-component collection over candidate nodes.
fn chain_collect_segments(
    lang: Language,
    node: &Node,
    source: &str,
    segs: &mut Vec<ChainPathSegment>,
) -> Option<()> {
    // java(/kotlin-family) member calls carry object + name fields directly.
    if node.kind() == "method_invocation" {
        let object = node.child_by_field_name("object")?;
        let name = node.child_by_field_name("name")?;
        chain_collect_segments(lang, &object, source, segs)?;
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
        chain_collect_segments(lang, &field, source, segs)?;
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
            .find(|child| {
                child.id() != object.id()
                    // PASS 129 (128A-F5, f129b): comment trivia between links
                    // / at the receiver is sg-transparent in the js/ts
                    // grammars — skip it instead of letting a comment child
                    // pose as the property segment (`a.b(1)/*m*/.c(2)` used
                    // to decompose [a, b, /*m*/] and refuse where sg binds).
                    // kt/swift keep their registered link-structural
                    // refusals, so the skip is scoped to the grid-proven
                    // grammars.
                    && !(matches!(lang, Language::JavaScript | Language::TypeScript)
                        && (is_trivia_kind(child.kind()) || child.is_extra()))
            })?;
        chain_collect_segments(lang, &object, source, segs)?;
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
/// If-node kinds matched by `NativeKind::If` across the 15 indexed languages.
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
    "block_expression",
];

/// Wrapper kinds that never hold statements directly; descend into their
/// single block-like child before counting (Swift `function_body { statements }`).
const STMT_WRAPPER_KINDS: &[&str] = &["function_body", "then", "statements"];

fn is_trivia_kind(kind: &str) -> bool {
    kind.contains("comment")
}

fn walk_ifs(
    lang: Language,
    node: Node,
    source: &str,
    pattern: &str,
    cond: Option<&str>,
    body: Option<&BodyTemplate>,
    body_braced: bool,
    alternative: Option<&IfAlternative>,
    out: &mut Vec<PatternMatch>,
) {
    // PASS 122 (f122a): three sg-exactness guards on the candidate if node.
    // (1) The node must be NAMED — the anonymous `if` KEYWORD TOKEN carries
    // kind "if" in several grammars and the old scan matched it as a full
    // if-site whenever the template had no body (`if $X` on go/py answered
    // the token row, oracle grid /tmp/phase122/f1). (2) A DIRECT trivia
    // child sitting BEFORE the consequence breaks sg's structural match
    // (`if (a) /*c*/ { b(); }` and `if /*c*/ (a) { b(); }` are sg `[]`;
    // comments AFTER the consequence — pre-`else` — and inside the
    // condition/body are transparent, probed same grid). (3) Brace-ness —
    // enforced inside `if_body_matches` via `body_braced`.
    let direct_trivia_before_consequence = {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        let consequence_index = children
            .iter()
            .position(|child| Some(*child) == if_consequence(&node));
        children.iter().enumerate().any(|(index, child)| {
            (is_trivia_kind(child.kind()) || child.is_extra())
                && consequence_index.is_some_and(|ci| index < ci)
        })
    };
    // PASS 124 (F3, f124c): the trivia-position doctrine is GRAMMAR-SCOPED,
    // not language-free. sg 0.45.2 treats if-level comment trivia as
    // TRANSPARENT on swift (pre-condition, pre-`{`, body-start, one-line,
    // call-condition — all sg n1, oracle /tmp/phase124/f3) and on python
    // (the direct-child positions its grammar has — condition-adjacent and
    // body-attached comments — answer n1); js/ts/c/php (122's f1 grid) and
    // kt/go (f3 controls) stay STRUCTURAL — pre-condition/pre-brace trivia
    // refuses. Unprobed grammars keep the conservative structural refusal.
    let if_trivia_structural =
        direct_trivia_before_consequence && !matches!(lang, Language::Swift | Language::Python);
    if IF_KINDS.contains(&node.kind())
        && node.is_named()
        && !is_in_comment_or_string(&node)
        && !if_trivia_structural
        && !cross_grammar_braced_if_refused(lang, node.clone(), pattern, body_braced)
        && if_body_matches(lang, &node, body, body_braced)
    {
        // PASS 137: the else-tail alignment gates the emission — an
        // else-carrying pattern whose candidate lacks (or mismatches) the
        // tail emits nothing. The descent below continues either way so a
        // refused OUTER candidate still yields its nested if candidates.
        if_alternative_matches(lang, &node, source, pattern, alternative, &mut |captures| {
            emit_if_match(lang, &node, source, pattern, cond, captures, out);
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ifs(
            lang,
            child,
            source,
            pattern,
            cond,
            body,
            body_braced,
            alternative,
            out,
        );
    }
}

/// PASS 137 (D6 elseif_same_line_F5): the else-tail alignment for one
/// candidate if node. `None` (else-less pattern) keeps the registered
/// behavior — the `emit` closure runs unconditionally and the callee returns
/// true. A parsed tail demands a matching candidate alternative:
/// * `Else` — the candidate's `alternative` field must exist and satisfy the
///   braced/count body grammar; a `$$$`/`$B` meta body binds its text.
/// * `ElseIf` — the candidate's alternative must itself be an if node whose
///   condition and body align (meta conditions bind the whole condition
///   text, concrete ones compare through the general-lane template), then
///   the nested tail recurses.
/// The `emit` closure runs ONCE with the extended captures when (and only
/// when) the whole tail aligned, so sg's four-capture else-if binding lands
/// in one match row.
fn if_alternative_matches(
    lang: Language,
    node: &Node,
    source: &str,
    pattern: &str,
    alternative: Option<&IfAlternative>,
    emit: &mut dyn FnMut(&mut BTreeMap<String, String>),
) -> bool {
    let Some(spec) = alternative else {
        emit(&mut BTreeMap::new());
        return true;
    };
    let Some(cand_alt) = node.child_by_field_name("alternative") else {
        return false;
    };
    let mut captures = BTreeMap::new();
    if !if_alternative_aligns(
        lang,
        spec,
        &cand_alt,
        source,
        pattern,
        &mut captures,
    ) {
        return false;
    }
    emit(&mut captures);
    true
}

/// One else-link alignment: `spec` (pattern side) against `cand` (candidate
/// side, the `alternative` node — for `ElseIf` an if node, for `Else` the
/// fallback statement). Returns false on any structural mismatch; binds the
/// tail's captures into `captures` (name conflicts veto, sg unification).
fn if_alternative_aligns(
    lang: Language,
    spec: &IfAlternative,
    cand: &Node,
    source: &str,
    pattern: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    // js/ts/c-style grammars wrap the else tail in an `else_clause` node
    // (field `alternative` never points at the inner if/block directly);
    // the alignment grammar below speaks in terms of the tail's BODY (block
    // or nested if node), so unwrap the wrapper first. python's `elif_clause`
    // skips this shape — it IS the nested if-like node (accepted below).
    let unwrapped = {
        let mut c = *cand;
        while c.kind() == "else_clause" {
            let Some(inner) = c.named_child(0) else {
                return false;
            };
            c = inner;
        }
        c
    };
    let cand = &unwrapped;
    match spec {
        IfAlternative::Else {
            body,
            body_braced,
            body_meta,
        } => {
            if *body_braced && !consequence_is_braced(cand) {
                return false;
            }
            if !if_body_template_matches(cand, *body) {
                return false;
            }
            if let Some(name) = body_meta {
                if let Some(text) = node_text(cand, source) {
                    if bind_capture(captures, name, strip_container(text)).is_none() {
                        return false;
                    }
                }
            }
            true
        }
        IfAlternative::ElseIf {
            cond,
            body,
            body_braced,
            cond_meta,
            body_meta,
            alternative,
        } => {
            // python spells the tail `elif` as its own node kind; its
            // condition/consequence/alternative fields line up with the
            // if-node grammar this arm speaks.
            if !(IF_KINDS.contains(&cand.kind()) || cand.kind() == "elif_clause")
                || !cand.is_named()
            {
                return false;
            }
            if let Some(cond_text) = cond {
                let Some(template) = cached_if_cond_template(lang, cond_text) else {
                    return false;
                };
                let Some(template_root) = general_template_root(&template) else {
                    return false;
                };
                let Some(cand_cond) = cand.child_by_field_name("condition") else {
                    return false;
                };
                let mut cand_cond = cand_cond;
                if template_root.kind() != "parenthesized_expression" {
                    while cand_cond.kind() == "parenthesized_expression" {
                        let Some(inner) = cand_cond.named_child(0) else {
                            break;
                        };
                        cand_cond = inner;
                    }
                }
                if general_eq(&template, template_root, cand_cond, source, captures).is_none() {
                    return false;
                }
            } else if let Some(name) = cond_meta {
                if let Some(cond_node) = cand.child_by_field_name("condition") {
                    if let Some(text) = node_text(&cond_node, source) {
                        if bind_capture(captures, name, strip_container(text)).is_none() {
                            return false;
                        }
                    }
                }
            }
            let Some(consequence) = if_consequence(cand) else {
                return false;
            };
            if *body_braced && !consequence_is_braced(&consequence) {
                return false;
            }
            if !if_body_template_matches(&consequence, *body) {
                return false;
            }
            if let Some(name) = body_meta {
                if let Some(text) = node_text(&consequence, source) {
                    if bind_capture(captures, name, strip_container(text)).is_none() {
                        return false;
                    }
                }
            }
            match alternative {
                Some(nested) => match cand.child_by_field_name("alternative") {
                    Some(nested_cand) => {
                        if_alternative_aligns(lang, nested, &nested_cand, source, pattern, captures)
                    }
                    None => false,
                },
                None => true,
            }
        }
    }
}

/// PASS 137: body-template count/Any check for an else-tail branch node
/// (the meta-capture binding itself lives in the callers, which know which
/// pattern section the meta came from).
fn if_body_template_matches(node: &Node, body: Option<BodyTemplate>) -> bool {
    match body {
        None => true,
        Some(BodyTemplate::Any) => true,
        Some(BodyTemplate::Exactly(want)) => {
            if BLOCK_KINDS.contains(&node.kind()) {
                count_statements(*node) == want
            } else {
                want == 1
            }
        }
    }
}

/// The shared if-emit path: registered-capture base (MATCH + head cond/body
/// metas via [`captures_for_node`]) plus the else-tail captures, unified
/// through [`bind_capture`] (conflicts veto the candidate), then one match
/// row.
fn emit_if_match(
    lang: Language,
    node: &Node,
    source: &str,
    pattern: &str,
    cond: Option<&str>,
    tail_captures: &mut BTreeMap<String, String>,
    out: &mut Vec<PatternMatch>,
) {
    match cond {
        None => {
            let Some(mut captures) = captures_for_node(node, source, pattern, Some("if")) else {
                return;
            };
            if let Some(name) = if_head_body_meta(pattern) {
                if !bind_if_head_body(&mut captures, name, node, source) {
                    return;
                }
            }
            for (name, text) in tail_captures.iter() {
                if bind_capture(&mut captures, name, text).is_none() {
                    return;
                }
            }
            push_match_with_captures(node, source, pattern, captures, out);
        }
        Some(cond) => {
            // The concrete-cond path binds cond metas first; the tail
            // captures must unify with them (a same-name conflict vetoes).
            let mut staged: Vec<PatternMatch> = Vec::new();
            push_cond_match(lang, node, source, pattern, cond, &mut staged);
            for m in staged.iter_mut() {
                if let Some(name) = if_head_body_meta(pattern) {
                    if !bind_if_head_body(&mut m.captures, name, node, source) {
                        return;
                    }
                }
                for (name, text) in tail_captures.iter() {
                    if bind_capture(&mut m.captures, name, text).is_none() {
                        return;
                    }
                }
            }
            out.extend(staged);
        }
    }
}

/// PASS 137 (f137h): the `$NAME` of the pattern's FIRST braced section when
/// the pattern carries an `else` tail — the head body meta. `body_capture`'s
/// first-`{`-to-last-`}` slice spans the tail for these patterns and can
/// never return the bare head meta, so the emit path binds it here (the
/// else-less spelling keeps `body_capture`, whose slice is the head itself).
fn if_head_body_meta(pattern: &str) -> Option<&str> {
    if !pattern.contains("else") {
        return None;
    }
    let open = pattern.find('{')?;
    let close = open + pattern[open..].find('}')?;
    capture_name(pattern.get(open + 1..close)?.trim())
}

/// Binds the head body meta to the head consequence's stripped text; false on
/// a same-name conflict (sg unification veto, [`bind_capture`]).
fn bind_if_head_body(
    captures: &mut BTreeMap<String, String>,
    name: &str,
    node: &Node,
    source: &str,
) -> bool {
    let Some(consequence) = if_consequence(node) else {
        return true;
    };
    let Some(text) = node_text(&consequence, source) else {
        return true;
    };
    bind_capture(captures, name, strip_container(text)).is_some()
}

/// PASS 131 (130A-F6, f131e): the concrete-condition candidate path. The
/// candidate's condition node unwraps its anonymous `parenthesized_expression`
/// wrapper whenever the template root is NOT itself a parenthesized
/// expression (js/ts/c/php/java wrap every condition; go/py/rust candidates
/// carry the condition directly or exactly as spelled — the go paren
/// structural doctrine stays enforced by
/// [`cross_grammar_braced_if_refused`]). `general_eq` compares the cond
/// template against the condition and binds cond metas; a same-name
/// conflict with the body/MATCH captures vetoes the candidate (sg
/// unification semantics, [`bind_capture`]).
fn push_cond_match(
    lang: Language,
    node: &Node,
    source: &str,
    pattern: &str,
    cond: &str,
    out: &mut Vec<PatternMatch>,
) {
    let Some(template) = cached_if_cond_template(lang, cond) else {
        return;
    };
    let Some(template_root) = general_template_root(&template) else {
        return;
    };
    let Some(mut candidate) = node.child_by_field_name("condition") else {
        return;
    };
    if template_root.kind() != "parenthesized_expression" {
        while candidate.kind() == "parenthesized_expression" {
            let Some(inner) = candidate.named_child(0) else {
                break;
            };
            candidate = inner;
        }
    }
    let mut cond_captures = BTreeMap::new();
    if general_eq(&template, template_root, candidate, source, &mut cond_captures).is_none() {
        return;
    }
    let Some(mut captures) = captures_for_node(node, source, pattern, Some("if")) else {
        return;
    };
    for (name, text) in cond_captures {
        if bind_capture(&mut captures, &name, &text).is_none() {
            return;
        }
    }
    push_match_with_captures(node, source, pattern, captures, out);
}

/// PASS 122 (f122a live grid): cross-grammar STRUCTURAL alignment of the
/// braced if template — the two file-language families where the brace-ness
/// rule alone is not sg-exact. (a) python cannot align a `{...}` if body at
/// all (suites are `:`-indented; even the literal `{q()}` suite face and the
/// parenthesized-condition face are sg semantic-empty — probed
/// /tmp/phase122/probe_py1, probe_py2), so a braced pattern is sg-empty on
/// EVERY py candidate. (b) go binds the pattern's condition parens
/// structurally: a `($X)` spelling is a parenthesized_expression node the
/// candidate condition must repeat (sg n1 on `if (x) { y() }`, n0 on
/// `if x { y() }`; the paren-free `if $X { $B }` pattern answers BOTH
/// candidate shapes — probed /tmp/phase122/probe_go1, probe_go2). js/ts/c/
/// php/kt are unaffected: their parens/braces are anonymous delimiters the
/// brace-ness gate already covers.
fn cross_grammar_braced_if_refused(
    lang: Language,
    node: Node,
    pattern: &str,
    body_braced: bool,
) -> bool {
    if !body_braced {
        return false;
    }
    if lang == Language::Python {
        return true;
    }
    if lang == Language::Go {
        let pattern_cond_parenthesized = pattern
            .trim()
            .strip_prefix("if")
            .is_some_and(|after| after.trim_start().starts_with('('));
        if pattern_cond_parenthesized
            && node
                .child_by_field_name("condition")
                .is_some_and(|cond| cond.kind() != "parenthesized_expression")
        {
            return true;
        }
    }
    false
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
    // PASS 120 (119A-F2): a `new`-led call head is the same family shape —
    // sg 0.45.2 answers `new q($$$A)` like the plain call (probe grid
    // 2026-09-08: binds [1, 2] on `new q(1, 2)`); strip the constructor
    // keyword and parse the family call identically. Non-js/ts `new` heads
    // still refuse later in the eligibility gate (the head admission is
    // language-scoped), so this strip is inert for them.
    let p = p
        .strip_prefix("new ")
        .filter(|rest| {
            rest.starts_with(char::is_alphabetic)
                || rest.starts_with('_')
                || rest.starts_with('$')
        })
        .unwrap_or(p);
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
    // PASS 120 (119A-F2): the language-free gate is the UNION of the
    // per-language admissions — "does ANY grammar build a template?" The
    // PASS 120 head admissions are language-scoped (`new` js/ts, `lambda`
    // py), so the union ORs the js base with the py evaluation LIMITED to
    // lambda-headed patterns — the py comment rules (`//` as floor-div)
    // must NOT widen the gate for comment-carrying spellings (the registered
    // `return $B// noteA` fail-closed rows stay refused). Without this
    // union the ingress census rejected `new`/`lambda` faces before the
    // per-file walk could answer them (CLI rc2 on match-bearing files,
    // first-hand probe on the post-build binary).
    general_lane_text_eligible_for(Language::JavaScript, pattern)
        || (pattern.split_whitespace().next() == Some("lambda")
            && general_lane_text_eligible_for(Language::Python, pattern))
        // PASS 127 (125A-F2/F4): the loop/control heads whose owning
        // grammar is NOT the js base — the same PASS 120 union discipline
        // (each arm is limited to the head's grid-probed language so no
        // language's comment/lexical rules leak into the gate).
        || match pattern.split_whitespace().next() {
            Some("for") => general_lane_text_eligible_for(Language::Go, pattern)
                || general_lane_text_eligible_for(Language::Rust, pattern),
            Some("while") => general_lane_text_eligible_for(Language::Rust, pattern),
            Some("loop") => general_lane_text_eligible_for(Language::Rust, pattern),
            Some("not") => general_lane_text_eligible_for(Language::Python, pattern),
            // PASS 135 (134A-F1/F7, f135c-ingress, grids A13/A1/X_py_del_two):
            // the per-language head admissions (:8331/:8333) carry the csharp
            // `lock`/`using` and python `del` statement heads, but this
            // language-free UNION never did — `needs_ast_grep_fallback` kept
            // rc2-ing faces sg 0.45.2 answers (Searcher::search loud before
            // the walk whose match_pattern answers each face sg-exactly n1;
            // `with`/`delete`/`void`/`var` never hit this because the js base
            // arm already admits them). Union discipline: each arm limited to
            // the owning grammar so no other language's comment/lexical rules
            // leak into the gate (the PASS 120 precedent).
            Some("lock") => general_lane_text_eligible_for(Language::CSharp, pattern),
            // PASS 137 (137B-F4 + B7 cpp_using_namespace): cpp joins the
            // `using` head — `using namespace $N;` sg n1 N=`std` (subject
            // rc2). The per-language eligible gates keep the grammars'
            // comment/lexical rules separate.
            Some("using") => {
                general_lane_text_eligible_for(Language::CSharp, pattern)
                    || general_lane_text_eligible_for(Language::Cpp, pattern)
            }
            Some("del") => general_lane_text_eligible_for(Language::Python, pattern),
            // PASS 137 (D3 grid, f137i): the java `assert`/`synchronized`
            // statement heads and the php `namespace`/`goto` heads — each
            // per-language admitted, none carried in this union, so the
            // ingress rc2'd faces sg 0.45.2 answers n1 (grid137: ja_assert,
            // ja_synchronized, php_namespace, php_goto).
            Some("assert") | Some("synchronized") => {
                general_lane_text_eligible_for(Language::Java, pattern)
            }
            Some("namespace") | Some("goto") => {
                general_lane_text_eligible_for(Language::Php, pattern)
            }
            _ => false,
        }
        // PASS 129 (128A-F2/F1, f129e/f129g): the ruby modifier statements
        // (`x if $C` family — the modifier keyword is the PENULTIMATE token)
        // and the braced BEGIN/END block heads. Union discipline: each arm
        // is limited to the owning grammar, so no other language's rules
        // leak into the gate.
        || (ruby_modifier_statement_pattern(
            pattern,
            pattern.split_whitespace().next().unwrap_or(""),
        ) && general_lane_text_eligible_for(Language::Ruby, pattern))
        || (matches!(pattern.split_whitespace().next(), Some("BEGIN") | Some("END"))
            && general_lane_text_eligible_for(Language::Ruby, pattern))
    }

/// PASS 105 (FB-104A-3): true when a bare-identifier head continues with a
/// BINARY OPERATOR (`x * q($A)`) — an ordinary expression face sg answers
/// through its binary parse, not declaration-keyword territory. The
/// php member connector `->` is excluded: `->`-spelled callees classify into
/// the dedicated member-call lane before the general lane is ever consulted.
/// The template build itself still decides admission (clean parse, general
/// root kind, span coverage), so a merely-admitted spelling whose build
/// refuses keeps its registered loud class byte-for-byte.
fn bare_ident_operator_continuation(after: &str) -> bool {
    let Some(first) = after.chars().next() else {
        return false;
    };
    if matches!(first, '+' | '-' | '*' | '/' | '%' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?') {
        // `->` is a member connector, never a binary continuation.
        return !after.starts_with("->");
    }
    // PASS 127 (125A-F5b, f127a): go's short-variable declaration
    // continuation (`x := $Y` sg n1, px_go_short2 oracle cell) — the `:=`
    // token is assignment syntax, not a labeled-statement colon; only the
    // tight two-byte spelling admits.
    if after.starts_with(":=") {
        return true;
    }
    false
}

/// F-93B-2 (r44): language-aware eligibility — `py` selects the python
/// comment judgment (`//` is python's FLOOR-DIV operator, not comment
/// syntax; sg 0.45.2 answers `$A // $B` {1,2,2,3}, `$A // 2` {3} on the
/// floor-div fixture, probes_run1.jsonl matrix D). The lang-free ingress
/// ([`general_lane_text_eligible`]) keeps the conservative line-comment
/// refusal.
fn general_lane_text_eligible_for(lang: Language, pattern: &str) -> bool {
    let py = lang == Language::Python;
    let p = pattern.trim();
    if p.contains('\n') || p.contains(GENERAL_MV_PREFIX) {
        // PASS 137 (f137b/f137i, 135 grids A9/A10 + lockB_conc + B3):
        // statement templates sg matches LAYOUT-INSENSITIVELY are exempt
        // from the newline blanket — csharp lock/using statement templates
        // (multi-line bodies bind, X=`o`) and java synchronized blocks
        // (grid B3). The build's root gate keeps the admission sg-exact.
        let head = p
            .split(|c: char| c.is_ascii_whitespace() || c == '(')
            .next();
        let layout_insensitive = match (lang, head) {
            (Language::CSharp, Some("lock" | "using")) => p.contains('('),
            (Language::Java, Some("synchronized")) => p.contains('('),
            _ => false,
        };
        if !layout_insensitive {
            return false;
        }
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
    let comment_refused = if py {
        contains_block_comment_syntax_outside_strings(p)
    } else {
        contains_comment_syntax_outside_strings(p)
    };
    if comment_refused {
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
    // PASS 120 (119A-F2): `new`-led and python-`lambda` expression heads
    // (below) plus the `arrow_function` root kind (see
    // `is_general_root_kind`) are the sg-grid-probed admissions. The
    // original bare-param-arrow census reading is RETIRED: f89b's registered
    // `$A => $B;` face (88a-L3) plus fresh sg probes (`x => q($X)` answers
    // its `x =>` candidate; `$X => q($Y)` answers too) prove sg ANSWERS
    // bare-param arrows — the earlier grid `[]` was fixture-specific
    // semantic empty, not an accepted-empty class.
    if let Some(first) = p.split_whitespace().next() {
        let bare_keyword = first.chars().all(|c| c.is_ascii_alphabetic());
        // PASS 120 (119A-F2): sg-answerable expression heads the bare-keyword
        // guard would refuse as declaration-keyword territory — `new`-led
        // constructor expressions (sg answers `new q($X)` / `new q($A, $B)` /
        // `new ns.Q($X)` js+ts, grid 2026-09-08; other languages keep their
        // census class, unprobed) and python `lambda` roots (sg answers
        // `lambda $X: $Y`, `lambda: $Y`, `lambda $X: q($X)` — py only, the
        // spelling is py syntax).
        // PASS 122 (F4, f122d, oracle grids /tmp/phase122/f4): `async`-led
        // arrows (sg answers `async (x) => q($X)`, `async ($A) => q($X)`,
        // `async x => q($X)` js n1 each) and the `let`/`const`/`var`
        // declaration heads (sg answers `let $A = $B` / `const $A = $B` /
        // `var $A = $B` js n1; the js/ts `lexical_declaration`/
        // `variable_declaration` root kinds are the PASS 122 kind admissions
        // below). The rust `let $A = $B` face keeps its registered
        // census-loud class per file: rust's build roots at `let_statement`,
        // which the root-kind gate still refuses, so the per-language census
        // stays loud there (the old language-free ingress reading of the
        // registered row is corrected — sg ANSWERS the js AND rust `let $A
        // = $B` spellings, first-hand grid; registered test rows
        // f120b/pass60 updated to the corrected premise, rust contract
        // asserted per-language in f122d).
        let admitted_expression_head = (matches!(
            lang,
            Language::JavaScript | Language::TypeScript
        ) && matches!(first, "new" | "async" | "let" | "const" | "var"))
            || (lang == Language::Python && first == "lambda")
            // PASS 127 (125A-F2/F4, f127b/f127c, oracle grids
            // /tmp/phase127/g1 + cells2): the loop/control heads sg answers
            // per grammar (js/ts `for`/`while`/`do`; py `for`/`while`/`not`;
            // go `for`; rust `for`/`while`/`loop`; php `foreach`/`while`/
            // `do`), plus js `typeof` (b3_js_typeof sg n1). Language-scoped
            // on purpose: unprobed heads keep their census class, and the
            // template build + sg gate still decide admission per file.
            || match first {
                // python `for`/`while` are deliberately ABSENT: the sg
                // bindings bind the WHOLE `:`-suite text (cells2 o7/o20:
                // B = "a()\n    b()") — a binding the general lane's
                // kind-exact child alignment cannot express (the inline
                // suite shape differs from the candidate block). Admitting
                // them would walk silent-empty (silent UNDER-answer), the
                // worst class — they stay census-loud as registered
                // (form-1: suite-text binding machinery per the 123B-F1
                // model, pinned in f127b_boundaries).
                "for" => matches!(
                    lang,
                    Language::JavaScript
                        | Language::TypeScript
                        | Language::Go
                        | Language::Rust
                ),
                "while" => matches!(
                    lang,
                    Language::JavaScript | Language::TypeScript | Language::Rust
                ),
                "do" => matches!(lang, Language::JavaScript | Language::TypeScript),
                "loop" => lang == Language::Rust,
                // PASS 129 (128B-F2, f129f): rb `not $X` is sg-ANSWERED
                // (oracle f2_rb_not n1, X=x) — the same unary-family face
                // py already admits; rb needs the plain `unary` root kind
                // (admitted in `is_general_root_kind`).
                "not" => matches!(lang, Language::Python | Language::Ruby),
                "typeof" => lang == Language::JavaScript,
                _ => false,
            };
        // PASS 129 (128A-F2/F1, f129e/f129g): ruby statement shapes the
        // bare-keyword head guard would rc2 — the modifier statements
        // (`x if $C` family, sg n1 each) and the braced BEGIN/END block
        // roots (`BEGIN { $B }` sg n1). The build + root-kind gate still
        // decide admission per file.
        let admitted_expression_head = admitted_expression_head
            || (lang == Language::Ruby
                && (matches!(first, "BEGIN" | "END")
                    || ruby_modifier_statement_pattern(p, first)))
            // PASS 135 (134A-F1/F7, grids /tmp/phase135/cells A/Y/X + F):
            // the statement heads whose sg bindings the general lane
            // expresses per grammar — csharp `lock`/`using` statements and
            // declarations (A9/A10 layout-insensitive binds, A13 meta
            // resource, A8 using-declaration T/N/E) plus `var` locals
            // (Y_cs_var_decl), js `with` (F_js_with X/B) and the unary
            // `delete`/`void` (F_js_delete X=`o.k`, F_js_void), python
            // `del` (X_py_del_two X=`x, y` whole list). The build + root
            // kind gates still decide admission per file.
            || (lang == Language::CSharp && matches!(first, "lock" | "using" | "var"))
            || (lang == Language::JavaScript && matches!(first, "with" | "delete" | "void"))
            || (lang == Language::Python && first == "del")
            // PASS 137 (grid137a B3/B5/B7/D6, all subject-rc2 faces sg
            // answers n1): the language-scoped head admissions — js/ts
            // `export` (export default $X; → X=`42`), java `assert`/
            // `synchronized` (assert $X : $M; binds X/M; synchronized ($X)
            // binds X), cpp `using` (using namespace $N; → N=`std`), php
            // `namespace`/`goto` (namespace $N; → N=`App`; goto $L; →
            // L=`a`). The template build + sg gate still decide admission
            // per file (union discipline: each arm limited to the owning
            // grammar).
            || (matches!(lang, Language::JavaScript | Language::TypeScript) && first == "export")
            || (lang == Language::Java && matches!(first, "assert" | "synchronized"))
            || (lang == Language::Cpp && first == "using")
            || (lang == Language::Php && matches!(first, "namespace" | "goto"));
        if bare_keyword
            && !admitted_expression_head
            && !DECL_PATTERN_PREFIXES.iter().any(|(prefix, _)| prefix.trim() == first)
            && !STATEMENT_HEAD_KEYWORDS.contains(&first)
        {
            // PASS 65a (F64-4): an ASSIGNMENT head (`name = "user-#{$N}"`) is
            // an ordinary identifier, not declaration-keyword territory — sg
            // answers the ruby string-interpolation face (probed 0.45.2).
            // The `let` shapes keep their registered fail-closed contract at
            // the root-kind gate (`is_general_root_kind` refuses let kinds),
            // and uppercase/other bare heads (no `=`) stay refused here.
            // PASS 105 (FB-104A-3): an OPERATOR continuation is ordinary
            // binary-expression territory, not declaration-keyword territory
            // either — sg 0.45.2 answers `x * q($A)` on every ident-LHS row
            // in the six probed languages (m2 oracle) while the bare-keyword
            // guard rc2'd the whole class (ident-LHS rows in go/py/rs/ts/
            // java/swift, subject rc2 where sg answers H).
            let after = p[first.len()..].trim_start();
            if !after.starts_with('=') && !bare_ident_operator_continuation(after) {
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

/// PASS 129 (128A-F2, f129e): a ruby MODIFIER statement tail — the pattern's
/// second-to-last whitespace token is a modifier keyword and the head is an
/// ordinary expression head (`x if $C`, `x unless $C`, `x while $C`,
/// `x until $C`; sg binds C, oracle cells f123_rb_mod_*). The 3-token floor
/// keeps the bare `if $C` end-root face (registered §43.7c) out.
fn ruby_modifier_statement_pattern(p: &str, head: &str) -> bool {
    let tokens: Vec<&str> = p.split_whitespace().collect();
    tokens.len() >= 3
        && !matches!(head, "if" | "unless" | "while" | "until")
        && matches!(
            tokens[tokens.len() - 2],
            "if" | "unless" | "while" | "until"
        )
}

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
    // F-93B-2 (r44): `//` is python's FLOOR-DIV operator, not comment
    // syntax — the language-free arm refused `$A // $B` (census rc2) where
    // sg 0.45.2 answers it (probes_run1.jsonl matrix D: `$A // $B`
    // {1,2,2,3}, `$A // 2` {3}, `a // b` {1,2} on the floor-div fixture;
    // probed 2026-09-08 ATTACHED). For python only `/*` is foreign comment
    // syntax; the registered py `#`-comment faces keep their refusal through
    // the hash arm above. Ruby keeps the `//` refusal (its comment char is
    // `#` too and no sg-answering `//` face is registered there); php and
    // every C-family language spell real line comments with `//`.
    if lang == Language::Python {
        return contains_block_comment_syntax_outside_strings(pattern.trim());
    }
    contains_comment_syntax_outside_strings(pattern.trim())
}

/// F-93B-2: the `/*`-only half of
/// [`contains_comment_syntax_outside_strings`] — true when a block-comment
/// opener appears outside string-literal quotes (same quote scan).
fn contains_block_comment_syntax_outside_strings(p: &str) -> bool {
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
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
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
    /// 88a-L3 (r39): the raw pattern spelled a trailing `;`. sg keeps a
    /// pattern-trailing `;` significant — the pattern root is the STATEMENT
    /// and only bare `expr;` statements answer — so on the plain span-less
    /// builds the root resolution stops AT `expression_statement` instead
    /// of unwrapping it. The php `<?php `-wrapped builds (span Some) keep
    /// their registered unwrapped behavior.
    had_semi: bool,
    /// Resolved template root kind (candidate prefilter).
    root_kind: String,
    /// PASS 137 (grid137a D4 cs_lock_meta_body/cs_using_meta_body): the
    /// csharp meta-BODY lock/using templates are sg-ACCEPTED faces that bind
    /// NOTHING (valid-empty on every candidate — oracle lockprobe re-probe:
    /// `lock ($X) { $B }` rc1 `[]` with the `lock (o) { x(); }` candidate
    /// present). The pass-135 build refusal composed the loud census class
    /// there — subject rc2 where sg answers ok:true-0. The template now
    /// BUILDS with this flag set; [`match_structural_general`] answers empty
    /// without touching the census.
    force_empty: bool,
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
        // Dart statements live only in function bodies and demand the `;`
        // terminator (the java/csharp shape); MoonBit blocks accept a
        // trailing expression without a terminator (the go/swift shape).
        Language::Dart => ("void __asgrep_ctx() { ", "; }"),
        Language::MoonBit => ("fn __asgrep_ctx() { ", " }"),
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
    // PASS 94b (FB-93A-4b): go wraps a context-built single statement in
    // `block > statement_list`; the template root must descend through the
    // single-statement statement_list or the structural comparison roots at
    // the wrapper (kind never matches a candidate call node, so go literal
    // calls only ever answered through the exact-text arm — `f(1, 2)`
    // missed the `f(1, /* n */ 2)` candidate sg answers, matrix C).
    "statement_list",
];

/// Allowed template root kinds for the general lane: expressions, calls, and
/// known declaration kinds. `if` templates stay in the dedicated If lane, and
/// let/bindings keep their registered fail-closed contract.
/// 88a-L3 (r39): `statement_root` admits the `expression_statement` root for
/// had_semi patterns on the plain builds (sg roots the `;`-terminated
/// spelling at the statement — only bare `expr;` statements answer).
fn is_general_root_kind(kind: &str, statement_root: bool) -> bool {
    if matches!(kind, "if_statement" | "if_expression" | "if") || kind.contains("let") {
        return false;
    }
    statement_root && kind == "expression_statement"
        || is_call_kind(kind)
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
        // PASS 120 (119A-F2): sg-answerable expression roots probed on the
        // 2026-09-08 grid — the js/ts arrow root (paren-spelled
        // `($X) => $Y` / `($A, $B) => q($A, $B)` / `() => q($X)` AND the
        // bare-param `x => q($X)` / `$X => q($Y)` spellings — all probed
        // sg HITS) and the python `lambda` root (`lambda $X: $Y` family).
        // `new_expression` already rides the `_expression` arm. The
        // tagged-template root keeps the B-97A-3 registered refusal upstream.
        || kind == "arrow_function"
        || kind == "lambda"
        // PASS 122 (F4, f122d, oracle grids /tmp/phase122/f4): sg-answering
        // root kinds whose walk machinery (kind-exact `general_eq` with
        // `bind_capture` leaf unification) already exists — the js/ts
        // generator root (`function* q($X) { $$$B }` / `{ $B }` sg n1,
        // matching-name grid), the js/ts `lexical_declaration`/
        // `variable_declaration` declarator roots (`let/const/var $A = $B`
        // sg n1; rust `let_statement` is NOT here, keeping the rust
        // census-loud class), and the collection-literal roots `array` +
        // `object` (`[$A, $B]` sg n1 on js/py/swift; `{ a: $A }` sg n1 js).
        // The walk is kind-exact, so a template can only ever match the
        // identical candidate kind (PASS 120 O-1 doctrine).
        || kind == "generator_function_declaration"
        || kind == "lexical_declaration"
        || kind == "variable_declaration"
        // Collection-literal roots per grammar: js/ts `array`/`object`,
        // python `list` (sg answers `[$A, $B]` py n1), swift
        // `array_literal` (sg answers `[$A, $B]` swift n1).
        || kind == "array"
        || kind == "object"
        || kind == "list"
        || kind == "array_literal"
        // PASS 107 (FB-106A-7): ruby's binary expression node kind is plain
        // `binary` — the only probed grammar whose binary root carries neither
        // the `_expression` nor the `_operator` tail. Without this arm every
        // ruby operator-continuation template refused the build and the face
        // composed into the loud census where sg 0.45.2 answers the aligned
        // rows (m5_loudfam: `x * q($A)` / `x + q($A)` / `x.y * q($A)` on the
        // aligned rb rows). Leaf mismatches still answer honest empty.
        || kind == "binary"
        // PASS 65a (F64-4): a string template root answers only when every
        // metavariable sits inside a `#{…}` INTERPOLATION subtree (sg binds
        // it; probed 0.45.2) — placeholders in PLAIN string content still
        // refuse the build in `pattern_has_placeholder_in_literal`, so the
        // registered string-metavar fail-closed rows are untouched.
        || kind == "string"
        // PASS 127 (125A-F2, f127b, oracle grids /tmp/phase127/g1 + cells2):
        // sg-answering loop/for-family template roots whose walk machinery
        // (kind-exact `general_eq`) already exists — js/ts `for_statement`
        // (classic + for-of/for-in spell `for_in_statement`),
        // `while_statement`, `do_statement`; python `for_statement` +
        // `while_statement`; go `for_statement` (all four head forms);
        // php `foreach_statement` + `while_statement` + `do_statement`;
        // rust `loop_expression`/`for_expression`/`while_expression`
        // (rust spellings carry no `let` substring, so the rust
        // `let_statement` census refusal above cannot leak in). The walk is
        // kind-exact, so a template can only ever match the identical
        // candidate kind (PASS 120 O-1 doctrine). `while ($X) { $B }` and
        // the brace-less `while ($X) $B` spellings answer sg n1 with the
        // same bindings (oracle o1/o4 cells) — the §30.6 for/while rows
        // that recorded "for-heads NOT sg-answering" are corrected in CNR.
        // Still REFUSED upstream: `$$$`-body statement templates (php
        // `do { $$$B } while ($X);`, go `for { $$$B }`, js IIFE `$$$B` —
        // the §42.4/§41.7 registered multi-span louds) and python faces
        // whose `:`-suite alignment the general lane cannot express
        // (registered form-1).
        || matches!(
            kind,
            "for_statement" | "for_in_statement" | "while_statement" | "do_statement"
                | "foreach_statement"
        )
        // PASS 127 (125A-F5b, f127a): go's short-variable declaration root
        // (`x := q()` × `$X := $Y` sg n1, `x := $Y` sg n1) — the `:=`
        // head-continuation admission lives in
        // [`bare_ident_operator_continuation`].
        || kind == "short_var_declaration"
        // PASS 129 (128A-F2/F1, f129e/f129g): ruby modifier statement roots
        // (`x if $C` → if_modifier etc., sg n1 each, oracle
        // f123_rb_mod_*) and the braced BEGIN/END block roots
        // (`BEGIN { $B }` sg n1, oracle f123_rb_*). Kind-exact: only
        // identical candidate kinds align. The multi-line `begin … end
        // while` face stays registered-loud (the newline eligibility
        // refusal).
        || matches!(
            kind,
            "if_modifier" | "unless_modifier" | "while_modifier" | "until_modifier"
                | "begin_block" | "end_block"
        )
        // PASS 129 (128B-F2, f129f): ruby's `not` unary root kind (rb
        // `not $X` sg n1). py's not_operator rides the `_operator` arm;
        // rb's plain `unary` needed its own admission.
        || kind == "unary"
        // PASS 127 (125A-F3, f127d): ruby's return-statement root kind is
        // plain `return` (the only probed grammar not spelling
        // `return_statement`); rb `return $X` sg n1 (b2_rb_return oracle
        // cell). Kind-exact: only a candidate `return` node can match.
        || kind == "return"
        // PASS 135 (134A-F1/F7, grids /tmp/phase135/cells): sg-answering
        // statement roots whose walk machinery (kind-exact `general_eq`)
        // already exists. csharp lock/using statements answer
        // layout-insensitively (A9/A10) — the meta-carrying BODY spellings
        // stay refused at the build (sg's own parse ERRORs them, grids
        // A2/A3/Y_cs_using_meta_body) via the csharp statement-body veto in
        // [`try_build_general_template`]; the using DECLARATION root
        // (`using var x = y();` → local_declaration_statement) binds
        // T/N/E (A8) and the literal-type sibling `var $N = $E;` answers
        // n1 (Y_cs_var_decl). py `delete_statement` (`del $X` binds X,
        // F_py_del/X_py_del_two), js `with_statement` (F_js_with: X/B
        // bind), the labeled-statement roots (js/ts/java:
        // F_js_labeled/X_js_label_concrete/Y_java_labeled bind L/X; the
        // mismatched-label face refuses through the bind-capture conflict
        // — X_js_label_mismatch sg rc1), and the ts `type_alias_declaration`
        // root for the non-object alias faces (`type $N = $V` binds V
        // across union/fn/literal spellings — F_ts_alias_union and the X
        // grid; the OBJECT-body alias face `type $N = { $B }` routes
        // through the Class member-count lane instead, where sg's B
        // capture lands on the member, not the braces).
        || matches!(
            kind,
            "lock_statement" | "using_statement" | "local_declaration_statement"
                | "delete_statement" | "with_statement" | "labeled_statement"
                | "type_alias_declaration"
        )
        // PASS 137 (grid137a B3/B5/B7): the language-scoped statement-root
        // admissions for the head faces above — js/ts export (export
        // default $X; sg n1), java assert/synchronized (sg n1 each),
        // cpp using_declaration (using namespace $N; sg n1), php
        // namespace_definition + goto_statement (sg n1 each). Kind-exact:
        // only identical candidate kinds align (the PASS 120 O-1 doctrine).
        || matches!(
            kind,
            "export_statement" | "assert_statement" | "synchronized_statement"
                | "using_declaration" | "namespace_definition" | "goto_statement"
        )
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
        // 88a-L3 (r39): a pattern-trailing `;` is sg-significant — the
        // pattern root is the statement (kind-exact: only bare `expr;`
        // statements answer), so the plain span-less builds stop HERE.
        if template.had_semi && template.span.is_none() && node.kind() == "expression_statement" {
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

fn pattern_has_placeholder_in_literal(
    node: &Node,
    doc: &str,
    placeholders: &BTreeMap<String, String>,
) -> bool {
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
        // PASS 96 (FB-95B-1): EXCEPT the WHOLE-CONTENT meta hole — sg's
        // parsed `string_content`/`string_fragment` leaf whose ENTIRE text
        // is a meta token IS a metavariable and binds the candidate content
        // (matrix ms_instring_meta: js `g("$A")` answers `g("hw")` AND
        // `g("other")`). When every placeholder-bearing leaf of the string
        // node IS exactly a placeholder (nothing else in the content), the
        // build is admitted; `general_eq`'s placeholder arm then binds the
        // candidate content leaf sg-style. Prefix/suffix spellings
        // (`"pre-$A"`) keep the refusal — sg reads those leaves as literal
        // text — and comment/regex nodes never qualify.
        let string_node = kind.contains("string");
        if !placeholders_only_inside_interpolations(node, doc)
            && !(string_node && placeholders_are_whole_string_content(node, doc, placeholders))
        {
            return true;
        }
    }
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|child| {
        pattern_has_placeholder_in_literal(&child, doc, placeholders)
    });
    found
}

/// PASS 96 (FB-95B-1): true when every leaf of this string node whose text
/// carries the placeholder prefix IS exactly a placeholder key — i.e. the
/// string's content is a bare metavariable hole (`"$A"`), the sg-truth meta
/// binding. Any placeholder that is a PROPER SUBSTRING of a content leaf
/// (`"pre-$A"`, `"$A-$B"`) makes this false and keeps the literal-text
/// refusal.
fn placeholders_are_whole_string_content(
    node: &Node,
    doc: &str,
    placeholders: &BTreeMap<String, String>,
) -> bool {
    let mut whole = true;
    fn scan(
        node: &Node,
        doc: &str,
        placeholders: &BTreeMap<String, String>,
        whole: &mut bool,
    ) {
        if node.kind() == "interpolation" {
            return; // interpolation subtrees are already-admitted meta holes
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        if children.is_empty() {
            if let Some(text) = node_text(node, doc) {
                if text.contains(GENERAL_MV_PREFIX) && !placeholders.contains_key(text) {
                    *whole = false;
                }
            }
            return;
        }
        for child in children {
            scan(&child, doc, placeholders, whole);
        }
    }
    scan(node, doc, placeholders, &mut whole);
    whole
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

#[allow(clippy::too_many_arguments)]
fn try_build_general_template(
    lang: Language,
    doc: String,
    span: Option<(usize, usize)>,
    had_semi: bool,
    placeholders: &BTreeMap<String, String>,
    multi_names: &BTreeSet<String>,
    root_gate: bool,
) -> Option<GeneralTemplate> {
    let tree = parse_source(lang, &doc).ok()?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    if pattern_has_placeholder_in_literal(&root, &doc, placeholders) {
        return None;
    }
    let mut probe = GeneralTemplate {
        doc,
        tree,
        placeholders: placeholders.clone(),
        multi_names: multi_names.clone(),
        span,
        had_semi,
        root_kind: String::new(),
        force_empty: false,
    };
    let node = general_template_root(&probe)?;
    // 88a-L3 (r39): the statement root is admitted only for had_semi
    // patterns on the plain span-less builds (the same condition the root
    // resolution stops under). PASS 131 (130A-F6): `root_gate == false`
    // (the if-CONDITION builder) bypasses the whole gate — a condition is
    // an expression POSITION whose candidate node the walk controls
    // (`push_cond_match` compares against the condition node only), so the
    // statement-root fail-closed contract has no statement root to protect.
    if root_gate && !is_general_root_kind(node.kind(), had_semi && span.is_none()) {
        return None;
    }
    // PASS 135 (134A-F1, grids A2/A3/A5/A6/Y_cs_using_meta_body): meta-BODY
    // lock/using templates. The pass-135 wording claimed sg's own parse
    // ERRORs them — CORRECTED PASS 137 (oracle lockprobe re-probe):
    // `lock ($X) { $B }` / `using ($D) { $B }` are ACCEPTED faces that bind
    // NOTHING (rc1 `[]` = valid empty with a matching candidate present).
    // tree-sitter parses those bodies cleanly, so without a gate the walk
    // would OVER-answer (bind B where sg binds nothing). The face must not
    // compose into the loud census either — sg answers it valid-empty. The
    // template therefore BUILDS with `force_empty` set; the walk answers
    // empty (sg-exact) and the census keeps the face answerable.
    let force_empty = lang == Language::CSharp
        && matches!(node.kind(), "lock_statement" | "using_statement")
        && !placeholders.is_empty()
        && csharp_statement_body_carries_placeholder(&node, &probe.doc, placeholders);
    probe.root_kind.push_str(node.kind());
    probe.force_empty = force_empty;
    Some(probe)
}

/// PASS 135: true when the csharp lock/using statement BODY (the `block`
/// child; `using_statement` spells it as the `body` field) carries a
/// metavariable leaf — the sg ERROR-node class the build must refuse.
fn csharp_statement_body_carries_placeholder(
    node: &Node,
    doc: &str,
    placeholders: &BTreeMap<String, String>,
) -> bool {
    let body = node.child_by_field_name("body").or_else(|| {
        let mut cursor = node.walk();
        let mut found = None;
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                found = Some(child);
                break;
            }
        }
        found
    });
    let Some(body) = body else {
        return false;
    };
    fn contains_placeholder(node: &Node, doc: &str, placeholders: &BTreeMap<String, String>) -> bool {
        if node_text(node, doc).is_some_and(|text| placeholders.contains_key(text.trim())) {
            return true;
        }
        let mut cursor = node.walk();
        let mut any = false;
        for child in node.children(&mut cursor) {
            if contains_placeholder(&child, doc, placeholders) {
                any = true;
                break;
            }
        }
        any
    }
    contains_placeholder(&body, doc, placeholders)
}

fn build_general_template(
    lang: Language,
    raw: &str,
    substituted: &str,
    placeholders: &BTreeMap<String, String>,
    multi_names: &BTreeSet<String>,
) -> Option<GeneralTemplate> {
    // 88a-L3 (r39): a pattern-trailing `;` is sg-significant — thread the
    // flag so the plain builds root the template at the statement.
    let had_semi = raw.trim().ends_with(';');
    if let Some(template) =
        try_build_general_template(lang, substituted.to_string(), None, had_semi, placeholders, multi_names, true)
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
        had_semi,
        placeholders,
        multi_names,
        true,
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
                had_semi,
                placeholders,
                multi_names,
                true,
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
        had_semi,
        placeholders,
        multi_names,
        true,
    )
}

/// PASS 131 (130A-F6, f131e): the concrete-condition if template, cached per
/// (language, cond text). The cond parses through the general-lane retries
/// (plain doc, newline doc, php `<?php ` wrap, expression-context wrap) but
/// BYPASSES the statement-root kind gate ([`try_build_general_template`]'s
/// `root_gate = false`): a condition is an expression position and the walk
/// compares it against the candidate's own condition node only, so bare
/// identifier/variable roots (`if (a)`, php `if ($a)`) build here where the
/// statement lane's gate would refuse them.
fn build_if_cond_template(lang: Language, cond: &str) -> Option<GeneralTemplate> {
    let (substituted, placeholders, multi_names) = substitute_general_metavariables(cond)?;
    if let Some(template) = try_build_general_template(
        lang,
        substituted.clone(),
        None,
        false,
        &placeholders,
        &multi_names,
        false,
    ) {
        return Some(template);
    }
    let newline_doc = format!("{substituted}\n");
    if let Some(template) = try_build_general_template(
        lang,
        newline_doc,
        Some((0, substituted.len())),
        false,
        &placeholders,
        &multi_names,
        false,
    ) {
        return Some(template);
    }
    if lang == Language::Php {
        for doc in [
            format!("<?php {substituted};"),
            format!("<?php {substituted}\n"),
        ] {
            if let Some(template) = try_build_general_template(
                lang,
                doc,
                Some((6, 6 + substituted.len())),
                false,
                &placeholders,
                &multi_names,
                false,
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
        false,
        &placeholders,
        &multi_names,
        false,
    )
}

thread_local! {
    static IF_COND_TEMPLATES: RefCell<HashMap<(Language, String), Option<GeneralTemplate>>> =
        RefCell::new(HashMap::new());
}

fn cached_if_cond_template(lang: Language, cond: &str) -> Option<GeneralTemplate> {
    let key = (lang, cond.trim().to_string());
    IF_COND_TEMPLATES.with(|cell| {
        let mut map = cell.borrow_mut();
        if let Some(template) = map.get(&key) {
            return template.clone();
        }
        let built = build_if_cond_template(lang, &key.1);
        map.insert(key.clone(), built.clone());
        built
    })
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
    // PASS 127 (125A-F3, f127d, oracle cells b2_php_return/o15 + b2_php_throw/
    // o18 sg n1): the statement-head templates (`return $X;`, `throw $X;`)
    // parse only behind the `<?php ` tag like families (i)/(ii) — sg itself
    // pre-processes php patterns behind the tag, and the wrapped build still
    // demands a clean parse and a general root kind (return_statement /
    // throw_statement are admitted roots), so non-building heads
    // (`raise`/`yield`/`defer` are not php syntax) keep their loud class.
    if raw
        .split_whitespace()
        .next()
        .is_some_and(|head| {
            STATEMENT_HEAD_KEYWORDS.contains(&head)
                // PASS 137 (B7 php_namespace/php_goto sg n1): the php roots
                // parse only behind the `<?php ` tag like the statement
                // heads — the plain build folds a bare `namespace $N;` into
                // a text node. The wrapped build still demands a clean parse
                // and an admitted root kind (namespace_definition /
                // goto_statement).
                || matches!(head, "namespace" | "goto")
        })
    {
        return true;
    }
    // PASS 127 (125A-F2, f127b): the php loop/control heads (`foreach …`,
    // `while …`, `do …`) were PROBED and deliberately NOT admitted: the
    // `$`-stripping meta substitution cannot spell php VARIABLE positions
    // (foreach `as`-targets parse as ERROR; braced `{ $B }` bodies too),
    // and the one spelling that builds (brace-less `while ($X) $B`) binds
    // B to the php expression-statement text (`b()`) where sg binds the
    // full statement (`b();`) — a capture-text divergence. All php loop
    // faces keep their census-loud class; registered form-1 rows
    // (f127b_boundaries): a php variable-preserving meta substitution (or
    // a dedicated foreach/while lane with sg's statement-span binding),
    // then re-grid.
    if raw.contains("->") {
        return true;
    }
    let Some(open) = raw.find('(') else {
        return false;
    };
    let head = raw[..open].trim();
    if head.contains("::") && head.split("::").all(is_pure_metavariable) {
        return true;
    }
    // PASS 107 (FB-106A-7 php twin): a bare-identifier head with an operator
    // continuation (`x * q($A)`) — sg pre-processes php patterns behind the
    // tag, so the plain build folds the pattern into a text node and the
    // face composed into the loud census where sg answers the aligned row
    // (m4_php_op phpbare_tag {a.php:2}). The wrapped build still demands a
    // clean parse and a general root kind, so non-building spellings keep
    // their registered loud class.
    let first = match head.split_whitespace().next() {
        Some(first) => first,
        None => return false,
    };
    let after = head[first.len()..].trim_start();
    !after.is_empty() && bare_ident_operator_continuation(after)
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
    // PASS 137 (137A-F2): the csharp statement-head lane is a language-free
    // "some lane serves this" admission for the ingress gate — the general
    // template cannot root these heads, so the any-language build loop below
    // would report unsupported and `needs_ast_grep_fallback` would rc2 faces
    // sg 0.45.2 answers (grid137a B3/D4). The lane parse is spelling-level
    // only; the walk + census keep per-file/per-language honesty.
    if matches!(
        pattern.split_whitespace().next(),
        Some("fixed" | "checked" | "unchecked" | "unsafe" | "lock" | "using")
    ) && csharp_statement_template(pattern).is_some()
    {
        return true;
    }
    // PASS 139 (139A-F5, grid139 E + release-binary spot-check): the java
    // class member-count lane is a language-free "some lane serves this"
    // admission for the ingress gate — the general template cannot
    // substitute a bare-meta class body (the starvation that kept the face
    // census-loud pre-139), so `needs_ast_grep_fallback` rc2s faces sg
    // 0.45.2 answers n1 (E01/E02/E08; f139_java_class_member_count_reaches_
    // the_walk RED at the routing seam). The lane parse is spelling-level
    // only; the walk + census keep per-file/per-language honesty.
    if classify_java_class_member_count(pattern).is_some() {
        return true;
    }
    // PASS 140 (grids /tmp/phase140R): the directive roots, the remaining
    // statement roots, the java synchronized NESTED/METHOD faces, the php
    // literal-name/`$$`-body namespace faces, the csharp checked/unchecked
    // expression root, and go's `$`-carrying `;`-ful spellings — the
    // language-free "some lane serves this" admissions (the 137A-F2 shape).
    // The lane parses are spelling-level only; the walk + census keep
    // per-file/per-language honesty.
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
        // PASS 140 (I_py_dollaratom/R3_py_del_whole_d3): the del lane's
        // `$$$`-slot faces ride the dedicated delete template — the
        // general lane's build refuses the multi-meta spelling and would
        // keep the sg-answering face ingress-loud. The census arm stays
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
    // PASS 96 (FB-95B-1): a whole-content string meta (`"$A"` in sg terms —
    // the parsed string-content leaf IS the metavariable, matrix
    // ms_instring_meta: py/js sg answers [8,9]) carries no punctuation, but
    // is a structural template whose build the any-language loop below
    // verifies (and whose per-language root-kind admission + whole-content
    // placeholder rule keep `"pre-$A"`/`"$A$B"`/escaped spellings refused).
    let bare_string_meta_template = substituted.len() >= 2
        && substituted.starts_with('"')
        && substituted.ends_with('"')
        && placeholders.len() == 1
        && placeholders.contains_key(&substituted[1..substituted.len() - 1]);
    // PASS 127 (125A-F4, f127c, oracle grid /tmp/phase127/g1 + cells2): a
    // KEYWORD-OPERATOR template (`$X and $Y` py/rb/kt sg n1 each, `$X or
    // $Y`, `$X as $Y` ts, `not $X` py, `typeof $X` js) carries no
    // punctuation from the structural set — the operator is an alphabetic
    // keyword — but is a structural template whose per-language build the
    // any-language loop below verifies (the root kinds land on the
    // existing `_operator`/`binary`/`_expression` admissions; the `not`/
    // `typeof` heads ride the head admissions above).
    // PASS 135 (134A-F7, f135e, grid /tmp/phase135/cells gridX/Y): the
    // unary KEYWORD faces join the same lane — `delete $X` js sg n1+n1
    // (X=`o.k`/`o['j']`, trivia cell sg n1), `void $X` js sg n1+n1,
    // `del $X` py sg n1 (X=`x, y` whole list). The root kinds
    // (`unary_expression`, `delete_statement`) are admitted in
    // `is_general_root_kind`.
    let keyword_operator_template = !bare_string_meta_template
        && placeholders.len() >= 1
        && pattern.split_whitespace().any(|token| {
            matches!(
                token,
                "and" | "or" | "as" | "not" | "typeof" | "delete" | "void" | "del"
            )
        });
    // PASS 129 (128A-F2, f129e): the ruby MODIFIER statement template
    // (`x if $C`) carries no structural punctuation but is structural — the
    // rb modifier root kinds are admitted in `is_general_root_kind` and the
    // any-language loop below verifies the build.
    let rb_modifier_template = placeholders.len() >= 1 && {
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

fn match_structural_general(lang: Language, source: &str, pattern: &str) -> Vec<PatternMatch> {
    let Some(template) = cached_general_template(lang, pattern) else {
        return Vec::new();
    };
    // PASS 137: the csharp meta-body lock/using faces are sg-ACCEPTED
    // bind-nothing faces (grid137a D4 + lockprobe re-probe) — the walk's
    // empty is the sg agreement, never a census concern.
    if template.force_empty {
        return Vec::new();
    }
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
            lang,
            &mut seen,
            &mut out,
        );
    }
    // PASS 124 (F2, f124b): the general lane is the member path the kt/swift
    // DOTTED `$`-less spellings ride (property chains `a.b.c`, plain calls
    // `a.b(1)`) — swift's registered §41.5 plain-face residual and the kt
    // dotted faces over-answered receiver-link trivia here because the
    // general lane had NO link-structural consult. Apply the union doctrine
    // PER-CANDIDATE (the candidate's own subtree only, so the sg-answered
    // trivia-free inner link of a longer chain keeps answering), scoped to
    // the two grammars whose sg trivia doctrine is grid-proven (CNR §39.13,
    // §41.4; oracle grids /tmp/phase124/f2, f2b). This satisfies §41.5's
    // form-1 retry predicate (demand arrived via the same-lane F2 finding).
    retain_member_link_trivia_free(lang, &tree, &mut out);
    // PASS 144 per-candidate general-lane gates (the structural alignment
    // skips trivia/unnamed children and over-served these sg refusals):
    // * 143A-F3 (grid U*): a cs candidate whose subtree carries an ERROR
    //   node holding a gap-junk char OUTSIDE the sg cs trivia class
    //   (U+2028/U+2029/U+0085 — sg's parse refuses those at cs token gaps)
    //   drops. The A0/FEFF spellings stay in-class and keep binding
    //   (U3/U4/U5); the junk-transparent directive/throw/return faces ride
    //   their dedicated lanes and never reach this gate.
    // * 143A-F2 (grid J*): a cs call candidate whose callee→`(` junction
    //   carries a comment/U+2028/U+2029 run drops (J1/J2/J7/J8 sg rc1);
    //   FEFF/NBSP junctions bind (J3/J4) and comment-inside-args stays
    //   outside the junction (J5).
    // * 143A-F9 (grid C*): a ts member-link candidate with a trivia child
    //   directly before the `?.` connector drops (C1 sg rc1 — the
    //   tree-sitter-typescript refusal); the js twin CJ1 binds and the
    //   after-connector position (C2) keeps binding.
    if matches!(lang, Language::CSharp | Language::TypeScript | Language::Go) {
        out.retain(|hit| {
            node_with_span(tree.root_node(), hit.byte_start, hit.byte_end).is_none_or(|node| {
                match lang {
                    // PASS 146 (145B-F2, grid uc*): the F3 outsider-junk gate
                    // is scoped to the using-var HEAD — sg's parse recovers
                    // junk-ERROR inside the initializer and still binds the
                    // outer meta (uc1/uc3/uc4/X2 bind sg-exact, junk
                    // retained), so the subtree-wide scan over-refused it.
                    // PASS 146 (145A-F9, grid h*): the head gate now also
                    // refuses COMMENT children in the keyword→name head
                    // (h1/h2 sg rc1) while comments after the name (h3) and
                    // at the initializer (h4) keep binding.
                    Language::CSharp => {
                        (node.kind() != "local_declaration_statement"
                            || cs_using_var_head_clean(&node, source))
                            && (!is_call_kind(node.kind())
                                || cs_call_junction_trivia_free(&node, source))
                    }
                    Language::TypeScript => !ts_member_link_receiver_trivia(&node),
                    // PASS 146 (145A-F8, grid g*): sg refuses comments AND
                    // the junk class (U+2028/FEFF/A0) at the
                    // `for`/`go`/`defer` keyword→body junctions (g1-g4/g7/g8
                    // rc1) while clean junctions bind (g5/g10), argument
                    // comments bind (g6), and `func` is NOT gated (g9 —
                    // sg's keyword→name refusal is per-keyword, the 143A-F6
                    // scope discipline).
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

/// PASS 146 (145A-F8, grid g*): true when the bytes between the leading
/// `for`/`go`/`defer` keyword token and the first non-trivia child are
/// comment-free ASCII whitespace — sg's law at these junctions (the
/// 143A-F6 type-gate class, scoped to the three receipted keywords).
/// The gated statement node for a general-lane go hit: the hit node itself
/// or a same-span descendant of the three gated kinds.
fn go_gate_target<'tree>(node: &Node<'tree>) -> Option<Node<'tree>> {
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

fn go_keyword_body_junction_clean(node: &Node, source: &str) -> bool {
    // Grid g3 vs the PROTECTED 129 cell (f129d go_lead): sg refuses the
    // head comment on the CLAUSE forms (range/3-clause — first named child
    // is a `for_clause`) while the while-form's head-leading comment stays
    // transparent (`for /* h */ n > 0 {` binds, A=`n > 0`).
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
    let Some(first) = children
        .find(|child| !is_trivia_kind(child.kind()) && !child.is_extra())
    else {
        return true;
    };
    let gap = &source[kw.end_byte()..first.start_byte()];
    !gap.contains("//")
        && !gap.contains("/*")
        && gap.chars().all(|c| c.is_ascii_whitespace())
}

/// PASS 146 (145A-F9 grid h* + 145B-F2 grid uc*): the using-var head gate —
/// the head spans from the statement start to the declarator NAME's first
/// byte. sg refuses comments (h1/h2) and outsider-junk ERROR runs (U-grid
/// U1/U6) there while binding everything after the name (h3/h4) and every
/// junk position INSIDE the initializer (uc1/uc3/uc4 — junk retained in the
/// capture).
fn cs_using_var_head_clean(node: &Node, source: &str) -> bool {
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
    // PASS 146 (145A-F9 grid h* + 145B-F2 grid uc*): sg refuses comments
    // (h1/h2) and outsider-junk ERROR runs (U-grid U1/U6) ANYWHERE in the
    // keyword->name head — the comment extras attach BELOW the statement
    // node, so the scan is recursive. After the name (h3/h4) everything
    // binds, junk retained (uc).
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

/// PASS 144 (143A-F9): true when a trivia child sits directly inside a
/// member link before the `?.` connector — the anonymous token or the named
/// `optional_chain` wrapper — the ts-only sg refusal position (the 113
/// doctrine's general-lane twin).
fn ts_member_link_receiver_trivia(node: &Node) -> bool {
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

/// PASS 144 (143A-F3): true when no ERROR node in the subtree carries a
/// candidate gap-junk char outside the pattern-side sg cs trivia class (see
/// the [`is_sg_cs_trivia`] / [`is_sg_cs_gap_junk`] seam distinction; the
/// OUTSIDER set is the junk that is neither sg-class trivia nor plain ASCII
/// whitespace — U+2028/U+2029/U+0085/U+1680/…, the spellings sg's cs parse
/// refuses at token gaps).
fn cs_subtree_outsider_junk_free(node: Node, source: &str) -> bool {
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

/// PASS 117 (116A-F1): the general-lane arm for `?.`-chain PROPERTY faces
/// outside {TypeScript, JavaScript}, GATED with the ONE chain rules. The
/// byte-identical preservation arm 115 shipped over-answered kotlin/swift
/// junction-comment and kotlin mid-chain/receiver-trivia faces where sg
/// 0.45.2 refuses (first-hand grid 2026-09-08). Every candidate the general
/// lane matched must be a chain whose EVERY consumed call level has an
/// exact callee→`(` junction and whose member links carry no trivia child;
/// statement-level trivia BEFORE the chain attaches outside the matched
/// node and keeps answering (kt/swift oracle HIT).
fn match_structural_chain_gated(lang: Language, source: &str, pattern: &str) -> Vec<PatternMatch> {
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

/// PASS 117: the chain-exactness gate for one general-lane candidate — the
/// candidate node (matched at its exact byte span) descends through
/// callee/receiver hops; every call level must pass [`call_junction_exact`]
/// and every member link must be trivia-free. Each hop lands on a strictly
/// smaller child span, so the walk terminates at the chain root. A call
/// hops to its CALLEE first so the member branch trivia-checks the callee
/// link itself (kotlin attaches mid-chain trivia to the callee
/// navigation_expression, first-hand parse dump 2026-09-08).
/// PASS 124 (F2, 123B-F2): swift's WRAPPED links let the kt-shaped
/// later-ANONYMOUS rule pass vacuously on this lane (the connector lives
/// inside a NAMED navigation_suffix), so swift — and kt's dotted links —
/// consult the union [`member_link_trivia_structural`] doctrine here too.
fn chain_candidate_exact(root: Node, start: usize, end: usize, lang: Language) -> bool {
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

/// PASS 117: a trivia (comment) or extra child directly inside a member
/// link — the general lane's child alignment skips these on BOTH sides,
/// which is exactly the over-answer 116A-F1 closed.
/// PASS 118 (117E-1): POSITION-SCOPED. kotlin-ng flattens
/// `navigation_suffix`, so a post-`?.` comment is ALSO a direct link child
/// (first-hand CST dump 2026-09-10: `a?. /*c*/ b` = [identifier, `?.` anon,
/// comment EXTRA, identifier]) and the unscoped veto silently refused faces
/// sg 0.45.2 ANSWERS (`a?. /*c*/ b?.c(1)`, `a?.b?. /*c*/ c(1)`, line
/// comments, dotted `.` connectors, 2/3/4-link chains, double comments —
/// oracle grid /tmp/phase118/g118a*.sh). The sg discriminator on the flat
/// child list is byte-POSITIONAL, not token-spelled: a trivia child with
/// ANY later ANONYMOUS sibling is link-STRUCTURAL trivia — sg refuses
/// (`a /*c*/ ?.b?.c(1)`, `a?.b /*c*/ ?.c(1)`, `a?.b(1) /*c*/ ?.c(2)` all
/// oracle []). On every probed face the later anonymous sibling IS the
/// link's `.`/`?.`/`?` connector token (CNR §39.13), but the operative
/// predicate is the anonymous-sibling test itself — PASS 119B-F2 corrected
/// this doc from the narrower connector-token wording to match the code,
/// which agrees with sg on every probed face (g118c 4/4). A trivia child
/// followed only by NAMED siblings sits in the callee-internal
/// position after the link's own connector — transparent trivia sg answers.
/// Swift never enters the allowed branch (its grammar wraps the comment
/// inside `navigation_suffix`, never a link child — CST dump), so its
/// refusals are untouched.
/// PASS 122 (121B-F1): the ONE kotlin link-structural gate for both kt
/// optional decomposers — hoisted verbatim from the two previously duplicated
/// inline expressions (`optional_chain_decompose` / `optional_call_chain_decompose`)
/// so a future grammar-pin edit cannot change one lane's language condition
/// without the other (the 119B-F1 split-doctrine class). The predicate is the
/// shared [`member_link_has_trivia`]; the unscoped [`chain_candidate_exact`]
/// consult at its own site is the general/literal arm's separate doctrine
/// (CNR §39.13/§40.1). No behavior change — the two sites were byte-identical.
fn kt_link_structural(lang: Language, node: &Node) -> bool {
    lang == Language::Kotlin && member_link_has_trivia(node)
}

fn member_link_has_trivia(node: &Node) -> bool {
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

/// PASS 122 (F2, 121A-F2 fail-open closed): the SWIFT member-link trivia
/// doctrine, same position rule as the kt consults above but read against the
/// swift grammar's WRAPPED links (first-hand CST dumps 2026-09-10,
/// tree-sitter-swift 0.7: `a /*c*/ .b(1)` = navigation_expression
/// [simple_identifier a, multiline_comment, navigation_suffix ".b"];
/// `a. /*c*/ b(1)` puts the comment INSIDE the navigation_suffix after its
/// `.`; `a.b(1) /*c*/ .c(2)` is a navigation_expression-level comment before
/// the next suffix). sg 0.45.2 refuses every swift candidate whose comment
/// sits BEFORE a later navigation_suffix — receiver-side, mid-link, and
/// doubled (`a /*c*/ /*d*/ .b(1)`) alike, oracle grid /tmp/phase122/f2 —
/// while the callee-INTERNAL position (inside the suffix, after the `.`)
/// stays transparent (2-link `a. /*c*/ b(1)` answers both engines). Swift
/// never enters the kt decomposers (§40.1 "swift never decomposes"), so the
/// plain-call and chain lanes consult THIS predicate instead: any
/// navigation_expression / navigation_suffix in the candidate's callee
/// subtree carrying a direct trivia/extra child with a LATER
/// navigation_suffix sibling is link-STRUCTURAL → refuse.
fn swift_member_link_structural(node: &Node) -> bool {
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

/// PASS 124 (F2, 123A-F2 + 123B-F2, oracle grids /tmp/phase124/f2, f2b): the
/// UNION member-link trivia doctrine, RECURSIVE over the candidate subtree —
/// at every node, a direct COMMENT-kind trivia child (extras excluded — the
/// registered doctrine is comment trivia, swift's error-glue EXTRA recovery
/// rows are extras sg answers through) with a later ANONYMOUS sibling
/// (kt's flat `?.` links, CNR §39.13) or a later `navigation_suffix` sibling
/// (swift/kt WRAPPED dotted links, CNR §41.4) is link-STRUCTURAL. The two
/// one-level predicates alone miss the nested shapes the per-candidate
/// retain sees (the trivia lives one link down inside the candidate span,
/// e.g. the kt dotted callee of `a /*c*/ .b(1)`), which is exactly why the
/// `$`-carrying consult (callee passed directly) fired while the
/// whole-candidate retain did not. Positional scope is unchanged:
/// callee-internal trivia (inside the suffix, after the `.`) and tail
/// trivia (outside the candidate span) never fire.
fn member_link_trivia_structural(node: &Node) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    for (index, child) in children.iter().enumerate() {
        // COMMENT-kind trivia only: the registered doctrine (CNR §39.13/
        // §41.4) is comment trivia. swift's error-glue EXTRA fragments
        // (µµµ$A recovery rows, m1/m2 oracle sets) are extras sg ANSWERS
        // through — folding `is_extra` in here over-refused them
        // (f100/f102/f105 swift faces, caught by the suite).
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

/// PASS 117: the node whose byte span is exactly `(start, end)`, descending
/// through the children that contain the span.
fn node_with_span<'a>(node: Node<'a>, start: usize, end: usize) -> Option<Node<'a>> {
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

/// PASS 129 (128A-F6, f129d): sg's loop-head comment discipline, grid-derived
/// at /tmp/phase129/cells/f6* + the 129 re-grid /tmp/phase129/verify (js/ts/
/// go/rs × for/for-of/while/do/loop, 30+ cells). A loop-kind candidate is
/// STRUCTURAL (refuse) when its direct comment children sit in an
/// sg-refused zone:
/// - js/ts `for`/`for_in`: a comment refuses unless it sits fully BEFORE the
///   `(` (`for /* f */ (`) or TRAILS a named header element with the next
///   non-comment sibling a `;`/`)` terminator (`i < n /* t */ ;`,
///   `i++ /* u */ )`, `xs /* of */ )` — trailing header comments surface at
///   the for ROOT and sg answers them). Refused: after-`(`, after-`;` gaps,
///   the `)`→`{` junction, plus an init-declaration comment preceding a
///   later sibling (`let /* c0 */ i`).
/// - js/ts `while`: a comment is transparent ONLY trailing the condition
///   before `)` (`n > 0 /* t */ )`); leading (`while /* w */ (`) and the
///   `)`→`{` junction refuse.
/// - js/ts `do`: comments BEFORE the body (`do /* d */ {`); post-body
///   comments (`} /* z */ while`) stay transparent.
/// - go `for`: comments after the first clause element's start (the
///   after-`;` gaps AND the `)`→`{`-analogue junction); head-leading trivia
///   (`for /* h */ n > 0`) stays transparent — the protected 127 cell.
/// - rs `for`/`while` expressions: comments at/after the last header child
///   (the junction); leading/mid-header trivia stays transparent.
/// - rs `loop`: any root-level comment (no header zone).
/// Non-loop kinds and comment-free candidates never fire.
fn loop_head_trivia_structural(lang: Language, node: &Node) -> bool {
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
                // PASS 129 (128A-F6, oracle re-grid /tmp/phase129/verify):
                // the discipline is PER-COMMENT-POSITION, not a whole-header
                // zone — tree-sitter surfaces trailing header comments at the
                // for ROOT (`i < n /* t */ ;`, `i++ /* u */ )`) and sg 0.45.2
                // ANSWERS those (the for-of `xs /* of */ )` junction
                // included, oracle n1 V=v/X=xs/B=use(v);). A root comment is
                // transparent iff (a) it sits fully before the `(`
                // (`for /* f */ (` — the protected 127-style lead) or (b) it
                // TRAILS a named header element whose next non-comment
                // sibling is the `;`/`)` terminator. Comments after `(`, in
                // the after-`;` gap, or between `)` and the body refuse.
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
                // 3-clause form's `for /* h */ i := 0` is sg-refused (oracle
                // n0; the single-condition `for /* h */ n > 0` protected
                // cell has no clause and stays transparent); (3) for_clause
                // comments are judged unconditionally (they never surface on
                // the for root): transparent ONLY trailing a named element
                // before its `;` terminator (`i := 0 /* c */ ;` — oracle
                // n1); the after-`;` gap (`; /* g */ i < n`) and the
                // post-update junction (`i++ /* j */ {`) refuse.
                let clause_head = named
                    .first()
                    .filter(|first| first.kind() == "for_clause");
                let lead_bad = comments.iter().any(|c| {
                    clause_head.is_some_and(|cl| c.end_byte() <= cl.start_byte())
                });
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
            // the condition before `)` (`n > 0 /* t */ )` — oracle n1,
            // A=`n > 0` clean); leading (`while /* w */ (`) and the
            // `)`→`{` junction refuse. (The mid-condition comment answers
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
                let next = children[idx + 1..].iter().find(|x| !is_trivia_kind(x.kind()));
                !(prev.is_some_and(|p| p.is_named()) && next.is_some_and(|n| n.kind() == ")"))
            })
        }
        "do_statement" => {
            // js/ts do: comments before the body refuse; post-body comments
            // (`} /* z */ while`) stay transparent. The body is the FIRST
            // named child (it directly follows `do`).
            comments
                .iter()
                .any(|c| named.first().is_some_and(|b| c.end_byte() <= b.start_byte()))
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

/// PASS 129 (128A-F6): go's `for a := 0; cond; update` clause wraps its
/// header elements in a named `for_clause` child, so clause-level comments
/// never surface on the for root — this predicate is consulted
/// UNCONDITIONALLY for every go for candidate. A clause comment is
/// transparent ONLY trailing a named element before its `;` terminator
/// (`i := 0 /* c */ ;`, sg n1); the after-`;` gap (`; /* g */ i < n`), the
/// head-leading 3-clause position (`for /* h */ i := 0`), and the
/// post-update junction (`i++ /* j */ {`) all refuse (oracle grid
/// /tmp/phase129, sg n0 each).
fn go_for_clause_comments_refused(clause: Option<Node>) -> bool {
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
    // post-update junction (`i++ /* j */ {`). Oracle grid /tmp/phase129.
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
        let next = children[idx + 1..].iter().find(|x| !is_trivia_kind(x.kind()));
        !(prev.is_some_and(|p| p.is_named()) && next.is_some_and(|n| n.kind() == ";"))
    })
}

fn walk_general<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    template: &GeneralTemplate,
    template_root: Node<'a>,
    lang: Language,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    // PASS 65a (F64-4): the guard is ANCESTOR-only — a string-rooted
    // template must match the string node itself, while everything nested
    // inside a comment/string ancestor stays skipped (calls inside `#{…}`
    // for other templates, string_content, …).
    if node.kind() == template.root_kind && !is_inside_comment_or_string(&node) {
        // PASS 129 (128A-F6, f129d): sg's loop-head comment discipline —
        // position-scoped refusals on trivia-carrying loop candidates
        // (junction comments, leading init/while comments, go/rs clause
        // zones). Clean and transparent-position candidates are untouched,
        // so the PASS 127 unwalled cells keep their bindings.
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
        // PASS 131 (130A-F3, f131b): sg's exact-children junction doctrine
        // covers the after-`>` gap of type-args-spelled calls — a trivia
        // child fully inside the typeargs→arguments gap refuses the
        // candidate (descent-only), whitespace-only gaps stay admitted.
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
    walk_children_general(node, source, pattern, template, template_root, lang, seen, out);
}

/// The descend-only tail of [`walk_general`], shared by the refused-candidate
/// path (a vetoed loop candidate still descends for nested candidates).
fn walk_children_general<'a>(
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
            // PASS 100 (FB-99A-1): tree-sitter marks in-file parse-error
            // children (ERROR, is_extra=true — verified against the
            // tree-sitter 0.26.13 binding) `extra`, and sg 0.45.2's Smart
            // strictness skips candidate extras during child alignment
            // (match_terminal returns SkipCandidate for a kind-mismatched
            // extra; should_skip_cand_for_metavar skips it under a meta
            // goal — match_tree/strictness.rs, match_node.rs). The
            // error-glued identifiers sg's own grammar recovers as
            // `identifier + ERROR($A) + …` (py/go/swift `µµµ$A + 1`,
            // `$$$A + 1`) therefore align on the NAMED children only and
            // the meta binds the identifier FRAGMENT (`µµµ`, `$$$`).
            // Keeping the extra here cost it an alignment slot: 4 candidate
            // children vs 3 template children → count mismatch → the row
            // went silently unanswered. The pattern side is NOT skipped —
            // sg never skips goal-side nodes under Smart (match_terminal's
            // skip_goal is false for Smart), so neither do we.
            .filter(|child| !child.is_extra())
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

fn if_body_matches(lang: Language, node: &Node, template: Option<&BodyTemplate>, body_braced: bool) -> bool {
    let Some(template) = template else {
        return true;
    };
    let Some(consequence) = if_consequence(node) else {
        return false;
    };
    // PASS 122 (F1, f122a): a BRACED pattern body (`{ $B }` / `{ $$$B }`) is
    // sg-STRUCTURAL — the candidate consequence must itself be a braced
    // block, or sg refuses the candidate (brace-less `if (c) d();` sites are
    // sg `[]` under braced patterns, oracle grid /tmp/phase122/f1: js/ts/c/
    // php all refuse). kt's `control_structure_body` wrapper counts as
    // braced exactly when its first non-trivia child is a block or the `{}`
    // tokens.
    if body_braced && !consequence_is_braced(&consequence) {
        return false;
    }
    match template {
        BodyTemplate::Any => true,
        BodyTemplate::Exactly(want) => {
            // PASS 124 (123B-F1, f124d): sg's py colon-suite rule —
            // `if $X: $B` answers EVERY python if and binds $B to the WHOLE
            // suite text regardless of statement count (oracle
            // /tmp/phase124/fpy: two/three-statement indented suites and the
            // one-line `a(); b()` suite all answer n1 with B = suite text;
            // the pre-124 Exactly(1) statement count refused each). The
            // f122e bare-colon EMPTY-suite pin is untouched: Exactly(0)
            // keeps the count comparison and refuses.
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

/// PASS 122 (F1): true when a candidate consequence node is a BRACED block.
/// Direct block kinds are braced by construction; a non-block wrapper (kt
/// `control_structure_body`) is braced exactly when its first non-trivia
/// child is `{` or a block kind.
fn consequence_is_braced(consequence: &Node) -> bool {
    if BLOCK_KINDS.contains(&consequence.kind()) {
        return true;
    }
    let mut cursor = consequence.walk();
    let children: Vec<Node> = consequence
        .children(&mut cursor)
        .filter(|child| !is_trivia_kind(child.kind()) && !child.is_extra())
        .collect();
    children.first().is_some_and(|first| {
        first.kind() == "{" || BLOCK_KINDS.contains(&first.kind())
    })
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
    // PASS 135 (134A-F7): the ts type_alias_declaration spells its
    // member-count body as the `value` field (the object_type) — the
    // `type-alias` Class face counts its members there.
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
    if node.kind() == "member_call_expression" || node.kind() == "nullsafe_member_call_expression" {
        // 86a-L5 (r37): the nullsafe spelling shares the object/name field
        // split; its segments serve the flat nullsafe slot faces (the
        // receiver-side `nullsafe_member_access` link still vetoes empty —
        // unresolvable-object discipline unchanged).
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
        // PASS 135 (134A-F5, f135d): a static-prop object (`C::$s->m()`,
        // `self::$s->m()`) decomposes into its scope::prop segments — the
        // same faithful two-segment head the scoped-call arm spells (grid
        // /tmp/phase135/cells gridE: `C::$s->$M();` sg n1 M=`m`, subject n0;
        // the concrete control `C::$s->m();` sg n1 stays aligned).
        "scoped_property_access_expression" => {
            let scope = node.child_by_field_name("scope")?;
            let prop = node.child_by_field_name("name")?;
            Some(vec![
                node_text(&scope, source)?.to_string(),
                node_text(&prop, source)?.to_string(),
            ])
        }
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
        // first named child. MoonBit apply/dot-apply calls are fieldless too. Safe
        // for other langs: C# uses `invocation_expression`, and field-bearing
        // grammars hit find_map first.
        .or_else(|| {
            matches!(
                node.kind(),
                "call_expression" | "apply_expression" | "dot_apply_expression"
            )
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

/// PASS 111 (110A-F1/F2/F3): sg's exact-children discipline at the matched
/// call node, in complement form: every child of the call node must end
/// at/before the resolved callee's end byte, or start at/after the argument
/// list's start byte, or be an UNNAMED token whose kind does not contain `?`.
/// Inter-token gap bytes belong to no child, so whitespace of any kind is
/// invisible to this scan — identical to sg's exact-children view of the same
/// tree. A named extra intersecting the open `(callee.end, arguments.start)`
/// gap (a `comment` child; a ts `type_arguments` sibling) or the anonymous
/// `?.` optional-call token breaks sg's match for LITERAL and META heads
/// alike (oracle grids 2026-09-08: `a?.(1)`, `a.b?.(1)`, `a?. /*c*/ (1)`,
/// `a /*c*/ (1)`, `a.b /*c*/ (1)`, `a<number>(1)`, `a.foo<number>(1)` all
/// stay empty even under `$F($X)` / `$F($$$A)`), while comments INSIDE the
/// argument list, INSIDE the member-chain callee, and OUTSIDE the call node
/// are transparent trivia sg answers. The unnamed exemption is a superset of
/// the bare `(`/`)` tokens several grammars hang directly off the call node
/// instead of the argument container (a named child in the gap never gets
/// the exemption — the first two clauses already refuse it). Unresolvable
/// callee or argument nodes skip the check (ruby receiver-bearing calls keep
/// the pass-15 posture). PASS 113 (112A-F2/112B-F1): this is THE one
/// junction rule — consulted from the capture path below AND from the lanes
/// that bypass it (`optional_call_matches`, `optional_chain_matches`,
/// `walk_calls`' slots arm), since the `simple_call` guard excludes
/// `?.`/type-args-spelled patterns and the slots arm never builds captures
/// through `capture_call_path`. NOT covered (the extra sits outside the
/// callee→arguments gap; registered, not fixed): java `a.<T>b(1)` — its
/// `type_arguments` precedes the `name` field.
fn call_junction_exact(node: &Node) -> bool {
    let Some(callee) = call_field_node(node) else {
        return true;
    };
    let Some(arguments) = argument_container(node, &["arguments"]) else {
        return true;
    };
    let mut cursor = node.walk();
    let exact = node.children(&mut cursor).all(|child| {
        child.end_byte() <= callee.end_byte()
            || child.start_byte() >= arguments.start_byte()
            || (!child.is_named() && !child.kind().contains('?'))
    });
    exact
}

/// PASS 144 (143A-F2): the cs callee→`(` junction gap law — the junction
/// bytes must be comment-free AND all [`is_sg_cs_trivia`]: a comment or a
/// Rust-ws outsider run (U+2028/U+2029) refuses (grid J1/J2/J7/J8 sg rc1)
/// while FEFF/NBSP/ASCII-ws gaps bind (J3/J4/J6 — sg's cs `\s` is Unicode
/// there). A zero-length gap trivially binds. Comments INSIDE the argument
/// list sit outside the junction and keep binding (J5).
fn cs_call_junction_trivia_free(node: &Node, source: &str) -> bool {
    let Some(callee) = call_field_node(node) else {
        return true;
    };
    let Some(arguments) = argument_container(node, &["arguments"]) else {
        return true;
    };
    let gap = &source[callee.end_byte()..arguments.start_byte()];
    strip_comment_spans(gap) == gap && gap.chars().all(is_sg_cs_trivia)
}

/// PASS 131 (130A-F3, f131b): the AFTER-`>` junction consult for
/// type-args-spelled calls. The general lane serves `g<T>($A)` faces
/// (`walk_calls`' simple-call gate excludes the typeargs spelling), so the
/// exact-children doctrine of [`call_junction_exact`] must be consulted
/// here too: a trivia child (comment extra) fully inside the
/// typeargs→arguments gap (`typeargs.end < child.start < arguments.start`)
/// breaks sg's structural match and the candidate is refused,
/// descent-only. A whitespace-only gap presents no child node and stays
/// admitted (oracle bws). Scoped to js/ts (the grammars whose
/// `type_arguments` spelling this lane serves), to templates whose root
/// carries a DIRECT type_arguments child, and to candidates carrying both
/// a type_arguments child and an arguments container.
fn ts_typeargs_junction_refused(
    lang: Language,
    template_root: Node,
    node: &Node,
    _source: &str,
) -> bool {
    if !matches!(lang, Language::TypeScript | Language::JavaScript) {
        return false;
    }
    let mut tcur = template_root.walk();
    if !template_root
        .children(&mut tcur)
        .any(|child| child.kind() == "type_arguments")
    {
        return false;
    }
    let mut ccur = node.walk();
    let children: Vec<Node> = node.children(&mut ccur).collect();
    let Some(typeargs) = children
        .iter()
        .find(|child| child.kind() == "type_arguments")
    else {
        return false;
    };
    let Some(arguments) = argument_container(node, &["arguments"]) else {
        return false;
    };
    children.iter().any(|child| {
        child.end_byte() > typeargs.end_byte() && child.start_byte() < arguments.start_byte()
    })
}

fn call_target_path(node: &Node, source: &str) -> Option<Vec<String>> {
    // MoonBit dot-apply calls: callee path is the object chain plus the
    // trailing accessor (`obj.method()` → `[obj, method]`).
    if node.kind() == "dot_apply_expression" {
        let mut segs = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "arguments" {
                break;
            }
            if let Some(accessor) = crate::extract::dot_accessor_text(&child, source) {
                segs.push(accessor.to_string());
                continue;
            }
            if let Some(mut path) = path_from_node(&child, source) {
                segs.append(&mut path);
            }
        }
        return (!segs.is_empty()).then_some(segs);
    }
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
/// PASS 107 (FB-106A-4): the swift/kotlin grammars wrap each member link in
/// a `navigation_suffix` node (`navigation_expression[a, navigation_suffix
/// [. b]]`, tree-dumped 0.7.3) — the suffix is part of sg's member-chain
/// decomposition, so the faithful resolver reads straight through it (its
/// named child contributes the next segment). The wrapper never hides a
/// `?.` connector: swift spells the optional link as a separate named `?`
/// SIBLING before the suffix, and the loop's `?`-kind veto below fires on
/// it — the `.`-template stays connector token-exact (sg: `a.b($A)` never
/// answers a `?.` site).
fn faithful_path_from_node(node: &Node, source: &str) -> Option<Vec<String>> {
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        return node_text(node, source).map(|t| vec![t.to_string()]);
    }
    if !is_member_expr_kind(node.kind()) && !is_navigation_suffix_kind(node.kind()) {
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
        // PASS 111 (110A-F4): sg's chain decomposition is comment-transparent
        // — `a /*c*/ .b(1)`, `a. /*c*/ b(1)`, `a /*c*/ . /*d*/ b(1)` and the
        // 3-link `a /*c*/ .b /*d*/ .c(1)` all answer `a.b(...)`/`a.b.c(...)`,
        // and a meta head binds the CLEAN identifier text (sg metaVariables:
        // `$A` = `a` at the identifier's byte range, probed 2026-09-08). Skip
        // trivia/extra children so comment-bearing chains stay on the
        // faithful arm instead of routing to the nonfaithful receiver slice
        // (mirroring `argument_nodes`' filters); the `?`-kind veto above runs
        // FIRST, so `a?. /*c*/ b(1)` keeps its registered refusal, and
        // non-member children (subscript/call receivers) still veto below.
        if is_trivia_kind(child.kind()) || child.is_extra() {
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
    if let Some(accessor) = crate::extract::dot_accessor_text(node, source) {
        return Some(vec![accessor.to_string()]);
    }
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
    // PASS 122 (F3, f122c): ANCESTOR-only trivia judgment. The old
    // self-inclusive `is_in_comment_or_string` excluded the string node
    // ITSELF, so a string-rooted literal pattern (`'q'`) silently answered
    // [] at every position where sg 0.45.2 answers the string node (oracle
    // grid /tmp/phase122/f3: init/arg/return/array faces js/ts/py all sg
    // n1, subject rc0 n0). Nodes nested UNDER a comment/string ancestor
    // stay skipped, so string/comment content never becomes matchable.
    if !is_inside_comment_or_string(&node) {
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
                    && literal_structural_eq(pat_root, &template.doc, node, source, true)
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
    if !node_text(node, source).is_some_and(|text| text.trim() == pattern) {
        return false;
    }
    // PASS 102 (F-101A-4, m4 oracle follow-up): innermost span wins. When a
    // DIRECT non-trivia CHILD's trimmed text also equals the pattern (the
    // co-extensive wrapper chain program > expression_statement > call on a
    // single-row file), this node is an outer shell — sg reports the
    // innermost node's span (m4 hit text `q(µAble) // c`, never the file
    // bytes with the trailing newline), and emitting every shell duplicated
    // each exact-text row at the search surface. The deepest co-text node
    // has no trim-equal child and is the one that pushes.
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
    // PASS 94b (FB-93A-4a): php literal faces wrap behind the `<?php ` tag —
    // the tag-less parse folds the document into a bare `text` node (or an
    // ERROR), which left every php literal pattern without a structural
    // comparator: `$x = $v + 1;` answered only through the exact-text arm
    // and missed the `$x = $v /* mid */ + 1;` candidate sg answers
    // (probes_run1.jsonl matrix C). The span covers only the pattern bytes,
    // so the statement root and comparison are tag-free; the exact-text
    // arms run in front of the structural comparator, so this only ever
    // ADDS matches.
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
            // PASS 102 (F-101A-4): a wrapper carrying ONE code child plus a
            // TRAILING comment run is the comment-carrying literal face sg
            // answers (`q(µAble) // c` — the comment is part of sg's
            // reported hit text, m4 oracle; js/ts attach the comment to the
            // statement). Root at the DEEPEST wrapper holding the comment
            // (the statement), so the comment children stay in the R3
            // comparison; leading-comment and multi-statement shapes keep
            // the `None` refusal (the registered multi-root posture).
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

/// PASS 102 (F-101A-4): admission of the meta-free WHOLE-TOKEN LITERAL
/// route's trailing line-comment faces (`q(µAble) // c`). sg 0.45.2 parses
/// the trailing comment as a real child of the pattern root and answers the
/// comment-carrying rows (m4 oracle: js/ts hit the identical AND the
/// whitespace-variant row); the subject census rc2'd the whole class. The
/// lane admits a face only when every leg of sg's observable class holds:
/// (a) the pattern carries no `$` (meta capability would reopen the
/// registered `$`-carrying placement refusals), (b) sg's parse gate accepts
/// the spelling in THIS language (go/rust/java/csharp rc8 — m4 matrix — and
/// keep the loud fold), (c) the pattern parses cleanly so the R3 comparator
/// exists (ruby `//`/python `/*` are operator soup — template-less faces
/// stay loud instead of trading rc2 for a whitespace-variant silent miss),
/// and (d) the root's tail is a line-comment run behind at least one code
/// child. Consumers: core's placement gate (the census carve) and the
/// per-language answerability consult.
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
        .all(|child| {
            node_text(child, &template.doc).is_some_and(|text| text.starts_with("//"))
        })
        && !children[..children.len() - comments]
            .iter()
            .any(|child| is_trivia_kind(child.kind()))
}

/// PASS 102 (F-101A-4): root-level alignment for a pattern-trailing
/// line-comment run. The code head aligns 1:1 (recursive R3, non-root), then
/// the candidate must carry the SAME number of trailing comments with EQUAL
/// node text (`// c` == `// c` — the whitespace between tokens is trivia,
/// which is why sg answers the two-space variant, m4). Leading/interior
/// pattern comments never reach here (the lane admission and the guard in
/// [`literal_structural_eq`] block them).
fn root_trailing_line_comments_eq(
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
/// pattern's matched root node — sg probes (pass 94b matrix C) show
/// candidate comments are transparent BELOW the matched root (`$x = $v
/// /* mid */ + 1;` answers: the comment sits inside the binary operand) but
/// still block as a DIRECT child of the matched root itself (the registered
/// T-B4 cell: a comment on the root call node refuses).
fn literal_structural_eq(
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
    // ROOT (T-B4 is that policy's mutation kill cell — the root-call
    // comment). Below the root, candidate comments are transparent
    // (FB-93A-4a/b, pass 94b): sg answers `$x = $v /* mid */ + 1;` for the
    // comment-free pattern, and the binary operand level is not a container.
    // Each half of the root guard alone is redundant defense — the
    // "comments invisible everywhere" R1 mutant (this guard AND the
    // container-only skip in `comparable_children` disabled together) flips
    // the T-B4 cell to a match; only the combined mutant is a valid
    // discrimination probe.
    if !container && has_comment_child(&p) {
        // PASS 102 (F-101A-4): at the matched ROOT, a pattern-trailing
        // line-comment run is a REQUIRED text-exact slot — sg's Smart
        // alignment matches the comment child one-for-one and answers the
        // comment-carrying row (m4 matrix: js/ts `q(µAble) // c`,
        // whitespace variants included). Interior/leading pattern comments
        // and non-root placements keep the registered block (the pass-60
        // container-slot contract and the T-B4 candidate guard are
        // untouched).
        // PASS 102 (F-101A-4): at the matched ROOT, a pattern-trailing
        // line-comment run is a REQUIRED text-exact slot — sg's Smart
        // alignment matches the comment child one-for-one and answers the
        // comment-carrying row (m4 matrix: js/ts `q(µAble) // c`,
        // whitespace variants included). Interior/leading pattern comments
        // and non-root placements keep the registered block (the pass-60
        // container-slot contract and the T-B4 candidate guard are
        // untouched).
        //
        // A successful trailing-run alignment IS the full root contract —
        // it already aligned the code head 1:1 (kinds, fields, texts) and
        // pinned the candidate comment tail — so it returns true here
        // rather than falling through to the T-B4 candidate-side guard
        // below, which would otherwise re-block the very comment child the
        // pattern demanded (the two-space `q(µAble)  // c` variant died
        // exactly there pre-fix).
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
    // Below the matched root, CANDIDATE-side comment children are invisible
    // at every node level (pass 94b matrix C: php mid-comment binaries and
    // go comment-carrying argument lists answer); the PATTERN side keeps the
    // container-only visibility (skip_trivia == container) so a pattern
    // comment stays the significant child it is registered to be.
    let mut p_children = comparable_children(p, container);
    let mut c_children = comparable_children(c, container || !is_root);
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
        if !literal_structural_eq(p_child.node, pattern_doc, c_child.node, source, false) {
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
        if !literal_structural_eq(p_child.node, pattern_doc, c_child.node, source, false) {
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

/// Deepest AST the extraction walk descends into. Pathological nesting
/// (`let x = ((((…1…))))`) made the walk superquadratic — 2.3 s at depth 500,
/// >90 s at depth 10000 (CNR H-CONF-024) — while real code stays far below
/// this bound. Subtrees beyond the cap are skipped and reported through
/// `ExtractionResult::depth_truncated` so the cached pattern lane can
/// refuse to serve those files as complete instead of failing open.
pub const MAX_EXTRACTION_DEPTH: usize = 256;

pub(crate) fn collect_pattern_nodes(root: Node, source: &str) -> (Vec<PatternNode>, bool) {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut depth_truncated = false;
    collect_node_signatures(root, source, &mut out, &mut seen, 0, &mut depth_truncated);
    (out, depth_truncated)
}

fn collect_node_signatures(
    node: Node,
    source: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
    depth: usize,
    depth_truncated: &mut bool,
) {
    if depth > MAX_EXTRACTION_DEPTH {
        *depth_truncated = true;
        // SEP15-3 (R-SEPT14E-2): decl rows stay budget-exempt — a node past
        // the depth bound still records its own declaration row (prefix +
        // name are direct-child reads) and recursion CONTINUES, so decl-exact
        // cached lanes are complete even for files that breached the budget.
        // ident/call rows past the budget stay unrecorded (incomplete by
        // contract: those shapes keep the walk/refusal paths under
        // truncation). Cost note (P-SEPT14E-2): the continued traversal pays
        // the same registered O(nodes×depth) class the native walk pays; it
        // only lands on files deeper than MAX_EXTRACTION_DEPTH.
        if declaration_prefix(&node, source).is_some() {
            record_node_signatures(&node, source, out, seen);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_node_signatures(child, source, out, seen, depth + 1, depth_truncated);
        }
        return;
    }
    if is_in_comment_or_string(&node) {
        // B1 (Sept 14 wave): comment/string subtrees hold no extractable
        // signatures. Recursing into them only burns depth budget and can
        // false-flag a file whose real code is shallow, so prune here.
        return;
    }
    record_node_signatures(&node, source, out, seen);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_signatures(child, source, out, seen, depth + 1, depth_truncated);
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
    ("struct_definition", "struct"),
    ("tuple_struct_definition", "struct"),
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
    ("mixin_declaration", "interface"),
    ("trait_definition", "interface"),
    ("enum_item", "enum"),
    ("enum_declaration", "enum"),
    ("enum_specifier", "enum"),
    ("enum_definition", "enum"),
    ("type_definition", "type"),
    ("local_function_declaration", "function"),
    ("impl_definition", "function"),
];

// e2hc/difu.5: invocation_expression is the C# tree-sitter grammar's call node.
// MoonBit calls are `apply_expression` (bare) / `dot_apply_expression` (`.m(...)`);
// the callee is the first named child in both.
const CALL_KINDS: &[&str] = &[
    "call_expression",
    "call",
    "method_invocation",
    "invocation_expression",
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
    "apply_expression",
    "dot_apply_expression",
];

fn is_call_kind(kind: &str) -> bool {
    CALL_KINDS.contains(&kind)
}

/// Full callee text for `call:` index rows. Split-field chains (java/php
/// `object`+`name`, ruby receiver-dot calls) have no single callee node, so
/// their text is reassembled from the resolved segments; every other grammar
/// keeps the callee node's exact source bytes.
fn call_target<'a>(node: &Node<'a>, source: &'a str) -> Option<Cow<'a, str>> {
    // MoonBit dot-apply calls carry the callee in a trailing accessor token.
    if node.kind() == "dot_apply_expression" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(text) = crate::extract::dot_accessor_text(&child, source) {
                return Some(Cow::Borrowed(text));
            }
        }
    }
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
    let Some(captures) = captures_for_node(node, source, pattern, name_text) else {
        // H-CONF-025 (pass 43): a repeated metavariable name is bound to two
        // different texts; sg unification semantics reject the candidate.
        return;
    };
    push_match_with_captures(node, source, pattern, captures, out);
}

/// Push a fully-built capture map (PASS 83a: the slot-matching lanes build
/// captures directly — the generic pattern-text path would mis-bind mixed
/// argument lists). Same byte-range dedup as the derived path.
fn push_match_with_captures(
    node: &Node,
    source: &str,
    pattern: &str,
    captures: BTreeMap<String, String>,
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
        bind_capture_kind(
            &mut captures,
            variable,
            name,
            declaration_head_is_multi_meta(pattern),
        )?;
    }
    // PASS 124 (F1, f124a): an if-prefixed pattern has NO argument template —
    // `pattern_argument_text` misreads the condition section (`if ($X) { $B }`
    // → `$X`) as one, `argument_container` descends into the condition's inner
    // call, and $X first binds the ARGUMENT text; `if_condition_capture` then
    // binds the whole condition text and the `bind_capture` conflict silently
    // dropped every candidate whose condition holds a call with >=1 argument
    // (js/ts/php + paren-spelled go/py; oracle grid /tmp/phase124/f1). Skip
    // the argument capture on the if lane — the same is_if_prefixed
    // special-case the body capture applies two arms below.
    if !is_if_prefixed(pattern.trim()) {
        capture_arguments(node, source, pattern, &mut captures)?;
    }
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

/// PASS 140 (I_ja_class_dollar): whether the declaration-head meta is
/// `$$$`-prefixed — the capture binds in the MULTI namespace (sg
/// `metaVariables.multi`); `$$`/`$` heads stay single (the PASS 75a law).
fn declaration_head_is_multi_meta(pattern: &str) -> bool {
    let (declaration, _) = strip_declaration_modifiers(pattern);
    DECL_PATTERN_PREFIXES
        .iter()
        .find_map(|(prefix, _)| {
            let rest = declaration.strip_prefix(prefix)?;
            let head = rest
                .split(|c: char| {
                    c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace()
                })
                .next()?;
            Some(head.trim().starts_with("$$$"))
        })
        .unwrap_or(false)
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
    // PASS 111 (110A-F1/F2/F3): sg's match is exact-children at the call
    // node — a `?.` optional-call token, a comment extra, or a
    // `type_arguments` sibling between the callee and the argument list
    // breaks the match for LITERAL and META heads alike, so refuse the
    // candidate here, before any segment binding. The `?.`- and
    // type-args-SPELLED patterns answer their aligned sites through their
    // dedicated lanes, which never reach this gate.
    if !call_junction_exact(node) {
        return None;
    }
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
    //
    // PASS 109B (CNR §39.2): that single-segment registered flattening is a
    // LIE for a literal segment — `last_identifier_in_chain` flattens the
    // subscript callee `a["b"]` to the tail identifier `a`, and a literal
    // segment binds nothing, so the flattened shape was accepted against
    // sg's empty answer set. A literal segment is accepted only when the
    // candidate's WHOLE callee text byte-equals it (sg matches a literal
    // callee against the exact identifier only); an unobtainable callee
    // text fails closed. Metavariable heads keep the registered
    // flattened-tail binding byte-exactly.
    let actual = if pattern_segments.len() >= 2 {
        match call_target_path_faithful(node, source) {
            Some(path) => path,
            None => return bind_nonfaithful_receiver(node, source, &pattern_segments, captures),
        }
    } else {
        if capture_name(pattern_segments[0]).is_none() {
            let Some(callee_node) = call_field_node(node) else {
                return None;
            };
            let Some(callee_text) = node_text(&callee_node, source) else {
                return None;
            };
            if callee_text != pattern_segments[0].trim() {
                return None;
            }
        }
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
/// which is exactly how sg rejects the same-name chains. PASS 109 (108A-F1):
/// a literal segment must byte-equal its text slice (head vs the receiver,
/// tail vs the chain-tail identifier) — sg answers `a.b($X)` with nothing on
/// `a[0].b(1)`. Split-callee grammars (java/php `object`+`name` fields,
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
    // Note: optional-link shapes (`a?.b`) never reach this arm — the path
    // resolvers' `?`-kind veto (PASS 65) refuses them before captures, so
    // the receiver slice is `?`-free by construction here (pinned by the
    // f107_swift_member_chain meta-receiver controls, oracle pass107).
    // PASS 109 (108A-F1): a non-metavariable segment must BYTE-EQUAL the
    // text it is accepted against — sg 0.45.2 answers `a.b($X)` with
    // NOTHING on `a[0].b(1)` / `a["x"].b(1)` (a subscript receiver is not
    // the literal identifier `a`), so a literal head vetoes the candidate
    // on receiver mismatch instead of binding through the empty capture
    // name (the pre-fix `unwrap_or_default()` accept was a fail-open).
    // Symmetrically the literal tail compares against the chain-tail
    // identifier. Metavariable segments keep the F62-3 whole-text binding
    // (`$O.b($Y)` answers with O = the receiver's literal text, oracle
    // grid probed 2026-09-08).
    match capture_name(pattern_segments[0]) {
        Some(name) => bind_capture(captures, name, receiver)?,
        None => {
            if receiver != pattern_segments[0].trim() {
                return None;
            }
        }
    }
    match capture_name(pattern_segments[1]) {
        Some(name) => bind_capture(captures, name, &tail_text)?,
        None => {
            if tail_text != pattern_segments[1].trim() {
                return None;
            }
        }
    }
    Some(())
}

/// The final identifier of a callee chain node: the node itself when it is
/// an identifier, else the last named child's own tail (member chains end in
/// the property identifier). PASS 107 (FB-106A-4): the swift
/// `navigation_suffix` wrapper is transparent here — its named child IS the
/// chain tail (see [`faithful_path_from_node`]); `?.` sites never reach
/// captures at all (the path resolvers' `?`-kind veto fires first).
fn chain_tail_identifier<'a>(node: &Node<'a>, source: &str) -> Option<(Node<'a>, String)> {
    if is_ident_kind(node.kind()) || KEYWORD_RECEIVER_KINDS.contains(&node.kind()) {
        let text = node_text(node, source)?.to_string();
        return Some((*node, text));
    }
    if !is_member_expr_kind(node.kind()) && !is_navigation_suffix_kind(node.kind()) {
        return None;
    }
    let mut cursor = node.walk();
    let last = node.named_children(&mut cursor).last()?;
    chain_tail_identifier(&last, source)
}

/// PASS 107 (FB-106A-4): the swift/kotlin `navigation_suffix` member-link
/// wrapper kind. Absent from [`MEMBER_EXPR_KINDS`] (that table also drives
/// index extraction), but part of sg's member-chain decomposition — the two
/// faithful resolvers above read through it.
fn is_navigation_suffix_kind(kind: &str) -> bool {
    kind == "navigation_suffix"
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


