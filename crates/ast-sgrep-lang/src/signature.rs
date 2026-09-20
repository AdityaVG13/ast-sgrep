//! Indexed pattern signature builders and SIMD prefilter literals.
//!
//! Exact string formats here are part of the on-disk `pattern_nodes` contract —
//! keep them byte-identical when refactoring.

use crate::pattern::{classify_native, is_pattern_ident, DECL_PATTERN_PREFIXES};

/// Declaration keyword prefixes used when classifying patterns / building index keys.
/// Shared with `classify_native` via [`DECL_PATTERN_PREFIXES`].
pub use crate::pattern::DECL_PATTERN_PREFIXES as DECL_PREFIXES;

/// Bare statement keywords: keyword tokens are NOT `pattern_nodes` rows
/// (the index stores identifier/decl/call nodes), so an ident-serve of
/// `break`/`return`/… can only ever answer a silent empty result while the
/// native statement-template lane answers the reference's hits (`break`
/// javascript probes exit 0 with the break_statement hit). They must fall
/// through to the native walk.
///
/// `debugger` joins the class. The bare (`;`-less) spelling is
/// `is_pattern_ident`-admitted, so `index_can_serve_pattern` early-returned
/// the empty ident rows as "ident-exact" on the CLI `--pattern` lane while
/// the reference answers the js/ts debugger_statement once — the library's
/// kinds arm was reachable only below the early return. The `;`-ful
/// spelling was never ident-shaped and kept walking.
///
/// The remaining bare drop shapes join the escape — `import` (js/ts/py),
/// `use` (rs), `fallthrough`/`goto` (go), `redo`/`retry` (rb),
/// `pass`/`global`/`del`/`assert` (py). All are `is_pattern_ident`-admitted
/// with NO `pattern_nodes` rows (keyword tokens are never identifier rows),
/// so the ident-exact early-return answered silent `ok:true []` where the
/// reference answers the statement family (every shape the reference
/// answers once, py `assert` twice; the subject answered none). Escaped to
/// the walk, most of them are answered reference-exactly by the literal
/// lane's keyword token leaf (the same bytes the reference reports) —
/// regression pins fail if the escape is removed. Two shapes needed MORE
/// than the escape, both in
/// `match_bare_statement_kind`: the `return` family (a STATEMENT_HEAD
/// keyword, so the general lane intercepts and under-answers arg-ful
/// forms without a kinds arm) and rb `break` (ruby names the node
/// `break`; the pre-existing arm's statement/expression kinds exist
/// nowhere in tree-sitter-ruby and short-circuited the reference's hit
/// away). For grammars outside an arm's scope the literal lane keeps
/// serving genuine identifier shapes (the rs `debugger` zero-drift rule).
const STATEMENT_KEYWORDS: &[&str] = &[
    "return",
    "raise",
    "yield",
    "throw",
    "await",
    "break",
    "continue",
    "next",
    "last",
    "debugger",
    "import",
    "pass",
    "global",
    "del",
    "assert",
    "use",
    "fallthrough",
    "goto",
    "redo",
    "retry",
    // Go's bare `defer`/`go` are STATEMENT_HEAD_KEYWORDS whose statement
    // shapes the reference answers (twice vs the subject's none) — the
    // ident-serve trap silenced them exactly like bare `debugger`. Because
    // both spellings are STATEMENT_HEADS, the escape alone was NOT enough:
    // the walk's general-lane intercept silently answered nothing for them
    // AND swallowed the genuine identifier shapes of the same spellings in
    // every other grammar (js/ts/py/java/c/rb/kt `go`: the reference
    // answers twice vs the subject's none).
    // `match_bare_statement_kind` now routes the un-armed escaped heads to
    // the literal/identifier lanes; removing them from this list
    // re-silences every shape, deleting the carve re-drops the ident
    // shapes.
    // Js bare `delete` — the keyword token is never a `pattern_nodes`
    // identifier row, so the ident-exact serve answered silent ok:true-0
    // where the reference answers the delete_expression family once per
    // site (`delete o.k;` answers once). Un-armed, the walk's literal lane
    // serves the keyword-token leaf at line parity (the same rule as
    // above). The escape cannot OVER-answer in the keyword grammars; in
    // grammars where the token is an ordinary identifier (ruby/php/go/c
    // for `delete`) the walk's literal lane RE-SERVES those identifier
    // rows at reference parity — the same literal-lane reserve that keeps
    // the rs `debugger` ident shape zero-drift (an earlier "keyword in
    // every indexed grammar" wording was wrong and has been corrected).
    // The csharp statement-keyword shapes join the same escape genus —
    // bare `lock`/`using`/`var`/`fixed`/`checked`/`unchecked`/`unsafe` are
    // keyword TOKENS in csharp (js `var`, go `var`, cpp `using`, rs
    // `unsafe` likewise), so the ident-exact serve answered silent
    // ok:true-0 where the reference answers the keyword-token rows per
    // site (the reference answers per site; the subject answered none).
    // Un-armed, the walk's literal lane serves the keyword-token leaf AND
    // every genuine identifier shape of the same spelling in other
    // grammars at reference parity (py/rs/rb identifier shapes stay
    // equal-count; the rs-debugger zero-drift rule).
    "defer",
    "go",
    "delete",
    "lock",
    "using",
    "var",
    "fixed",
    "checked",
    "unchecked",
    "unsafe",
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
    if !pattern.contains('$') && is_pattern_ident(pattern) {
        return Some(vec![pattern.to_string()]);
    }
    // `fn foo` / `struct Bar` fall through to decl: rows. A raw
    // "fn foo" string is not stored as a pattern_nodes signature.
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
    // A member-call CHAIN is not indexable by a single `call:`/`call-name:`
    // row. `pattern_nodes` stores (callee path, kind) pairs that cannot
    // express the head's argument template, the segment count, or the
    // per-connector optional flags the native matcher enforces; serving
    // `fetch()?.$M($$$A)` from `call:fetch` rows answered every bare
    // `fetch` call in the search lane while the codemod lane — which runs
    // `match_pattern` — planned reference-exact edits on the same pattern.
    // Returning `None` routes the search through the native walk, so both
    // lanes answer through one matcher.
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
        // The R3 structural lane matches $-less patterns whose source
        // spelling may differ byte-wise from the pattern text (trailing
        // commas in argument lists, comments between arguments). The whole
        // pattern text was an unsound prefilter there:
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
    // A segment containing `$` anywhere is not a usable byte literal
    // (`Some($A).unwrap_or($A)` previously yielded the bogus prefilter
    // `Some($A)` once the general lane served such shapes — every file would
    // have been prefiltered away). Metavariable-bearing segments are dropped;
    // the longest clean segment is the literal.
    // Each segment is TRIMMED before the pick. The callee split keeps
    // interior layout, so the admitted
    // multiline chain `$O.out\n.$M($A)` previously selected the segment
    // `"out\n"` (and its collapsed-ingress twin `$O.out .$M($A)` the
    // segment `"out "`) — whitespace-carrying literals no matching file's
    // `out.` bytes can contain, so the memchr prefilter skipped every file
    // and the (already green) matcher never ran: silent [] search, 0-edit
    // codemod plans. Trimmed segments stay sound — the matched property
    // link bytes are literal — and a segment that is ONLY layout leaves no
    // literal: `None` means both consumers (core/pattern.rs, codemod/plan.rs)
    // scan the file instead of filtering it.
    // A trailing `?` is the `?.` CONNECTOR MARKER, not file content —
    // the reference treats it as connector syntax only (`a?.b($X)` answers
    // the trivia-bearing `a /*c*/ ?.b(1)` whose bytes never contain
    // contiguous `a?`; verified first-hand: the marker is required as a
    // matcher distinction — `a?.b($X)` refuses `a.b(1)`, `a?.($X)` refuses
    // `a(1)` — but is never matchable text in isolation). The raw segment
    // `"a?"` dropped every reference-answered commented-`?.` file at the
    // prefilter. Trim it per segment before the pick; a shorter literal is
    // the over-broad (sound) direction.
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
        // A segment may carry INTERIOR layout (`namespace A { f` — the
        // callee of `namespace A { f(); $B }`), and layout is invisible to
        // the structural matcher: the reference binds the pretty-printed
        // namespace body whose bytes never contain the single-space run, so
        // the whitespace-carrying literal silently prefiltered
        // reference-answered files away (same unsoundness genus as the
        // edge-trim above).
        // A segment with interior whitespace degrades to its longest
        // whitespace-free TOKEN — still a required byte run of any match,
        // over-broad in the sound direction. A token-only segment never
        // needs this fallback.
        .and_then(|segment| {
            if segment.chars().any(char::is_whitespace) {
                segment
                    .split_whitespace()
                    .max_by_key(|token| token.len())
                    .map(str::to_string)
            } else {
                Some(segment.to_string())
            }
        })
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
                // Latin-1 byte cast: a byte >= 0x80 casts to a char that is
                // never
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
/// `fn ` keeps the historical `kind:function_item` entry. An earlier
/// widening gave `def ` ruby's `method` / `singleton_method` and
/// `function ` php's `function_definition`: a missing kind here narrows
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
            // indexed `fn $NAME` search. Over-broad is sound.
            "function_definition",
            "impl_definition",
            "named_lambda_expression",
        ],
    ),
    (
        "def ",
        &["function_definition", "method", "singleton_method"],
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
            // Namespace-qualified scope text (`\Foo`, `Foo\Bar`) — the
            // re-keyed index emits `call:\Foo::bar`-style rows, so the
            // pattern-side derivation must accept the same spellings to
            // address them.
            .all(|segment| {
                is_pattern_ident(segment) || crate::pattern::namespace_qualified_segment(segment)
            })
}

/// True when the pattern decomposes into MORE THAN ONE depth-0 call
/// segment (`fetch()?.$M($$$A)`, `a.b($$$C).d()`) — a chain
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
