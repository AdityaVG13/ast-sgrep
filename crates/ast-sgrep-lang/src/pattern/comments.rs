//! Comment-syntax scanning outside string literals.

use crate::Language;

/// Test each unquoted, non-delimiter byte's suffix. Backslashes escape the
/// next byte inside quotes; an unterminated quote keeps the tail in-string.
/// Callers own the comment policy; this scan does not interpret a language.
fn any_outside_strings(p: &str, mut predicate: impl FnMut(&[u8]) -> bool) -> bool {
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
            _ if predicate(&bytes[i..]) => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// True when `#` appears outside string-literal quotes.
pub(crate) fn contains_hash_outside_strings(p: &str) -> bool {
    any_outside_strings(p, |tail| tail.starts_with(b"#"))
}

/// The language-free comment-CONTENT facts of a pattern: which comment
/// syntaxes appear outside string-literal quotes. Pure content — no language
/// judgment. The single placement adjudicator (the core placement table)
/// combines these facts with the per-language rows; the lang-side ingress
/// consults [`comment_face_refused_for_lang`], the same rows in lang-side
/// form, so the two sites cannot drift apart.
#[derive(Clone, Copy, Default)]
pub(crate) struct PatternCommentContent {
    pub hash: bool,
    pub line: bool,
    pub block: bool,
    pub glued_hash: bool,
}

/// One scan bundling every content predicate below over the shared
/// quote-state primitive. Callers needing a single fact keep using the
/// individual predicates; callers needing several take this struct so the
/// pattern bytes are classified once.
pub(crate) fn pattern_comment_content(pattern: &str) -> PatternCommentContent {
    let p = pattern.trim();
    PatternCommentContent {
        hash: contains_hash_outside_strings(p),
        line: contains_line_comment_syntax_outside_strings(p),
        block: contains_block_comment_syntax_outside_strings(p),
        glued_hash: contains_comment_glued_hash(p),
    }
}

/// The per-language comment-placement row over precomputed content: `#` is
/// comment syntax only in the hash-comment languages (python/ruby/php); in
/// rust (`#[derive]`, `#![allow]`, `r#"…"#`), C/C++ (`#include`, `#define`),
/// swift (`#selector`), and JS/TS (`this.#x`) it is real syntax and stays
/// templatable. `//` is python's FLOOR-DIV operator, not comment syntax —
/// a language-free arm refused `$A // $B` where the reference answers it.
/// For python only `/*` is foreign comment syntax; the registered py
/// `#`-comment faces keep their refusal through the hash arm above. Ruby
/// keeps the `//` refusal (its comment char is `#` too and no answering
/// `//` face is registered there); php and every C-family language spell
/// real line comments with `//`.
pub(crate) fn comment_face_refused_for_lang(lang: Language, content: &PatternCommentContent) -> bool {
    if matches!(lang, Language::Python | Language::Ruby | Language::Php) && content.hash {
        return true;
    }
    if lang == Language::Python {
        return content.block;
    }
    content.line || content.block
}

/// The `/*`-only half of [`contains_comment_syntax_outside_strings`] — true
/// when a block-comment opener appears outside string-literal quotes (same
/// quote scan).
pub(crate) fn contains_block_comment_syntax_outside_strings(p: &str) -> bool {
    any_outside_strings(p, |tail| tail.starts_with(b"/*"))
}

/// True when `//` appears OUTSIDE string-literal quotes (quote state
/// scanned with escape handling). The `/*`-only half lives in
/// [`contains_block_comment_syntax_outside_strings`]; the combined
/// `//`-or-`/*` predicate is [`contains_comment_syntax_outside_strings`].
pub(crate) fn contains_line_comment_syntax_outside_strings(p: &str) -> bool {
    any_outside_strings(p, |tail| tail.starts_with(b"//"))
}

/// True when some quote-external `#` starts a tail that carries no code
/// punctuation at all — a trailing comment glue (`foo($A) # note`), never
/// leading syntax (`#[derive($A)]`, `#include $X` — their remainders contain
/// `[`/`$`), never a mid-chain private field (`this.#x = $V` — `=` follows),
/// never a raw-string delimiter (`tag(r#"$A"#)` — a quote follows).
pub(crate) fn contains_comment_glued_hash(pattern: &str) -> bool {
    any_outside_strings(pattern, |tail| {
        tail[0] == b'#'
            && tail[1..].iter().all(|&rest| {
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
            })
    })
}

/// True when `//` or `/*` appears OUTSIDE string-literal quotes
/// (quote state scanned with escape handling). Unterminated quotes make the
/// tail in-string — conservative in the loud direction only for comment
/// syntax, and the parse-level template checks govern the rest. The `#`
/// arm moved to the language-aware [`comment_face_refused_for_lang`] — a
/// language-free `#` refusal failed rust/c/swift/js `#`-syntax patterns
/// closed where the reference answers.
pub(crate) fn contains_comment_syntax_outside_strings(p: &str) -> bool {
    any_outside_strings(p, |tail| tail.starts_with(b"//") || tail.starts_with(b"/*"))
}
