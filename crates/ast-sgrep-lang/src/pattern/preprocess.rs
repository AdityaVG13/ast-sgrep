//! Pattern-acceptance gate and `$` expando preprocessing.

use super::*;
use crate::Language;
use std::borrow::Cow;

/// The pattern-acceptance gate: pre-process the pattern (the conditional
/// sigil rewrite), parse it under the language's grammar, and require the
/// root to be a SINGLE node (one child, or two where the second is a
/// missing/empty-kind token). Multi-root patterns reject; ERROR nodes never
/// reject and there is no descent at acceptance time. One grammar-drift
/// sharpening: php without an opening tag folds the whole document into a
/// single `text` node, so two or more quote-external semicolons refuse.
pub(crate) fn sg_pattern_gate_accepts(lang: Language, pattern: &str) -> bool {
    if lang == Language::Php && quote_external_semicolon_count(pattern) >= 2 {
        return false;
    }
    let doc = sg_preprocess_pattern(lang, pattern);
    let Ok(tree) = parse_source(lang, &doc) else {
        return false;
    };
    let root = tree.root_node();
    // The reference's pinned grammars (older minors than this workspace's
    // python/javascript/java grammars) recover parse errors as a SINGLE
    // ERROR root where these grammars split the document into an ERROR
    // fragment plus the recovered remainder. The reference ACCEPTS those
    // faces in the probed languages, so an ERROR-led two-fragment
    // PAREN-FREE root is accepted THERE — every probed acceptance is a
    // paren-free statement face, while the paren-bearing compounds refuse
    // or keep parse verdicts elsewhere. Three or more fragments is a real
    // multi-root and stays refused.
    is_sg_single_node(root)
        || (matches!(
            lang,
            Language::Python | Language::JavaScript | Language::Java
        ) && !pattern.contains('(')
            && root.child_count() == 2
            && root.child(0).is_some_and(|c| c.kind() == "ERROR"))
}

/// The BARE reference acceptance (preprocess + parse + `is_sg_single_node`)
/// WITHOUT the documented workspace-grammar-drift arm — the discriminator
/// the bracket-fragment class rides. A face the reference's own parse splits
/// into multiple roots must NOT enter the accepted-empty fragment admission;
/// those are exactly the parses where the bare single-node check refuses
/// while the 2-fragment drift arm would accept.
pub(crate) fn sg_pattern_gate_bare_single(lang: Language, pattern: &str) -> bool {
    if lang == Language::Php && quote_external_semicolon_count(pattern) >= 2 {
        return false;
    }
    let doc = sg_preprocess_pattern(lang, pattern);
    let Ok(tree) = parse_source(lang, &doc) else {
        return false;
    };
    is_sg_single_node(tree.root_node())
}

/// The SHAPE half of the bracket-fragment class — paren-free, ends in an
/// unbalanced `]`/`)`/`}` closer. Shared by the fragment admission below and
/// by the `$`-less census arm of [`native_pattern_answerable`], so both
/// spellings consult the identical per-language gate.
pub(crate) fn sg_bracket_fragment_shape(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.contains('(') {
        return false;
    }
    let Some(tail) = p.chars().last() else {
        return false;
    };
    if !matches!(tail, ']' | ')' | '}') {
        return false;
    }
    let (open, close) = match tail {
        ']' => ('[', ']'),
        ')' => ('(', ')'),
        _ => ('{', '}'),
    };
    let opens = p.chars().filter(|c| *c == open).count();
    let closes = p.chars().filter(|c| *c == close).count();
    closes > opens
}

/// True for the paren-free ERROR-repair BRACKET-fragment faces the reference
/// accepts and answers EMPTY (`$A ]` / `$A )` / `$A }` — the stray closer
/// parses into a single ERROR root that matches no clean source node). The
/// subject keeps these walk-admissible (the walk's empty IS the agreement)
/// instead of ingress-refused, while refused spellings stay census-loud:
/// the class requires the BARE single-node gate, so split-parse languages
/// (2 roots) and js/ts `$A }` keep the registered loud fold. `;`/operator
/// tails are NOT brackets and keep their registered classes. `pattern` must
/// already be general-lane-unsupported at the call sites (the walk answering
/// empty is what makes the face an honest empty).
pub(crate) fn sg_bracket_fragment_accepted_empty(lang: Language, pattern: &str) -> bool {
    if !sg_bracket_fragment_shape(pattern) {
        return false;
    }
    sg_pattern_gate_bare_single(lang, pattern.trim())
}

/// The number of `;` bytes OUTSIDE string-literal quotes — the
/// statement-root counter behind the php sharpening. Quote-aware like the
/// rest of the gates; a `;` inside a string literal is text, not a root.
pub(crate) fn quote_external_semicolon_count(pattern: &str) -> usize {
    let bytes = pattern.as_bytes();
    let mut quote: Option<u8> = None;
    let mut semis = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' | b'"' => quote = Some(b),
            b';' => semis += 1,
            _ => {}
        }
        i += 1;
    }
    semis
}

/// CONDITIONAL sigil replacement — a `$` run is rewritten to the language's
/// expando char when the run is followed by `[A-Z_]` or when it is a
/// 3-dollar run; a lowercase-led run (`$a`) stays LITERAL `$` text (php
/// variable syntax / js-ts identifier syntax). Java, JavaScript and
/// TypeScript have no preprocessing (the raw `$` is used there), and no
/// language here gets the php `<?php ` pattern wrapping the other gates
/// apply (php pattern documents parse bare).
/// The language's expando char — the shared table behind both halves: the
/// preprocessed gate AND the meta-var matcher. C/Cpp use U+10000,
/// CSharp/Go/Kotlin/Php/Python/Ruby/Rust/Swift use µ, and
/// Java/JavaScript/TypeScript have no expando.
pub(crate) fn sg_expando_char(lang: Language) -> Option<char> {
    match lang {
        Language::C | Language::Cpp => Some('\u{10000}'),
        Language::CSharp
        | Language::Go
        | Language::Kotlin
        | Language::Php
        | Language::Python
        | Language::Ruby
        | Language::Rust
        | Language::Swift => Some('µ'),
        Language::Java | Language::JavaScript | Language::TypeScript => None,
        // Dart identifiers may contain `$` (and strings interpolate `$var`),
        // so it joins the µ-expando group; MoonBit has no `$` syntax at all,
        // so `$` stays the raw metavariable sigil (the Java/JS/TS group).
        Language::Dart => Some('µ'),
        Language::MoonBit => None,
    }
}

pub(crate) fn sg_preprocess_pattern(lang: Language, pattern: &str) -> String {
    let Some(expando) = sg_expando_char(lang) else {
        return pattern.to_string();
    };
    let mut ret = String::with_capacity(pattern.len());
    let mut dollar_count = 0usize;
    for c in pattern.chars() {
        if c == '$' {
            dollar_count += 1;
            continue;
        }
        let expando_run = c.is_ascii_uppercase() || c == '_' || dollar_count == 3;
        let sigil = if expando_run { expando } else { '$' };
        for _ in 0..dollar_count {
            ret.push(sigil);
        }
        dollar_count = 0;
        ret.push(c);
    }
    for _ in 0..dollar_count {
        ret.push(if dollar_count == 3 { expando } else { '$' });
    }
    ret
}

/// Byte spans of the META-shaped expando runs. Expando-spelled identifiers
/// act as METAVARIABLES under these rules over a run of length n with tail T:
/// - n==1/2 (`µA`, `µµA`): meta iff T = `[A-Z_][A-Z_0-9]*`;
/// - n==3 (`µµµA`): meta iff T empty or `[A-Z_0-9]+` (digit-led included);
/// - n>=4 or a non-matching tail: NOT a meta — matched literally.
///
/// Validation runs over the ENTIRE token text, so a token continuing past
/// the prefix with an identifier char (`µAble`) is a LITERAL. A µ-run
/// immediately followed by `$`s is ONE combined run (preprocessing maps
/// every `$` to the expando char), and rewriting meta-shaped runs back to
/// `$` restores the identical metavariable structure.
pub(crate) struct ExpandoSpan {
    /// Byte offset of the first expando char of the run.
    start: usize,
    /// Byte offset of the NAME tail inside the span (after the run — expando
    /// chars plus any absorbed `$`s).
    name_start: usize,
    /// End of the `[A-Z_0-9]` tail = end of the rewritten span.
    end: usize,
    /// Combined run count to emit as `$`s (expando chars + absorbed `$`s).
    run_count: usize,
}

pub(crate) fn expando_meta_spans(
    pattern: &str,
    expando: char,
    suffix_continuations: &[char],
) -> Vec<ExpandoSpan> {
    let mut spans = Vec::new();
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let Some(ch) = pattern[i..].chars().next() else {
            break;
        };
        if ch != expando {
            i += ch.len_utf8();
            continue;
        }
        let mut run = 0usize;
        while pattern[i + run..].starts_with(expando) {
            run += expando.len_utf8();
        }
        let mut run_count = run / expando.len_utf8();
        // Absorb an adjacent `$` run — the reference's preprocess turns it
        // into expando chars too, so `µµ$A` is the 3-run `µµµA`, not a µ-run
        // plus a separate `$A` token.
        let mut name_start = i + run;
        while pattern[name_start..].starts_with('$') {
            name_start += 1;
            run_count += 1;
        }
        let mut end = name_start;
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end] == b'_'
                || bytes[end].is_ascii_digit())
        {
            end += 1;
        }
        let tail = &bytes[name_start..end];
        let first_ok = tail
            .first()
            .is_some_and(|b| b.is_ascii_uppercase() || *b == b'_');
        let tail_all_valid = tail
            .iter()
            .all(|b| b.is_ascii_uppercase() || *b == b'_' || b.is_ascii_digit());
        // Whole-node validation: meta extraction runs on the full node text,
        // and every probed grammar keeps ascii-lowercase and non-ASCII
        // letters INSIDE one identifier node — so any such continuation
        // after the valid prefix means the node text cannot match the meta
        // grammar (`µAble` is a literal identifier). The literal verdict is
        // the lean direction: rewriting the truncated prefix was the silent
        // miss. Ruby folds the `?` method-name suffix INTO the identifier
        // node, so an immediately adjacent `?` is node-text continuation too
        // and the run must NOT fold into a meta. The `!` twin is NOT
        // admitted: in expression position `!` is the negation operator.
        let continues_ident = pattern[end..].chars().next().is_some_and(|c| {
            c.is_ascii_lowercase() || !c.is_ascii() || suffix_continuations.contains(&c)
        });
        let is_meta = !continues_ident
            && match run_count {
                1 | 2 => first_ok && tail_all_valid,
                3 => tail.is_empty() || tail_all_valid,
                _ => false,
            };
        if is_meta {
            spans.push(ExpandoSpan {
                start: i,
                name_start,
                end,
                run_count,
            });
            i = end;
        } else {
            // Literal token: skip past the WHOLE node text (run + absorbed
            // `$`s + tail + ident continuation) so a later expando char
            // inside the same literal node is not rescanned as a new run
            // (`µµAµB` is one literal node in the reference, never `µµA` + meta `µB`).
            let mut skip = end;
            while let Some(c) = pattern[skip..].chars().next() {
                if c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii() {
                    skip += c.len_utf8();
                } else {
                    break;
                }
            }
            i = skip;
        }
    }
    spans
}

/// Byte spans of 1-run UPPERCASE-led MixedCase `$`-tokens (`$Bx` —
/// uppercase-led with a lowercase continuation). The reference preprocess
/// maps the run to the expando char and `extract_meta_var` then REJECTS the
/// whole node text (the lowercase continuation), so the token parses as
/// literal code and answers the verbatim rows under BOTH spellings.
/// Rewriting the run to the expando spelling hands the pattern to the plain
/// `$`-less literal lane on exactly the bytes the parse matches.
/// Underscore-led mixed tokens (`$_a`), multi-run mixed (`$$Bx`),
/// lowercase-led (`$x`) and canonical runs never enter this scanner — they
/// keep their registered classes.
pub(crate) fn dollar_mixed_case_literal_spans(pattern: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut dollars = 0usize;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        if dollars == 1 {
            let name_start = i + 1;
            let mut name_end = name_start;
            while name_end < bytes.len()
                && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
            {
                name_end += 1;
            }
            if name_end > name_start
                && bytes[name_start].is_ascii_uppercase()
                && dollar_name_class(pattern.get(name_start..name_end).unwrap_or(""))
                    == Some(DollarTokenClass::MixedCase)
            {
                spans.push(i..name_end);
                i = name_end;
                continue;
            }
        }
        i += dollars;
    }
    spans
}

/// Byte spans of 1-run CANONICAL `$`-tokens immediately followed by ruby's
/// `?` method-name suffix (`$A?`). The reference preprocess maps the run to
/// the expando char and the ruby grammar folds the suffix INTO the identifier
/// node, so `extract_meta_var` rejects the whole node text (`µA?` — the
/// trailing `?`) and the token parses as a LITERAL, answering the verbatim
/// rows under BOTH spellings. Rewriting the run to the expando spelling
/// (`$A?` → `µA?`) hands the pattern to the `$`-less literal lane on the
/// exact bytes the parse matches — the µ≡$ twin convergence. MixedCase
/// names keep the [`dollar_mixed_case_literal_spans`] arm (same
/// destination), lowercase-led names keep their registered classes, and
/// multi-run tokens never enter this scanner.
pub(crate) fn dollar_suffix_literal_spans(
    pattern: &str,
    suffixes: &[char],
) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    if suffixes.is_empty() {
        return spans;
    }
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        let mut dollars = 0usize;
        while i + dollars < bytes.len() && bytes[i + dollars] == b'$' {
            dollars += 1;
        }
        if dollars == 1 {
            let name_start = i + 1;
            let mut name_end = name_start;
            while name_end < bytes.len()
                && (bytes[name_end] == b'_' || bytes[name_end].is_ascii_alphanumeric())
            {
                name_end += 1;
            }
            let canonical = name_end > name_start
                && dollar_name_class(pattern.get(name_start..name_end).unwrap_or(""))
                    == Some(DollarTokenClass::Canonical);
            let suffixed = pattern[name_end..]
                .chars()
                .next()
                .is_some_and(|c| suffixes.contains(&c));
            if canonical && suffixed {
                spans.push(i..name_end);
                i = name_end;
                continue;
            }
        }
        i += dollars;
    }
    spans
}

/// Rewrite expando META runs to `$` runs so the native `$` machinery
/// (classification, templates, the gate's own preprocess) sees exactly the
/// pattern the reference `extract_meta_var` would extract. Non-meta expando
/// text stays verbatim, and languages without an expando are returned
/// untouched. The meta decision is whole-node, so `µAble`-class tokens stay
/// verbatim; a µ-run immediately followed by `$`s composes into one
/// combined `$`-run; 1-run MixedCase `$`-tokens rewrite the INVERSE way —
/// to the expando spelling — since they parse as literal expando-space
/// code. `preprocess(normalize(src)) == preprocess(src)` for every
/// spelling. Pathological hand-mixed runs (`$µA`) normalize where a blind
/// byte preprocess would keep them literal — the disclosed boundary of a
/// byte-level ingress normalization (extraction reads PARSED node text).
pub(crate) fn normalize_expando_meta_spelling(lang: Language, pattern: &str) -> Cow<'_, str> {
    let Some(expando) = sg_expando_char(lang) else {
        return Cow::Borrowed(pattern);
    };
    // Ruby folds the `?` method-name suffix into the identifier node, so
    // `µA?`/`$A?` are literal node texts — the suffix blocks the meta fold
    // on the µ side and inversely rewrites the `$` spelling onto the
    // expando bytes on the other.
    let suffix_continuations: &[char] = if lang == Language::Ruby { &['?'] } else { &[] };
    let meta_spans = expando_meta_spans(pattern, expando, suffix_continuations);
    let mixed_spans = dollar_mixed_case_literal_spans(pattern);
    let suffix_spans = dollar_suffix_literal_spans(pattern, suffix_continuations);
    if meta_spans.is_empty() && mixed_spans.is_empty() && suffix_spans.is_empty() {
        return Cow::Borrowed(pattern);
    }
    // Merge both span sets in byte order — disjoint by construction (µ-spans
    // start at the expando char, mixed `$`-spans at `$`; a µ-span's absorbed
    // `$`s are followed by a META name, which the mixed scanner rejects).
    let mut edits: Vec<(usize, usize, usize, Option<usize>)> = meta_spans
        .into_iter()
        .map(|s| (s.start, s.name_start, s.end, Some(s.run_count)))
        .collect();
    edits.extend(
        mixed_spans
            .into_iter()
            .map(|r| (r.start, r.start + 1, r.end, None)),
    );
    edits.extend(
        suffix_spans
            .into_iter()
            .map(|r| (r.start, r.start + 1, r.end, None)),
    );
    edits.sort_by_key(|(start, _, _, _)| *start);
    let mut out = String::with_capacity(pattern.len());
    let mut last = 0usize;
    for (start, name_start, end, to_dollar) in edits {
        out.push_str(&pattern[last..start]);
        match to_dollar {
            // The expando RUN (plus absorbed `$`s) maps 1:1 to `$`s; the NAME
            // tail is copied verbatim (`µµA` -> `$$A`, `µµ$A` -> `$$$A`).
            Some(run_count) => {
                for _ in 0..run_count {
                    out.push('$');
                }
            }
            // The inverse rewrite: one MixedCase `$` becomes the expando char
            // (`$Bx` -> `µBx`), the literal NAME tail is copied verbatim.
            None => out.push(expando),
        }
        out.push_str(&pattern[name_start..end]);
        last = end;
    }
    out.push_str(&pattern[last..]);
    Cow::Owned(out)
}

/// The reference's `PatternBuilder::build` REFUSES a pattern whose root is a
/// bare multi meta variable (`PatternError::RootMultiMetaVar`: `$$$`,
/// `$$$NAME`, `$$$_`), a check the reference runs AFTER the single-node
/// parse. The check runs on the NORMALIZED spelling, so `µµµ`-style roots
/// fold to the registered loud class instead of the fail-open silent empty.
pub(crate) fn sg_root_multi_meta_pattern(pattern: &str) -> bool {
    let Some(rest) = pattern.trim().strip_prefix("$$$") else {
        return false;
    };
    rest.is_empty()
        || rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit())
}
