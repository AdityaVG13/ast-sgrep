//! Indexed pattern signature builders and SIMD prefilter literals.
//!
//! Exact string formats here are part of the on-disk `pattern_nodes` contract —
//! keep them byte-identical when refactoring.

use crate::pattern::{classify_native, is_pattern_ident, DECL_PATTERN_PREFIXES};

/// Declaration keyword prefixes used when classifying patterns / building index keys.
/// Shared with `classify_native` via [`DECL_PATTERN_PREFIXES`].
pub use crate::pattern::DECL_PATTERN_PREFIXES as DECL_PREFIXES;

/// True when `pattern_nodes` rows for these signatures are the same nodes the
/// native matcher would return, so a tree-sitter re-walk cannot add hits.
///
/// Kind-only signatures over-match (`fn $NAME` → every function) and still
/// need native confirmation. Ident, `decl:`, `call:`, and `call-name:`
/// signatures are exact, so the indexed rows are the result.
pub fn index_can_serve_pattern(pattern: &str, signatures: &[String]) -> bool {
    if signatures.is_empty() || signatures.iter().any(|s| s.starts_with("kind:")) {
        return false;
    }
    let pattern = pattern.trim();
    is_pattern_ident(pattern)
        || signatures.iter().all(|s| {
            s.starts_with("decl:") || s.starts_with("call:") || s.starts_with("call-name:")
        })
}

/// Map a structural pattern to the exact index signatures stored in `pattern_nodes`.
///
/// Returns `None` when the pattern shape is not indexable (exotic / nested).
pub fn cached_pattern_signatures(pattern: &str) -> Option<Vec<String>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Some(vec![]);
    }
    if !pattern.contains('$') {
        if is_pattern_ident(pattern) {
            return Some(vec![pattern.to_string()]);
        }
        // `fn foo` / `struct Bar` fall through to decl: rows. A raw
        // "fn foo" string is not stored as a pattern_nodes signature.
    }
    // Never let a broad cached signature bypass native validation. In
    // particular, malformed declaration tails must remain match-none.
    classify_native(pattern)?;
    // Nested body templates (`fn $N($$$) { $STMT }`, `if $COND { $BODY }`)
    // are not indexable: `pattern_nodes` signatures cannot express statement
    // counts, so serving them from the index would over-match. The native
    // tree-sitter scan is the sole source for these shapes.
    if pattern.contains('{') {
        return None;
    }
    for (prefix, kinds) in CACHED_DECL_KIND_TABLE {
        if let Some(rest) = pattern.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| ch == '(' || ch == '{' || ch.is_whitespace())
                .next()
                .unwrap_or_default();
            if name.starts_with('$') {
                return Some(kinds.iter().map(|kind| format!("kind:{kind}")).collect());
            }
            if is_pattern_ident(name) {
                return Some(vec![format!("decl:{}:{name}", prefix.trim())]);
            }
            return None;
        }
    }
    let open = pattern.find('(')?;
    let close = pattern.rfind(')')?;
    if close + 1 != pattern.len() || !pattern[open + 1..close].contains("$$$") {
        return None;
    }
    let callee = pattern[..open].trim();
    if callee.starts_with('$') && !callee.contains('.') {
        // Byte-identical to the historical core classifier.
        return Some(vec!["kind:call_expression".into(), "kind:call".into()]);
    }
    if let Some(name) = callee.rsplit('.').next() {
        if callee.contains('$') && is_pattern_ident(name) {
            return Some(vec![format!("call-name:{name}")]);
        }
    }
    is_pattern_path(callee).then(|| vec![format!("call:{callee}")])
}

/// Candidate KIND signatures for patterns whose exact shape is not indexable
/// (braced declaration templates like `fn $NAME($$$) { $$$ }`) but whose
/// matches must still be nodes of a known kind.
///
/// Soundness for candidate narrowing: every native match of such a pattern IS
/// a node of the returned kind, so any file containing a match necessarily
/// contains a `pattern_nodes` row with one of these signatures. The index
/// narrows the file set; the native tree-sitter matcher still decides every
/// hit, so over-broad kind candidates never change results.
pub fn candidate_kind_signatures(pattern: &str) -> Option<Vec<String>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    classify_native(pattern)?;
    for (prefix, kinds) in CACHED_DECL_KIND_TABLE {
        if pattern.starts_with(prefix) {
            return Some(kinds.iter().map(|kind| format!("kind:{kind}")).collect());
        }
    }
    None
}

/// Longest concrete token suitable for a byte-level SIMD prefilter.
///
/// Declaration keywords alone are never returned (they are not cross-language
/// literals). Metavariable-only callees yield `None`.
pub fn required_pattern_literal(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    if !pattern.contains('$') {
        return Some(pattern.to_string());
    }
    // If templates: every indexed language spells the keyword `if`, so any
    // file that can hold an if-node must contain those bytes.
    if pattern.starts_with("if ") || pattern.starts_with("if(") {
        return Some("if".to_string());
    }
    for (prefix, _) in DECL_PATTERN_PREFIXES {
        if let Some(rest) = pattern.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| ch == '(' || ch == '{' || ch == '<' || ch.is_whitespace())
                .next()
                .unwrap_or_default();
            return (!name.is_empty() && !name.starts_with('$')).then(|| name.to_string());
        }
    }
    let callee = pattern.split_once('(')?.0.trim();
    callee
        .split(['.', ':'])
        .filter(|segment| !segment.is_empty() && !segment.starts_with('$'))
        .max_by_key(|segment| segment.len())
        .map(str::to_string)
}

/// Hybrid structural boost signatures for a bare identifier term.
///
/// Formats must stay byte-identical to the historical `structural_index_pass` keys.
pub fn structural_term_signatures(term: &str) -> [String; 6] {
    [
        format!("call-name:{term}"),
        format!("call:{term}"),
        format!("decl:fn:{term}"),
        format!("decl:def:{term}"),
        format!("decl:function:{term}"),
        term.to_string(),
    ]
}

/// Prefix → tree-sitter kind names used for metavariable declaration lookups.
///
/// `fn ` / `def ` entries stay byte-identical to the historical core classifier
/// (`kind:function_item` / `kind:function_definition` only). Broader prefixes
/// cover kinds emitted by `collect_pattern_nodes` across all 13 languages.
const CACHED_DECL_KIND_TABLE: &[(&str, &[&str])] = &[
    ("fn ", &["function_item"]),
    ("def ", &["function_definition"]),
    (
        "function ",
        &[
            "function_declaration",
            "protocol_function_declaration",
            "method_definition",
            "method_declaration",
            "method",
            "singleton_method",
            "local_function_statement",
        ],
    ),
    ("func ", &["function_declaration"]),
    (
        "class ",
        &[
            "class_definition",
            "class_declaration",
            "class",
            "record_declaration",
            "class_specifier",
        ],
    ),
    (
        "struct ",
        &["struct_item", "struct_declaration", "struct_specifier"],
    ),
    (
        "interface ",
        &[
            "trait_item",
            "interface_declaration",
            "protocol_declaration",
        ],
    ),
];

fn is_pattern_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('$')
        && value
            .split(['.', ':'])
            .filter(|p| !p.is_empty())
            .all(is_pattern_ident)
}

#[cfg(test)]
mod index_serve_tests {
    use super::{cached_pattern_signatures, index_can_serve_pattern};

    #[test]
    fn ident_and_decl_are_index_complete_kind_is_not() {
        let ident = cached_pattern_signatures("SearchHit").unwrap();
        assert!(index_can_serve_pattern("SearchHit", &ident));
        let decl = cached_pattern_signatures("fn greet_user").unwrap();
        assert!(index_can_serve_pattern("fn greet_user", &decl));
        let kind = cached_pattern_signatures("fn $NAME").unwrap();
        assert!(!index_can_serve_pattern("fn $NAME", &kind));
        assert!(!index_can_serve_pattern("fn $NAME() { $$$BODY }", &[]));
    }
}
