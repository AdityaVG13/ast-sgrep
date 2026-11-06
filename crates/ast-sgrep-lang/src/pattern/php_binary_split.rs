//! PHP binary-operator splitting for the operand lanes.

use super::*;

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
