use std::collections::HashMap;

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
/// [`PatternCommentScans::for_lang`] refines them per language for the
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

/// Blind + per-language comment scans computed ONCE per `search_pattern*`
/// entry (O3). The old ingress path re-ran the same scans 1 + N×2 times per
/// search: blind at the gate, blind + per-language refinement per language
/// in `comment_carrying_face_gate_accepted`, and blind again per native-walk
/// entry with a per-file-route refinement. The values here are byte-identical
/// to the rescanned ones — only the repetition is gone.
#[derive(Clone)]
pub(crate) struct PatternCommentScans {
    blind: PatternCommentScan,
    per_lang: Vec<(ast_sgrep_lang::Language, PatternCommentScan)>,
    /// Per-language walk-answerability verdicts, filled once at search
    /// ingress (`with_answerability`) and consulted by the native walk's
    /// census + per-file skip (d-NEEDLESS hoist 1). Empty until filled —
    /// lookups on an unfilled table answer `false` (walk decides).
    unanswerable: HashMap<ast_sgrep_lang::Language, bool>,
}

impl PatternCommentScans {
    pub(crate) fn compute(pattern: &str) -> Self {
        let blind = scan_pattern_comment_syntax(pattern);
        let per_lang = ast_sgrep_lang::Language::all()
            .iter()
            .map(|lang| (*lang, refine_blind_for_lang(*lang, pattern, blind)))
            .collect();
        Self {
            blind,
            per_lang,
            unanswerable: HashMap::new(),
        }
    }

    /// The language-blind scan: feeds the NeverMatches route and the
    /// lane-refusal override precondition, exactly as the old per-gate
    /// blind rescan did.
    pub(crate) fn blind(&self) -> PatternCommentScan {
        self.blind
    }

    /// The language-aware refinement for `lang`: feeds the template route.
    /// Falls back to the blind scan for unlisted languages (defensive only —
    /// every `Language::all()` member is precomputed above).
    pub(crate) fn for_lang(&self, lang: ast_sgrep_lang::Language) -> PatternCommentScan {
        self.per_lang
            .iter()
            .find_map(|(candidate, scan)| (*candidate == lang).then_some(*scan))
            .unwrap_or(self.blind)
    }

    /// Comment-free patterns may take the exact-signature index lane; every
    /// comment-carrying spelling stays walk-decided or refused.
    pub(crate) fn comment_free(&self) -> bool {
        let blind = self.blind;
        !(blind.line || blind.block || blind.hash)
    }

    /// Fill the per-language walk-answerability table for `walk_pattern`
    /// (d-NEEDLESS hoist 1): one classification verdict per language per
    /// search, computed here at ingress instead of re-derived per file in
    /// the walk. Call with the exact spelling the walk gates on (the
    /// de-collided spelling on the search path). Consumes and returns
    /// `self` so the ingress can rebind without `mut`.
    pub(crate) fn with_answerability(self, walk_pattern: &str) -> Self {
        let unanswerable = compute_language_unanswerable_table(walk_pattern, &self);
        Self {
            unanswerable,
            ..self
        }
    }

    /// The ingress-computed walk-answerability table, for walk helpers that
    /// take it directly (`&HashMap<Language, bool>`).
    pub(crate) fn unanswerable_table(&self) -> &HashMap<ast_sgrep_lang::Language, bool> {
        &self.unanswerable
    }
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
/// Refine a precomputed blind scan for `lang` without rescanning the blind
/// bytes. [`PatternCommentScans::compute`] runs the blind scan once and maps
/// this over every language; the removed `scan_pattern_comment_syntax_for`
/// wrapper (blind rescan per language) was its only other caller.
fn refine_blind_for_lang(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    blind: PatternCommentScan,
) -> PatternCommentScan {
    let mut scan = blind;
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

/// The single comment-placement decision table. Rows = comment-content
/// facts × language class; columns = routing route (NeverMatches vs
/// template). Every placement rule the reference probes is exactly one row
/// here; the three routing arms below are thin filters over these rows, so
/// no rule is narrated twice and never two refusals consult in series.
///
/// Row provenance (reference behavior each row pins):
/// - `HashBracket` in ts/js: `#[` is INVALID syntax there — refused on both
///   routes. Elsewhere the bracket rides the plain hash row (rust
///   `#[derive($A)]` answers on the template route).
/// - `Hash`: refused in python/ruby/php at EVERY placement — leading and
///   trailing are reference-refused, and the EMBEDDED placement is
///   UNREACHABLE through the query layer, which whitespace-collapses the
///   newline terminating the comment. `#`-as-syntax languages
///   (rust/c/cpp/go/js/ts) keep answering; the NeverMatches route refuses
///   every hash face.
/// - `Line`: refused everywhere EXCEPT python (floor-div — the reference
///   answers `a // b`). The embedded-line acceptance the reference shows
///   (`a + // c\n b` answers in rust) is unreachable for the same
///   newline-collapse reason. The NeverMatches route refuses every line.
/// - `BlockLeading`: refused in EVERY language on both routes (a second
///   top-level node; the php walk answered it silent where the reference
///   refuses).
/// - `BlockTrailing`: refused in every language on both routes (registered
///   `;`-statement-trailing `//`/`/*` faces).
/// - `BlockInline`: the php hook-family exemption on the NeverMatches route
///   (assignment-hook and operand-template lanes decide comments by AST
///   structure); refused for ruby on the template route (no `/* */` syntax
///   — the reference refuses) and ACCEPTED in every other language (the
///   reference is lenient on `a + /* c */ b`, answering the rust
///   container-slot faces through the comment-slot machinery).
/// - `PhpAssignmentTail`: php `$`-carrying assignment-rooted patterns whose
///   block-comment trivia sits after a top-level `=` are reference-refused
///   (the php pattern gate refuses the metavariable assignment doc) — even
///   at the inline placement the other ten languages answer. Never emitted
///   without a pattern (the blind route has no textual proxy).
/// - `LiteralTrailingLane`: the meta-free whole-token literal route's
///   trailing line-comment faces (`q(µAble) // c`) — the reference parses
///   the trailing comment as a real child and ANSWERS the js/ts rows. The
///   lane admits exactly the reference-accepted, cleanly-parsing,
///   trailing-line-comment faces (the verdict arrives precomputed — the
///   gate no longer calls back into lang); everything else keeps its row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlacementRoute {
    NeverMatches,
    Template,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlacementFact {
    HashBracket,
    Hash,
    Line,
    BlockLeading,
    BlockTrailing,
    BlockInline,
    PhpAssignmentTail,
    LiteralTrailingLane,
}

/// Derive the table facts for (`lang`, `scan`). `pattern` feeds the
/// php-assignment-tail textual proxy the scans cannot see (`None` on the
/// blind route — no proxy there); `lane_admits` is the precomputed
/// literal-trailing-lane verdict (evaluated once at ingress/table-build, so
/// this gate never calls back into lang). The derivation preserves the
/// historical arm order: the lane acceptance short-circuits the block arm
/// (it returned before the block arm ran) but never the earlier
/// bracket/hash arms, whose rows keep refusing those faces.
fn placement_facts(
    lang: ast_sgrep_lang::Language,
    scan: PatternCommentScan,
    pattern: Option<&str>,
    lane_admits: bool,
) -> Vec<PlacementFact> {
    use ast_sgrep_lang::Language;
    let mut facts = Vec::new();
    let bracket_fires = scan.hash_bracket && matches!(lang, Language::TypeScript | Language::JavaScript);
    let hash_fires = scan.hash && matches!(lang, Language::Python | Language::Ruby | Language::Php);
    // The lane consult lives in the line arm: `-less` trailing-line-comment
    // faces the lane admits (it refuses everything else by construction).
    let lane_face = scan.line && !matches!(lang, Language::Python) && lane_admits;
    if lane_face {
        facts.push(PlacementFact::LiteralTrailingLane);
        // An earlier arm already refused the face — its row carries the
        // refusal; otherwise the lane acceptance stands alone.
        if !(bracket_fires || hash_fires) {
            return facts;
        }
    } else if scan.line {
        facts.push(PlacementFact::Line);
    }
    if scan.hash_bracket {
        facts.push(PlacementFact::HashBracket);
    }
    if scan.hash {
        facts.push(PlacementFact::Hash);
    }
    if scan.block {
        // The lane face returned before the block arm ran — unless an
        // earlier arm already refused (handled above), no block fact applies.
        if !(lane_face && !bracket_fires && !hash_fires) {
            facts.push(if scan.block_leading {
                PlacementFact::BlockLeading
            } else if scan.block_trailing {
                PlacementFact::BlockTrailing
            } else {
                PlacementFact::BlockInline
            });
        }
        if pattern.is_some_and(|p| {
            matches!(lang, Language::Php)
                && p.contains('$')
                && php_comment_follows_top_level_assignment(p)
        }) {
            facts.push(PlacementFact::PhpAssignmentTail);
        }
    }
    facts
}

/// One row of the placement table: the per-route verdict for a single
/// content fact. Faces carrying several facts take the AND across rows —
/// any refusing row refuses the face on that route.
fn placement_row(
    route: PlacementRoute,
    lang: ast_sgrep_lang::Language,
    fact: PlacementFact,
) -> bool {
    use ast_sgrep_lang::Language;
    use PlacementFact::*;
    match fact {
        HashBracket => !matches!(lang, Language::TypeScript | Language::JavaScript),
        Hash => match route {
            PlacementRoute::NeverMatches => false,
            PlacementRoute::Template => {
                !matches!(lang, Language::Python | Language::Ruby | Language::Php)
            }
        },
        Line => match route {
            PlacementRoute::NeverMatches => false,
            PlacementRoute::Template => matches!(lang, Language::Python),
        },
        BlockLeading | BlockTrailing => false,
        BlockInline => match route {
            PlacementRoute::NeverMatches => matches!(lang, Language::Php),
            PlacementRoute::Template => !matches!(lang, Language::Ruby),
        },
        PhpAssignmentTail => matches!(route, PlacementRoute::NeverMatches),
        LiteralTrailingLane => matches!(route, PlacementRoute::Template),
    }
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

/// Whether the reference accepts a NeverMatches pattern's comment placement
/// in `lang` — a thin arm over the placement table's NeverMatches column
/// (see [`placement_row`]): comment-free faces (the walk decides) and
/// INLINE `/* */` faces in php (the hook family) are exempt from the skip +
/// census; every other placement keeps the census-loud contract. (The old
/// shared block-placement helper folded into the table's block rows.)
pub(crate) fn never_matches_comment_placement_sg_accepted(
    lang: ast_sgrep_lang::Language,
    scan: PatternCommentScan,
) -> bool {
    placement_facts(lang, scan, None, false)
        .into_iter()
        .all(|fact| placement_row(PlacementRoute::NeverMatches, lang, fact))
}

/// Per-language walk-answerability table for one search (d-NEEDLESS hoist
/// 1). The native walk used to re-derive these classification verdicts per
/// file per language (the `matcher_decides` / `template_comment_refused` /
/// `language_unanswerable` closures); they depend only on the walk spelling
/// and the precomputed scans, so search ingress computes them once and the
/// walk consults the table. Verdicts are byte-identical to the old per-file
/// closures — only the repetition is gone.
fn compute_language_unanswerable_table(
    pattern: &str,
    scans: &PatternCommentScans,
) -> HashMap<ast_sgrep_lang::Language, bool> {
    let comment_scan = scans.blind();
    let comment_free = scans.comment_free();
    let never_matches = matches!(
        ast_sgrep_lang::classify_native(pattern),
        Some(ast_sgrep_lang::NativeKind::NeverMatches)
    );
    ast_sgrep_lang::Language::all()
        .iter()
        .map(|lang| {
            (
                *lang,
                walk_language_unanswerable(
                    *lang,
                    pattern,
                    scans,
                    comment_scan,
                    comment_free,
                    never_matches,
                ),
            )
        })
        .collect()
}

/// Moved verbatim from the old walk-local `matcher_decides` closure: the
/// NeverMatches exemption is per-language because the accepted block
/// placement is language-scoped (php).
fn walk_matcher_decides(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    comment_scan: PatternCommentScan,
    comment_free: bool,
    never_matches: bool,
) -> bool {
    never_matches
        && never_matches_comment_placement_sg_accepted(lang, comment_scan)
        && (comment_free && ast_sgrep_lang::native_pattern_answerable(lang, pattern)
            || (!comment_free && ast_sgrep_lang::php_comment_transparent_operand_lane(pattern)))
}

/// Moved verbatim from the old walk-local `template_comment_refused`
/// closure. The trailing-comment lane verdict arrives precomputed
/// (`literal_trailing_lane_verdict`) — the gate no longer calls into lang.
fn walk_template_comment_refused(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    scans: &PatternCommentScans,
    never_matches: bool,
) -> bool {
    if never_matches {
        return false;
    }
    let scan = scans.for_lang(lang);
    comment_placement_template_route_refused(
        lang,
        pattern,
        scan,
        literal_trailing_lane_verdict(lang, pattern, scan),
    )
}

/// Moved verbatim from the old walk-local `language_unanswerable` closure:
/// a language is unanswerable when the NeverMatches exemption does not cover
/// it AND (the comment gate refuses the placement OR the lang-side gate
/// finds no template), with the single scoped lane-refusal override.
fn walk_language_unanswerable(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    scans: &PatternCommentScans,
    comment_scan: PatternCommentScan,
    comment_free: bool,
    never_matches: bool,
) -> bool {
    if walk_matcher_decides(lang, pattern, comment_scan, comment_free, never_matches) {
        return false;
    }
    if walk_template_comment_refused(lang, pattern, scans, never_matches) {
        return true;
    }
    if !ast_sgrep_lang::native_pattern_answerable(lang, pattern) {
        // The lane refusal stands (language unanswerable) UNLESS the
        // override fires: non-NeverMatches AND the pattern actually
        // carries comment bytes — i.e. the refusal is the lane's
        // placement-unaware/content-blind comment verdict on a face the
        // core adjudication accepted above. Carve-out: a `$`-carrying
        // php attribute face keeps the refusal — the walk cannot bind
        // metas inside attribute argument lists.
        let php_attribute_carved = matches!(lang, ast_sgrep_lang::Language::Php)
            && comment_scan.hash_bracket
            && php_leading_attribute_function_face(pattern);
        let lane_refusal_overridden = !never_matches
            && !php_attribute_carved
            && (comment_scan.line || comment_scan.block || comment_scan.hash);
        !lane_refusal_overridden
    } else {
        false
    }
}

/// The same comment-placement acceptance governs the template-exists route —
///
/// A `$`-less pattern returns answerable from `native_pattern_answerable`
/// before that function's comment refusal is ever consulted, so this thin
/// arm over the placement table's template column (see [`placement_row`])
/// keeps those faces honest. A `$`-less literal trailing line-comment face
/// the lane admits (`q(µAble) // c`) is accepted and the walk decides;
/// every other refused placement keeps its census-loud class.
/// NeverMatches-classified faces never take this gate.
/// Parse-backed trailing line-comment verdict for the template-route gate,
/// evaluated ONCE per (language, pattern) at ingress/table-build and passed
/// in below (d-NEEDLESS hairpin). The gate used to call
/// `ast_sgrep_lang::literal_trailing_comment_lane` itself — a core→lang→core
/// hairpin inside the hottest placement gate. Reaching the arm needs a
/// line-comment scan in a non-python language on a meta-free spelling, so
/// unreachable faces skip the parse-backed consult entirely.
pub(crate) fn literal_trailing_lane_verdict(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    scan: PatternCommentScan,
) -> bool {
    scan.line
        && !matches!(lang, ast_sgrep_lang::Language::Python)
        && !pattern.contains('$')
        && ast_sgrep_lang::literal_trailing_comment_lane(lang, pattern)
}

/// `literal_trailing_lane_admits` arrives precomputed from
/// ingress/table-build — the gate itself no longer calls back into lang.
pub(crate) fn comment_placement_template_route_refused(
    lang: ast_sgrep_lang::Language,
    pattern: &str,
    scan: PatternCommentScan,
    literal_trailing_lane_admits: bool,
) -> bool {
    !placement_facts(lang, scan, Some(pattern), literal_trailing_lane_admits)
        .into_iter()
        .all(|fact| placement_row(PlacementRoute::Template, lang, fact))
}
