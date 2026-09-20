//! Call-chain and member-call pattern classifiers.

use super::*;

/// Statement-count template inside a nested `{ ... }` (or `:` suite) section.
///
/// Reference semantics: a single metavariable statement (`{ $STMT }`)
/// matches a body with **exactly one** statement; `$$$` matches any body;
/// `{}` matches an empty body. Comments are not counted as statements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyTemplate {
    /// `{ $$$ }` / `{ $$$BODY }` — any statements, but a body must exist.
    Any,
    /// `{}` → 0 statements, `{ $STMT }` → exactly 1 statement.
    Exactly(usize),
}

/// Argument-list shape for call templates (same contract as
/// [`BodyTemplate`], for argument lists).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentTemplate {
    Any,
    Exactly(usize),
}

/// One per-position slot in a php member-call argument list that mixes
/// literal tokens with canonical metas — the reference binds metas
/// positionally and matches literal tokens byte-exactly, and a `$$$NAME`
/// slot binds the remaining arguments' source text from any position
/// (leading/mid/trailing, zero-length included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgSlot {
    /// `$NAME` / `$$NAME` — binds its positional candidate argument.
    Meta(String),
    /// `$$$NAME` — binds the remaining arguments' source text (multi
    /// namespace), zero or more.
    Rest(String),
    /// A literal token — the candidate argument text must equal it
    /// byte-exactly.
    Literal(String),
}

/// One dotted segment of a [`NativeKind::CallChain`] pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallChainSegment {
    /// Literal segment text (`None` when the segment is a `$METAVAR`).
    pub literal: Option<String>,
    /// Capture name when the segment is a metavariable.
    pub capture: Option<String>,
    /// Argument template for call segments (`None` = plain receiver).
    pub args: Option<ArgumentTemplate>,
    /// `(name, multi)` when the segment's WHOLE argument list is one
    /// metavariable (`$A` / `$$$A`).
    pub args_capture: Option<(String, bool)>,
    /// Per-argument capture names when the argument list is a comma list of
    /// TWO OR MORE single canonical metas (`$A, $B`) — each name binds its
    /// positional candidate argument node text. Single-meta/rest lists keep
    /// the whole-list `args_capture` arm (`None` here) so their registered
    /// bindings stay byte-compatible.
    pub arg_metas: Option<Vec<String>>,
    /// Per-position slots when the member-call argument list MIXES literal
    /// tokens with metas (`1, $B`, `$$$A, 3`). Meta-only lists keep the
    /// registered `arg_metas`/`args_capture` arms (this stays `None`) so
    /// their bindings stay byte-compatible.
    pub arg_slots: Option<Vec<ArgSlot>>,
}

/// Classify a dotted member-call chain with per-segment argument lists.
/// `None` unless MORE THAN ONE segment carries an argument list — every
/// single-argument-list face keeps the simple [`NativeKind::Call`] lane.
///
/// A `?.`-SPELLED chain with PROPERTY links (mid-chain argument-free
/// segments) classifies into the gated [`NativeKind::OptionalCallChain`]
/// lane instead of the general structural lane (which consults none of the
/// junction gates). Admission stays conservative: at least one `?.`
/// connector, at least three segments, at least one call segment, the
/// TERMINAL segment must be a call, every property segment is an
/// identifier-shaped literal or pure metavariable, and all other segment
/// rules are unchanged. All-dotted chains keep the ≥2-call threshold.
pub(crate) fn classify_call_chain(p: &str) -> Option<NativeKind> {
    if !p.contains('.') || !p.contains('(') {
        return None;
    }
    // Split on depth-0 dots; parentheses and subscripts nest.
    let mut raw_segments: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in p.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            '.' if depth == 0 => raw_segments.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    raw_segments.push(current);
    if raw_segments.len() < 2 {
        return None;
    }
    // A trailing `?` on a segment spells the OPTIONAL connector INTO the NEXT
    // segment — the head link (`$O?.first().second()`) and the mid-chain
    // links (`$O.first()?.second()`). A trailing `?` on the LAST segment has
    // no next link and no classified contract (rust-try spellings), and any
    // other `?` (ternaries, conditional arguments) keeps the loud refusal.
    let last = raw_segments.len() - 1;
    let mut optional_flags = vec![false; raw_segments.len()];
    for (index, raw) in raw_segments.iter().enumerate() {
        if raw.trim_end().ends_with('?') {
            if index == last {
                return None;
            }
            optional_flags[index + 1] = true;
        }
    }
    let mut segments = Vec::with_capacity(raw_segments.len());
    let mut call_segments = 0usize;
    // The chain spells at least one `?.` connector — the admission that
    // unlocks property segments below.
    let optional_spelled = optional_flags.iter().any(|flag| *flag);
    for (index, raw) in raw_segments.iter().enumerate() {
        let raw = raw.trim();
        let raw = if index < last && raw.ends_with('?') {
            raw[..raw.len() - 1].trim_end()
        } else {
            raw
        };
        if raw.is_empty() {
            return None;
        }
        if raw.contains('?') {
            return None;
        }
        let (name_part, args_text) = match raw.find('(') {
            Some(open) => {
                // A call segment must end at its argument list.
                if !raw.ends_with(')') {
                    return None;
                }
                if !(is_pure_metavariable(name_head(raw, open))
                    || is_pattern_ident(name_head(raw, open)))
                {
                    return None;
                }
                call_segments += 1;
                (
                    name_head(raw, open),
                    Some(raw[open + 1..raw.len() - 1].trim()),
                )
            }
            None => {
                // Only the leading receiver segment may be argument-free —
                // EXCEPT in a `?.`-spelled chain, where a mid-chain PROPERTY
                // link classifies. The segment must be identifier-shaped —
                // the walk's text-unification contract. Template-literal and
                // numeric link texts are DELIBERATELY NOT admitted (this
                // workspace's TSX parse shapes those sources differently
                // than the reference grammar), so every template/numeric
                // face keeps the fail-closed loud envelope.
                if index != 0
                    && (!optional_spelled || !(is_pure_metavariable(raw) || is_pattern_ident(raw)))
                {
                    return None;
                }
                (raw, None)
            }
        };
        let (literal, capture) = if is_pure_metavariable(name_part) {
            (None, capture_name(name_part).map(str::to_string))
        } else {
            (Some(name_part.to_string()), None)
        };
        let mut arg_slots: Option<Vec<ArgSlot>> = None;
        let args = match args_text {
            None => None,
            Some("") => Some(ArgumentTemplate::Exactly(0)),
            // `$$$` / `$$$NAME` is the reference's ANY-arity rest argument
            // (the chains bind EMPTY argument lists, multi A=[]).
            Some("$$$") => Some(ArgumentTemplate::Any),
            Some(args) if args.starts_with("$$$") && is_metavar_name(&args[3..]) => {
                Some(ArgumentTemplate::Any)
            }
            // Plain single/metavar lists keep the exact-arity contract; a
            // rest metavariable mixed with singles has no reference
            // evidence and never templates (the single-call classifier
            // refuses the same mix through validate_argument_pattern).
            Some(args) if !args.contains("$$$") && args.split(',').all(is_pure_metavariable) => {
                Some(ArgumentTemplate::Exactly(args.split(',').count()))
            }
            // A MIXED rest list (one or two rests with singles, or two rests)
            // classifies ONLY for `?.`-spelled chains, riding the registered
            // plain-call rest-slot semantics (`parse_call_arg_slots` +
            // `call_arg_slots_match`): the reference answers the chain faces
            // at exactly those arities (trailing rest + single ⇒ n ≥ 2,
            // k ≥ 2 rests ⇒ n ≥ k-1, non-trailing rest ⇒ n == 1). Dotted
            // chains keep the refusal (the reference refuses `a.b.c($A,
            // $$$B)` at every arity), and `parse_call_arg_slots`' same-name
            // collision refusal keeps the census-loud envelope
            // (`a?.b?.c($$$A, $A)`).
            Some(args) if args.contains("$$$") => {
                if !optional_spelled {
                    return None;
                }
                {
                    let slots = parse_call_arg_slots(args)?;
                    arg_slots = Some(slots)
                }
                None
            }
            Some(_) => return None,
        };
        let args_capture = match args_text {
            Some("$$$") => None,
            Some(args) => {
                if let Some(name) = args.strip_prefix("$$$") {
                    is_metavar_name(name).then(|| (name.to_string(), true))
                } else {
                    capture_name(args).map(|name| (name.to_string(), false))
                }
            }
            None => None,
        };
        // A comma list of 2+ single metas binds each name to its positional
        // candidate argument.
        let arg_metas = match args_text {
            Some(args)
                if !args.contains("$$$")
                    && args.split(',').all(is_pure_metavariable)
                    && args.contains(',') =>
            {
                let names: Vec<String> = args
                    .split(',')
                    .filter_map(capture_name)
                    .map(str::to_string)
                    .collect();
                (!names.is_empty()).then_some(names)
            }
            _ => None,
        };
        segments.push(CallChainSegment {
            literal,
            capture,
            args,
            args_capture,
            arg_metas,
            // The dotted chain lane keeps its registered meta-only argument
            // contract (every mixed dotted face already answers through the
            // general lane). The `?.`-spelled chain lane carries mixed-rest
            // slot lists here — the walker enforces them with the registered
            // plain-call rest-slot semantics.
            arg_slots,
        });
    }
    // The 3+-segment optional chain is its own kind — the two-segment
    // spelling keeps the registered [`NativeKind::OptionalCall`] lane
    // (call_segments == 1 falls through to it via the single-call arm).
    // The per-connector flags ride along: `optional_flags[j]` is true when
    // the connector INTO pattern segment j is the spelled `?.`. A `?.`-
    // spelled chain with property links admits on ONE call segment when the
    // chain is ≥3 segments and the TERMINAL segment is a call (property
    // tails have no call-node walk). The historical ≥2-call admission keeps
    // any segment count (the 2-seg all-call face `fetch()?.$M($$$A)` stays
    // here, NOT `OptionalCall`), the `CallChain` arm stays byte-identical
    // for all-dotted chains, and a 2-segment single-call `?.` spelling
    // keeps `OptionalCall`.
    if optional_spelled {
        let terminal_is_call = segments
            .last()
            .is_some_and(|segment| segment.args.is_some() || segment.arg_slots.is_some());
        if !terminal_is_call || call_segments < 1 || (call_segments < 2 && segments.len() < 3) {
            return None;
        }
        return Some(NativeKind::OptionalCallChain {
            segments,
            optional_flags,
        });
    }
    if call_segments < 2 {
        return None;
    }
    Some(NativeKind::CallChain { segments })
}

/// Classify the FLAT php member-property face — `receiver->PROP` /
/// `receiver?->PROP` with a LITERAL receiver segment, a pure-metavariable
/// name tail, and no call parens. The tail meta binds the candidate
/// property name text. A pure-meta receiver head (`$A->$M`) keeps its
/// refusal, as does ternary `?`.
pub(crate) fn classify_php_member_property(p: &str) -> Option<NativeKind> {
    let (head, tail) = p.split_once("->")?;
    let (head, nullsafe) = match head.strip_suffix('?') {
        Some(head) => (head, true),
        None => (head, false),
    };
    if head.contains('?') || tail.contains('?') {
        return None;
    }
    let literal = call_path_segment(head.trim())??;
    let tail = tail.trim();
    if !is_pure_metavariable(tail) {
        return None;
    }
    Some(NativeKind::MemberCallChain {
        segments: vec![
            CallChainSegment {
                literal: Some(literal),
                capture: None,
                args: None,
                args_capture: None,
                arg_metas: None,
                arg_slots: None,
            },
            CallChainSegment {
                literal: None,
                capture: capture_name(tail).map(str::to_string),
                args: None,
                args_capture: None,
                arg_metas: None,
                arg_slots: None,
            },
        ],
        nullsafe_flags: vec![false, nullsafe],
    })
}

/// Classify a php `->` member-call chain with per-segment argument lists.
/// `None` unless more than one segment carries an argument list —
/// single-call-segment faces keep the simple [`NativeKind::MemberCall`]
/// lane. Mid-chain PROPERTY segments (`->prop` / `?->prop`, no call
/// parens) classify, as does the nullsafe `?->` connector anywhere
/// (recorded per link so the walk stays connector token-exact). A property
/// segment may also END the pattern, and a lowercase-led `$name` property
/// link admits as byte-exact literal dynamic-property text. Any other `?`
/// (ternary text, `??->`) refuses.
pub(crate) fn classify_member_call_chain(p: &str) -> Option<NativeKind> {
    // The `;`-terminated chain spelling is the SAME face; strip it before
    // splitting so it cannot ride into the last segment's `)`-close check.
    let p = p.trim();
    let p = p.strip_suffix(';').unwrap_or(p).trim();
    if !p.contains("->") {
        return None;
    }
    // The FLAT member-property face — a literal receiver, one `->`/`?->`
    // link, a pure-metavariable name tail, and no call parens anywhere —
    // classifies as a two-segment property chain. All other no-paren
    // shapes keep the refusal.
    if !p.contains('(') {
        return classify_php_member_property(p);
    }
    // Split on depth-0 `->`; parentheses and subscripts nest. A `?`
    // immediately before a split is the nullsafe `?->` connector INTO the
    // next segment — stripped from the left segment and recorded.
    let bytes = p.as_bytes();
    let mut raw_segments: Vec<String> = Vec::new();
    let mut nullsafe_flags: Vec<bool> = vec![false];
    let mut depth = 0usize;
    let mut segment_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'-' if depth == 0 && bytes.get(i + 1) == Some(&b'>') => {
                let mut segment = p[segment_start..i].to_string();
                let nullsafe = segment.ends_with('?');
                if nullsafe {
                    segment.pop();
                }
                raw_segments.push(segment);
                nullsafe_flags.push(nullsafe);
                i += 2;
                segment_start = i;
            }
            _ => i += 1,
        }
    }
    raw_segments.push(p[segment_start..].to_string());
    if raw_segments.len() < 2 {
        return None;
    }
    let mut segments = Vec::with_capacity(raw_segments.len());
    let mut call_segments = 0usize;
    for (index, raw) in raw_segments.iter().enumerate() {
        let raw = raw.trim();
        // Any `?` that survived the connector strip (ternaries, `??->`,
        // a trailing marker with no `->` after it) has no probed chain
        // contract here — fail-closed.
        if raw.contains('?') {
            return None;
        }
        if raw.is_empty() {
            return None;
        }
        let (name_part, args_text) = match raw.find('(') {
            Some(open) => {
                // A call segment must end at its argument list.
                if !raw.ends_with(')') {
                    return None;
                }
                if !(is_pure_metavariable(name_head(raw, open))
                    || is_pattern_ident(name_head(raw, open)))
                {
                    return None;
                }
                call_segments += 1;
                (
                    name_head(raw, open),
                    Some(raw[open + 1..raw.len() - 1].trim()),
                )
            }
            None if index == 0 => {
                // Only the leading receiver segment may be argument-free.
                // Receiver class discipline: a canonical metavar (`$A`/`$$A`),
                // a plain identifier, or a lowercase-led literal variable
                // (`$obj` — the reference's literal-source reading, matched
                // byte-exactly) classify; a MixedCase or garbage-led `$`-token
                // refuses (php poisons the whole pattern — the NeverMatches
                // gate keeps those faces).
                let receiver_ok = is_pure_metavariable(raw)
                    || is_pattern_ident(raw)
                    || dollar_name_class(raw.strip_prefix('$').unwrap_or(""))
                        == Some(DollarTokenClass::LowercaseLed);
                if !receiver_ok {
                    return None;
                }
                (raw, None)
            }
            None => {
                // A bare segment is a PROPERTY link: the property bytes match
                // exactly and a canonical metavariable binds the property
                // text, so plain identifiers and canonical metas admit. A
                // property segment may also END the pattern. A
                // lowercase-led `$name` / `$$name` is a DYNAMIC property link
                // matched as BYTE-EXACT literal text (only its own line
                // answers, while a canonical `$$DYN` wildcards every
                // property link). Mixed-case `$$Dyn` keeps the refusal.
                if is_pattern_ident(raw)
                    || is_pure_metavariable(raw)
                    || dollar_name_class(raw.strip_prefix('$').unwrap_or(""))
                        == Some(DollarTokenClass::LowercaseLed)
                    || is_dollar2_lowercase_token(raw)
                {
                    (raw, None)
                } else {
                    return None;
                }
            }
        };
        let (literal, capture) = if is_pure_metavariable(name_part) {
            (None, capture_name(name_part).map(str::to_string))
        } else {
            (Some(name_part.to_string()), None)
        };
        // Same argument table as [`classify_call_chain`] — `is_pure_metavariable`
        // already carries the canonical `$$A` arm, so 2-dollar chain arguments
        // classify exactly like `$A`. Lists mixing literal tokens with metas
        // classify through per-position slots (literal/metal/rest binding);
        // meta-only lists keep the registered arms byte-compatible.
        let (args, arg_slots) = match args_text {
            None => (None, None),
            Some("") => (Some(ArgumentTemplate::Exactly(0)), None),
            Some("$$$") => (Some(ArgumentTemplate::Any), None),
            Some(args) if args.starts_with("$$$") && is_metavar_name(&args[3..]) => {
                (Some(ArgumentTemplate::Any), None)
            }
            Some(args) if !args.contains("$$$") && args.split(',').all(is_pure_metavariable) => (
                Some(ArgumentTemplate::Exactly(args.split(',').count())),
                None,
            ),
            Some(args) => {
                let slots = parse_member_arg_slots(args)?;
                let has_rest = slots.iter().any(|slot| matches!(slot, ArgSlot::Rest(_)));
                let template = if has_rest {
                    ArgumentTemplate::Any
                } else {
                    ArgumentTemplate::Exactly(slots.len())
                };
                (Some(template), Some(slots))
            }
        };
        let args_capture = match args_text {
            Some("$$$") => None,
            Some(args) => {
                if let Some(name) = args.strip_prefix("$$$") {
                    is_metavar_name(name).then(|| (name.to_string(), true))
                } else {
                    capture_name(args).map(|name| (name.to_string(), false))
                }
            }
            None => None,
        };
        // Same per-argument table as [`classify_call_chain`] —
        // `$w->q2($A, $B)` binds A and B positionally.
        let arg_metas = match args_text {
            Some(args)
                if !args.contains("$$$")
                    && args.split(',').all(is_pure_metavariable)
                    && args.contains(',') =>
            {
                let names: Vec<String> = args
                    .split(',')
                    .filter_map(capture_name)
                    .map(str::to_string)
                    .collect();
                (!names.is_empty()).then_some(names)
            }
            _ => None,
        };
        segments.push(CallChainSegment {
            literal,
            capture,
            args,
            args_capture,
            arg_metas,
            arg_slots,
        });
    }
    // Single-call-segment faces keep the [`NativeKind::MemberCall`] /
    // [`NativeKind::OptionalCall`] lanes — unless a property or nullsafe
    // link makes the chain deeper than those two-segment shapes (the
    // reference answers `$x->prop->m($A)` and `$x?->p1->q1($A)`, which no
    // other lane serves). A call followed by a property TAIL also
    // classifies (`$o->c1($A)->tailProp`), while the plain single-call
    // shape keeps its flat lane. Zero-call chains stay out (plain
    // member-access faces keep their registered lanes).
    let last_is_property = segments.len() >= 2 && segments.last().is_some_and(|s| s.args.is_none());
    if call_segments < 2
        && (call_segments != 1 || segments.len() < 3 && !(segments.len() == 2 && last_is_property))
    {
        return None;
    }
    Some(NativeKind::MemberCallChain {
        segments,
        nullsafe_flags,
    })
}
