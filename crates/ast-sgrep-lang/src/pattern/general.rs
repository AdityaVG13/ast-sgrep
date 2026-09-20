//! General structural lane: templates and eligibility.

use super::*;
use crate::extract::node_text;
use crate::Language;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// General structural lane for the duplicate-metavariable family beyond the
// three declaration shapes (operator chains, nested-call arguments, typed
// params, return/assignment bodies). The lane parses the
// METAVARIABLE-SUBSTITUTED pattern with the same tree-sitter grammar as the
// candidate and compares the two trees pairwise, binding metavariable
// leaves through `bind_capture` (a repeated name is one variable).
//
// Scope guardrails (fail-closed contract rows keep their loud rejection):
// - language-free text guards refuse `$$$` templates, multi-line tails,
//   comment syntax, and bare-keyword heads outside `DECL_PATTERN_PREFIXES`;
// - the template root must be an expression / call / known-declaration kind;
//   `if` templates stay in the dedicated If lane;
// - a pattern no grammar parses cleanly (ERROR/missing nodes) is unsupported;
// - a metavariable substituted INSIDE a string/comment/regex literal is
//   unsupported (treated as literal text there).
// ---------------------------------------------------------------------------

/// Placeholder prefix substituted for `$NAME` in general-lane pattern docs.
/// Underscore-led so it parses as an identifier in all 13 indexed grammars.
pub(crate) const GENERAL_MV_PREFIX: &str = "__asgrep_mv_";

/// Language-free eligibility guards for the general structural lane.
/// Comment syntax is NOT refused here — `#` is comment syntax ONLY in
/// python/ruby/php, so a language-free `#` refusal failed rust-attribute /
/// C-preprocessor / swift-`#selector` / js-private-field patterns closed.
/// The per-language builders and [`native_pattern_answerable`] apply
/// [`lane_comment_refused`] instead.
/// The nested-call rest-argument family: a call-shaped pattern where every
/// argument list holds only single metavariables or nested calls of the
/// same restricted shape, plus lists that are exactly one `$$$` rest.
/// Literal atoms are NOT in the family, and a `$$$` rest may never share
/// its list with a sibling. Paths spell `.` and `::` separators with
/// identifier/`$meta` segments.
pub(crate) fn nested_call_rest_template(pattern: &str) -> bool {
    let p = pattern.trim();
    if !p.contains('$') || !p.contains('(') {
        return false;
    }
    // A `new`-led call head is the same family shape — the reference answers
    // `new q($$$A)` like the plain call; strip the constructor keyword and
    // parse the family call identically. Non-js/ts `new` heads still refuse
    // later in the eligibility gate (the head admission is language-scoped),
    // so this strip is inert for them.
    let p = p
        .strip_prefix("new ")
        .filter(|rest| {
            rest.starts_with(char::is_alphabetic) || rest.starts_with('_') || rest.starts_with('$')
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

pub(crate) fn parse_family_call(b: &[u8], i: &mut usize) -> bool {
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
                // A canonical 2-dollar name is a family argument exactly like
                // `$NAME` (sole-or-sibling, single capture); a malformed run
                // (`$$`, `$$3`, `$$x`) stays out.
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

pub(crate) fn general_lane_text_eligible(pattern: &str) -> bool {
    // The language-free gate is the UNION of the per-language admissions —
    // "does ANY grammar build a template?" The head admissions are
    // language-scoped (`new` js/ts, `lambda` py), so the union ORs the js
    // base with the py evaluation LIMITED to lambda-headed patterns — the
    // py comment rules (`//` as floor-div) must NOT widen the gate for
    // comment-carrying spellings (the registered `return $B// noteA`
    // fail-closed rows stay refused). Without this union the ingress
    // census rejected `new`/`lambda` faces before the per-file walk could
    // answer them.
    general_lane_text_eligible_for(Language::JavaScript, pattern)
        || (pattern.split_whitespace().next() == Some("lambda")
            && general_lane_text_eligible_for(Language::Python, pattern))
        // The loop/control heads whose owning grammar is NOT the js base —
        // the same union discipline (each arm is limited to the head's
        // covered language so no language's comment/lexical rules leak into
        // the gate).
        || match pattern.split_whitespace().next() {
            Some("for") => general_lane_text_eligible_for(Language::Go, pattern)
                || general_lane_text_eligible_for(Language::Rust, pattern),
            Some("while") => general_lane_text_eligible_for(Language::Rust, pattern),
            Some("loop") => general_lane_text_eligible_for(Language::Rust, pattern),
            Some("not") => general_lane_text_eligible_for(Language::Python, pattern),
            // The per-language head admissions carry the csharp `lock`/
            // `using` and python `del` statement heads, but the language-free
            // UNION never did — the fallback gate kept refusing faces the
            // reference answers (loud before the walk whose match_pattern
            // answers each face exactly; `with`/`delete`/`void`/`var` never
            // hit this because the js base arm already admits them). Union
            // discipline: each arm limited to the owning grammar so no other
            // language's comment/lexical rules leak into the gate.
            Some("lock") => general_lane_text_eligible_for(Language::CSharp, pattern),
            // Cpp joins the `using` head — `using namespace $N;` binds N.
            // The per-language eligible gates keep the grammars'
            // comment/lexical rules separate.
            Some("using") => {
                general_lane_text_eligible_for(Language::CSharp, pattern)
                    || general_lane_text_eligible_for(Language::Cpp, pattern)
            }
            Some("del") => general_lane_text_eligible_for(Language::Python, pattern),
            // The java `assert`/`synchronized` statement heads and the php
            // `namespace`/`goto` heads — each per-language admitted, none
            // carried in this union, so the ingress refused faces the
            // reference answers.
            Some("assert") | Some("synchronized") => {
                general_lane_text_eligible_for(Language::Java, pattern)
            }
            Some("namespace") | Some("goto") => {
                general_lane_text_eligible_for(Language::Php, pattern)
            }
            _ => false,
        }
        // The ruby modifier statements (`x if $C` family — the modifier
        // keyword is the PENULTIMATE token) and the braced BEGIN/END block
        // heads. Union discipline: each arm is limited to the owning grammar,
        // so no other language's rules leak into the gate.
        || (ruby_modifier_statement_pattern(
            pattern,
            pattern.split_whitespace().next().unwrap_or(""),
        ) && general_lane_text_eligible_for(Language::Ruby, pattern))
        || (matches!(pattern.split_whitespace().next(), Some("BEGIN") | Some("END"))
            && general_lane_text_eligible_for(Language::Ruby, pattern))
}

/// True when a bare-identifier head continues with a BINARY OPERATOR
/// (`x * q($A)`) — an ordinary expression face the reference answers
/// through its binary parse, not declaration-keyword territory. The
/// php member connector `->` is excluded: `->`-spelled callees classify into
/// the dedicated member-call lane before the general lane is ever consulted.
/// The template build itself still decides admission (clean parse, general
/// root kind, span coverage), so a merely-admitted spelling whose build
/// refuses keeps its registered loud class byte-for-byte.
pub(crate) fn bare_ident_operator_continuation(after: &str) -> bool {
    let Some(first) = after.chars().next() else {
        return false;
    };
    if matches!(
        first,
        '+' | '-' | '*' | '/' | '%' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?'
    ) {
        // `->` is a member connector, never a binary continuation.
        return !after.starts_with("->");
    }
    // Go's short-variable declaration continuation (`x := $Y`) — the `:=`
    // token is assignment syntax, not a labeled-statement colon; only the
    // tight two-byte spelling admits.
    if after.starts_with(":=") {
        return true;
    }
    false
}

/// Language-aware eligibility — `py` selects the python comment
/// judgment (`//` is python's FLOOR-DIV operator, not comment syntax; the
/// reference answers `$A // $B` {1,2,2,3}, `$A // 2` {3} on the floor-div
/// fixture). The lang-free ingress ([`general_lane_text_eligible`]) keeps
/// the conservative line-comment refusal.
pub(crate) fn general_lane_text_eligible_for(lang: Language, pattern: &str) -> bool {
    let py = lang == Language::Python;
    let p = pattern.trim();
    if p.contains('\n') || p.contains(GENERAL_MV_PREFIX) {
        // Statement templates the reference matches LAYOUT-INSENSITIVELY are
        // exempt from the newline blanket — csharp lock/using statement
        // templates (multi-line bodies bind) and java synchronized blocks.
        // The build's root gate keeps the admission exact.
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
    // The blanket `$$$` refusal narrows to its registered core. The probed
    // nested-call rest family — call-shaped patterns whose argument lists
    // hold only metavariables and nested calls, with `$$$` as the SOLE
    // argument of its list — templates like any other structural shape
    // (`g(fetch($$$A))` answers all arities). Everything else carrying
    // `$$$` (flat mixed lists, rest + sibling, statement templates) keeps
    // the loud fail-closed contract.
    if p.contains("$$$") && !nested_call_rest_template(p) {
        return false;
    }
    // `//`-`/*` comment syntax is refused only OUTSIDE string-literal quotes
    // — the `//` in `parse($U, "https://default")` is URL content, not a
    // comment, and the old byte scan failed such patterns closed where the
    // reference answered. The per-language template builder adds the precise
    // judgment (clean parse, allowed root, span coverage).
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
    // Lowercase statement heads the reference parses as statements are
    // native templates (raise/yield/throw/await answer; break/continue
    // answer `$`-less). `new`-led and python-`lambda` expression heads plus
    // the `arrow_function` root kind are admitted — bare-param arrows
    // answer too.
    if let Some(first) = p.split_whitespace().next() {
        let bare_keyword = first.chars().all(|c| c.is_ascii_alphabetic());
        // Answerable expression heads the bare-keyword guard would refuse
        // as declaration-keyword territory: `new`-led constructor
        // expressions (js+ts) and python `lambda` roots (py-only spelling),
        // `async`-led arrows and the `let`/`const`/`var` declaration heads
        // (js+ts; rust's `let` build roots at `let_statement`, which the
        // root-kind gate still refuses, so its census stays loud there).
        let admitted_expression_head = (matches!(
            lang,
            Language::JavaScript | Language::TypeScript
        ) && matches!(first, "new" | "async" | "let" | "const" | "var"))
            || (lang == Language::Python && first == "lambda")
            // The loop/control heads the reference answers per grammar
            // (js/ts `for`/`while`/`do`; py `for`/`while`/`not`; go `for`;
            // rust `for`/`while`/`loop`; php `foreach`/`while`/`do`), plus
            // js `typeof`. Language-scoped on purpose: unprobed heads keep
            // their census class, and the template build + gate still
            // decide admission per file.
            || match first {
                // python `for`/`while` are deliberately ABSENT: the
                // reference bindings bind the WHOLE `:`-suite text
                // (B = "a()\n    b()") — a binding the general lane's
                // kind-exact child alignment cannot express (the inline
                // suite shape differs from the candidate block). Admitting
                // them would walk silent-empty (silent UNDER-answer), the
                // worst class — they stay census-loud by construction.
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
                // rb `not $X` is answered — the same unary-family face py
                // already admits; rb needs the plain `unary` root kind
                // (admitted in `is_general_root_kind`).
                "not" => matches!(lang, Language::Python | Language::Ruby),
                "typeof" => lang == Language::JavaScript,
                _ => false,
            };
        // Ruby statement shapes the bare-keyword head guard would refuse —
        // the modifier statements (`x if $C` family) and the braced BEGIN/END
        // block roots (`BEGIN { $B }`). The build + root-kind gate still
        // decide admission per file.
        let admitted_expression_head = admitted_expression_head
            || (lang == Language::Ruby
                && (matches!(first, "BEGIN" | "END")
                    || ruby_modifier_statement_pattern(p, first)))
            // The statement heads whose bindings the general lane expresses
            // per grammar — csharp `lock`/`using` statements and declarations
            // plus `var` locals, js `with` and the unary `delete`/`void`,
            // python `del`. The build + root kind gates still decide
            // admission per file.
            || (lang == Language::CSharp && matches!(first, "lock" | "using" | "var"))
            || (lang == Language::JavaScript && matches!(first, "with" | "delete" | "void"))
            || (lang == Language::Python && first == "del")
            // The language-scoped head admissions — js/ts `export` (export
            // default $X; → X=`42`), java `assert`/`synchronized` (assert $X
            // : $M; binds X/M; synchronized ($X) binds X), cpp `using`
            // (using namespace $N; → N=`std`), php `namespace`/`goto`
            // (namespace $N; → N=`App`; goto $L; → L=`a`). The template
            // build + gate still decide admission per file (union
            // discipline: each arm limited to the owning grammar).
            || (matches!(lang, Language::JavaScript | Language::TypeScript) && first == "export")
            || (lang == Language::Java && matches!(first, "assert" | "synchronized"))
            || (lang == Language::Cpp && first == "using")
            || (lang == Language::Php && matches!(first, "namespace" | "goto"));
        if bare_keyword
            && !admitted_expression_head
            && !DECL_PATTERN_PREFIXES
                .iter()
                .any(|(prefix, _)| prefix.trim() == first)
            && !STATEMENT_HEAD_KEYWORDS.contains(&first)
        {
            // An ASSIGNMENT head (`name = "user-#{$N}"`) is an ordinary
            // identifier, not declaration-keyword territory — the reference
            // answers the ruby string-interpolation face. The `let` shapes
            // keep their registered fail-closed contract at the root-kind
            // gate (`is_general_root_kind` refuses let kinds), and
            // uppercase/other bare heads (no `=`) stay refused here.
            // An OPERATOR continuation is ordinary binary-expression
            // territory, not declaration-keyword territory either — the
            // reference answers `x * q($A)` on every ident-LHS row while
            // the bare-keyword guard refused the whole class.
            let after = p[first.len()..].trim_start();
            if !after.starts_with('=') && !bare_ident_operator_continuation(after) {
                return false;
            }
        }
    }
    true
}

/// Lowercase statement keywords the general lane templates at statement root
/// (including go's `defer`/`go`).
pub(crate) const STATEMENT_HEAD_KEYWORDS: &[&str] = &[
    "return", "raise", "yield", "throw", "await", "break", "continue", "defer", "go",
];

/// A ruby MODIFIER statement tail — the pattern's second-to-last whitespace
/// token is a modifier keyword and the head is an ordinary expression head
/// (`x if $C`, `x unless $C`, `x while $C`, `x until $C`; the reference binds
/// C). The 3-token floor keeps the bare `if $C` end-root face out.
pub(crate) fn ruby_modifier_statement_pattern(p: &str, head: &str) -> bool {
    let tokens: Vec<&str> = p.split_whitespace().collect();
    tokens.len() >= 3
        && !matches!(head, "if" | "unless" | "while" | "until")
        && matches!(
            tokens[tokens.len() - 2],
            "if" | "unless" | "while" | "until"
        )
}

/// True when `#` appears OUTSIDE string-literal quotes. Quote state scanned
/// with escape handling, exactly like the `//`-`/*` scan below; unterminated
/// quotes make the tail in-string (conservative).
pub(crate) fn contains_hash_outside_strings(p: &str) -> bool {
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

/// The per-language comment-syntax refusal. `#` is comment syntax only in
/// the hash-comment languages (python/ruby/php); in rust (`#[derive]`,
/// `#![allow]`, `r#"…"#`), C/C++ (`#include`, `#define`), swift (`#selector`),
/// and JS/TS (`this.#x`) it is real syntax and must stay templatable. The
/// registered python/ruby comment-glued faces keep their fail-closed contract
/// through this same guard.
pub(crate) fn lane_comment_refused(lang: Language, pattern: &str) -> bool {
    if matches!(lang, Language::Python | Language::Ruby | Language::Php)
        && contains_hash_outside_strings(pattern.trim())
    {
        return true;
    }
    // `//` is python's FLOOR-DIV operator, not comment syntax — the
    // language-free arm refused `$A // $B` where the reference answers it.
    // For python only `/*` is foreign comment syntax; the registered py
    // `#`-comment faces keep their refusal through the hash arm above. Ruby
    // keeps the `//` refusal (its comment char is `#` too and no answering
    // `//` face is registered there); php and every C-family language spell
    // real line comments with `//`.
    if lang == Language::Python {
        return contains_block_comment_syntax_outside_strings(pattern.trim());
    }
    contains_comment_syntax_outside_strings(pattern.trim())
}

/// The `/*`-only half of [`contains_comment_syntax_outside_strings`] — true
/// when a block-comment opener appears outside string-literal quotes (same
/// quote scan).
pub(crate) fn contains_block_comment_syntax_outside_strings(p: &str) -> bool {
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

/// True when some quote-external `#` starts a tail that carries no code
/// punctuation at all — a trailing comment glue (`foo($A) # note`), never
/// leading syntax (`#[derive($A)]`, `#include $X` — their remainders contain
/// `[`/`$`), never a mid-chain private field (`this.#x = $V` — `=` follows),
/// never a raw-string delimiter (`tag(r#"$A"#)` — a quote follows).
pub(crate) fn contains_comment_glued_hash(pattern: &str) -> bool {
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
                        b'(' | b')'
                            | b'['
                            | b']'
                            | b'{'
                            | b'}'
                            | b'='
                            | b'.'
                            | b'"'
                            | b'\''
                            | b'$'
                            | b'/'
                            | b'*'
                            | b'<'
                            | b'>'
                            | b'!'
                            | b'?'
                            | b';'
                            | b':'
                            | b','
                            | b'&'
                            | b'|'
                            | b'^'
                            | b'~'
                            | b'+'
                            | b'-'
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
/// syntax, and the parse-level template checks govern the rest. The `#`
/// arm moved to the language-aware [`lane_comment_refused`] — a
/// language-free `#` refusal failed rust/c/swift/js `#`-syntax patterns
/// closed where the reference answers.
pub(crate) fn contains_comment_syntax_outside_strings(p: &str) -> bool {
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
/// Every `$`-run must be a canonical metavariable (`$NAME` / `$$$NAME`,
/// ASCII `[A-Z_][A-Za-z0-9_]*`); any other `$` shape (`$3.14`, `$a`, bare
/// `$`) refuses substitution entirely so the general lane can never template
/// — let alone wildcard-match — a pattern the grammar classifies as
/// universal / NeverMatches / loud-reject. A canonical `$$NAME` run is NOT
/// in the refusal set — it substitutes exactly like `$NAME` into the single
/// namespace (`(1..=3)`-dollar arm below), matching the reference capture
/// key; only `$$$NAME` lands in the multi namespace.
pub(crate) fn substitute_general_metavariables(
    pattern: &str,
) -> Option<(String, BTreeMap<String, String>, BTreeSet<String>)> {
    let mut out = String::with_capacity(pattern.len() + 32);
    let mut placeholders = BTreeMap::new();
    // Names spelled `$$$NAME` — the sole-rest arm of `general_eq` binds
    // these in the MULTI namespace over the whole argument list.
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
            // A canonical `$$NAME` substitutes exactly like `$NAME` (same
            // single-namespace placeholder); only `$$$NAME` lands in the
            // multi namespace.
            .filter(|_| (1..=3).contains(&dollars));
        {
            let name = canonical?;
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
        }
    }
    Some((out, placeholders, multi_names))
}

/// A built general-lane pattern: the substituted document, its tree, and the
/// metavariable map. Cheap to clone (`Tree` clone is reference-counted).
#[derive(Clone)]
pub(crate) struct GeneralTemplate {
    pub(crate) doc: String,
    pub(crate) tree: tree_sitter::Tree,
    pub(crate) placeholders: BTreeMap<String, String>,
    /// Metavariable NAMES spelled `$$$NAME` in the source pattern — the
    /// sole-rest arm of [`general_eq`] binds these over the whole aligned
    /// argument list (multi namespace).
    pub(crate) multi_names: BTreeSet<String>,
    /// Byte span of the substituted pattern inside `doc` (context wraps only).
    pub(crate) span: Option<(usize, usize)>,
    /// The raw pattern spelled a trailing `;`. The reference keeps a
    /// pattern-trailing `;` significant — the pattern root is the STATEMENT
    /// and only bare `expr;` statements answer — so on the plain span-less
    /// builds the root resolution stops AT `expression_statement` instead of
    /// unwrapping it. The php `<?php `-wrapped builds (span Some) keep their
    /// registered unwrapped behavior.
    pub(crate) had_semi: bool,
    /// Resolved template root kind (candidate prefilter).
    pub(crate) root_kind: String,
    /// The csharp meta-BODY lock/using templates are ACCEPTED faces that
    /// bind NOTHING (valid-empty on every candidate: `lock ($X) { $B }`
    /// answers `[]` with the `lock (o) { x(); }` candidate present). The old
    /// build refusal composed the loud census class there — loud where the
    /// reference answers ok:true-0. The template now BUILDS with this flag
    /// set; [`match_structural_general`] answers empty without touching
    /// the census.
    pub(crate) force_empty: bool,
}

/// Per-language context wrapping for patterns that are expressions (the
/// top-level grammar only accepts items). The substituted pattern text sits
/// exactly at byte span `[prefix.len(), prefix.len() + len)` in the doc.
pub(crate) fn general_expression_context(lang: Language) -> Option<(&'static str, &'static str)> {
    Some(match lang {
        // Java and rust statements demand the terminating `;` —
        // expression/let/return templates only parse (and only align their
        // statement children with candidate sources) as terminated
        // statements.
        Language::Rust => ("fn __asgrep_ctx() { ", "; }"),
        Language::Go => ("func __asgrep_ctx() { ", " }"),
        Language::Java => ("class __AsgrepCtx { void __m() { ", "; } }"),
        // C# statement heads demand the terminator exactly like java —
        // `throw $A` / `await $A` only parse (and only align their statement
        // children with candidate sources) as terminated statements inside
        // the method body.
        Language::CSharp => ("class __AsgrepCtx { void M() { ", "; } }"),
        // C/C++ statements demand the terminating `;` — `return $A` only
        // parses (and only aligns its statement children with candidate
        // sources) as a terminated statement.
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
/// child only). `translation_unit` joins for the C/C++ preprocessor faces —
/// `#include $X` parses bare at a TU root whose single child is the preproc
/// node.
pub(crate) const GENERAL_WRAPPER_KINDS: &[&str] = &[
    "module",
    "program",
    "source_file",
    "expression_statement",
    "translation_unit",
    // Go wraps a context-built single statement in `block > statement_list`;
    // the template root must descend through the single-statement
    // statement_list or the structural comparison roots at the wrapper (kind
    // never matches a candidate call node, so go literal calls only ever
    // answered through the exact-text arm — `f(1, 2)` missed the
    // `f(1, /* n */ 2)` candidate the reference answers).
    "statement_list",
];

/// Allowed template root kinds for the general lane: expressions, calls, and
/// known declaration kinds. `if` templates stay in the dedicated If lane, and
/// let/bindings keep their registered fail-closed contract.
/// `statement_root` admits the `expression_statement` root for had_semi
/// patterns on the plain builds (the reference roots the `;`-terminated
/// spelling at the statement — only bare `expr;` statements answer).
pub(crate) fn is_general_root_kind(kind: &str, statement_root: bool) -> bool {
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
        // Statement-root heads (`return $A`) template to return_statement
        // roots; rust templates to return_expression via the `_expression`
        // arm above.
        || kind == "return_statement"
        // The statement-head family templates to these roots (python
        // raise_statement / yield, ts+java+js throw_statement, js
        // break/continue_statement) plus the extended family — python
        // `await` (kind `await`), go `defer`/`go` statements.
        || matches!(
            kind,
            "raise_statement" | "yield" | "yield_statement" | "throw_statement"
                | "break_statement" | "continue_statement" | "await" | "defer_statement"
                | "go_statement"
        )
        // Ruby `raise x` / `yield x` are receiver-less command calls (kind
        // `command`); the head guard already restricts which patterns may
        // lead, this only admits their template root.
        || kind == "command"
        // `#`-syntax faces template to these roots — rust attributes (outer
        // + inner) and C/C++ preprocessor lines.
        || matches!(
            kind,
            "attribute_item" | "inner_attribute_item" | "preproc_include" | "preproc_def"
                | "preproc_function_def" | "preproc_call"
        )
        || matches!(kind, "attribute" | "field_expression")
        // Answerable expression roots — the js/ts arrow root (paren-spelled
        // `($X) => $Y` / `($A, $B) => q($A, $B)` / `() => q($X)` AND the
        // bare-param `x => q($X)` / `$X => q($Y)` spellings — all probed
        // HITS) and the python `lambda` root (`lambda $X: $Y` family).
        // `new_expression` already rides the `_expression` arm. The
        // tagged-template root keeps the registered refusal upstream.
        || kind == "arrow_function"
        || kind == "lambda"
        // Reference-answering root kinds whose walk machinery (kind-exact
        // `general_eq` with `bind_capture` leaf unification) already exists
        // — the js/ts generator root (`function* q($X) { $$$B }` / `{ $B }`),
        // the js/ts `lexical_declaration`/`variable_declaration` declarator
        // roots (`let/const/var $A = $B`; rust `let_statement` is NOT here,
        // keeping the rust census-loud class), and the collection-literal
        // roots `array` + `object` (`[$A, $B]`; `{ a: $A }`). The walk is
        // kind-exact, so a template can only ever match the identical
        // candidate kind.
        || kind == "generator_function_declaration"
        || kind == "lexical_declaration"
        || kind == "variable_declaration"
        // Collection-literal roots per grammar: js/ts `array`/`object`,
        // python `list` (the reference answers `[$A, $B]`), swift
        // `array_literal` (likewise).
        || kind == "array"
        || kind == "object"
        || kind == "list"
        || kind == "array_literal"
        // Ruby's binary expression node kind is plain `binary` — the only
        // grammar whose binary root carries neither `_expression` nor
        // `_operator`. Without this arm every ruby operator-continuation
        // template refused the build. Leaf mismatches still answer empty.
        || kind == "binary"
        // A string template root answers only when every metavariable sits
        // inside a `#{…}` INTERPOLATION subtree — placeholders in PLAIN
        // string content still refuse the build.
        || kind == "string"
        // Answering loop/for-family template roots whose walk machinery
        // (kind-exact `general_eq`) already exists — js/ts `for_statement`
        // (+ `for_in_statement`), `while_statement`, `do_statement`; python
        // `for`/`while`; go `for_statement` (all head forms); php
        // `foreach`/`while`/`do`; rust `loop`/`for`/`while` expressions
        // (rust spellings carry no `let` substring, so the rust
        // `let_statement` census refusal cannot leak in). The walk is
        // kind-exact, so a template only matches the identical candidate
        // kind. Still REFUSED upstream: `$$$`-body statement templates and
        // python faces whose `:`-suite alignment the general lane cannot
        // express.
        || matches!(
            kind,
            "for_statement" | "for_in_statement" | "while_statement" | "do_statement"
                | "foreach_statement"
        )
        // Go's short-variable declaration root — the `:=` head-continuation
        // admission lives in [`bare_ident_operator_continuation`].
        || kind == "short_var_declaration"
        // Ruby modifier statement roots (`x if $C` → if_modifier etc.) and
        // the braced BEGIN/END block roots (`BEGIN { $B }`). Kind-exact:
        // only identical candidate kinds align. The multi-line `begin …
        // end while` face stays registered-loud (the newline eligibility
        // refusal).
        || matches!(
            kind,
            "if_modifier" | "unless_modifier" | "while_modifier" | "until_modifier"
                | "begin_block" | "end_block"
        )
        // Ruby's `not` unary root kind. py's not_operator rides the
        // `_operator` arm; rb's plain `unary` needed its own admission.
        || kind == "unary"
        // Ruby's return-statement root kind is plain `return` (the only
        // covered grammar not spelling `return_statement`). Kind-exact: only
        // a candidate `return` node can match.
        || kind == "return"
        // Answering statement roots whose walk machinery (kind-exact
        // `general_eq`) already exists. csharp lock/using statements answer
        // layout-insensitively — meta-carrying BODY spellings stay refused
        // at the build (parse errors) via the csharp statement-body veto;
        // the using DECLARATION root binds T/N/E. py `delete_statement`,
        // js `with_statement`, the labeled-statement roots (mismatched
        // labels refuse through the bind-capture conflict), and the ts
        // `type_alias_declaration` root for non-object aliases (the
        // OBJECT-body alias routes through the Class member-count lane,
        // where the B capture lands on the member, not the braces).
        || matches!(
            kind,
            "lock_statement" | "using_statement" | "local_declaration_statement"
                | "delete_statement" | "with_statement" | "labeled_statement"
                | "type_alias_declaration"
        )
        // The language-scoped statement-root admissions for the head faces
        // above — js/ts export, java assert/synchronized, cpp
        // using_declaration, php namespace_definition + goto_statement.
        // Kind-exact: only identical candidate kinds align.
        || matches!(
            kind,
            "export_statement" | "assert_statement" | "synchronized_statement"
                | "using_declaration" | "namespace_definition" | "goto_statement"
        )
}

pub(crate) fn general_template_root<'a>(template: &'a GeneralTemplate) -> Option<Node<'a>> {
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
        // A pattern-trailing `;` is reference-significant — the pattern
        // root is the statement (kind-exact: only bare `expr;` statements
        // answer), so the plain span-less builds stop HERE.
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

pub(crate) fn pattern_has_placeholder_in_literal(
    node: &Node,
    doc: &str,
    placeholders: &BTreeMap<String, String>,
) -> bool {
    let kind = node.kind();
    // A placeholder inside a string INTERPOLATION is a metavariable hole,
    // not literal text — the reference binds it (`"user-#{$N}"` answers with
    // N = the interpolation content). The interpolation subtree is exempt
    // from the scan; plain string content keeps the literal-metavariable
    // refusal.
    if kind == "interpolation" {
        return false;
    }
    // A raw string literal (`r#"$A"#`) is TEXT to the reference — its
    // expando-replaced pattern still carries the placeholder bytes inside
    // the raw string, and the reference answers accepted-empty on such
    // faces, not a refusal. Exempting the literal AND its string_content
    // child lets the template build; the leaf text comparison then answers
    // match-none exactly like the reference (the placeholder bytes never
    // appear in a real source raw string). Ordinary string literals keep
    // the literal-metavariable refusal.
    let exempt = kind == "raw_string_literal"
        || (kind == "string_content"
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == "raw_string_literal"));
    if !exempt
        && (kind.contains("string") || kind.contains("comment") || kind.contains("regex"))
        && node_text(node, doc).is_some_and(|text| text.contains(GENERAL_MV_PREFIX))
    {
        // The placeholder may reach a string node ONLY through interpolation
        // children (exempted below) — when every placeholder occurrence
        // inside this node sits in an interpolation subtree, the node itself
        // is container syntax, not literal text, and must not refuse. Plain
        // string content keeps the literal-metavariable refusal.
        // EXCEPT the WHOLE-CONTENT meta hole — a `string_content` leaf whose
        // ENTIRE text is a meta token IS a metavariable and binds the
        // candidate content. When every placeholder-bearing leaf of the
        // string node IS exactly a placeholder, the build is admitted;
        // prefix/suffix spellings (`"pre-$A"`) keep the refusal — those
        // leaves read as literal text — and comment/regex nodes never
        // qualify.
        let string_node = kind.contains("string");
        if !placeholders_only_inside_interpolations(node, doc)
            && !(string_node && placeholders_are_whole_string_content(node, doc, placeholders))
        {
            return true;
        }
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| pattern_has_placeholder_in_literal(&child, doc, placeholders));
    found
}

/// True when every leaf of this string node whose text carries the
/// placeholder prefix IS exactly a placeholder key — i.e. the string's
/// content is a bare metavariable hole (`"$A"`), the reference-truth meta
/// binding. Any placeholder that is a PROPER SUBSTRING of a content leaf
/// (`"pre-$A"`, `"$A-$B"`) makes this false and keeps the literal-text
/// refusal.
pub(crate) fn placeholders_are_whole_string_content(
    node: &Node,
    doc: &str,
    placeholders: &BTreeMap<String, String>,
) -> bool {
    let mut whole = true;
    fn scan(node: &Node, doc: &str, placeholders: &BTreeMap<String, String>, whole: &mut bool) {
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
/// interpolation node (the only string context the reference treats as a
/// metavariable hole).
pub(crate) fn placeholders_only_inside_interpolations(node: &Node, doc: &str) -> bool {
    let mut cursor = node.walk();
    let all_clear = node.children(&mut cursor).all(|child| {
        let carries = node_text(&child, doc).is_some_and(|text| text.contains(GENERAL_MV_PREFIX));
        !carries || child.kind() == "interpolation"
    });
    all_clear
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn try_build_general_template(
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
    // The statement root is admitted only for had_semi patterns on the
    // plain span-less builds (the same condition the root resolution stops
    // under). `root_gate == false` (the if-CONDITION builder) bypasses the
    // whole gate — a condition is an expression POSITION whose candidate
    // node the walk controls (`push_cond_match` compares against the
    // condition node only), so the statement-root fail-closed contract has
    // no statement root to protect.
    if root_gate && !is_general_root_kind(node.kind(), had_semi && span.is_none()) {
        return None;
    }
    // Meta-BODY lock/using templates. `lock ($X) { $B }` / `using ($D) { $B }`
    // are ACCEPTED faces that bind NOTHING (valid empty with a matching
    // candidate present). tree-sitter parses those bodies cleanly, so
    // without a gate the walk would OVER-answer (bind B where the reference
    // binds nothing). The face must not compose into the loud census either
    // — the reference answers it valid-empty. The template therefore BUILDS
    // with `force_empty` set; the walk answers empty (exact) and the census
    // keeps the face answerable.
    let force_empty = lang == Language::CSharp
        && matches!(node.kind(), "lock_statement" | "using_statement")
        && !placeholders.is_empty()
        && csharp_statement_body_carries_placeholder(&node, &probe.doc, placeholders);
    probe.root_kind.push_str(node.kind());
    probe.force_empty = force_empty;
    Some(probe)
}

/// True when the csharp lock/using statement BODY (the `block` child;
/// `using_statement` spells it as the `body` field) carries a metavariable
/// leaf — the ERROR-node class the build must refuse.
pub(crate) fn csharp_statement_body_carries_placeholder(
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
    fn contains_placeholder(
        node: &Node,
        doc: &str,
        placeholders: &BTreeMap<String, String>,
    ) -> bool {
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

pub(crate) fn build_general_template(
    lang: Language,
    raw: &str,
    substituted: &str,
    placeholders: &BTreeMap<String, String>,
    multi_names: &BTreeSet<String>,
) -> Option<GeneralTemplate> {
    // A pattern-trailing `;` is reference-significant — thread the flag
    // so the plain builds root the template at the statement.
    let had_semi = raw.trim().ends_with(';');
    if let Some(template) = try_build_general_template(
        lang,
        substituted.to_string(),
        None,
        had_semi,
        placeholders,
        multi_names,
        true,
    ) {
        return Some(template);
    }
    // Line-oriented grammars terminate preprocessor lines with the physical
    // newline — `#include __asgrep_mv_X` without one parses with a MISSING
    // terminator token and refuses; the newline-terminated twin parses clean
    // (the span still covers only the pattern text).
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
    // The reference pre-processes php patterns behind a `<?php ` tag — a bare
    // substituted pattern parses to a `text` node. The two gated families
    // (php_wrapped_general_lane) retry behind the tag; every build still
    // demands a clean parse and a general root kind, so anything beyond them
    // keeps the fail-closed refusal.
    if lang == Language::Php && php_wrapped_general_lane(raw) {
        // tree-sitter-php demands the terminating `;` on a statement the tag
        // leads (the reference tolerates the ERROR-wrapped twin; the wrapper
        // build does not), so the retry carries the terminator inside the doc.
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

/// The concrete-condition if template, cached per (language, cond text).
/// The cond parses through the general-lane retries (plain doc, newline
/// doc, php `<?php ` wrap, expression-context wrap) but BYPASSES the
/// statement-root kind gate ([`try_build_general_template`]'s `root_gate =
/// false`): a condition is an expression position and the walk compares it
/// against the candidate's own condition node only, so bare
/// identifier/variable roots (`if (a)`, php `if ($a)`) build here where the
/// statement lane's gate would refuse them.
pub(crate) fn build_if_cond_template(lang: Language, cond: &str) -> Option<GeneralTemplate> {
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

pub(crate) fn cached_if_cond_template(lang: Language, cond: &str) -> Option<GeneralTemplate> {
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

/// The two php families whose general-lane template only parses behind the
/// reference's own `<?php ` pattern pre-process: (i) `->`-carrying member
/// chains (`Foo::bar($A)->baz()` — the reference answers the root member
/// call), and (ii) ALL-META scoped calls `$X::$Y(...)`. The registered php
/// loud cases stay loud by construction: `$A::bar(1)` (meta scope +
/// LITERAL name) fails the (ii) all-meta test and carries no `->`;
/// `Foo::nested(Foo::bar($A))` likewise. Every `<?php `-led build still
/// demands a clean parse and a general root kind, so anything beyond these
/// two families keeps the fail-closed refusal.
pub(crate) fn php_wrapped_general_lane(raw: &str) -> bool {
    // The statement-head templates (`return $X;`, `throw $X;`) parse only
    // behind the `<?php ` tag like families (i)/(ii) — the reference
    // itself pre-processes php patterns behind the tag, and the wrapped
    // build still demands a clean parse and a general root kind
    // (return_statement / throw_statement are admitted roots), so
    // non-building heads (`raise`/`yield`/`defer` are not php syntax)
    // keep their loud class.
    if raw.split_whitespace().next().is_some_and(|head| {
        STATEMENT_HEAD_KEYWORDS.contains(&head)
                // The php roots parse only behind the `<?php ` tag like the
                // statement heads — the plain build folds a bare
                // `namespace $N;` into a text node. The wrapped build still
                // demands a clean parse and an admitted root kind
                // (namespace_definition / goto_statement).
                || matches!(head, "namespace" | "goto")
    }) {
        return true;
    }
    // The php loop/control heads (`foreach …`, `while …`, `do …`) were
    // deliberately NOT admitted: the `$`-stripping meta substitution cannot
    // spell php VARIABLE positions (foreach `as`-targets parse as ERROR;
    // braced `{ $B }` bodies too), and the one spelling that builds
    // (brace-less `while ($X) $B`) binds B to the expression-statement text
    // (`b()`) where the reference binds the full statement (`b();`) — a
    // capture-text divergence. All php loop faces keep their census-loud
    // class; admission needs a php variable-preserving meta substitution
    // (or a dedicated foreach/while lane with statement-span binding).
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
    // A bare-identifier head with an operator continuation (`x * q($A)`) —
    // the reference pre-processes php patterns behind the tag, so the plain
    // build folds the pattern into a text node and the face composed into
    // the loud census where the reference answers the aligned row. The
    // wrapped build still demands a clean parse and a general root kind,
    // so non-building spellings keep their registered loud class.
    let first = match head.split_whitespace().next() {
        Some(first) => first,
        None => return false,
    };
    let after = head[first.len()..].trim_start();
    !after.is_empty() && bare_ident_operator_continuation(after)
}
