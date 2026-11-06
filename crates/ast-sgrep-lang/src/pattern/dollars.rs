//! Dollar-token classification: `$`-run taxonomy and literal-lane routing.

use super::*;
use crate::Language;

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
