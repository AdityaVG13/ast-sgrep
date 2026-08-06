//! Indexed pattern signature builders and SIMD prefilter literals.
//!
//! Exact string formats here are part of the on-disk `pattern_nodes` contract —
//! keep them byte-identical when refactoring.

use crate::pattern::{is_pattern_ident, DECL_PATTERN_PREFIXES};

/// Declaration keyword prefixes used when classifying patterns / building index keys.
/// Shared with `classify_native` via [`DECL_PATTERN_PREFIXES`].
pub use crate::pattern::DECL_PATTERN_PREFIXES as DECL_PREFIXES;

/// Map a structural pattern to the exact index signatures stored in `pattern_nodes`.
///
/// Returns `None` when the pattern shape is not indexable (exotic / nested).
pub fn cached_pattern_signatures(pattern: &str) -> Option<Vec<String>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Some(vec![]);
    }
    if !pattern.contains('$') {
        return Some(vec![pattern.to_string()]);
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
        &[
            "struct_item",
            "struct_declaration",
            "struct_specifier",
        ],
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
mod tests {
    use super::*;

    #[test]
    fn cached_signatures_stay_byte_identical_for_legacy_shapes() {
        // No metavariables → exact pattern text is the index key.
        assert_eq!(
            cached_pattern_signatures("fn parse_low").unwrap(),
            vec!["fn parse_low".to_string()]
        );
        // Historical core classifier: fn/def metavariable → single kind key.
        assert_eq!(
            cached_pattern_signatures("fn $NAME($$$)").unwrap(),
            vec!["kind:function_item".to_string()]
        );
        assert_eq!(
            cached_pattern_signatures("def $NAME").unwrap(),
            vec!["kind:function_definition".to_string()]
        );
        assert_eq!(
            cached_pattern_signatures("fn parse_low($$$)").unwrap(),
            vec!["decl:fn:parse_low".to_string()]
        );
        assert_eq!(
            cached_pattern_signatures("$OBJ.method($$$)").unwrap(),
            vec!["call-name:method".to_string()]
        );
        assert_eq!(
            cached_pattern_signatures("foo.bar($$$)").unwrap(),
            vec!["call:foo.bar".to_string()]
        );
        assert_eq!(
            cached_pattern_signatures("kind:function_item").unwrap(),
            vec!["kind:function_item".to_string()]
        );
    }

    #[test]
    fn structural_term_signatures_match_legacy_formats() {
        assert_eq!(
            structural_term_signatures("renew"),
            [
                "call-name:renew".to_string(),
                "call:renew".to_string(),
                "decl:fn:renew".to_string(),
                "decl:def:renew".to_string(),
                "decl:function:renew".to_string(),
                "renew".to_string(),
            ]
        );
    }

    #[test]
    fn required_literal_skips_decl_keywords() {
        assert_eq!(
            required_pattern_literal("Needle($$$ARGS)").as_deref(),
            Some("Needle")
        );
        assert_eq!(required_pattern_literal("$FUNC($$$ARGS)"), None);
        assert_eq!(required_pattern_literal("fn $NAME($$$ARGS)"), None);
        assert_eq!(
            required_pattern_literal("fn parse_low").as_deref(),
            Some("fn parse_low")
        );
        assert_eq!(
            required_pattern_literal("fn parse_low($$$)").as_deref(),
            Some("parse_low")
        );
    }

    #[test]
    fn wildcard_call_signatures_stay_byte_identical() {
        assert_eq!(
            cached_pattern_signatures("$F($$$)").unwrap(),
            vec![
                "kind:call_expression".to_string(),
                "kind:call".to_string(),
            ]
        );
    }
}
