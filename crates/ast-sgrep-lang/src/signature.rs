//! Indexed pattern signature builders and SIMD prefilter literals.
//!
//! Exact string formats here are part of the on-disk `pattern_nodes` contract —
//! keep them byte-identical when refactoring.

use crate::pattern::{classify_native, is_pattern_ident, DECL_PATTERN_PREFIXES};

/// Declaration keyword prefixes used when classifying patterns / building index keys.
/// Shared with `classify_native` via [`DECL_PATTERN_PREFIXES`].
pub use crate::pattern::DECL_PATTERN_PREFIXES as DECL_PREFIXES;

/// Bare statement keywords (F62-2, pass 63): keyword tokens are NOT
/// `pattern_nodes` rows (the index stores identifier/decl/call nodes), so an
/// ident-serve of `break`/`return`/… can only ever answer a silent empty
/// result while the native statement-template lane answers sg's hits
/// (`break` -l javascript probes exit 0 with the break_statement hit). They
/// must fall through to the native walk.
const STATEMENT_KEYWORDS: &[&str] = &[
    "return", "raise", "yield", "throw", "await", "break", "continue", "next", "last",
];

/// True when `pattern_nodes` rows for these signatures are the same nodes the
/// native matcher would return, so a tree-sitter re-walk cannot add hits.
///
/// Kind-only signatures over-match (`fn $NAME` → every function) and still
/// need native confirmation. Ident, `decl:`, `call:`, and `call-name:`
/// signatures are exact, so the indexed rows are the result.
pub fn index_can_serve_pattern(pattern: &str, signatures: &[String]) -> bool {
    if signatures.is_empty() || signatures.iter().any(|s| s.starts_with("kind:")) {
        return false;
    }
    let pattern = pattern.trim();
    if STATEMENT_KEYWORDS.contains(&pattern) {
        return false;
    }
    is_pattern_ident(pattern)
        || signatures.iter().all(|s| {
            s.starts_with("decl:") || s.starts_with("call:") || s.starts_with("call-name:")
        })
}

/// Map a structural pattern to the exact index signatures stored in `pattern_nodes`.
///
/// Returns `None` when the pattern shape is not indexable (exotic / nested).
pub fn cached_pattern_signatures(pattern: &str) -> Option<Vec<String>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Some(vec![]);
    }
    if !pattern.contains('$') {
        if is_pattern_ident(pattern) {
            return Some(vec![pattern.to_string()]);
        }
        // `fn foo` / `struct Bar` fall through to decl: rows. A raw
        // "fn foo" string is not stored as a pattern_nodes signature.
    }
    // Never let a broad cached signature bypass native validation. In
    // particular, malformed declaration tails must remain match-none.
    classify_native(pattern)?;
    // Nested body templates (`fn $N($$$) { $STMT }`, `if $COND { $BODY }`)
    // are not indexable: `pattern_nodes` signatures cannot express statement
    // counts, so serving them from the index would over-match. The native
    // tree-sitter scan is the sole source for these shapes.
    if pattern.contains('{') {
        return None;
    }
    // PASS 71a (F70a-1a): a member-call CHAIN is not indexable by a single
    // `call:`/`call-name:` row. `pattern_nodes` stores (callee path, kind)
    // pairs that cannot express the head's argument template, the segment
    // count, or the per-connector optional flags the native matcher enforces;
    // serving `fetch()?.$M($$$A)` from `call:fetch` rows answered every bare
    // `fetch` call in the search lane while the codemod lane — which runs
    // `match_pattern` — planned sg-exact edits on the same pattern. Returning
    // `None` routes the search through the native walk, so both lanes answer
    // through one matcher.
    if has_multiple_call_segments(pattern) {
        return None;
    }
    for (prefix, kinds) in CACHED_DECL_KIND_TABLE {
        if let Some(rest) = pattern.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| ch == '(' || ch == '{' || ch.is_whitespace())
                .next()
                .unwrap_or_default();
            if name.starts_with('$') {
                return Some(kinds.iter().map(|kind| format!("kind:{kind}")).collect());
            }
            if is_pattern_ident(name) {
                return Some(vec![format!("decl:{}:{name}", prefix.trim())]);
            }
            return None;
        }
    }
    let open = pattern.find('(')?;
    let close = pattern.rfind(')')?;
    if close + 1 != pattern.len() || !pattern[open + 1..close].contains("$$$") {
        return None;
    }
    let callee = pattern[..open].trim();
    if callee.starts_with('$') && !callee.contains('.') {
        // Byte-identical to the historical core classifier.
        return Some(vec!["kind:call_expression".into(), "kind:call".into()]);
    }
    if let Some(name) = callee.rsplit('.').next() {
        if callee.contains('$') && is_pattern_ident(name) {
            return Some(vec![format!("call-name:{name}")]);
        }
    }
    is_pattern_path(callee).then(|| vec![format!("call:{callee}")])
}

/// Candidate KIND signatures for patterns whose exact shape is not indexable
/// (braced declaration templates like `fn $NAME($$$) { $$$ }`) but whose
/// matches must still be nodes of a known kind.
///
/// Soundness for candidate narrowing: every native match of such a pattern IS
/// a node of the returned kind, so any file containing a match necessarily
/// contains a `pattern_nodes` row with one of these signatures. The index
/// narrows the file set; the native tree-sitter matcher still decides every
/// hit, so over-broad kind candidates never change results.
pub fn candidate_kind_signatures(pattern: &str) -> Option<Vec<String>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    classify_native(pattern)?;
    for (prefix, kinds) in CACHED_DECL_KIND_TABLE {
        if pattern.starts_with(prefix) {
            return Some(kinds.iter().map(|kind| format!("kind:{kind}")).collect());
        }
    }
    None
}

/// Longest concrete token suitable for a byte-level SIMD prefilter.
///
/// Declaration keywords alone are never returned (they are not cross-language
/// literals). Metavariable-only callees yield `None`.
pub fn required_pattern_literal(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    if !pattern.contains('$') {
        if is_pattern_ident(pattern) {
            return Some(pattern.to_string());
        }
        // PASS 60 (H-CONF-030 i/ii): the R3 structural lane matches $-less
        // patterns whose source spelling may differ byte-wise from the
        // pattern text (trailing commas in argument lists, comments between
        // arguments). The whole pattern text was an unsound prefilter there:
        // it dropped files that DO hold a structural match. The longest
        // concrete token of the COMMENT-STRIPPED pattern is sound — every
        // code token of the pattern must appear in any matching file, while
        // layout, commas, and comment TEXT are invisible to the R3 lane, so
        // they must not enter the prefilter (the comment-glued shapes that
        // must stay fail-closed are `$`-patterns refused upstream, not this
        // $-less arm).
        return longest_concrete_token(pattern);
    }
    // If templates: every indexed language spells the keyword `if`, so any
    // file that can hold an if-node must contain those bytes.
    if pattern.starts_with("if ") || pattern.starts_with("if(") {
        return Some("if".to_string());
    }
    for (prefix, _) in DECL_PATTERN_PREFIXES {
        if let Some(rest) = pattern.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| ch == '(' || ch == '{' || ch == '<' || ch.is_whitespace())
                .next()
                .unwrap_or_default();
            return (!name.is_empty() && !name.starts_with('$')).then(|| name.to_string());
        }
    }
    let callee = pattern.split_once('(')?.0.trim();
    // PASS 51: a segment containing `$` anywhere is not a usable byte literal
    // (`Some($A).unwrap_or($A)` previously yielded the bogus prefilter
    // `Some($A)` once the general lane served such shapes — every file would
    // have been prefiltered away). Metavariable-bearing segments are dropped;
    // the longest clean segment is the literal.
    callee
        .split(['.', ':'])
        .filter(|segment| !segment.is_empty() && !segment.contains('$'))
        .max_by_key(|segment| segment.len())
        .map(str::to_string)
}

/// Longest concrete code token of a `$`-less pattern, with comment regions
/// blanked first (quote-aware, escape-handling scan): `//` and `#` run to end
/// of line, `/* … */` to its close. Comment TEXT is invisible to the R3
/// structural comparator, so it must never become a required prefilter byte.
/// A pattern that is entirely comments yields `None` (no prefilter — sound).
fn longest_concrete_token(pattern: &str) -> Option<String> {
    let bytes = pattern.as_bytes();
    let mut code = String::with_capacity(pattern.len());
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            code.push(b as char);
            if b == b'\\' && bytes.get(i + 1).is_some() {
                code.push(bytes[i + 1] as char);
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
            b'"' | b'\'' | b'`' => {
                quote = Some(b);
                code.push(b as char);
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') || bytes.get(i + 1) == Some(&b'*') => {
                let line_comment = bytes.get(i + 1) == Some(&b'/');
                i += 2;
                while i < bytes.len() {
                    if line_comment && bytes[i] == b'\n' {
                        code.push('\n');
                        break;
                    }
                    if !line_comment && bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            _ => {
                // Latin-1 byte cast (pass-65 minor-cluster adjudication): a
                // byte >= 0x80 casts to a char that is never
                // ascii-alphanumeric, so multibyte UTF-8 degrades to token
                // separators — the emitted literal can only come from the
                // pattern's ASCII runs, which is the sound (over-broad)
                // direction for a prefilter.
                code.push(b as char);
                i += 1;
            }
        }
    }
    code.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|token| !token.is_empty())
        .max_by_key(|token| token.len())
        .map(str::to_string)
}

/// Hybrid structural boost signatures for a bare identifier term.
///
/// Formats must stay byte-identical to the historical `structural_index_pass` keys.
pub fn structural_term_signatures(term: &str) -> [String; 6] {
    [
        format!("call-name:{term}"),
        format!("call:{term}"),
        format!("decl:fn:{term}"),
        format!("decl:def:{term}"),
        format!("decl:function:{term}"),
        term.to_string(),
    ]
}

/// Prefix → tree-sitter kind names used for metavariable declaration lookups.
///
/// `fn ` keeps the historical `kind:function_item` entry. Pass 14 (EXP-010,
/// H-CONF-012) widened `def ` with ruby's `method` / `singleton_method` and
/// `function ` with php's `function_definition`: a missing kind here narrows
/// the candidate file set below the set of files that can hold a match, which
/// turned native-claim patterns into silent empty results. Over-broad entries
/// are always sound — the native tree-sitter matcher still decides every hit.
const CACHED_DECL_KIND_TABLE: &[(&str, &[&str])] = &[
    ("fn ", &["function_item"]),
    (
        "def ",
        &[
            "function_definition",
            "method",
            "singleton_method",
        ],
    ),
    (
        "function ",
        &[
            "function_declaration",
            "protocol_function_declaration",
            "function_definition",
            "method_definition",
            "method_declaration",
            "method",
            "singleton_method",
            "local_function_statement",
        ],
    ),
    ("func ", &["function_declaration"]),
    (
        "class ",
        &[
            "class_definition",
            "class_declaration",
            "class",
            "record_declaration",
            "class_specifier",
        ],
    ),
    (
        "struct ",
        &["struct_item", "struct_declaration", "struct_specifier"],
    ),
    (
        "interface ",
        &[
            "trait_item",
            "interface_declaration",
            "protocol_declaration",
        ],
    ),
];

fn is_pattern_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('$')
        && value
            .split(['.', ':'])
            .filter(|p| !p.is_empty())
            // PASS 75a (74c-F3): namespace-qualified scope text (`\Foo`,
            // `Foo\Bar`) — the re-keyed index emits `call:\Foo::bar`-style
            // rows, so the pattern-side derivation must accept the same
            // spellings to address them.
            .all(|segment| {
                is_pattern_ident(segment)
                    || crate::pattern::namespace_qualified_segment(segment)
            })
}

/// PASS 71a (F70a-1a): true when the pattern decomposes into MORE THAN ONE
/// depth-0 call segment (`fetch()?.$M($$$A)`, `a.b($$$C).d()`) — a chain
/// shape whose per-segment argument templates, segment count, and connector
/// flags no single `pattern_nodes` signature can express. Nested parentheses
/// stay out of the count (`a.b(f($$$X).g($$$Y))` is one call segment; its
/// exactness is the tail's `$$$`-containment contract, unchanged here).
fn has_multiple_call_segments(pattern: &str) -> bool {
    let mut depth = 0usize;
    let mut segment_has_call = false;
    let mut call_segments = 0usize;
    for ch in pattern.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                segment_has_call = true;
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
            }
            '.' if depth == 0 => {
                if segment_has_call {
                    call_segments += 1;
                }
                segment_has_call = false;
            }
            _ => {}
        }
    }
    call_segments + usize::from(segment_has_call) > 1
}

#[cfg(test)]
mod index_serve_tests {
    use super::{cached_pattern_signatures, index_can_serve_pattern};

    #[test]
    fn ident_and_decl_are_index_complete_kind_is_not() {
        let ident = cached_pattern_signatures("SearchHit").unwrap();
        assert!(index_can_serve_pattern("SearchHit", &ident));
        let decl = cached_pattern_signatures("fn greet_user").unwrap();
        assert!(index_can_serve_pattern("fn greet_user", &decl));
        let kind = cached_pattern_signatures("fn $NAME").unwrap();
        assert!(!index_can_serve_pattern("fn $NAME", &kind));
        assert!(!index_can_serve_pattern("fn $NAME() { $$$BODY }", &[]));
    }
}
