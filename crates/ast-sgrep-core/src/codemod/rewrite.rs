//! Rewrite-span construction for planning: overlap resolution, template
//! interpolation, and edit application. Pure functions over match spans.

use super::CodemodEdit;
use anyhow::{bail, Context};
use ast_sgrep_lang::PatternMatch;

/// Reduce matches to a disjoint, OUTERMOST-wins set. Structural matches
/// come from real tree nodes, so spans are identical, properly nested, or
/// disjoint. Sorted by (start asc, end DESC) the outer span of any nested
/// pair is visited first, and a candidate that starts inside the last kept
/// span is contained in (or partially overlaps) it — dropped, because the
/// reference keeps the outer rewrite and skips the inner row. Kept matches
/// stay sorted by start, the order `apply_edits` requires.
pub(crate) fn keep_outermost_matches(mut matches: Vec<PatternMatch>) -> Vec<PatternMatch> {
    matches.sort_by(|left, right| {
        left.byte_start
            .cmp(&right.byte_start)
            .then_with(|| right.byte_end.cmp(&left.byte_end))
    });
    let mut kept: Vec<PatternMatch> = Vec::with_capacity(matches.len());
    for matched in matches {
        if kept
            .last()
            .is_some_and(|outer| matched.byte_start < outer.byte_end)
        {
            continue;
        }
        kept.push(matched);
    }
    kept
}

pub(crate) fn interpolate_rewrite(
    template: &str,
    matched: &PatternMatch,
) -> anyhow::Result<String> {
    let bytes = template.as_bytes();
    let mut output = String::with_capacity(template.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            let next = template[index..]
                .find('$')
                .map_or(bytes.len(), |offset| index + offset);
            output.push_str(&template[index..next]);
            index = next;
            continue;
        }
        if template[index..].starts_with("$$") && !template[index..].starts_with("$$$") {
            // `$$NAME` in a rewrite template is a capture reference whose
            // bound text itself begins with a literal `$` (the reference's
            // rewrite output substitutes the capture whenever a name follows
            // the `$$`). Keys are the stripped names (single namespace), so
            // `$$A` reads the same key the single-`$A` template reference
            // reads — emitting the name as literal text corrupted sources
            // on apply. When NO name follows, the reference keeps the `$$`
            // verbatim — there is no `$$`→`$` escape reduction (probed
            // spellings: end of template, space, punctuation).
            let mut name_end = index + 2;
            while name_end < bytes.len()
                && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
            {
                name_end += 1;
            }
            if name_end > index + 2 {
                let name = &template[index + 2..name_end];
                let value = matched
                    .captures
                    .get(name)
                    .with_context(|| format!("rewrite references unbound metavariable $${name}"))?;
                output.push_str(value);
                index = name_end;
                continue;
            }
            output.push_str("$$");
            index += 2;
            continue;
        }
        let prefix_len = if template[index..].starts_with("$$$") {
            3
        } else {
            1
        };
        let name_start = index + prefix_len;
        let mut name_end = name_start;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
        {
            name_end += 1;
        }
        if name_end == name_start {
            bail!("rewrite contains an invalid metavariable at byte {index}");
        }
        let name = &template[name_start..name_end];
        // `$$$NAME` template references read the multi namespace key;
        // there is deliberately no flat fallback so a template that asks for
        // an unbound multi capture fails loudly instead of silently reusing
        // a same-named single capture (the reference keeps the two
        // namespaces distinct in its own rewrite output).
        let key = if prefix_len == 3 {
            format!("$$${name}")
        } else {
            name.to_string()
        };
        let value = matched
            .captures
            .get(&key)
            .with_context(|| format!("rewrite references unbound metavariable ${name}"))?;
        output.push_str(value);
        index = name_end;
    }
    Ok(output)
}

pub(crate) fn apply_edits(original: &str, edits: &[CodemodEdit]) -> String {
    let replaced_bytes: usize = edits
        .iter()
        .map(|edit| edit.byte_end - edit.byte_start)
        .sum();
    let replacement_bytes: usize = edits.iter().map(|edit| edit.after.len()).sum();
    let mut rewritten = String::with_capacity(
        original
            .len()
            .saturating_sub(replaced_bytes)
            .saturating_add(replacement_bytes),
    );
    let mut cursor = 0;
    for edit in edits {
        rewritten.push_str(&original[cursor..edit.byte_start]);
        rewritten.push_str(&edit.after);
        cursor = edit.byte_end;
    }
    rewritten.push_str(&original[cursor..]);
    rewritten
}
