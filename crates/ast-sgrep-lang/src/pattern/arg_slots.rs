//! Argument-slot grammar + matchers (S8 family, cut from templates.rs).

use super::*;
use crate::extract::node_text;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Which lane's token grammar the unified slot parser enforces — the one
/// classification path for the S3 arg-slot family (`calls.rs` + `php.rs` +
/// `native.rs` + `classify.rs` all route here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgSlotGrammar {
    /// Plain (dotted/bare) calls: pure-metavariable slots only — literal
    /// tokens refuse (fail-closed); rests obey the reference's rest-slot
    /// contract (exactly one rest with singles, or ≥2 rests with no
    /// singles, all names distinct).
    PlainCall,
    /// Php member calls / assignment-RHS faces: metavariable slots AND
    /// single literal TOKENs, at most one rest. The "at least one literal"
    /// floor is GONE: meta-only lists classify through the slots now (the
    /// `;`-terminated flat spelling had no answering arm — see
    /// [`classify_member_call_mixed_args`]) and same-name meta lists
    /// inherit bind_capture's conflict semantics. The floor was vacuous at
    /// the other two call sites (pure-meta lists never reach them).
    Member,
}

/// Parse an argument list into per-position slots under one lane's grammar.
/// This is the single parser behind the former triple
/// (`parse_call_arg_slots` / `parse_member_arg_slots` /
/// `parse_rhs_arg_slots`): the comma-split loop is shared, only the token
/// admission (`allow_literal`) and the rest-count/collision tail differ.
pub fn parse_arg_slots(args: &str, grammar: ArgSlotGrammar) -> Option<Vec<ArgSlot>> {
    let mut slots = Vec::new();
    let mut rests = 0usize;
    let mut singles = 0usize;
    let allow_literal = matches!(grammar, ArgSlotGrammar::Member);
    for part in args.split(',') {
        let slot = classify_arg_part(part.trim(), allow_literal)?;
        rests += matches!(slot, ArgSlot::Rest(_)) as usize;
        singles += matches!(slot, ArgSlot::Meta(_)) as usize;
        slots.push(slot);
    }
    match grammar {
        // The assignment-RHS slot grammar — at most one rest (multiple
        // rests have no unambiguous contract).
        ArgSlotGrammar::Member => (rests <= 1).then_some(slots),
        // The PLAIN-call rest-slot grammar — pure metavariable slots where
        // at least one is a `$$$NAME` rest. Sole rests and pure-single
        // lists never reach here (earlier templates classify them);
        // literal tokens and 2+-rests-with-singles refuse (fail-closed). A
        // rest sharing its NAME with another slot in the SAME list also
        // refuses: the k >= 2 arm binds every HEAD rest to `""` and only
        // the tail rest to the whole-args text, so a shared name would
        // conflict-bind and silently empty the face. Distinct names only.
        ArgSlotGrammar::PlainCall => {
            if !((rests == 1 && singles >= 1) || (rests >= 2 && singles == 0)) {
                return None;
            }
            // Same-name collision refusal, order-independent: a rest
            // sharing its name with a single (either position), or any two
            // rests sharing a name, refuses (classify None -> ingress
            // loud).
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
    }
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

/// Which matching semantics the unified slot matcher enforces — the one
/// match path for the S3 slot family (`structural.rs` + `calls.rs` +
/// `php.rs` all route here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgSlotMode {
    /// Plain-call POSITIONAL semantics — deliberately NOT the php member
    /// lane's backtracking matcher: no backtracking of the split
    /// (`q($$$A, $B)` answers ONLY the 1-arg row, never the 2-arg row).
    ///   - a TRAILING rest sharing the list with singles binds >= 1 argument
    ///     (`q($A, $$$B)` refuses the 1-arg row);
    ///   - a NON-TRAILING rest binds EXACTLY ZERO and the singles anchor the
    ///     candidate arity to the single count (`q($$$A, $B)` only the 1-arg
    ///     row; `q($A, $$$B, $C)` only the 2-arg row);
    ///   - k >= 2 rests with no singles answer every arity >= k-1 (head rests
    ///     bind zero, the tail binds all — the split choice is unobservable).
    Positional,
    /// Member/RHS BACKTRACKING semantics: literal slots compare the
    /// candidate argument text byte-exactly; meta slots bind positionally;
    /// a rest slot binds the remaining arguments' ORIGINAL source bytes
    /// (comma-joined in the source, zero-or-more) in the multi namespace
    /// and backtracks over split points. A trailing rest SHARING the list
    /// with an earlier slot must bind at least one argument (`$w->q9(1,
    /// $$$A)` refuses `q9(1)`, `f($v, $$$A)` refuses `f($v)`); a
    /// whole-list rest may bind zero (`q9($$$A)` answers `q9()`) and a
    /// non-trailing rest may bind zero (registered mid-rest probes).
    Backtracking,
}

/// Match per-position slots under one lane's semantics: the single matcher
/// behind the former pair (`call_arg_slots_match` /
/// `arg_slots_match`+`arg_slots_match_from`). The plain lane keeps its
/// positional contract; the member/RHS lane keeps its backtracking
/// contract — one entry, no per-lane copies.
pub(crate) fn match_arg_slots(
    slots: &[ArgSlot],
    nodes: &[Node],
    source: &str,
    captures: &mut BTreeMap<String, String>,
    mode: ArgSlotMode,
) -> Option<()> {
    match mode {
        ArgSlotMode::Positional => match_positional_slots(slots, nodes, source, captures),
        ArgSlotMode::Backtracking => {
            match_backtracking_slots_from(slots, nodes, source, captures, true)
        }
    }
}

fn match_positional_slots(
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
    // distinct by the plain lane's collision refusal (see [`ArgSlotGrammar`]).
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

/// `first` marks the un-recursed entry call: a sole-slot rest (first and
/// last at once) is the whole-list rest that may bind an empty argument
/// list.
fn match_backtracking_slots_from(
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
            match_backtracking_slots_from(rest, &nodes[1..], source, captures, false)
        }
        Some((ArgSlot::Meta(name), rest)) => {
            let text = node_text(nodes.first()?, source)?;
            bind_capture(captures, name, text)?;
            match_backtracking_slots_from(rest, &nodes[1..], source, captures, false)
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
                    && match_backtracking_slots_from(rest, remain, source, &mut trial, false).is_some()
                {
                    *captures = trial;
                    return Some(());
                }
            }
            None
        }
    }
}
