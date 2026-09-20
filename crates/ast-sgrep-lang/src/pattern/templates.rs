//! If/optional-call/body parsers, dollar tokens, arg slots.

use super::*;
use crate::extract::node_text;
use crate::Language;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, RwLock};
use tree_sitter::{Node, Parser, Query};

/// `HEAD?.$TAIL` ends: (head_capture, head_literal, tail_capture, tail_literal).
type OptionalCallEnds = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Parse the two-segment optional-chain callee spelling `HEAD?.$TAIL` — the
/// trailing `?` on the head marks the `?.` connector. Php spells the
/// nullsafe connector `?->` — the marker rides BETWEEN head and tail
/// (`$O?->$M`), so it splits there instead. Arbitrary expression heads have
/// no structural contract here and stay fail-closed; only metavariable and
/// plain-identifier heads classify.
pub(crate) fn parse_optional_call_path(callee: &str) -> Option<OptionalCallEnds> {
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
        // A lowercase-led `$name` head is php variable TEXT — the literal head
        // compares byte-exactly against the object node.
        None if dollar_name_class(head.strip_prefix('$').unwrap_or(""))
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
pub(crate) fn parse_function_tail(tail: &str) -> Option<Option<BodyTemplate>> {
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
/// Identifiers like `iffy(...)` are not if-prefixed. The reading is aligned
/// with `classify_if_template`, which tolerates ANY whitespace after the
/// keyword (`trim_start`). The old space/paren-only acceptance re-armed the
/// silent drop one whitespace away: `if\n($X) { $B }` classified as If yet
/// skipped the argument-capture skip here, so `capture_arguments` misbound
/// `$X` from the condition's inner call and `if_condition_capture`
/// conflicted → candidates silently dropped (and the paren-free py spelling
/// lost its `: $B` body binding via [`body_capture`]).
pub(crate) fn is_if_prefixed(p: &str) -> bool {
    p.strip_prefix("if")
        .is_some_and(|rest| rest.starts_with('(') || rest.starts_with(char::is_whitespace))
}

/// Classify an if-template condition: a single metavariable rides the capture
/// path (`None`); a concrete spelling-admissible condition is kept as text.
/// Anything else refuses.
fn if_cond_slot(condition: &str) -> Option<Option<String>> {
    if is_single_metavariable(condition) {
        Some(None)
    } else if if_cond_admissible(condition) {
        Some(Some(condition.to_string()))
    } else {
        None
    }
}

/// Parse `if ($COND) { $BODY }` / `if $COND { $BODY }` / `if $COND: $BODY`.
///
/// A SINGLE-METAVARIABLE condition rides the registered capture path
/// (`cond: None`); a CONCRETE condition joins the lane when it is
/// spelling-admissible — the per-language template build then decides
/// answerability at the gate, and the walk binds cond metas through
/// `general_eq`. Unsupported shapes keep the `None` fail-closed refusal —
/// never fall through to call classification.
///
/// The pattern body's brace-ness is threaded through (`body_braced`) because
/// it is structural — see the [`NativeKind::If`] doc. A brace-less meta body
/// still refuses here, keeping the loud class for `if ($X) $B` byte-stable.
pub(crate) fn classify_if_template(p: &str) -> Option<NativeKind> {
    let rest = p.strip_prefix("if")?.trim_start();
    let (condition, after) = if let Some(inner) = rest.strip_prefix('(') {
        // The section close is the paren that BALANCES the leading `(` — a
        // depth scan. A first-`)` slice would truncate any paren-bearing
        // condition (`if (f($X))` would read cond `f($X`).
        let close = balanced_paren_close(inner)?;
        (inner[..close].trim(), inner[close + 1..].trim_start())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '{' || c == ':')
            .unwrap_or(rest.len());
        (rest[..end].trim(), rest[end..].trim_start())
    };
    let cond = if_cond_slot(condition)?;
    // The else tail parses only after a BRACED body — the balanced-brace scan
    // finds the body section's true extent so the tail after it can be read.
    // A `:`-suite form keeps its registered else-less class (py `else`
    // clauses are unprobed; the suite-text extent is line-ambiguous at the
    // spelling level).
    let (body, body_braced, rest) = if let Some(inner) = after.trim_start().strip_prefix('{') {
        let close = balanced_brace_close(inner)?;
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
        Some(r) => {
            let alternative = parse_else_tail(r)?;
            Some(alternative)
        }
        None => None,
    };
    Some(NativeKind::If {
        cond,
        body,
        body_braced,
        alternative,
    })
}

/// Byte index of the `}` that closes the section opened by a leading `{` —
/// the same depth scan [`balanced_paren_close`] runs for conditions. String
/// literals carrying unbalanced braces inside a body are an unprobed residual
/// (the scan may mis-terminate; the face keeps its fail-closed class
/// downstream).
pub(crate) fn balanced_brace_close(inner: &str) -> Option<usize> {
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

/// The body-section grammar shared by the head and the else tail — the
/// [`parse_body_template_braced`] acceptance WITHOUT its outer-brace spelling
/// (the caller already unwrapped and consumed the braces).
pub(crate) fn parse_body_section(inner: &str) -> Option<Option<BodyTemplate>> {
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

/// The `else …` tail after an if template's body section. Only the two
/// covered spellings admit: `else { B }` and `else if (COND) { B }`
/// (recursively, so deep chains parse). Anything else keeps the face's
/// registered census-loud class.
pub(crate) fn parse_else_tail(rest: &str) -> Option<IfAlternative> {
    let rest = rest.trim();
    let tail = rest.strip_prefix("else")?.trim_start();
    if let Some(if_rest) = tail.strip_prefix("if") {
        let if_rest = if_rest.trim_start();
        let inner = if_rest.strip_prefix('(')?;
        let close = balanced_paren_close(inner)?;
        let condition = inner[..close].trim();
        let cond = if_cond_slot(condition)?;
        // Per-level bare-meta captures: each tail level binds ITS OWN
        // section's meta — the pattern-global second-paren / last-brace
        // scans re-bind the wrong names at depth ≥ 2 and the same-name
        // conflict refuses the whole chain.
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

/// Spelling-level admission for a concrete if condition. Trivia and
/// container spellings (comments, braces, terminators, ternaries, colon
/// families) stay out; every `$` token must substitute canonically. Deeper
/// exactness is the per-language template build (the answerability gate
/// demands it) plus `general_eq` at walk time.
pub(crate) fn if_cond_admissible(cond: &str) -> bool {
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

/// Byte index of the `)` that closes the section opened by a leading `(` —
/// a depth scan over the raw text. String literals carrying unbalanced
/// parens inside a condition are an unprobed residual: the scan may
/// mis-terminate and the face keeps its fail-closed refusal downstream.
pub(crate) fn balanced_paren_close(inner: &str) -> Option<usize> {
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
/// was BRACED. The reference's if-matching is brace-ness structural, so
/// the If lane needs the flag; the shared [`parse_body_template`] keeps its
/// function/class signature and discards the flag.
///
/// A BARE `:` suite section is the EMPTY suite, not "no constraint" —
/// a suite-less statement never exists, so the bare-colon arm yields
/// `Exactly(0)` instead of an unconstrained body. The old unconstrained
/// reading over-answered those py faces.
pub(crate) fn parse_body_template_braced(after: &str) -> Option<(Option<BodyTemplate>, bool)> {
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
        // `{}` matches an empty body; a bare `:` suite is the reference's EMPTY suite.
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
/// discarded (the bare-colon EMPTY-suite correction still applies through
/// the shared arm).
pub(crate) fn parse_body_template(after: &str) -> Option<Option<BodyTemplate>> {
    parse_body_template_braced(after).map(|(body, _)| body)
}

/// True when the pattern's root is the universal node metavariable
/// (`$$NAME`), expando spellings normalized first. The core census uses this
/// to keep the universal lane's registered line-collapsed rows by
/// suppressing dedup byte spans there.
pub fn is_universal_root_pattern(lang: Language, pattern: &str) -> bool {
    let normalized = normalize_expando_meta_spelling(lang, pattern.trim());
    normalized
        .trim()
        .strip_prefix("$$")
        .is_some_and(is_metavar_name)
}

/// `$NAME` — exactly one metavariable, not `$$$`.
pub(crate) fn is_single_metavariable(s: &str) -> bool {
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

/// Canonical metavariable NAME := ASCII `[A-Z_][A-Z0-9_]*` — underscore-led
/// OK, digit tail OK, but a lowercase byte ANYWHERE in the tail (`$ABc`,
/// `$A1b`, `$A_b`, `$_a`) is NOT canonical: every mixed-case tail token
/// parses empty / poisons the pattern, while `$A`, `$A1`, `$A_B`, `$A_`,
/// `$_`, `$_A` wildcard-match. Code identifiers keep [`is_pattern_ident`];
/// only `$`-token consumers switch to this grammar, so lowercase code
/// identifiers (`a(1)` patterns, `def a` names) stay literal.
pub(crate) fn is_metavar_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    match bytes.first() {
        Some(&first) if first == b'_' || first.is_ascii_uppercase() => {}
        _ => return false,
    }
    bytes[1..]
        .iter()
        .all(|&b| b == b'_' || b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Classification of one `$`-token NAME against the reference meta grammar.
/// `None` — empty or garbage-led names — keeps the token's existing
/// registered class (loud residuals); those never route here.
///
/// The class is DOLLARS-COUNT-AWARE: `$$`/`$$$`-led NON-canonical tokens
/// answer as literal code in js/ts (identifiers) and `$$`-led lowercase
/// tokens as php variable-variables, but the pattern REFUSES for php
/// `$$ABc`/`$$$/lowercase`, python and rust. Classifying a 3-dollar name
/// by the 1-dollar table made php `$$$u + 2` literal-answer where the
/// reference refuses it — the destructive fail-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DollarTokenClass {
    /// `$A`, `$_` — and the canonical 2/3-dollar runs (`$$A`, `$$$ARGS`):
    /// metavariables that every literal-lane consumer must exclude.
    Canonical,
    /// `$x` — lowercase-LED. NOT a meta: literal code where `$` is name
    /// syntax (js/ts identifiers, php variables), parse error elsewhere.
    LowercaseLed,
    /// `$ABc`, `$_a` — uppercase/underscore-LED with a lowercase byte in the
    /// tail. The reference tokenizer rejects them; php poisons the WHOLE
    /// pattern (ERROR node, answers nothing — probed `echo $ABc;` empty).
    MixedCase,
    /// `$$x`, `$$_x` — 2-dollar lowercase-led: js/ts identifier literal AND
    /// php variable-variable literal (the reference answers both, face present).
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

pub(crate) fn dollar_name_class(name: &str) -> Option<DollarTokenClass> {
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

/// True for a `$$name` LOWERCASE double-dollar token (`$$dyn`) — a php
/// variable-variable literal the reference matches byte-exactly in
/// dynamic-property LINK positions. Canonical `$$DYN` runs are pure
/// metavariables (handled before this check); mixed-case `$$Dyn` keeps the
/// registered refusal.
pub(crate) fn is_dollar2_lowercase_token(raw: &str) -> bool {
    raw.strip_prefix("$$")
        .is_some_and(|name| dollar_name_class(name) == Some(DollarTokenClass::LowercaseLed))
}

/// True when the pattern carries a `$`-token (1, 2, or 3 dollars) of ANY
/// non-canonical class. The gate covers lowercase-LED tokens, MixedCase
/// tails (which the wide `[A-Za-z0-9_]` tail grammar let through the
/// structural wildcard lanes where the reference answers nothing), and the
/// 2/3-dollar non-canonical classes: the refusing `$$`/`$$$` faces keep
/// the NeverMatches accepted-empty class, while the literal faces are
/// intercepted ahead of this gate by [`dollar_literal_lane`]. Canonical
/// `$$A`/`$$_` runs stay with the caller (universal-or-loud), and
/// garbage-led tokens keep their existing classes.
pub(crate) fn pattern_has_noncanonical_metavar(p: &str) -> bool {
    dollar_token_classes(p)
        .iter()
        .any(|&class| !matches!(class, DollarTokenClass::Canonical))
}

/// True when `p` is a call shape whose ONLY non-canonical `$`-tokens are
/// 1-dollar lowercase-led tokens sitting as COMPLETE callee-head segments
/// (before the first `(`, `::`/`->`/`.` delimited). Those tokens are source
/// variable text in the `$`-name languages, and the reference matches them
/// literally; the structural call lanes then compare the segment bytes
/// exactly (`call_path_segment`). Any mixed-case/2/3-dollar class, or any
/// non-canonical token OUTSIDE the head (arguments, operator shapes),
/// keeps the registered NeverMatches class.
pub(crate) fn literal_variable_callee_admitted(p: &str) -> bool {
    let classes = dollar_token_classes(p);
    if !classes.iter().all(|&class| {
        matches!(
            class,
            DollarTokenClass::Canonical | DollarTokenClass::LowercaseLed
        )
    }) || !classes.contains(&DollarTokenClass::LowercaseLed)
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

/// A namespace-qualified callee segment (`\Foo`, `App\Models\User`) —
/// backslash-led or interior `\`-separated plain identifiers. The segment
/// text is matched raw (the scope node's exact source bytes), mirroring the
/// re-keyed `call:` rows.
pub(crate) fn namespace_qualified_segment(segment: &str) -> bool {
    let led = segment.starts_with('\\');
    let rest = segment.strip_prefix('\\').unwrap_or(segment);
    let parts: Vec<&str> = rest.split('\\').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || !is_pattern_ident(part))
    {
        return false;
    }
    led || parts.len() > 1
}

/// Classify every `$`/`$$`/`$$$` token in the pattern (deduplicated).
/// The dollars COUNT participates — a 2-dollar run never rides the 1-dollar
/// table. A canonical 2-dollar run classes `Canonical` like the 1/3-dollar
/// canonical cases — invisibility there made member patterns like
/// `$svc->run($$A)` look all-lowercase-literal to the literal lane, which
/// hijacked them out of the structural member lane into a silent `[]`.
/// Canonical entries keep every consumer contract: the noncanonical
/// predicates ignore them, and the literal lane's arms all exclude
/// `Canonical`, so only the literal hijack changes (bare `$$A` keeps the
/// universal lane, which never consults this table).
pub(crate) fn dollar_token_classes(p: &str) -> Vec<DollarTokenClass> {
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
        if (1..=3).contains(&dollars) {
            let name = p.get(name_start..name_end).unwrap_or("");
            let class = match (dollars, dollar_name_class(name)) {
                (_, None) => None,
                // A 2-dollar canonical name (`$$A`, `$$_`) classes `Canonical`
                // like the 1/3-dollar canonical cases — skipping it hid it
                // from [`dollar_literal_lane`] and hijacked php member
                // patterns into the literal lane.
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

/// True when the pattern's faces answer through LITERAL code semantics —
/// every `$`-token is a non-canonical NAME-class token and the language
/// gives `$` name syntax. js/ts parse such tokens as identifiers and answer
/// the literal faces; php answers lowercase-led variable faces the same
/// way, while a php MixedCase token POISONS the whole pattern (ERROR node,
/// answers nothing) and canonical tokens keep the structural lanes. Mixed
/// canonical+non-canonical patterns keep today's NeverMatches class.
///
/// The rule is dollars-aware: js/ts answer EVERY non-canonical run
/// literally (1-, 2-, and 3-dollar); php answers 1- and 2-dollar
/// LOWERCASE-led runs (variable-variables) but REFUSES 3-dollar runs of
/// any name. Canonical `$$A` runs never land here (the universal lane
/// keeps them); py/rust never enter the lane.
pub(crate) fn dollar_literal_lane(lang: Language, pattern: &str) -> bool {
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
        // Java has NO expando preprocessing and its meta char IS `$` — a
        // 1-run MixedCase token (`$Bx`) fails whole-node meta validation
        // and parses as a literal java IDENTIFIER: route the all-MixedCase
        // class to the literal lane on the raw bytes. The arm is
        // ALL-MixedCase only — a canonical token is a real java meta, so a
        // canonical+mixed mix (`$A + $Bx`) is NOT a literal face. Lowercase-led
        // / multi-run faces keep their registered classes.
        Language::Java => classes
            .iter()
            .all(|&class| class == DollarTokenClass::MixedCase),
        Language::Php => classes.iter().all(|&class| {
            matches!(
                class,
                DollarTokenClass::LowercaseLed | DollarTokenClass::Dollar2Lowercase
            )
        }),
        _ => false,
    }
}

/// The reference's `extract_meta_var` runs over the WHOLE node text. In the
/// no-expando languages (js/ts/java) a `$`-run glued inside a larger
/// identifier token (`µµµ$A`, `$AµµB`, `foo$$$A`) never validates: the full
/// token text fails the `[A-Z_0-9]` grammar, so the reference parses the
/// token as an ordinary identifier and answers the verbatim rows. This
/// mirrors that whole-token validation for one candidate token.
pub(crate) fn sg_whole_token_is_meta(token: &str) -> bool {
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
            tail.chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
                && tail.chars().all(valid_tail)
        }
        // The reference's ellipsis branch: 3 runs strip to an empty or
        // all-valid name; 4+ runs are never metas (literal).
        3 => tail.is_empty() || tail.chars().all(valid_tail),
        _ => false,
    }
}

/// True when every `$`-carrying identifier token in `pattern` is a GLUED
/// literal identifier — its `$`-run continues into a would-be meta name
/// ([A-Z_]) but the WHOLE token text fails the whole-token meta validation
/// (`µµµ$A`, `$AµµB`, `$A$$B`, `foo$$$A`): in the no-expando languages the
/// reference parses these as ordinary identifiers and answers the verbatim
/// rows. Registered genera stay OUT of the class: dollars-only tokens
/// (`$`, `$$`, `g($$)`, `$)(` — the bare-meta loud class), runs continuing
/// into anything but a meta name (`$3`, `$Ü`, `$x`-led faces keep their own
/// lanes), whole canonical meta tokens (`$A`, `$$A`, `$$$A` — the
/// structural meta lanes), and string-literal spellings (`"$A$B"`,
/// `"pre-$A"` — the in-string meta faces).
pub(crate) fn pattern_tokens_are_all_literal(pattern: &str) -> bool {
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
        let dollars = 1 + token[first + 1..]
            .bytes()
            .take_while(|&b| b == b'$')
            .count();
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

pub(crate) fn is_pure_metavariable(arg: &str) -> bool {
    let arg = arg.trim();
    arg.strip_prefix("$$$")
        .or_else(|| arg.strip_prefix('$'))
        .is_some_and(is_metavar_name)
        // A canonical 2-dollar name in an argument slot behaves EXACTLY like
        // `$NAME` — the reference answers `g($$A)` with the same line set
        // and the same capture key `A` as `g($A)`.
        || arg.strip_prefix("$$").is_some_and(is_metavar_name)
}

/// Parse a php member-call argument list into per-position slots — the same
/// token grammar as [`parse_rhs_arg_slots`] (pure metavariable
/// `$$$NAME`/`$NAME`/`$$NAME` slots or single literal TOKENs, at most one
/// rest). The "at least one literal" floor is GONE: meta-only lists
/// classify through the slots now (the `;`-terminated flat spelling had no
/// answering arm — see [`classify_member_call_mixed_args`]) and same-name
/// meta lists inherit bind_capture's conflict semantics. The floor was
/// vacuous at the other two call sites (pure-meta lists never reach them).
pub(crate) fn parse_member_arg_slots(args: &str) -> Option<Vec<ArgSlot>> {
    parse_rhs_arg_slots(args)
}

/// Classify one comma-separated argument part into its slot: a `$$$NAME`
/// rest, a pure `$NAME`/`$$NAME` meta, or — where the grammar admits them —
/// a literal token. Anything else refuses.
fn classify_arg_part(part: &str, allow_literal: bool) -> Option<ArgSlot> {
    if let Some(name) = part.strip_prefix("$$$") {
        is_metavar_name(name).then_some(())?;
        Some(ArgSlot::Rest(name.to_string()))
    } else if is_pure_metavariable(part) {
        Some(ArgSlot::Meta(capture_name(part)?.to_string()))
    } else if allow_literal && (is_dollar_literal_token(part) || is_literal_arg_token(part)) {
        Some(ArgSlot::Literal(part.to_string()))
    } else {
        None
    }
}

/// The assignment-RHS argument-list slot grammar — the shared
/// member/rhs token grammar ([`parse_member_arg_slots`] reuses this body):
/// a meta/rest-only list (`f($A, $$$B)` — probed reference answers) and
/// the sole whole-list rest (`f($$$A)` — answers every arity incl. the
/// empty list) parse here. At most one rest (multiple rests have no
/// unambiguous contract).
pub(crate) fn parse_rhs_arg_slots(args: &str) -> Option<Vec<ArgSlot>> {
    let mut slots = Vec::new();
    let mut rests = 0usize;
    for part in args.split(',') {
        let slot = classify_arg_part(part.trim(), true)?;
        rests += matches!(slot, ArgSlot::Rest(_)) as usize;
        slots.push(slot);
    }
    (rests <= 1).then_some(slots)
}

/// The PLAIN-call rest-slot grammar — pure metavariable slots where at
/// least one is a `$$$NAME` rest. Sole rests and pure-single lists never
/// reach here (earlier templates classify them); literal tokens and
/// 2+-rests-with-singles refuse (fail-closed). A rest sharing its NAME
/// with another slot in the SAME list also refuses: the k >= 2 arm binds
/// every HEAD rest to `""` and only the tail rest to the whole-args text,
/// so a shared name would conflict-bind and silently empty the face.
/// Distinct names only.
pub(crate) fn parse_call_arg_slots(args: &str) -> Option<Vec<ArgSlot>> {
    let mut slots = Vec::new();
    let mut rests = 0usize;
    let mut singles = 0usize;
    for part in args.split(',') {
        let slot = classify_arg_part(part.trim(), false)?;
        rests += matches!(slot, ArgSlot::Rest(_)) as usize;
        singles += matches!(slot, ArgSlot::Meta(_)) as usize;
        slots.push(slot);
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

/// REST-SLOT argument semantics for plain calls — deliberately NOT the php
/// member lane's backtracking matcher: no backtracking of the split
/// (`q($$$A, $B)` answers ONLY the 1-arg row, never the 2-arg row).
///   - a TRAILING rest sharing the list with singles binds >= 1 argument
///     (`q($A, $$$B)` refuses the 1-arg row);
///   - a NON-TRAILING rest binds EXACTLY ZERO and the singles anchor the
///     candidate arity to the single count (`q($$$A, $B)` only the 1-arg
///     row; `q($A, $$$B, $C)` only the 2-arg row);
///   - k >= 2 rests with no singles answer every arity >= k-1 (head rests
///     bind zero, the tail binds all — the split choice is unobservable).
pub(crate) fn call_arg_slots_match(
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
        // tail (the reference answers n >= k-1: two rests -> >= 1 arg,
        // three -> >= 2).
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
                bind_capture(captures, name, text)?;
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

/// A LOWERCASE-led `$var` / `$$var` token in a mixed argument list is
/// LITERAL code — byte-compared against the candidate argument, never a
/// capture (canonical metas stay Meta slots; mixed-case keeps its registered
/// refusal by failing here).
pub(crate) fn is_dollar_literal_token(part: &str) -> bool {
    let dollars = part.bytes().take_while(|&b| b == b'$').count();
    (dollars == 1 || dollars == 2)
        && dollar_name_class(&part[dollars..]) == Some(DollarTokenClass::LowercaseLed)
}

/// A single literal argument token — a balanced quoted string (`'a'`,
/// `"x"`, no `$`) or a plain alphanumeric token (`1`, `-1`, `1.5`, `true`,
/// `FOO`). A comma inside a string literal splits the list at the comma;
/// the unbalanced halves refuse here (fail-closed).
pub(crate) fn is_literal_arg_token(part: &str) -> bool {
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

/// Per-position slot matching. Literal slots compare the candidate argument
/// text byte-exactly; meta slots bind positionally; a rest slot binds the
/// remaining arguments' ORIGINAL source bytes (comma-joined in the source,
/// zero-or-more) in the multi namespace and backtracks over split points.
/// A trailing rest SHARING the list with an earlier slot must bind at least
/// one argument (`$w->q9(1, $$$A)` refuses `q9(1)`, `f($v, $$$A)` refuses
/// `f($v)`); a whole-list rest may bind zero (`q9($$$A)` answers `q9()`)
/// and a non-trailing rest may bind zero (registered mid-rest probes).
pub(crate) fn arg_slots_match(
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
pub(crate) fn arg_slots_match_from(
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
            bind_capture(captures, name, text)?;
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

pub(crate) fn validate_argument_pattern(arguments: &str) -> Option<()> {
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

pub(crate) fn pattern_argument_text(pattern: &str) -> Option<&str> {
    let (pattern, _) = strip_declaration_modifiers(pattern);
    let open = pattern.find('(')?;
    let tail = pattern.get(open + 1..)?;
    let close = tail.find(')')?;
    Some(tail[..close].trim())
}

pub(crate) fn argument_template(pattern: &str) -> Option<ArgumentTemplate> {
    let arguments = pattern_argument_text(pattern)?;
    if arguments.starts_with("$$$") {
        Some(ArgumentTemplate::Any)
    } else if arguments.is_empty() {
        Some(ArgumentTemplate::Exactly(0))
    } else {
        Some(ArgumentTemplate::Exactly(arguments.split(',').count()))
    }
}

pub(crate) fn parse_call_path(callee: &str) -> Option<Vec<Option<String>>> {
    let callee = callee.strip_prefix("::").unwrap_or(callee);
    // Php spells the member connector `->` (the nullsafe `?->` keeps its
    // dedicated lane — the leftover `?` fails the segment check below,
    // exactly like the `.`-lane refusal of `?.`). `::` and `->` both
    // normalize to the dotted separator.
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
/// lowercase-led `$variable` token (source variable text in the `$`-name
/// languages, matched byte-exactly like the reference), or a
/// namespace-qualified scope.
pub(crate) fn call_path_segment(part: &str) -> Option<Option<String>> {
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

pub(crate) fn parse_source(lang: Language, source: &str) -> anyhow::Result<tree_sitter::Tree> {
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
pub(crate) type QueryCache = RwLock<HashMap<(Language, usize), Option<Arc<Query>>>>;

pub(crate) fn compiled_query(
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
