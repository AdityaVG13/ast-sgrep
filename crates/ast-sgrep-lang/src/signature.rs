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
///
/// PASS 132 (F-131E-2): `debugger` joins the class. The bare (`;`-less)
/// spelling is `is_pattern_ident`-admitted, so `index_can_serve_pattern`
/// early-returned the empty ident rows as "ident-exact" on the CLI
/// `--pattern` lane while sg 0.45.2 answers the js/ts debugger_statement
/// n1 (oracle dbg_{js,ts}_bare faces) — the library's f131d kinds arm was
/// reachable only below the early return. The `;`-ful spelling was never
/// ident-shaped and kept walking.
///
/// PASS 133 (F-132E class sweep): the remaining bare drop faces join the
/// escape — `import` (js/ts/py), `use` (rs), `fallthrough`/`goto` (go),
/// `redo`/`retry` (rb), `pass`/`global`/`del`/`assert` (py). All are
/// `is_pattern_ident`-admitted with NO `pattern_nodes` rows (keyword
/// tokens are never identifier rows), so the ident-exact early-return
/// answered silent `ok:true []` where sg 0.45.2 answers the statement
/// family (52-cell grid /tmp/phase133/live/grid.json: every face sg n1,
/// py `assert` n2; subject n0). Escaped to the walk, most of them are
/// answered sg-exactly by the literal lane's keyword-token leaf (the same
/// bytes sg reports) — mutants M-133b/M-133c2 kill the escape and the
/// f133b/f133c pins fail. Two shapes needed MORE than the escape, both in
/// `match_bare_statement_kind`: the `return` family (a STATEMENT_HEAD
/// keyword, so the general lane intercepts and under-answers arg-ful
/// forms without a kinds arm) and rb `break` (ruby names the node
/// `break`; the pre-existing arm's statement/expression kinds exist
/// nowhere in tree-sitter-ruby and short-circuited sg's hit away). For
/// grammars outside an arm's scope the literal lane keeps serving
/// genuine identifier faces (the pass-132 rs `debugger` zero-drift
/// doctrine).
const STATEMENT_KEYWORDS: &[&str] = &[
    "return", "raise", "yield", "throw", "await", "break", "continue", "next", "last", "debugger",
    "import", "pass", "global", "del", "assert", "use", "fallthrough", "goto", "redo", "retry",
    // PASS 135 (134B-F1): go's bare `defer`/`go` are STATEMENT_HEAD_KEYWORDS
    // whose statement faces sg 0.45.2 answers (grids G_go_defer/G_go_go:
    // sg n2 vs subject n0) — the ident-serve trap silenced them exactly like
    // bare `debugger` pre-132. Because both spellings are STATEMENT_HEADS,
    // the escape alone was NOT enough: the walk's general-lane intercept
    // silently answered nothing for them AND swallowed the genuine
    // identifier faces of the same spellings in every other grammar
    // (fresh Z-grid 2026-09-11: js/ts/py/java/c/rb/kt `go` sg n2 vs subject
    // n0). `match_bare_statement_kind` now routes the un-armed escaped
    // heads to the literal/identifier lanes (the F66a-8 doctrine); mutant
    // M-135e (removed from this list) re-silences every face, M-135d
    // (carve deleted) re-drops the ident faces.
    // PASS 136 (F-136 grid sibling): js bare `delete` — the keyword token is
    // never a `pattern_nodes` identifier row, so the ident-exact serve
    // answered silent ok:true-0 where sg 0.45.2 answers the
    // delete_expression family n1 per site (grid /tmp/phase136 sgprobe:
    // `delete o.k;` sg n1). Un-armed, the walk's literal lane serves the
    // keyword-token leaf at line parity (the 133 doctrine). The escape
    // cannot OVER-answer in the keyword grammars; in grammars where the
    // token is an ordinary identifier (ruby/php/go/c for `delete`) the
    // walk's literal lane RE-SERVES those identifier rows at sg parity —
    // the same literal-lane reserve that keeps the pass-132 rs `debugger`
    // ident face zero-drift (the 136-era "keyword in every indexed grammar"
    // safety wording was wrong of record; corrected PASS 137, 137B-F5a).
    // PASS 137 (137B-F4 + 137A-F2 bare cells): the csharp statement-keyword
    // faces join the same escape genus — bare `lock`/`using`/`var`/
    // `fixed`/`checked`/`unchecked`/`unsafe` are keyword TOKENS in csharp
    // (js `var`, go `var`, cpp `using`, rs `unsafe` likewise), so the
    // ident-exact serve answered silent ok:true-0 where sg 0.45.2 answers
    // the keyword-token rows per site (grid137 A-lane: cs bare
    // fixed/checked/unchecked/unsafe/lock/using sg n1, `var` sg n2, subject
    // n0 everywhere; js/go bare `var` sg n2, cpp `using` n1, rs `unsafe` n1
    // — all subject n0). Un-armed, the walk's literal lane serves the
    // keyword-token leaf AND every genuine identifier face of the same
    // spelling in other grammars at sg parity (grid137: py/rs/rb
    // `var`/`lock`/`checked` identifier faces n==n; the f132 rs-debugger
    // zero-drift doctrine).
    "defer", "go", "delete", "lock", "using", "var", "fixed", "checked", "unchecked", "unsafe",
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
    // PASS 81 (FB-80a-06 root cause): each segment is TRIMMED before the
    // pick. The callee split keeps interior layout, so the admitted
    // multiline chain `$O.out\n.$M($A)` previously selected the segment
    // `"out\n"` (and its collapsed-ingress twin `$O.out .$M($A)` the
    // segment `"out "`) — whitespace-carrying literals no matching file's
    // `out.` bytes can contain, so the memchr prefilter skipped every file
    // and the (already green) matcher never ran: silent [] search, 0-edit
    // codemod plans. Trimmed segments stay sound — the matched property
    // link bytes are literal — and a segment that is ONLY layout leaves no
    // literal: `None` means both consumers (core/pattern.rs, codemod.rs)
    // scan the file instead of filtering it.
    // PASS 113B (CNR §39.9 residual 1): a trailing `?` is the `?.`
    // CONNECTOR MARKER, not file content — sg 0.45.2 treats it as connector
    // syntax only (`a?.b($X)` answers the trivia-bearing `a /*c*/ ?.b(1)`
    // whose bytes never contain contiguous `a?`; first-hand grid: the
    // marker is required as a matcher distinction — `a?.b($X)` refuses
    // `a.b(1)`, `a?.($X)` refuses `a(1)` — but is never matchable text in
    // isolation). The raw segment `"a?"` dropped every sg-answering
    // commented-`?.` file at the prefilter. Trim it per segment before the
    // pick; a shorter literal is the over-broad (sound) direction.
    callee
        .split(['.', ':'])
        .map(|segment| {
            let mut segment = segment.trim();
            while let Some(stripped) = segment.strip_suffix('?') {
                segment = stripped.trim_end();
            }
            segment
        })
        .filter(|segment| !segment.is_empty() && !segment.contains('$'))
        .max_by_key(|segment| segment.len())
        // PASS 146 (145B-F1, oracle grid /tmp/phase146R cases k1/k2/k6/k7 +
        // v1-v8): a segment may carry INTERIOR layout (`namespace A { f` —
        // the callee of `namespace A { f(); $B }`), and layout is invisible
        // to the structural matcher: sg binds the pretty-printed namespace
        // body whose bytes never contain the single-space run, so the
        // whitespace-carrying literal silently prefiltered sg-answering
        // files away (same unsoundness genus as the FB-80a-06 edge-trim).
        // A segment with interior whitespace degrades to its longest
        // whitespace-free TOKEN — still a required byte run of any match,
        // over-broad in the sound direction. A token-only segment never
        // needs this fallback.
        .map(|segment| {
            if segment.chars().any(char::is_whitespace) {
                segment
                    .split_whitespace()
                    .max_by_key(|token| token.len())
                    .map(str::to_string)
            } else {
                Some(segment.to_string())
            }
        })
        .flatten()
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
    (
        "fn ",
        &[
            "function_item",
            // MoonBit `fn` / `impl ... with fn` share C/Python's
            // `function_definition` kind; omitting them here silently empties
            // indexed `fn $NAME` search (H-CONF-012). Over-broad is sound.
            "function_definition",
            "impl_definition",
            "named_lambda_expression",
        ],
    ),
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
            "local_function_declaration",
            "getter_declaration",
            "setter_declaration",
            "external_function_declaration",
            "external_getter_declaration",
            "external_setter_declaration",
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
        &[
            "struct_item",
            "struct_declaration",
            "struct_specifier",
            "struct_definition",
            "tuple_struct_definition",
        ],
    ),
    (
        "interface ",
        &[
            "trait_item",
            "interface_declaration",
            "protocol_declaration",
            "mixin_declaration",
            "trait_definition",
        ],
    ),
    (
        "type ",
        &[
            "type_item",
            "type_definition",
            "type_alias_declaration",
            "extension_declaration",
            "extension_type_declaration",
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
