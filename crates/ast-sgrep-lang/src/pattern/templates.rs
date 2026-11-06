//! If/optional-call/body parsers, dollar tokens, arg slots.

use super::*;
use crate::extract::node_text;
use crate::Language;
use std::collections::BTreeMap;
use std::sync::Arc;
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

pub(crate) fn parse_source(lang: Language, source: &str) -> anyhow::Result<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_language(lang))
        .map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
    parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse source"))
}

/// THE silent-parse convention (P4): source-parse failure → `None`.
///
/// Every lane that answers empty on unparseable source goes through this
/// one function instead of scattering `.ok()?` / `let Ok(..) else` at each
/// call site, so "no match" vs "unparseable" has a single documented home
/// and a single future instrumentation point. Loud lanes keep using
/// [`parse_source`] with `?`. Same outcomes — `None` maps to each caller's
/// existing empty value (`Vec::new()`, `None`, `false`, early return).
pub(crate) fn parse_source_silent(lang: Language, source: &str) -> Option<tree_sitter::Tree> {
    parse_source(lang, source).ok()
}

pub(crate) fn compiled_query(
    language: &tree_sitter::Language,
    _lang: Language,
    source: &'static str,
) -> Option<Arc<Query>> {
    Query::new(language, source).ok().map(Arc::new)
}
