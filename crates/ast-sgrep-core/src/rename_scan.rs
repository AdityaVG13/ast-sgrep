//! Rename-collision scan helpers, split verbatim from `pattern.rs`.

use std::borrow::Cow;

/// The reference keeps `$$$Rest` (multi-capture) and `$Rest` (single
/// capture) in DISTINCT capture slots even when they share a base name; the
/// native chain matcher binds both through one map key, so a
/// classifier-accepted pattern like `$O.out.$$$A($A)` cannot bind the
/// property-name rest — every file answers match-none and the walk composes
/// into a silent `ok:true` empty where the reference answers hits
/// (`System.out.println(total)`). The chain lane otherwise implements the
/// reference's rest-binds-property semantics exactly, so the fix renames
/// ONLY the colliding callee-path rest occurrences to a fresh deterministic
/// name. This is a deliberate behavior change, not a hit-set-preserving
/// no-op: pre-fix the colliding rest bound through the SAME map key as the
/// single capture, so the property text had to unify with the pattern's `$A`
/// argument and colliding patterns answered match-none; post-fix the fresh
/// name binds the rest the reference's way and those faces answer hits. The
/// preserved invariant is name-distinct ingress: non-colliding patterns are
/// untouched, and every changed face moved from wrong-empty (or wrong
/// unification) to the reference-agreed hit set. Scoped to whole dot-segment
/// rests in the callee path (name/property slots); argument-slot rests keep
/// their registered semantics, and `::`-path name slots are out of the
/// probed scope and untouched.
/// Determinism: the fresh-name loop is a pure function of the pattern string.
pub(crate) fn rename_colliding_callee_rest(pattern: &str) -> Cow<'_, str> {
    let Some(head_end) = pattern.find('(') else {
        return Cow::Borrowed(pattern);
    };
    let (head, tail) = pattern.split_at(head_end);
    let mut singles = std::collections::BTreeSet::new();
    let mut rests = std::collections::BTreeSet::new();
    scan_meta_names(pattern, &mut singles, &mut rests);
    let mut renames: Vec<(&str, String)> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for segment in head.split('.') {
        let Some(base) = segment.strip_prefix("$$$") else {
            continue;
        };
        if base.is_empty() || !base.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if seen.contains(&base) {
            continue;
        }
        seen.push(base);
        if !singles.contains(base) {
            continue;
        }
        let mut ordinal = 1usize;
        // The fresh name MUST stay inside the canonical metavariable
        // alphabet `[A-Z_][A-Z0-9_]*`. The original `_r` suffix carried a
        // lowercase tail byte, so the de-collided spelling itself classified
        // MixedCase and routed to NeverMatches — the collision faces went
        // silent-empty again. Uppercase suffix + ordinal digits keep the
        // rename canonical.
        let mut fresh = format!("{base}_R");
        while singles.contains(fresh.as_str()) || rests.contains(fresh.as_str()) {
            ordinal += 1;
            fresh = format!("{base}_R{ordinal}");
        }
        rests.insert(fresh.clone());
        renames.push((base, fresh));
    }
    if renames.is_empty() {
        return Cow::Borrowed(pattern);
    }
    let renamed_head = head
        .split('.')
        .map(|segment| match segment.strip_prefix("$$$") {
            Some(base) => match renames.iter().find(|(name, _)| *name == base) {
                Some((_, fresh)) => format!("$$${fresh}"),
                None => segment.to_string(),
            },
            None => segment.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".");
    let mut renamed = String::with_capacity(pattern.len() + 8);
    renamed.push_str(&renamed_head);
    renamed.push_str(tail);
    Cow::Owned(renamed)
}

/// Every metavariable NAME in `pattern`, split by capture class: `$Name`
/// (single) vs `$$$Name`+ (rest). `$$A` universal tokens are neither.
fn scan_meta_names(
    pattern: &str,
    singles: &mut std::collections::BTreeSet<String>,
    rests: &mut std::collections::BTreeSet<String>,
) {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            i += 1;
            continue;
        }
        let mut run = 0usize;
        while i + run < chars.len() && chars[i + run] == '$' {
            run += 1;
        }
        let name_start = i + run;
        let mut name_end = name_start;
        while name_end < chars.len()
            && (chars[name_end].is_ascii_alphanumeric() || chars[name_end] == '_')
        {
            name_end += 1;
        }
        if name_end > name_start {
            let name: String = chars[name_start..name_end].iter().collect();
            match run {
                1 => {
                    singles.insert(name);
                }
                2 => {}
                _ => {
                    rests.insert(name);
                }
            }
            i = name_end;
        } else {
            i = name_start.max(i + 1);
        }
    }
}
