//! PHP assignment, member, and operand lanes.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

/// The callee text before the argument list of a call segment.
pub(crate) fn name_head(raw: &str, open: usize) -> &str {
    raw[..open].trim()
}

/// Classify a php assignment/binary pattern whose LHS is a LOWERCASE-LITERAL
/// operand (`$alpha`, `$$zeta`, `$this->prop`, `$eps[0]` — byte-exact
/// literal source text) and whose operator is the plain `=`, an augmented
/// assignment, or a binary operator. The RHS is either ONE bare canonical
/// metavariable (`$V`/`$$V`/`$$$V`) or an expression pattern mixing
/// canonical metas with literal operands (matched structurally with meta
/// binding). Refusals: a canonical or MixedCase LHS, and a META
/// assignment-target anywhere in the pattern. PHP ONLY — statement
/// discipline: `;`-terminated patterns bind at expression-statement roots
/// only, while a `;`-less pattern also answers embedded nodes.
pub(crate) fn classify_php_assignment(p: &str) -> Option<NativeKind> {
    let p = p.trim();
    let (lhs, op, rhs) = split_php_binary(p)?;
    // The statement `;` rides the RHS side only, and is OPTIONAL (the
    // reference answers `$alpha = $V;` and `$alpha = $V` with the same
    // nodes).
    let rhs = rhs.trim();
    let rhs = rhs.strip_suffix(';').unwrap_or(rhs).trim();
    // The reference's tree-sitter-php parse ERRORS on every doubled-sign face
    // whose `--`/`++` token is TIGHT to an operand — prefix (`--$V`/`++$V`),
    // postfix (`$V--`/`$V++`), ANYWHERE in the RHS, parens included. A sign
    // SPACED on both sides is the lenient binary-minus + unary-minus spelling
    // both engines answer, so the scan is tightness-exact, not
    // substring-exact. This grammar parses the tight faces as real unary
    // operators, so refuse the classification explicitly: token-exact, the
    // single-sign unary faces (`-$V`/`+$V`) and the bare-meta wildcard over
    // postfix lines (`$alpha = $V;` answers `$alpha = $v--;`) keep their
    // lanes.
    if php_rhs_has_tight_doubled_sign(rhs) {
        return None;
    }
    // A fully-literal STATIC scoped target (`C::$s`, `Foo\Bar::$s`,
    // `\Foo::$s`, literal + meta-index subscripts) is ANSWERED — byte-exact
    // LHS compare (structural when a subscript index is one canonical
    // metavariable), `;`-optional. The operator gate is exact across all
    // three candidate families: assignments, augmented assignments, AND
    // binary faces. A DYNAMIC-CLASS head (`$C::$s`, `$$C::$s`) rides the
    // static lane for `=` assignments only (binding the whole candidate
    // scope text dollar-included); augmented/binary dynamic faces refuse.
    let static_target = (is_php_static_scope_target(lhs)
        || (op == "=" && php_static_target_dynamic_head(lhs).is_some()))
        && php_op_class(op).is_some();
    if !is_php_literal_target(lhs) {
        // PHP `=`-assignment targets carrying canonical metavariables BIND:
        // the whole-meta target (`$X = $Y` binds X to the candidate LHS
        // text) and the meta member LINK (`$o->$A = $Y` binds A to the link
        // name text). The whole-meta `;`-FUL spelling stays REFUSED
        // (`$X = $Y;` answers []), while the meta-LINK semi face
        // (`$o->$A = $Y;`) is answered — so the `;` guard is WHOLE-META-LHS
        // ONLY. Whole-meta admission is `=`-ONLY: meta-LHS binary faces
        // belong to the operand-template lane.
        let meta_target = if op == "=" {
            if is_pure_metavariable(lhs) {
                !p.ends_with(';')
            } else {
                php_meta_link_target(lhs)
            }
        } else {
            false
        };
        if !meta_target && !static_target {
            return None;
        }
    }
    // A block comment at the RHS HEAD (`$a = /* h */ $V + 1;`) is trivia at
    // the operator slot — the reference's CST puts it as a direct child of
    // the assignment node between `=` and `right`, and the face ANSWERS when
    // the candidate carries the text-exact comment there. Strip the head
    // comments and decide bare-meta vs rhs_expr on the stripped body; the
    // walker re-derives the same sequence from the pattern text and enforces
    // the positional text-exactness at the candidate's operator node. The
    // tight-sign scan above deliberately ran on the UNSTRIPPED rhs (the scan
    // skips comment bodies itself, so its straddle bytes are unchanged).
    let (_, rhs_body) = php_rhs_head_block_comments(rhs);
    let rhs_body = rhs_body.trim();
    // The "single-bare-meta-arg call RHS is the ONE refused static-target
    // shape" veto is deleted — the re-probe refuted it (`C::$s = f($V)` binds
    // V). The face now rides the structural rhs_expr machinery below.
    // One bare canonical meta keeps the registered capture path.
    if let Some((value, value_multi)) = php_bare_meta(rhs_body) {
        return Some(NativeKind::Assignment {
            target: lhs.to_string(),
            op,
            op_class: php_op_class(op)?,
            rhs_expr: None,
            value: value.to_string(),
            value_multi,
        });
    }
    // Anything else is an expression pattern: it must parse and carry no
    // META assignment-target (the reference refuses those faces).
    let rhs = rhs_body.to_string();
    validate_php_rhs_expr(&rhs)?;
    Some(NativeKind::Assignment {
        target: lhs.to_string(),
        op,
        op_class: php_op_class(op)?,
        rhs_expr: Some(rhs),
        value: String::new(),
        value_multi: false,
    })
}

/// Split leading `/* */` block comments off the head of a php assignment
/// RHS — returns the comment texts (byte-exact, the reference compares
/// comment NODE text) and the remainder. Only a comment run that starts at
/// the very head (whitespace-separated) is head placement; comments after
/// code stay inside the RHS expression where the structural matcher's
/// comment-alignment discipline already rules. An unterminated `/*` is not a
/// head comment (the doc parse refuses that pattern, fail-closed either way).
pub(crate) fn php_rhs_head_block_comments(rhs: &str) -> (Vec<String>, &str) {
    let mut rest = rhs.trim_start();
    let mut out = Vec::new();
    while let Some(tail) = rest.strip_prefix("/*") {
        let Some(end) = tail.find("*/") else {
            break;
        };
        out.push(format!("/*{}*/", &tail[..end]));
        // Comments in the head run are whitespace-separated
        // (`$a = /* a */ /* b */ $V + 1;` — the reference answers the line
        // carrying both slots); the run must trim BETWEEN the comments or
        // the second `/*` hides behind the leading space and rides into
        // the RHS body where the expr grammar refuses it.
        rest = tail[end + 2..].trim_start();
    }
    (out, rest)
}

/// The walker-side derivation of the same head-comment sequence from the
/// raw pattern text — the pattern's literal `target`
/// (byte-exact LHS), the operator, and then a whitespace-separated run of
/// block comments. An operator-continuation byte (`=` of `==`, `=>`, ...)
/// right after the stripped op bails (no head comments — the spellings the
/// assignment lane admits never carry one there).
pub(crate) fn php_assignment_head_comments(pattern: &str, target: &str, op: &str) -> Vec<String> {
    let Some(rest) = pattern.trim().strip_prefix(target) else {
        return Vec::new();
    };
    let Some(rest) = rest.trim_start().strip_prefix(op) else {
        return Vec::new();
    };
    if rest.starts_with('=') {
        return Vec::new();
    }
    let (comments, _) = php_rhs_head_block_comments(rest.trim_start());
    comments
}

/// True when `rhs` carries a `--`/`++` token TIGHT to an operand on either
/// side — the reference's parse-error class for doubled signs (see
/// [`classify_php_assignment`]). A sign surrounded by whitespace on both
/// sides is the lenient binary+unary spelling both engines answer. String
/// literal bodies are skipped (a sign inside a literal is text), as are
/// pattern-side `/* */` block-comment bodies (comment content is not an
/// operand). `//`/`#` comment kinds are NOT skipped: the reference refuses
/// patterns carrying them outright. The scan is ASCII-byte-exact, so no
/// slice can land mid-code-point.
pub(crate) fn php_rhs_has_tight_doubled_sign(rhs: &str) -> bool {
    let bytes = rhs.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        break;
                    }
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                // A block comment is reference parse trivia — skip to the
                // closing `*/`. An unterminated comment consumes the rest
                // of the rhs (the doc parse refuses that pattern,
                // fail-closed either way).
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            sign @ (b'-' | b'+') if bytes.get(i + 1) == Some(&sign) => {
                let spaced_left = i > 0 && matches!(bytes[i - 1], b' ' | b'\t');
                let spaced_right = bytes[i + 2..]
                    .first()
                    .is_none_or(|&b| matches!(b, b' ' | b'\t'));
                if !(spaced_left && spaced_right) {
                    return true;
                }
                i += 2;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// One top-level binary operator of the widened php operand family,
/// longest-first so prefix operators (`===` vs `==` vs `=`, `??=` vs `??`,
/// `<=>` vs `<=`) split correctly.
pub(crate) const PHP_BINARY_OPERATORS: &[&str] = &[
    "**=", "<<=", ">>=", "===", "!==", "<=>", "??=", "+=", "-=", "*=", "/=", "%=", ".=", "|=",
    "&=", "^=", "==", "!=", "<=", ">=", "&&", "||", "??", "=", "<", ">", "+", "-", "*", "/", "%",
    ".",
];

/// Split `p` at its FIRST top-level operator (paren/bracket-depth aware,
/// php string literals skipped). Returns None when no family operator is
/// present at the top level.
pub(crate) fn split_php_binary(p: &str) -> Option<(&str, &'static str, &str)> {
    let bytes = p.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'\'' | b'"' => {
                // Skip the string literal body (quote escapes included).
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        break;
                    }
                    i += 1;
                }
            }
            _ if depth == 0 => {
                // The `->` member connector (and `=>`) are NOT binary
                // operators — skip them whole.
                if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'>') {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'=' && bytes.get(i + 1) == Some(&b'>') {
                    i += 2;
                    continue;
                }
                for &op in PHP_BINARY_OPERATORS {
                    // `i` advances one BYTE at a time, so a multi-byte
                    // character outside a string literal (`$α = $V;` — PHP
                    // allows non-ASCII identifier bytes) lands mid-code-point;
                    // slice only at char boundaries.
                    if p.is_char_boundary(i) && p[i..].starts_with(op) {
                        return Some((p[..i].trim(), op, &p[i + op.len()..]));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Which candidate node kind serves the pattern operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhpBinaryClass {
    /// `=` — `assignment_expression` candidates.
    Assign,
    /// `+=` etc — `augmented_assignment_expression` candidates.
    Augmented,
    /// `==`, `+`, ... — `binary_expression` candidates.
    Binary,
}

pub(crate) fn php_op_class(op: &str) -> Option<PhpBinaryClass> {
    match op {
        "=" => Some(PhpBinaryClass::Assign),
        "**=" | "<<=" | ">>=" | "+=" | "-=" | "*=" | "/=" | "%=" | ".=" | "??=" | "|=" | "&="
        | "^=" => Some(PhpBinaryClass::Augmented),
        "===" | "!==" | "<=>" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "??" | "<" | ">"
        | "+" | "-" | "*" | "/" | "%" | "." => Some(PhpBinaryClass::Binary),
        _ => None,
    }
}

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

/// The RHS is exactly one bare canonical metavariable (`$V` / `$$V`
/// single namespace, `$$$V` multi).
pub(crate) fn php_bare_meta(rhs: &str) -> Option<(&str, bool)> {
    if let Some(name) = rhs.strip_prefix("$$$") {
        is_metavar_name(name).then_some((name, true))
    } else {
        capture_name(rhs).map(|name| (name, false))
    }
}

/// Validate a general RHS expression pattern — it must parse under php and
/// contain no META assignment-target (`$V = $W` — the reference refuses
/// meta LHS targets anywhere in the pattern). The veto MUST slice against
/// the wrapper DOC — the tree was parsed from `<?php {rhs};`, so every
/// pattern-node byte offset is doc-relative (shifted by the 6-byte prefix).
/// Slicing the bare `rhs` made the verdict a function of byte-length
/// coincidence: on `$alpha = $V = $W;` the inner target's doc range [6..8]
/// is out of bounds for the 7-byte rhs (veto skipped — the registered
/// refusal answered) while longer rhs texts mis-sliced onto unrelated
/// nodes (answering faces falsely refused).
pub(crate) fn validate_php_rhs_expr(rhs: &str) -> Option<()> {
    let template = parse_php_rhs_tree(rhs)?;
    validate_no_meta_target(template.tree.root_node(), &template.doc)
}

/// The php RHS expression pattern parses as a `;`-terminated statement
/// under the `<?php` tag — the grammar's `program` rule admits statements
/// ONLY after the tag, and the bare expression is a parse ERROR
/// (expression statements demand the terminator), so with php's absent
/// expression-context wrapper every rhs_expr face would classify None and
/// fall to the NeverMatches gate. Pattern node byte offsets are
/// DOC-relative — callers pass `template.doc` as the pattern source for
/// `php_rhs_expr_matches`.
pub(crate) fn parse_php_rhs_tree(rhs: &str) -> Option<std::sync::Arc<LiteralTemplate>> {
    let doc = format!("<?php {rhs};");
    parse_pattern_tree(Language::Php, &doc)
}

pub(crate) fn validate_no_meta_target(node: Node, source: &str) -> Option<()> {
    if matches!(
        node.kind(),
        "assignment_expression" | "augmented_assignment_expression"
    ) {
        if let Some(left) = node.child_by_field_name("left") {
            if let Some(text) = node_text(&left, source) {
                if capture_name(text.trim()).is_some() {
                    return None;
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        validate_no_meta_target(child, source)?;
    }
    Some(())
}

/// A classified bare php binary-expression META template — the parsed
/// `<?php {core};` doc plus whether the raw pattern spelled a trailing `;`
/// (the reference roots that spelling at the statement).
pub(crate) struct PhpOperandTemplate {
    parsed: std::sync::Arc<LiteralTemplate>,
    had_semi: bool,
}

impl PhpOperandTemplate {
    /// The comparison root inside the doc: `program` → `expression_statement`
    /// → the binary/paren expression (no-semi), or the statement itself
    /// (semi — the reference's statement-rooted `;` discipline).
    pub(crate) fn comparison_root(&self) -> Option<Node<'_>> {
        php_operand_comparison_root(&self.parsed, self.had_semi)
    }
}

/// Admission of a bare php binary-expression META template — no `=`
/// assignment root, no literal LHS: `$A && $B`, `($A && $B)`, `$A && $B;`.
/// Every `$`-token must be canonical; the `;`-stripped doc's comparison
/// root must be a `binary_expression` or `parenthesized_expression`
/// (no-semi: kind-exact bind) or an `expression_statement` wrapping a binary
/// (semi: only bare `expr;` statements answer). Rootless NON-binary faces
/// (`$A = $B`, `$A;`) are NOT admitted — they keep their loud riders. The
/// sibling assignment lane's [`validate_no_meta_target`] veto runs over the
/// parsed doc here too: an operand template carrying an embedded
/// assignment/augmented-assignment with a META target is refused the lane.
pub(crate) fn classify_php_operand_template(pattern: &str) -> Option<PhpOperandTemplate> {
    let p = pattern.trim();
    let had_semi = p.ends_with(';');
    let core = p.strip_suffix(';').unwrap_or(p).trim();
    if !core.contains('$')
        || pattern_has_noncanonical_metavar(core)
        || php_operand_has_two_dollar_token(core)
        || php_operand_has_literal_operand(core)
    {
        return None;
    }
    let parsed = parse_php_rhs_tree(core)?;
    php_operand_comparison_root(&parsed, had_semi)?;
    validate_no_meta_target(parsed.tree.root_node(), &parsed.doc)?;
    Some(PhpOperandTemplate { parsed, had_semi })
}

/// Scope guard: a LITERAL leaf operand anywhere in the pattern (an
/// identifier/number/quoted byte outside a `$`-token). The operand family is
/// all-meta; literal-operand faces keep their registered classes.
pub(crate) fn php_operand_has_literal_operand(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'$' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'(' | b')' | b';' | b'+' | b'-' | b'*' | b'/'
            | b'%' | b'=' | b'<' | b'>' | b'!' | b'?' | b'.' | b'|' | b'&' | b':' => i += 1,
            _ => return true,
        }
    }
    false
}

/// Scope guard: a `$$NAME` dynamic-variable token. The registered `$$`
/// wildcard binds plain AND dynamic VARIABLE candidates only
/// ([`php_rhs_expr_matches`]), while the reference answers the universal
/// spelling over every operand kind (probed `$$A && $B` == the `$A && $B`
/// set, integers included) — the lane must not serve it with narrower
/// semantics, so the face keeps its registered loud rider.
pub(crate) fn php_operand_has_two_dollar_token(p: &str) -> bool {
    let bytes = p.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let mut run = 0usize;
            while i < bytes.len() && bytes[i] == b'$' {
                run += 1;
                i += 1;
            }
            let name_len = bytes[i..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            if run == 2 && name_len > 0 {
                return true;
            }
            i += name_len;
        } else {
            i += 1;
        }
    }
    false
}

/// Named children for the operand-template descent: the php tag and
/// comment trivia never shape the single-child chain (a pattern-side
/// comment stays INSIDE the comparison root, where
/// [`php_rhs_expr_matches`] text-matches it).
pub(crate) fn php_operand_children<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() != "php_tag" && !child.kind().contains("comment"))
        .collect()
}

pub(crate) fn php_operand_comparison_root<'a>(
    parsed: &'a LiteralTemplate,
    had_semi: bool,
) -> Option<Node<'a>> {
    let mut node = parsed.tree.root_node();
    // `program`: descend through the single named child once the php tag
    // and trivia are filtered (the `unwrap_pattern_expression` discipline).
    while node.kind() == "program" {
        let named = php_operand_children(node);
        match named.as_slice() {
            [only] => node = *only,
            _ => return None,
        }
    }
    if node.kind() != "expression_statement" {
        return None;
    }
    let named = php_operand_children(node);
    match named.as_slice() {
        [only] => {
            if had_semi {
                // The reference roots the `;`-terminated pattern at the
                // statement; the statement's single child must still be the
                // binary shape.
                (only.kind() == "binary_expression").then_some(node)
            } else if matches!(
                only.kind(),
                "binary_expression" | "parenthesized_expression"
            ) {
                Some(*only)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Match the widened php operand lane. Candidates are the operator class's
/// node kind whose LEFT text equals the pattern's literal target
/// byte-exactly and whose operator token equals the pattern's (binary
/// candidates of every operator share one node kind). The RHS binds either
/// through the bare-meta capture path or the structural expression matcher.
/// Statement discipline: a `;`-terminated pattern binds ONLY candidates
/// rooted at an `expression_statement`, and the match span is that statement
/// INCLUDING the `;`; a `;`-less pattern also answers embedded nodes and
/// keeps the assignment-node span.
#[allow(clippy::too_many_arguments)]
pub(crate) fn walk_php_assignments(
    node: Node,
    source: &str,
    pattern: &str,
    target: &str,
    op: &str,
    op_class: PhpBinaryClass,
    rhs_root: Option<(Node, &str)>,
    lhs_root: Option<(Node, &str)>,
    value: &str,
    value_multi: bool,
    head_comments: &[String],
    out: &mut Vec<PatternMatch>,
) {
    let pattern_is_semi = pattern.trim().ends_with(';');
    let candidate_kind = match op_class {
        PhpBinaryClass::Assign => "assignment_expression",
        PhpBinaryClass::Augmented => "augmented_assignment_expression",
        PhpBinaryClass::Binary => "binary_expression",
    };
    if node.kind() == candidate_kind && !is_in_comment_or_string(&node) {
        // The matched operator node's own children align strictly: a comment
        // sitting directly inside the candidate breaks alignment and the face
        // is refused; comments between RHS operands sit inside the RHS
        // sub-expression, where the structural matcher stays transparent. When
        // the PATTERN ITSELF carries RHS-head block comments, the face answers
        // exactly when the candidate's direct comment children equal the
        // pattern's sequence byte-for-byte. A BARE-META RHS swallows the
        // candidate's direct comment children; expression-RHS faces keep the
        // refusal (the pattern's structural child must align with the
        // candidate's).
        let head_comment_ok = if !head_comments.is_empty() {
            let mut child_cursor = node.walk();
            let cand_comments: Vec<String> = node
                .children(&mut child_cursor)
                .filter(|child| child.kind().contains("comment"))
                .filter_map(|child| node_text(&child, source).map(str::to_string))
                .collect();
            cand_comments == head_comments
        } else if rhs_root.is_none() {
            true
        } else {
            let mut child_cursor = node.walk();
            let any_comment = node
                .children(&mut child_cursor)
                .any(|child| child.kind().contains("comment"));
            !any_comment
        };
        let op_token_ok = head_comment_ok
            && match op_class {
                PhpBinaryClass::Assign => true,
                PhpBinaryClass::Augmented | PhpBinaryClass::Binary => {
                    // The operator is an anonymous child token; text-equal.
                    let mut cursor = node.walk();
                    let mut children = node.children(&mut cursor);
                    children.any(|child| {
                        !child.is_named()
                            && node_text(&child, source).is_some_and(|text| text == op)
                    })
                }
            };
        // `;`-terminated patterns root at expression statements.
        let statement = node
            .parent()
            .filter(|parent| parent.kind() == "expression_statement");
        if op_token_ok && (!pattern_is_semi || statement.is_some()) {
            if let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                // A meta target (`$X = $Y`, `$o->$A = $Y`) binds through the
                // RHS expression machinery — the `variable_name` pattern arm
                // binds the candidate LHS text, and a meta LINK binds the
                // candidate link node text (`a` literal / `$a` dynamic) —
                // instead of the verbatim text equality the literal targets
                // keep.
                let mut captures = BTreeMap::new();
                // A dynamic-class static head (`$C::$s` / `$$C::$s`, `=`-only)
                // binds the WHOLE candidate scope text — variable heads
                // dollar-included AND concrete identifier/qualified heads.
                // The admitted scope kinds are name, qualified_name,
                // variable_name — anything else (call/member scopes) stays
                // fail-closed. The prop is literal text equality.
                let target_ok = if let Some(meta_name) = php_static_target_dynamic_head(target) {
                    let scope = left.child_by_field_name("scope");
                    let prop = left.child_by_field_name("name");
                    let scope_ok = left.kind() == "scoped_property_access_expression"
                        && scope.is_some_and(|scope| {
                            matches!(scope.kind(), "name" | "qualified_name" | "variable_name")
                        });
                    let prop_ok =
                        prop.and_then(|prop| node_text(&prop, source))
                            .is_some_and(|have| {
                                have == target
                                    .split_once("::")
                                    .map(|(_, want)| want.trim())
                                    .unwrap_or_default()
                            });
                    scope_ok
                        && prop_ok
                        && scope
                            .and_then(|scope| node_text(&scope, source))
                            .is_some_and(|text| {
                                bind_capture(&mut captures, meta_name, text).is_some()
                            })
                } else if let Some((lhs_pat, lhs_doc)) = lhs_root {
                    let lhs_expr = unwrap_pattern_expression(lhs_pat);
                    php_rhs_expr_matches(lhs_expr, left, source, lhs_doc, &mut captures)
                } else {
                    node_text(&left, source).map(str::to_string).as_deref() == Some(target)
                };
                if target_ok {
                    // The `;`-terminated pattern's match node is the
                    // `;`-rooted statement (span consumes the `;`).
                    let span_node = if pattern_is_semi {
                        statement.unwrap_or(node)
                    } else {
                        node
                    };
                    let rhs_bound = if let Some((rhs_pattern, rhs_source)) = rhs_root {
                        // General expression RHS: structural match with meta
                        // binding (rhs_expr faces never carry the bare-meta
                        // binding). The pattern root is unwrapped to the
                        // expression node and text-compared against its own
                        // pattern source.
                        let rhs_expr_node = unwrap_pattern_expression(rhs_pattern);
                        php_rhs_expr_matches(
                            rhs_expr_node,
                            right,
                            source,
                            rhs_source,
                            &mut captures,
                        )
                    } else if let Some(right_text) = node_text(&right, source).map(str::to_string) {
                        bind_capture_kind(&mut captures, value, &right_text, value_multi).is_some()
                    } else {
                        false
                    };
                    if rhs_bound {
                        if let Some(text) = node_text(&span_node, source) {
                            captures.insert("MATCH".to_string(), text.to_string());
                        }
                        out.push(hit_for_node(&span_node, source, pattern, captures));
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_assignments(
            child,
            source,
            pattern,
            target,
            op,
            op_class,
            rhs_root,
            lhs_root,
            value,
            value_multi,
            head_comments,
            out,
        );
    }
}

/// Walk a classified bare php binary-expression META template
/// ([`classify_php_operand_template`]). Candidates are nodes of the
/// pattern's OWN comparison root kind — the binary/paren expression
/// (no-semi: the reference binds at expression level, nested binaries
/// included) or the wrapping `expression_statement` (semi: the
/// reference's statement-rooted `;` discipline, so only bare `expr;`
/// statements answer) — unified through [`php_rhs_expr_matches`] (the
/// expression machinery: metavariable binds with the same-name veto,
/// text-exact leaves/tokens, transparent
/// candidate comments, text-exact pattern comments).
pub(crate) fn walk_php_operand_template(
    tree: &tree_sitter::Tree,
    source: &str,
    pattern: &str,
    tpl: &PhpOperandTemplate,
    out: &mut Vec<PatternMatch>,
) {
    let Some(pat_root) = tpl.comparison_root() else {
        return;
    };
    let doc = tpl.parsed.doc.as_str();
    // P7-style hash-set dedup: nested self-similar binaries can emit the
    // same node twice through distinct binds; insertion order is preserved.
    let mut seen = std::collections::HashSet::new();
    walk_php_operand_node(
        tree.root_node(),
        source,
        pattern,
        pat_root,
        doc,
        &mut seen,
        out,
    );
}

pub(crate) fn walk_php_operand_node<'a>(
    node: Node<'a>,
    source: &str,
    pattern: &str,
    pat_root: Node<'_>,
    doc: &str,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    out: &mut Vec<PatternMatch>,
) {
    if node.kind() == pat_root.kind() && !is_in_comment_or_string(&node) {
        let mut captures = BTreeMap::new();
        if php_rhs_expr_matches(pat_root, node, source, doc, &mut captures) {
            let byte_start = node.start_byte();
            let byte_end = node.end_byte();
            if seen.insert((byte_start, byte_end)) {
                if let Some(text) = node_text(&node, source) {
                    captures.insert("MATCH".to_string(), text.to_string());
                }
                out.push(hit_for_node(&node, source, pattern, captures));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_operand_node(child, source, pattern, pat_root, doc, seen, out);
    }
}
/// Structural match of a general RHS expression pattern against the
/// candidate's right-hand node. A `variable_name` pattern leaf is either a
/// canonical meta (`$V`/`$$V` single namespace, `$$$V` multi — binds the
/// candidate node text, same-name veto via `bind_capture_kind`) or literal
/// variable text (byte-exact). A `$$V` dynamic-variable pattern token
/// wildcards plain AND dynamic candidate variables. Any other node requires
/// the SAME kind with aligned children: childless leaves (integers, names,
/// string content) compare text-exactly, anonymous tokens (operators,
/// punctuation) compare text-exactly, named children recurse pairwise.
/// `pat`'s text is read from `pat_source` (pattern node offsets index the
/// pattern text, not the candidate source).
pub(crate) fn php_rhs_expr_matches(
    pat: Node,
    cand: Node,
    source: &str,
    pat_source: &str,
    captures: &mut BTreeMap<String, String>,
) -> bool {
    if pat.kind() == "variable_name" {
        if let Some(text) = node_text(&pat, pat_source) {
            if let Some(name) = capture_name(text) {
                let multi = text.starts_with("$$$");
                if let Some(cand_text) = node_text(&cand, source) {
                    return bind_capture_kind(captures, name, cand_text, multi).is_some();
                }
                return false;
            }
        }
        return match (node_text(&pat, pat_source), node_text(&cand, source)) {
            (Some(want), Some(have)) => want == have,
            _ => false,
        };
    }
    // A `$$V` dynamic-variable pattern token is a canonical wildcard over
    // BOTH variable spellings (plain and dynamic); candidate nodes of any
    // other kind refuse (fail-closed — only the probed faces admit).
    if pat.kind() == "dynamic_variable_name" {
        if let Some(text) = node_text(&pat, pat_source) {
            if let Some(name) = text.strip_prefix("$$").filter(|n| is_metavar_name(n)) {
                if matches!(cand.kind(), "variable_name" | "dynamic_variable_name") {
                    if let Some(cand_text) = node_text(&cand, source) {
                        return bind_capture_kind(captures, name, cand_text, false).is_some();
                    }
                }
                return false;
            }
        }
    }
    // An argument list carrying a `$$$NAME` rest slot is the
    // rest-metavariable contract — bind the remaining arguments' source
    // bytes (whole-list incl. empty, leading, mid, or trailing) instead
    // of the strict child zip, which cannot decompose the raw `$$$` text
    // (probed: `$b = f($v, $$$A);` answers f($v,1)/f($v,1,2)/f($v,$u)/
    // f($v,1,2,3) and refuses the zero-argument trailing face;
    // `$b = f($$$A);` answers every arity incl. empty).
    // Lists without a rest keep the strict structural path byte-compatible.
    if pat.kind() == "arguments" {
        if let Some(text) = node_text(&pat, pat_source) {
            if text.contains("$$$") {
                let (Some(open), Some(close)) = (text.find('('), text.rfind(')')) else {
                    return false;
                };
                if close <= open {
                    return false;
                }
                let inner = &text[open + 1..close];
                let Some(slots) = parse_rhs_arg_slots(inner) else {
                    return false;
                };
                if cand.kind() != "arguments" {
                    return false;
                }
                let mut cand_cursor = cand.walk();
                let nodes: Vec<_> = cand
                    .named_children(&mut cand_cursor)
                    .filter(|child| !is_trivia_kind(child.kind()))
                    .collect();
                return arg_slots_match(&slots, &nodes, source, captures).is_some();
            }
        }
    }
    // Childless leaves are literal tokens — byte-exact text compare (the
    // zero-children recursion below would otherwise admit ANY same-kind
    // leaf, e.g. `1` matching `2`).
    if pat.child_count() == 0 {
        return match (node_text(&pat, pat_source), node_text(&cand, source)) {
            (Some(want), Some(have)) => want == have,
            _ => false,
        };
    }
    if pat.kind() != cand.kind() {
        return false;
    }
    // The reference aligns expression children with CANDIDATE comment
    // children transparent (probed: `$a = $V + 1;` answers
    // `$a = $v /* = */ + 1;` and the paren spellings), while a PATTERN-side
    // comment must find its text-exact counterpart (probed: the commented
    // paren pattern answers only the self line — a different comment text
    // does not match). Walk the pattern children against the candidate
    // stream with that asymmetry.
    let pat_children: Vec<_> = {
        let mut cursor = pat.walk();
        pat.children(&mut cursor).collect()
    };
    let cand_children: Vec<_> = {
        let mut cursor = cand.walk();
        cand.children(&mut cursor).collect()
    };
    let mut cand_index = 0usize;
    for pat_child in &pat_children {
        if pat_child.kind().contains("comment") {
            let Some(want) = node_text(pat_child, pat_source) else {
                return false;
            };
            let mut matched = false;
            while cand_index < cand_children.len() {
                let cand_child = &cand_children[cand_index];
                cand_index += 1;
                if cand_child.kind().contains("comment")
                    && node_text(cand_child, source).is_some_and(|text| text == want)
                {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return false;
            }
            continue;
        }
        while cand_index < cand_children.len()
            && cand_children[cand_index].kind().contains("comment")
        {
            cand_index += 1;
        }
        let Some(cand_child) = cand_children.get(cand_index) else {
            return false;
        };
        cand_index += 1;
        if pat_child.is_named() {
            if !cand_child.is_named()
                || !php_rhs_expr_matches(*pat_child, *cand_child, source, pat_source, captures)
            {
                return false;
            }
        } else if cand_child.is_named() {
            return false;
        } else {
            match (
                node_text(pat_child, pat_source),
                node_text(cand_child, source),
            ) {
                (Some(want), Some(have)) if want == have => {}
                _ => return false,
            }
        }
    }
    // Trailing candidate comments are transparent; any other leftover
    // candidate child breaks the alignment.
    while cand_index < cand_children.len() && cand_children[cand_index].kind().contains("comment") {
        cand_index += 1;
    }
    cand_index == cand_children.len()
}
