//! Return-section scanners + function-tail parser (S2+S3, cut from templates.rs).

use super::*;

/// Admissible concrete return text: a path identifier (`u32`, `int`,
/// `number`) optionally carrying ONE balanced generic layer
/// (`Option<u32>`, `Vec<String>`). Probed shapes only; anything else keeps
/// the loud fail-closed class.
fn concrete_return_admissible(s: &str) -> bool {
    let b = s.as_bytes();
    let Some(mut i) = path_type(b) else {
        return false;
    };
    if i == b.len() {
        return true;
    }
    // One generic layer over the same interior class.
    if b[i] != b'<' {
        return false;
    }
    let mut depth = 1usize;
    i += 1;
    let start = i;
    while i < b.len() {
        match b[i] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1 == b.len() && interior_type_admissible(&b[start..i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Generic-layer interior: path types, separators, whitespace, nesting.
fn interior_type_admissible(b: &[u8]) -> bool {
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b',' | b'\t' => i += 1,
            b'<' | b'>' => i += 1,
            _ => {
                let n = path_type(&b[i..]).unwrap_or(0);
                if n == 0 {
                    return false;
                }
                i += n;
            }
        }
    }
    true
}

/// Length of the leading `_`/alpha-led identifier of `b`, 0 if none.
fn ident_len(b: &[u8]) -> usize {
    let mut i = 0;
    if i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphabetic()) {
        i += 1;
        while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) {
            i += 1;
        }
        i
    } else {
        0
    }
}

/// Length of the leading `path::Type` segment of `b`, if it starts with a
/// path identifier.
fn path_type(b: &[u8]) -> Option<usize> {
    let mut i = ident_len(b);
    if i == 0 {
        return None;
    }
    while b[i..].starts_with(b"::") {
        let seg = ident_len(&b[i + 2..]);
        if seg == 0 {
            return None;
        }
        i += 2 + seg;
    }
    Some(i)
}

/// The next whitespace/`{`/`;`-bounded token of a tail.
fn bare_token(tail: &str) -> (&str, &str) {
    // ':' terminates the return type for the py suite (`-> int: $$$B`);
    // no admissible type text contains a colon (go `map[k]v` results keep
    // the loud refusal — unprobed shape).
    let end = tail
        .find(|c: char| c.is_whitespace() || c == '{' || c == ';' || c == ':')
        .unwrap_or(tail.len());
    (&tail[..end], &tail[end..])
}

/// Strip an optional spelled return section from a post-argument-list tail,
/// per decl keyword family (br-hs1). `fn`/`def` spell `-> T`, `function`
/// spells `: T`, `func` spells a bare `T` (the go result position).
/// Returns the remaining tail and the parsed section; a present-but-
/// inadmissible section is a loud refusal (`None`).
fn strip_return_section<'a>(tail: &'a str, keyword: &str) -> Option<(&'a str, Option<ReturnSpec>)> {
    let arrow = keyword == "fn" || keyword == "def";
    let colon = keyword == "function";
    let bare = keyword == "func";
    if arrow {
        if let Some(rest) = tail.strip_prefix("->") {
            let (token, after) = bare_token(rest.trim_start());
            let spec = return_spec_for_token(token)?;
            return Some((after.trim_start(), Some(spec)));
        }
    } else if colon {
        if let Some(rest) = tail.strip_prefix(':') {
            let (token, after) = bare_token(rest.trim_start());
            // A `$$$MULTI` rest is the legacy suite-body reading (`: $$$B`),
            // not a return — leave it to the body parser below.
            if !token.starts_with("$$$") {
                let spec = return_spec_for_token(token)?;
                return Some((after.trim_start(), Some(spec)));
            }
        }
    } else if bare {
        // `{`-led tails are bodies; `;`-led tails are the semicolon form.
        if !tail.starts_with('{') && !tail.starts_with(';') && !tail.is_empty() {
            let (token, after) = bare_token(tail);
            let spec = return_spec_for_token(token)?;
            return Some((after.trim_start(), Some(spec)));
        }
    }
    Some((tail, None))
}

fn return_spec_for_token(token: &str) -> Option<ReturnSpec> {
    if is_single_metavariable(token) {
        Some(ReturnSpec::Metavar(
            token.strip_prefix('$').unwrap_or(token).to_string(),
        ))
    } else if concrete_return_admissible(token) {
        Some(ReturnSpec::Concrete(token.to_string()))
    } else {
        None
    }
}

/// Parse the full function tail: optional argument list, optional spelled
/// return section, optional `;` terminator, optional body. br-hs1 shapes;
/// every inadmissible byte keeps the loud fail-closed class.
pub(crate) fn parse_function_tail_sections(
    tail: &str,
    keyword: &str,
) -> Option<(Option<ReturnSpec>, bool, Option<BodyTemplate>)> {
    let mut after = tail.trim();
    if let Some(args) = after.strip_prefix('(') {
        let close = args.find(')')?;
        let inner = args[..close].trim();
        if !inner.is_empty() && inner != "$$$" && !inner.split(',').all(is_pure_metavariable) {
            return None;
        }
        after = args[close + 1..].trim();
    }
    let (mut after, return_spec) = strip_return_section(after, keyword)?;
    let semicolon = after.starts_with(';');
    if semicolon {
        after = after[1..].trim();
    }
    let body = parse_body_template(after)?;
    Some((return_spec, semicolon, body))
}

/// The return metavariable of a decl pattern, if the return section is a
/// single metavariable (concrete sections bind nothing). Used by the
/// capture binder.
pub(crate) fn function_return_metavar(pattern: &str) -> Option<String> {
    for prefix in ["fn ", "def ", "function ", "func "] {
        if let Some(rest) = pattern.trim().strip_prefix(prefix) {
            let (_, head) = rest.split_once('(')?;
            let tail = format!("({head}");
            let (spec, _, _) = parse_function_tail_sections(&tail, prefix.trim())?;
            if let Some(ReturnSpec::Metavar(name)) = spec {
                return Some(name);
            }
            return None;
        }
    }
    None
}
