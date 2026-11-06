//! PHP static-scope assignment/binary targets (isomorphic split of php.rs).

use super::*;

/// A php assignment target whose `->` member LINK carries a canonical
/// metavariable — a lowercase-led `$var` root with links of a plain
/// identifier OR a canonical `$META` (`$o->$A`), plus the literal-subscript
/// tail rule of [`is_php_literal_target`]. At least one meta link must be
/// present (pure-literal targets stay on the text-exact path). The
/// structural comparison runs through [`php_rhs_expr_matches`], whose
/// `variable_name` arm binds the candidate link node text — `a` for a
/// literal `name`, `$a` for a dynamic `variable_name` — exactly the
/// reference's metaVariables.
pub(crate) fn php_meta_link_target(lhs: &str) -> bool {
    let mut rest = lhs.trim();
    let Some(root) = rest.strip_prefix('$') else {
        return false;
    };
    let name_len = root
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(root.len());
    if dollar_name_class(&root[..name_len]) != Some(DollarTokenClass::LowercaseLed) {
        return false;
    }
    rest = &root[name_len..];
    let mut saw_meta_link = false;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("->") {
            let after = after.trim_start();
            rest = match after.strip_prefix('$') {
                Some(name_rest) => {
                    let len = name_rest
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(name_rest.len());
                    if !is_metavar_name(&name_rest[..len]) {
                        return false;
                    }
                    saw_meta_link = true;
                    &name_rest[len..]
                }
                None => {
                    let len = after
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(after.len());
                    if !is_pattern_ident(&after[..len]) {
                        return false;
                    }
                    &after[len..]
                }
            };
            continue;
        }
        if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return false;
            };
            let index = &after[..close];
            if index.trim().is_empty() || index.contains('$') {
                return false;
            }
            rest = &after[close + 1..];
            continue;
        }
        break;
    }
    saw_meta_link && rest.trim().is_empty()
}

/// A fully-literal STATIC scoped assignment/binary target — a class-name-ish
/// head (`C`, `self`, `parent`, `static`) optionally NAMESPACED (`Foo\Bar`
/// and the leading-`\` `\Foo\Bar` FQN spellings; the reference binds every
/// one) followed by `::$prop` with a lowercase-led `$var` property, plus
/// optional `[index]` subscript tails admitted per
/// [`php_static_index_admissible`]. Doubled-dollar props and canonical-meta
/// props stay out. Disjoint from [`is_php_literal_target`] (`$`-rooted)
/// and [`php_meta_link_target`] (meta links) by construction.
pub(crate) fn is_php_static_scope_target(lhs: &str) -> bool {
    let mut rest = lhs.trim();
    if let Some(after) = rest.strip_prefix('\\') {
        rest = after.trim_start();
    }
    // Head segments: plain identifier-ish tokens joined by `\` (alphabetic/
    // underscore-led — covers class names and the self/parent/static
    // keywords).
    loop {
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if name_len == 0 || !rest.is_char_boundary(name_len) {
            return false;
        }
        let head = &rest[..name_len];
        let head_ok = head.starts_with('_')
            || head
                .chars()
                .next()
                .is_some_and(|c: char| c.is_ascii_alphabetic());
        if !head_ok {
            return false;
        }
        rest = rest[name_len..].trim_start();
        match rest.strip_prefix('\\') {
            Some(after) => rest = after.trim_start(),
            None => break,
        }
    }
    let Some(after) = rest.strip_prefix("::") else {
        return false;
    };
    rest = after.trim_start();
    let Some(prop) = rest.strip_prefix('$') else {
        return false;
    };
    let prop_len = prop
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(prop.len());
    if dollar_name_class(&prop[..prop_len]) != Some(DollarTokenClass::LowercaseLed) {
        return false;
    }
    rest = &prop[prop_len..];
    // Optional subscript tails.
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return false;
            };
            if !php_static_index_admissible(after[..close].trim()) {
                return false;
            }
            rest = &after[close + 1..];
            continue;
        }
        return false;
    }
}

/// The subscript-index admission of a static scoped target. `$`-free
/// indexes are literal text (verbatim byte compare). ONE canonical
/// metavariable is a whole-index structural bind (`[$K]` binds K to the
/// candidate's whole index text, `f($V)` and `$i + 1` included). A bare
/// LOWERCASE php variable spelling is the LITERAL variable read:
/// text-equal candidates answer with no capture. Any other `$`-carrying
/// shape (a meta nested inside a call, member links, multi-`$`
/// expressions) is unprobed — fail-closed refusal.
pub(crate) fn php_static_index_admissible(index: &str) -> bool {
    if index.is_empty() || index.contains('[') || index.contains(']') {
        return false;
    }
    if !index.contains('$') {
        return true;
    }
    let literal_var = index.strip_prefix('$').is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .next()
                .is_some_and(|c: char| c.is_ascii_lowercase() || c == '_')
            && rest
                .chars()
                .all(|c: char| c.is_ascii_alphanumeric() || c == '_')
    });
    literal_var || is_single_metavariable(index)
}

/// True when a static scoped target carries a single-canonical-meta
/// subscript index — the walker must parse the LHS structurally (binding the
/// meta to the whole candidate index text, [`php_static_index_admissible`])
/// instead of the verbatim byte compare the literal tails keep.
pub(crate) fn php_static_target_has_meta_index(target: &str) -> bool {
    let mut rest = target.trim();
    if let Some(after) = rest.strip_prefix('\\') {
        rest = after.trim_start();
    }
    loop {
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if name_len == 0 {
            return false;
        }
        rest = rest[name_len..].trim_start();
        match rest.strip_prefix('\\') {
            Some(after) => rest = after.trim_start(),
            None => break,
        }
    }
    let Some(after) = rest.strip_prefix("::") else {
        return false;
    };
    rest = after.trim_start();
    let Some(prop) = rest.strip_prefix('$') else {
        return false;
    };
    let prop_len = prop
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(prop.len());
    rest = &prop[prop_len..];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return false;
        }
        if let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else {
                return false;
            };
            if is_single_metavariable(after[..close].trim()) {
                return true;
            }
            rest = &after[close + 1..];
            continue;
        }
        return false;
    }
}

/// The DYNAMIC-CLASS head of a static scoped assignment target — `$C::$s`
/// / `$$C::$s` where the head is ONE canonical metavariable (optionally
/// behind one literal `$`, php's dynamic-variable spelling) and the prop is
/// a literal lowercase-led `$var` with NO subscript tails (unprobed →
/// fail-closed). The reference binds the WHOLE candidate scope text, dollar
/// included (`$C::$s = $V` → C=`$name`; `$$C::$s = $V` → C=`$c`); a
/// literal-class candidate head is unprobed and stays refused. Returns the
/// metavariable NAME for the walker's scope binding.
pub(crate) fn php_static_target_dynamic_head(target: &str) -> Option<&str> {
    let rest = target.trim().strip_prefix('$')?;
    let (head, prop) = rest.split_once("::")?;
    let meta_name = match head.strip_prefix('$') {
        Some(inner) => inner, // the `$$C` spelling
        None => head,         // the `$C` spelling
    };
    if !is_metavar_name(meta_name) {
        return None;
    }
    let prop_name = prop.trim().strip_prefix('$')?;
    if dollar_name_class(prop_name) != Some(DollarTokenClass::LowercaseLed) {
        return None;
    }
    Some(meta_name)
}

/// True when the classified assignment target carries a canonical
/// metavariable the walker must bind structurally (whole-meta `$X` or a
/// [`php_meta_link_target`] link) instead of comparing the candidate LHS
/// text verbatim.
pub(crate) fn php_assignment_target_is_meta(target: &str) -> bool {
    is_pure_metavariable(target) || php_meta_link_target(target)
}

/// A literal assignment TARGET — a lowercase-led `$var` / `$$var` root
/// optionally followed by `->` literal links and `[literal]` subscripts
/// (`$alpha`, `$$zeta`, `$this->prop`, `$eps[0]`). No metavariables anywhere
/// (the reference refuses meta assignment-targets), no calls.
pub(crate) fn is_php_literal_target(lhs: &str) -> bool {
    let (mut rest, _) = match lhs.strip_prefix("$$") {
        Some(rest) => (rest, 2usize),
        None => match lhs.strip_prefix('$') {
            Some(rest) => (rest, 1usize),
            None => return false,
        },
    };
    // The root name must be a lowercase-led variable name.
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let (name, tail) = rest.split_at(name_len);
    if dollar_name_class(name) != Some(DollarTokenClass::LowercaseLed) {
        return false;
    }
    rest = tail;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if let Some(after) = rest.strip_prefix("->") {
            let after = after.trim_start();
            // A member link: a plain identifier or a lowercase/2-dollar var.
            let (link_rest, link_dollars) = match after.strip_prefix("$$") {
                Some(r) => (r, 2usize),
                None => match after.strip_prefix('$') {
                    Some(r) => (r, 1usize),
                    None => (after, 0usize),
                },
            };
            let name_len = link_rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(link_rest.len());
            let (name, tail) = link_rest.split_at(name_len);
            if link_dollars == 0 {
                if !is_pattern_ident(name) {
                    return false;
                }
            } else if dollar_name_class(name) != Some(DollarTokenClass::LowercaseLed) {
                return false;
            }
            rest = tail;
            continue;
        }
        if let Some(after) = rest.strip_prefix('[') {
            // A subscript: literal content only (no `$`), closed.
            let Some(close) = after.find(']') else {
                return false;
            };
            let index = &after[..close];
            if index.trim().is_empty() || index.contains('$') {
                return false;
            }
            rest = &after[close + 1..];
            continue;
        }
        return false;
    }
}
