/// Quote-external comment-syntax scan of a pattern — escape-aware string
/// discipline, the same shape as the lang-side comment refusal the census
/// fallback consults for every non-exempt face
/// (`contains_hash_outside_strings` /
/// `contains_comment_syntax_outside_strings`).
/// `line` flags `//`, `block` flags `/*`, `hash` flags `#`; `block_trailing`
/// records that the final `*/` ends the pattern (only whitespace follows, or
/// the `/*` never closes) — the placement the reference refuses in every
/// probed language, unlike the inline placements the hook family answers.
/// `block_leading` records that a block comment STARTS the pattern (only
/// ASCII whitespace before its `/*`) — the leading placement spawns a
/// second top-level node, so the reference refuses it in every probed
/// language exactly like the trailing placement, and it is excluded from
/// the php inline exemption.
/// `hash_bracket` records that a quote-external `#` is immediately followed
/// by `[`. That pair is attribute-like syntax, not a comment: in php a
/// leading `#[...]` is an attribute (the reference answers/accepts), in
/// rust it is the real `#[derive($A)]` syntax, and in ts/js it is INVALID
/// syntax the reference refuses. The blind flags (this scan) stay
/// content-blind so the NeverMatches route and the lane-refusal override
/// precondition keep their registered inputs;
/// [`scan_pattern_comment_syntax_for`] refines them per language for the
/// template route.
#[derive(Clone, Copy)]
pub(crate) struct PatternCommentScan {
    pub(crate) line: bool,
    pub(crate) block: bool,
    pub(crate) hash: bool,
    pub(crate) block_trailing: bool,
    pub(crate) block_leading: bool,
    pub(crate) hash_bracket: bool,
}

pub(crate) fn scan_pattern_comment_syntax(pattern: &str) -> PatternCommentScan {
    let bytes = pattern.as_bytes();
    let mut scan = PatternCommentScan {
        line: false,
        block: false,
        hash: false,
        block_trailing: false,
        block_leading: false,
        hash_bracket: false,
    };
    scan.block_leading = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|first| bytes[first..].starts_with(b"/*"));
    let mut quote: Option<u8> = None;
    let mut in_block = false;
    let mut last_block_end = 0usize;
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
        if in_block {
            if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_block = false;
                last_block_end = i + 2;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                scan.block = true;
                in_block = true;
                i += 2;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => scan.line = true,
            b'#' => {
                scan.hash = true;
                if bytes.get(i + 1) == Some(&b'[') {
                    scan.hash_bracket = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    scan.block_trailing = scan.block
        && (in_block
            || bytes[last_block_end.min(bytes.len())..]
                .iter()
                .all(|byte| byte.is_ascii_whitespace()));
    scan
}

/// The language-aware adjudication of the same byte scan. The BLIND scan
/// keeps feeding the NeverMatches route
/// (`never_matches_comment_placement_sg_accepted`) and the lane-refusal
/// override precondition byte-identically; the TEMPLATE route consumes this
/// refined scan. Two reference grammar facts the language-free byte scan
/// cannot see:
///   - ruby: `/`-delimited regex literals are not comment territory — every
///     quote-external `#` inside `/.../` bodies (plain `#` or `#{...}`
///     interpolation) clears the hash flag (the reference answers
///     `re = /ab#{x}cd/`, `h2 = /a#b/`), while `#` after division stays a
///     comment (the reference refuses `a / b # c`).
///   - php: a LEADING `#[...]` attribute list whose remainder is empty or a
///     `function` declaration is attribute syntax, not a comment (the
///     reference answers/accepts); attribute-after-code and
///     attribute+property tails keep the comment flag (the reference
///     refuses).
pub(crate) fn scan_pattern_comment_syntax_for(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
) -> PatternCommentScan {
    let mut scan = scan_pattern_comment_syntax(pattern);
    if scan.hash {
        match lang {
            ast_sgrep_lang::Language::Ruby if ruby_hash_all_inside_regex_literals(pattern) => {
                scan.hash = false;
            }
            ast_sgrep_lang::Language::Php if php_leading_attribute_function_face(pattern) => {
                scan.hash = false;
                scan.hash_bracket = false;
            }
            _ => {}
        }
    }
    scan
}

/// True when EVERY quote-external `#` in `pattern` lies inside a `/.../`
/// regex literal — the interpolation `#{...}` and plain `#` bodies alike.
/// A `/` opens a regex literal when division is impossible: pattern start,
/// after an operator/open-bracket byte, or after a ruby control keyword;
/// after an identifier, number, or closing bracket it is division and
/// following `#` bytes are real comments (`re = /ab#{x}cd/` regex vs
/// `a / b # c` / `m2 = (p1 + p2) / 2 # tail` division). Escapes, `[...]`
/// char classes, and `#{...}` interpolation nesting are tracked; strings
/// are quote-excluded by the caller's blind scan and re-checked here the
/// same way.
fn ruby_hash_all_inside_regex_literals(pattern: &str) -> bool {
    const KEYWORDS: [&str; 16] = [
        "if", "elsif", "unless", "while", "until", "and", "or", "not", "then", "when", "case",
        "begin", "do", "else", "return", "yield",
    ];
    let bytes = pattern.as_bytes();
    let mut saw_hash = false;
    let mut hash_outside_regex = false;
    let mut quote: Option<u8> = None;
    let mut in_regex = false;
    let mut in_class = false;
    let mut interp_depth = 0usize;
    // last significant (non-whitespace) byte outside quotes/regex; 0 = none yet
    let mut last_sig: u8 = 0;
    let mut i = 0usize;
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
        if in_regex {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'#' {
                saw_hash = true;
                if bytes.get(i + 1) == Some(&b'{') && interp_depth == 0 {
                    interp_depth = 1;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if interp_depth > 0 {
                if b == b'{' {
                    interp_depth += 1;
                } else if b == b'}' {
                    interp_depth -= 1;
                }
                i += 1;
                continue;
            }
            if b == b'[' {
                in_class = true;
            } else if b == b']' {
                in_class = false;
            } else if b == b'/' && !in_class {
                in_regex = false;
                last_sig = b'/';
            }
            i += 1;
            continue;
        }
        if b == b'#' {
            saw_hash = true;
            hash_outside_regex = true;
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' || b == b'`' {
            quote = Some(b);
            i += 1;
            continue;
        }
        if b == b'/'
            && bytes.get(i + 1) != Some(&b'=')
            && ruby_regex_plausible(bytes, i, last_sig, &KEYWORDS)
        {
            in_regex = true;
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() {
            last_sig = b;
        }
        i += 1;
    }
    saw_hash && !hash_outside_regex
}

/// Whether a `/` at `at` can open a ruby regex literal: yes at pattern
/// start, after an operator/open-bracket byte, or directly after a control
/// keyword; no after an identifier/number/closing bracket (division).
fn ruby_regex_plausible(bytes: &[u8], at: usize, last_sig: u8, keywords: &[&str]) -> bool {
    if last_sig == 0 {
        return true;
    }
    if matches!(
        last_sig,
        b'(' | b'['
            | b'{'
            | b','
            | b';'
            | b'='
            | b'~'
            | b'!'
            | b'?'
            | b':'
            | b'&'
            | b'|'
            | b'+'
            | b'-'
            | b'*'
            | b'%'
            | b'<'
            | b'>'
    ) {
        return true;
    }
    let mut end = at;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_alphabetic() {
        start -= 1;
    }
    std::str::from_utf8(&bytes[start..end])
        .map(|word| keywords.contains(&word))
        .unwrap_or(false)
}

/// True when `pattern` is a php attribute face the reference accepts — the
/// first non-whitespace bytes are `#[`, brackets balance (quote- and
/// escape-aware), and the remainder after the matching `]` is empty or a
/// `function` declaration. Attribute-after-code and attribute+property
/// tails are NOT this face (the reference refuses them) and keep the
/// comment flag.
pub(crate) fn php_leading_attribute_function_face(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let Some(start) = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .filter(|first| bytes[*first..].starts_with(b"#["))
    else {
        return false;
    };
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut i = start;
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
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let rest = pattern[i + 1..].trim_start();
                    return rest.is_empty() || rest.starts_with("function");
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// True when a quote-external, depth-0 `=` (an assignment, not `==` `=>`
/// `<=` `>=` `!=`) occurs BEFORE the first quote-external `/*` comment
/// start. The reference's php pattern gate refuses assignment-rooted
/// metavariable patterns whose comment trivia sits in the assignment tail
/// (`$A = /* w */ $B;`, `$A = f(/* w */ $B);`) while call/operand/before-`=`
/// comment faces answer. Textual proxy for that per-language gate;
/// php-only like the php lanes it mirrors.
fn php_comment_follows_top_level_assignment(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut quote: Option<u8> = None;
    let mut depth = 0usize;
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
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'/' if bytes.get(i + 1) == Some(&b'*') => return false,
            b'=' if depth == 0 => {
                let prev = if i == 0 { b' ' } else { bytes[i - 1] };
                let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                if !matches!(next, b'=' | b'>') && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// The block-placement acceptance shared by both routing routes.
/// Comment-free (`!block`) is accepted; a LEADING block comment is refused
/// in EVERY language (the reference refuses the placement everywhere
/// probed); the php inline non-leading non-trailing placement keeps the
/// hook-family exemption; every other block placement (trailing, unclosed,
/// non-php) is refused.
fn block_placement_sg_accepted(lang: ast_sgrep_lang::Language, scan: PatternCommentScan) -> bool {
    if !scan.block {
        return true;
    }
    if scan.block_leading {
        return false;
    }
    matches!(lang, ast_sgrep_lang::Language::Php) && !scan.block_trailing
}

/// Whether the reference accepts a NeverMatches pattern's comment
/// placement in `lang`: comment-free faces (the walk decides) and INLINE
/// `/* */` faces in php (the hook family) are exempt from the skip +
/// census. Every glued line comment and every trailing/unclosed block
/// comment keeps the census-loud contract — the reference refuses the
/// registered spellings (py/rb/php `$x # note`, php/rust `$x // note`,
/// `;`-statement-trailing `//`/`/*` in every probed language). A LEADING
/// block comment is exempt in NO language
/// (`block_placement_sg_accepted`) — the reference refuses it everywhere
/// probed, and the php walk answered it silent where the reference refused
/// the pattern. For the `#`-as-syntax languages (rust/c/cpp/go/js/ts) the
/// census fallback finds the face answerable and the walk decides.
pub(crate) fn never_matches_comment_placement_sg_accepted(
    lang: ast_sgrep_lang::Language,
    scan: PatternCommentScan,
) -> bool {
    !scan.line && !scan.hash && block_placement_sg_accepted(lang, scan)
}

/// The same comment-placement acceptance must govern the
/// template-exists route. A `$`-less pattern returns answerable from
/// `native_pattern_answerable` before that function's comment refusal is
/// ever consulted (the `!contains('$')` arm), so a comment face that dodges
/// NeverMatches classification built a template over the comment trivia,
/// walked, and answered a silent empty where the reference refuses the
/// placement (py/rb hash faces, js/rust/ts/php leading blocks).
///
/// PLACEMENT-AWARE narrowing: blanket arms over-refused placements the
/// reference ANSWERS — the two registered rust container-slot block faces
/// flipped census-loud, and py floor-div faces (`a // b`: `//` is NOT
/// comment syntax in python, the reference answers hits). The narrowed gate:
///   - hash `#`: refused in python/ruby/php at EVERY placement — leading
///     and trailing are reference-refused, and the EMBEDDED placement is
///     UNREACHABLE through the query layer, which whitespace-collapses the
///     newline that would terminate the comment, so no embedded-hash
///     spelling can ever reach this gate. `#`-as-syntax languages
///     (rust/c/cpp/go/js/ts) keep answering (`#include <stdio.h>`,
///     `#[derive($A)]`, `this.#x = 1`).
///   - line `//`: refused in every language EXCEPT python (floor-div, the
///     reference answers `a // b`); leading/trailing spellings are
///     reference-refused or registered-census-loud, and the embedded-line
///     acceptance the reference shows (`a + // c\n b` answers hits in
///     rust) is unreachable for the same newline-collapse reason.
///   - block: LEADING and TRAILING refused in every language; ruby inline
///     refused (no `/* */` syntax — the reference refuses); INLINE
///     non-leading non-trailing blocks ACCEPTED in every other language:
///     the reference is lenient on `a + /* c */ b` everywhere probed and
///     ANSWERS the rust container-slot faces via the comment-slot
///     machinery (php's hook-family exemption is the already-accepted
///     subset).
///
/// `#[` gets its own arms. In ts/js the pair is INVALID syntax — the
/// reference refuses the face — so a quote-external `hash_bracket`
/// refuses. In php the language-aware scan has ALREADY cleared the flag
/// for the leading-attribute faces the reference accepts; everything else
/// `#`-carrying keeps the py/rb/php refusal.
/// Php assignment-rooted patterns whose block-comment trivia sits AFTER a
/// top-level `=` are reference-refused — the php pattern gate refuses the
/// metavariable assignment doc — so they stay refused even at the inline
/// placement the other ten languages answer. The `pattern` parameter feeds
/// that textual proxy; both consumers (template gate + lane override) call
/// this one function so the class cannot drift apart.
/// NeverMatches-classified faces never take this gate — their loudness
/// comes from the lang-side refusal inside `native_pattern_answerable` and
/// their walk-decided registered silent cases (rust/js `$x # note`) must
/// not move.
pub(crate) fn comment_placement_template_route_refused(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    scan: PatternCommentScan,
) -> bool {
    use ast_sgrep_lang::Language;
    if scan.hash_bracket && matches!(lang, Language::TypeScript | Language::JavaScript) {
        return true;
    }
    // `$`-carrying only: the `$`-LESS assignment-rooted php faces are the
    // hook family the walk answers (accepted placement
    // `alpha = 1 + /* c */ 2;`) — refusing them here flipped them loud.
    // The reference-refused class is entirely metavariable-carrying.
    if matches!(lang, Language::Php)
        && pattern.contains('$')
        && scan.block
        && php_comment_follows_top_level_assignment(pattern)
    {
        return true;
    }
    if scan.hash && matches!(lang, Language::Python | Language::Ruby | Language::Php) {
        return true;
    }
    if scan.line && !matches!(lang, Language::Python) {
        // The meta-free whole-token literal route takes the literal
        // trailing-comment lane's verdict — the reference parses the
        // trailing line comment as a real child and ANSWERS the
        // comment-carrying js/ts rows (`q(µAble) // c`), while the subject
        // rejected the whole class. The lane admits exactly the
        // reference-accepted, cleanly-parsing, trailing-line-comment faces
        // (the `$`-carrying faces and the go/rust/java/csharp refused
        // spellings stay refused — the lane refuses them by construction),
        // so the walk decides every admitted face and the loud fold keeps
        // everything else exactly where registered.
        if !pattern.contains('$') && ast_sgrep_lang::literal_trailing_comment_lane(lang, pattern) {
            return false;
        }
        return true;
    }
    if scan.block && (scan.block_leading || scan.block_trailing || matches!(lang, Language::Ruby)) {
        return true;
    }
    false
}
