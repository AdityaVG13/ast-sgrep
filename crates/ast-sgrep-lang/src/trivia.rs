//! ONE trivia convention (H7).
//!
//! Every comment/string/gap-trivia definition in `ast-sgrep-lang` lives
//! here. Node-kind checks use [`is_trivia_kind`] (any `comment`-carrying
//! kind) or [`is_comment_or_string_kind`] (the indexed extraction table);
//! pattern-text gap classes use [`is_sg_cs_trivia`] /
//! [`is_sg_php_gap_trivia`] / [`is_sg_cs_gap_junk`]; directive demarcation
//! uses [`strip_comment_spans`]. Previously these lived in six places
//! (`extract.rs`, `pattern/ifs.rs`, `pattern/gates.rs`, inline
//! `kind.contains("comment")`); they now delegate here so the classes
//! cannot drift apart. Same predicates, same outcomes — one home.

/// Comment / string trivia kinds skipped by pattern and call extraction.
pub(crate) const COMMENT_OR_STRING_KINDS: &[&str] = &[
    "comment",
    "line_comment",
    "block_comment",
    "multiline_comment",
    "string_literal",
    "raw_string_literal",
    "string",
    "line_string_literal",
    "multi_line_string_literal",
    "template_string",
    "interpreted_string_literal",
    "quoted_string_literal",
    "documentation_block_comment",
    "multiline_string_literal",
    "bytes_literal",
    "regex_literal",
];

/// Comment kinds skipped by the identifier collector.
pub(crate) const SKIP_IDENT_COMMENT_KINDS: &[&str] = &["comment", "line_comment", "block_comment"];

#[inline]
pub(crate) fn is_comment_or_string_kind(kind: &str) -> bool {
    COMMENT_OR_STRING_KINDS.contains(&kind)
}

/// Canonical node-kind trivia predicate: any `comment`-carrying kind.
/// (String/regex kinds are trivia only in the extraction-table sense
/// above, never here — call sites needing those compose explicitly.)
#[inline]
pub(crate) fn is_trivia_kind(kind: &str) -> bool {
    kind.contains("comment")
}

/// The cs trivia class of record — the PATTERN-side class. The cs grammar
/// extras are /[\s\u00A0\uFEFF\u3000]+/ whose `\s` the grammar generation
/// compiles with UNICODE semantics, but the PATTERN parse answers these
/// outsider spellings accepted-empty at the nested-head seam. At the
/// throw-semi/using TOKEN GAPS the candidate parse accepts the full
/// Unicode White_Space set — that is [`is_sg_cs_gap_junk`], NOT this class.
#[inline]
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
#[inline]
pub(crate) fn is_sg_cs_gap_junk(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}' || c.is_control()
}

/// The php gap-trivia class of record at the use-kind clause gaps — the php
/// grammar skips exactly ASCII whitespace, U+00A0, U+FEFF and U+001A there.
/// Every other Rust-whitespace spelling GLUES into the surrounding php name
/// token (name chars run to U+0080+), which is why the zero-gap glue
/// cases refuse and why glue chars belong to name-token text.
#[inline]
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
