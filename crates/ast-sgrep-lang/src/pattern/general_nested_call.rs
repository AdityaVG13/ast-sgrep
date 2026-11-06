//! General structural lane: nested-call rest family.

use super::*;

/// The nested-call rest-argument family: a call-shaped pattern where every
/// argument list holds only single metavariables or nested calls of the
/// same restricted shape, plus lists that are exactly one `$$$` rest.
/// Literal atoms are NOT in the family, and a `$$$` rest may never share
/// its list with a sibling. Paths spell `.` and `::` separators with
/// identifier/`$meta` segments.
pub(crate) fn nested_call_rest_template(pattern: &str) -> bool {
    let p = pattern.trim();
    if !p.contains('$') || !p.contains('(') {
        return false;
    }
    // A `new`-led call head is the same family shape — the reference answers
    // `new q($$$A)` like the plain call; strip the constructor keyword and
    // parse the family call identically. Non-js/ts `new` heads still refuse
    // later in the eligibility gate (the head admission is language-scoped),
    // so this strip is inert for them.
    let p = p
        .strip_prefix("new ")
        .filter(|rest| {
            rest.starts_with(char::is_alphabetic) || rest.starts_with('_') || rest.starts_with('$')
        })
        .unwrap_or(p);
    let b = p.as_bytes();
    let mut i = 0usize;
    if !parse_family_call(b, &mut i) {
        return false;
    }
    // Trailing whitespace only after the outermost call.
    b[i..].iter().all(|c| c.is_ascii_whitespace())
}

pub(crate) fn parse_family_call(b: &[u8], i: &mut usize) -> bool {
    // Dotted / scoped path: segment (`.` | `::` segment)*.
    let start = *i;
    loop {
        if *i >= b.len() {
            return false;
        }
        if b[*i] == b'_' || b[*i].is_ascii_alphanumeric() {
            while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                *i += 1;
            }
        } else if b[*i] == b'$' {
            *i += 1;
            while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                *i += 1;
            }
        } else {
            return false;
        }
        if *i < b.len() && b[*i] == b'.' {
            *i += 1;
            continue;
        }
        if *i + 1 < b.len() && b[*i] == b':' && b[*i + 1] == b':' {
            *i += 2;
            continue;
        }
        break;
    }
    if *i == start || *i >= b.len() || b[*i] != b'(' {
        return false;
    }
    *i += 1;
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
    // Empty list.
    if b.get(*i) == Some(&b')') {
        *i += 1;
        return true;
    }
    // Sole rest: `$$$` + optional name, then nothing but whitespace to `)`.
    if b.get(*i) == Some(&b'$') && b.get(*i + 1) == Some(&b'$') && b.get(*i + 2) == Some(&b'$') {
        *i += 3;
        while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
            *i += 1;
        }
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if b.get(*i) == Some(&b')') {
            *i += 1;
            return true;
        }
        // Rest with a sibling: out of the family (registered loud class).
        return false;
    }
    // Argument list: (arg `,`)* arg — each arg is a `$meta` or a nested call.
    loop {
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        match b.get(*i) {
            Some(b'$') => {
                if b.get(*i + 1) == Some(&b'$') && b.get(*i + 2) == Some(&b'$') {
                    return false; // rest with a sibling: out of the family
                }
                // A canonical 2-dollar name is a family argument exactly like
                // `$NAME` (sole-or-sibling, single capture); a malformed run
                // (`$$`, `$$3`, `$$x`) stays out.
                if b.get(*i + 1) == Some(&b'$') {
                    *i += 2;
                    let start = *i;
                    while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                        *i += 1;
                    }
                    let name = std::str::from_utf8(&b[start..*i]).unwrap_or("");
                    if !is_metavar_name(name) {
                        return false;
                    }
                } else {
                    *i += 1;
                    while *i < b.len() && (b[*i] == b'_' || b[*i].is_ascii_alphanumeric()) {
                        *i += 1;
                    }
                }
            }
            Some(c) if *c == b'(' || *c == b'_' || c.is_ascii_alphanumeric() => {
                if !parse_family_call(b, i) {
                    return false;
                }
            }
            _ => return false, // literal atom: out of the family
        }
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        match b.get(*i) {
            Some(b',') => *i += 1,
            Some(b')') => {
                *i += 1;
                return true;
            }
            _ => return false,
        }
    }
}
