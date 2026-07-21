//! Structural and literal pattern matching over tree-sitter ASTs.
//!
//! **Why this exists (vs shelling out to ast-grep):**
//! - Indexed hybrid search needs a fast, in-process structural channel.
//! - External `ast-grep` is excellent for full metavariable rules, but process
//!   spawn + JSON parse is too heavy for tight loops and offline agents.
//! - We implement the common ~80% of patterns natively (function/method/class
//!   decls and calls with `$NAME` / `$$$` holes). Complex rules still fall
//!   through to external ast-grep when installed.

use crate::extract::{byte_to_line, is_in_comment_or_string, node_lines, node_text};
use crate::{Language, PatternNode};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
}

/// True when the pattern needs external ast-grep (we cannot handle it natively).
///
/// Patterns without `$` always run in-process. Patterns with `$`/`$$$` use the
/// native structural matcher when they fit a known shape; only exotic shapes
/// still require the external binary.
pub fn needs_ast_grep_fallback(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() || !p.contains('$') {
        return false;
    }
    // Native shapes we handle: fn/def/function/class $NAME, calls, member calls.
    classify_native(p).is_none()
}

pub fn tree_sitter_language(lang: Language) -> tree_sitter::Language {
    match lang {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Java | Language::CSharp => tree_sitter_java::LANGUAGE.into(),
        Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
    }
}

/// Unified entry: literal identifier match, or native structural match for `$` patterns.
pub fn match_pattern(
    lang: Language,
    source: &str,
    pattern: &str,
) -> anyhow::Result<Vec<PatternMatch>> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    if !pattern.contains('$') {
        return match_literal_pattern(lang, source, pattern);
    }
    match classify_native(pattern) {
        Some(kind) => match_structural(lang, source, &kind),
        None => Ok(Vec::new()), // caller may fall back to external ast-grep
    }
}

/// Matches identifier text exactly, including case.
///
/// This syntax-level policy intentionally differs from relevance ranking, where symbol
/// comparisons are case-folded. A pattern for `Foo` does not match an identifier `foo`.
pub fn match_literal_pattern(
    lang: Language,
    source: &str,
    pattern: &str,
) -> anyhow::Result<Vec<PatternMatch>> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let tree = parse_source(lang, source)?;
    let mut matches = Vec::new();
    walk_literal(tree.root_node(), source, pattern, &mut matches);
    Ok(matches)
}

#[derive(Debug, Clone)]
enum NativeKind {
    /// Function-like declaration; `name == None` means any name (`$NAME`).
    Function { name: Option<String> },
    /// Class/struct/type declaration.
    Class { name: Option<String> },
    /// Free or method call; method path segments may be `$` wildcards.
    Call {
        /// Exact path like `foo.bar` or single name; segments that were `$X` are None.
        path: Vec<Option<String>>,
    },
}

fn classify_native(pattern: &str) -> Option<NativeKind> {
    let p = pattern.trim();
    // fn $NAME / fn $NAME($$$) / fn foo / fn foo($$$)
    for (prefix, is_class) in [
        ("fn ", false),
        ("def ", false),
        ("function ", false),
        ("func ", false),
        ("class ", true),
        ("struct ", true),
        ("interface ", true),
        ("type ", true),
    ] {
        if let Some(rest) = p.strip_prefix(prefix) {
            let head = rest
                .split(|c: char| c == '(' || c == '{' || c == '<' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim();
            if head.is_empty() {
                return None;
            }
            let name = if head.starts_with('$') {
                None
            } else if is_ident(head) {
                Some(head.to_string())
            } else {
                return None;
            };
            return Some(if is_class {
                NativeKind::Class { name }
            } else {
                NativeKind::Function { name }
            });
        }
    }

    // Calls: $F($$$), foo($$$), $O.$M($$$), a.b.$$$c($$$)
    let open = p.find('(')?;
    let close = p.rfind(')')?;
    if close + 1 != p.len() {
        // allow trailing whitespace only
        if p[close + 1..].trim().is_empty() {
            // ok
        } else {
            return None;
        }
    }
    let args = p[open + 1..close].trim();
    // Args must be empty, $$$, or pure metavars / commas — no nested patterns.
    if !args.is_empty()
        && args != "$$$"
        && !args
            .split(',')
            .all(|a| a.trim().is_empty() || a.trim().starts_with('$'))
    {
        return None;
    }
    let callee = p[..open].trim();
    if callee.is_empty() {
        return None;
    }
    let path = parse_call_path(callee)?;
    Some(NativeKind::Call { path })
}

fn parse_call_path(callee: &str) -> Option<Vec<Option<String>>> {
    let mut segs = Vec::new();
    for part in callee.split(['.', ':']).filter(|s| !s.is_empty()) {
        let part = part.trim();
        if part.starts_with('$') {
            segs.push(None);
        } else if is_ident(part) {
            segs.push(Some(part.to_string()));
        } else {
            return None;
        }
    }
    if segs.is_empty() {
        None
    } else {
        Some(segs)
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

fn parse_source(lang: Language, source: &str) -> anyhow::Result<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_language(lang))
        .map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
    parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse source"))
}

fn match_structural(
    lang: Language,
    source: &str,
    kind: &NativeKind,
) -> anyhow::Result<Vec<PatternMatch>> {
    let language = tree_sitter_language(lang);
    let tree = parse_source(lang, source)?;
    let mut out = Vec::new();
    match kind {
        NativeKind::Function { name } => {
            let queries = function_queries(lang);
            run_queries(
                &language,
                tree.root_node(),
                source,
                &queries,
                name.as_deref(),
                &mut out,
            )?;
        }
        NativeKind::Class { name } => {
            let queries = class_queries(lang);
            run_queries(
                &language,
                tree.root_node(),
                source,
                &queries,
                name.as_deref(),
                &mut out,
            )?;
        }
        NativeKind::Call { path } => {
            walk_calls(tree.root_node(), source, path, &mut out);
        }
    }
    Ok(out)
}

fn function_queries(lang: Language) -> Vec<&'static str> {
    match lang {
        Language::Rust => vec![
            "(function_item name: (identifier) @name) @match",
            "(impl_item body: (declaration_list (function_item name: (identifier) @name) @match))",
        ],
        Language::Python => vec!["(function_definition name: (identifier) @name) @match"],
        Language::Go => vec!["(function_declaration name: (identifier) @name) @match"],
        Language::Java | Language::CSharp => vec![
            "(method_declaration name: (identifier) @name) @match",
            "(constructor_declaration name: (identifier) @name) @match",
        ],
        Language::JavaScript | Language::TypeScript => vec![
            "(function_declaration name: (identifier) @name) @match",
            "(method_definition name: (property_identifier) @name) @match",
            "(lexical_declaration (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @match)",
        ],
        Language::Ruby => vec![
            "(method name: (identifier) @name) @match",
            "(singleton_method name: (identifier) @name) @match",
        ],
    }
}

fn class_queries(lang: Language) -> Vec<&'static str> {
    match lang {
        Language::Rust => vec![
            "(struct_item name: (type_identifier) @name) @match",
            "(enum_item name: (type_identifier) @name) @match",
            "(trait_item name: (type_identifier) @name) @match",
        ],
        Language::Python => vec!["(class_definition name: (identifier) @name) @match"],
        Language::Go => vec!["(type_declaration (type_spec name: (type_identifier) @name) @match)"],
        Language::Java | Language::CSharp => vec![
            "(class_declaration name: (identifier) @name) @match",
            "(interface_declaration name: (identifier) @name) @match",
        ],
        Language::JavaScript | Language::TypeScript => {
            vec!["(class_declaration name: (identifier) @name) @match"]
        }
        Language::Ruby => vec!["(class name: (constant) @name) @match"],
    }
}

fn run_queries(
    language: &tree_sitter::Language,
    root: Node,
    source: &str,
    queries: &[&str],
    name_filter: Option<&str>,
    out: &mut Vec<PatternMatch>,
) -> anyhow::Result<()> {
    for qsrc in queries {
        let Ok(query) = Query::new(language, qsrc) else {
            continue;
        };
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let name_idx = query.capture_index_for_name("name");
        let match_idx = query.capture_index_for_name("match");
        while let Some(m) = matches.next() {
            let mut name_text: Option<&str> = None;
            let mut match_node: Option<Node> = None;
            for cap in m.captures {
                if name_idx == Some(cap.index) {
                    name_text = node_text(&cap.node, source);
                }
                if match_idx == Some(cap.index) {
                    match_node = Some(cap.node);
                }
            }
            let node = match_node.or_else(|| m.captures.first().map(|c| c.node));
            let Some(node) = node else {
                continue;
            };
            if is_in_comment_or_string(&node) {
                continue;
            }
            if let Some(want) = name_filter {
                if name_text != Some(want) {
                    continue;
                }
            }
            push_match(&node, source, name_text.unwrap_or(""), out);
        }
    }
    Ok(())
}

fn walk_calls(node: Node, source: &str, path: &[Option<String>], out: &mut Vec<PatternMatch>) {
    if !is_in_comment_or_string(&node) && is_call_kind(node.kind()) {
        if let Some(callee) = call_target_path(&node, source) {
            if path_matches(&callee, path) {
                push_match(&node, source, &callee.join("."), out);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls(child, source, path, out);
    }
}

fn call_target_path(node: &Node, source: &str) -> Option<Vec<String>> {
    let target = ["function", "name"]
        .into_iter()
        .find_map(|f| node.child_by_field_name(f))?;
    path_from_node(&target, source)
}

fn path_from_node(node: &Node, source: &str) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" | "property_identifier" => {
            node_text(node, source).map(|t| vec![t.to_string()])
        }
        "field_expression"
        | "member_expression"
        | "selector_expression"
        | "member_access_expression"
        | "scoped_identifier" => {
            let mut segs = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(mut p) = path_from_node(&child, source) {
                    segs.append(&mut p);
                }
            }
            if segs.is_empty() {
                None
            } else {
                Some(segs)
            }
        }
        _ => last_identifier_chain(node, source).map(|s| vec![s]),
    }
}

fn last_identifier_chain(node: &Node, source: &str) -> Option<String> {
    crate::extract::last_identifier_in_chain(node, source)
}

fn path_matches(actual: &[String], pattern: &[Option<String>]) -> bool {
    if actual.len() != pattern.len() {
        // Allow pattern `$M($$$ )` to match last segment of multi-part calls? No — keep exact length.
        // Exception: single-segment pattern matches last segment of call (method name only).
        if pattern.len() == 1 {
            let want = &pattern[0];
            return actual
                .last()
                .is_some_and(|last| want.as_ref().map(|w| w == last).unwrap_or(true));
        }
        return false;
    }
    actual
        .iter()
        .zip(pattern.iter())
        .all(|(a, p)| p.as_ref().map(|w| w == a).unwrap_or(true))
}

fn walk_literal(node: Node, source: &str, pattern: &str, out: &mut Vec<PatternMatch>) {
    if !is_in_comment_or_string(&node) {
        if identifier_matches(&node, source, pattern) {
            push_match(&node, source, pattern, out);
        }
        if let Some(name_node) = node.child_by_field_name("name") {
            if identifier_matches(&name_node, source, pattern) {
                push_match(&node, source, pattern, out);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_literal(child, source, pattern, out);
    }
}

fn identifier_matches(node: &Node, source: &str, pattern: &str) -> bool {
    is_identifier_kind(node.kind()) && node_text(node, source).is_some_and(|t| t == pattern)
}

fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "package_identifier"
            | "constant"
    )
}

pub(crate) fn collect_pattern_nodes(root: Node, source: &str) -> Vec<PatternNode> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_node_signatures(root, source, &mut out, &mut seen);
    out
}

fn collect_node_signatures(
    node: Node,
    source: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    if !is_in_comment_or_string(&node) {
        if is_identifier_kind(node.kind()) {
            if let Some(text) = node_text(&node, source) {
                push_pattern_node(node, source, text, out, seen);
            }
        }
        if let Some(prefix) = declaration_prefix(node.kind()) {
            push_pattern_node(node, source, &format!("kind:{}", node.kind()), out, seen);
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| node_text(&n, source))
            {
                push_pattern_node(node, source, &format!("{prefix} {name}"), out, seen);
                push_pattern_node(node, source, &format!("decl:{prefix}:{name}"), out, seen);
            }
        }
        if is_call_kind(node.kind()) {
            push_pattern_node(node, source, &format!("kind:{}", node.kind()), out, seen);
            if let Some(callee) = call_target(&node, source) {
                push_pattern_node(node, source, &format!("call:{callee}"), out, seen);
                if let Some(name) = callee.rsplit(['.', ':']).find(|p| !p.is_empty()) {
                    push_pattern_node(node, source, &format!("call-name:{name}"), out, seen);
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_signatures(child, source, out, seen);
    }
}

fn declaration_prefix(kind: &str) -> Option<&'static str> {
    match kind {
        "function_item" => Some("fn"),
        "struct_item" => Some("struct"),
        "function_definition" => Some("def"),
        "function_declaration" | "method_definition" | "method_declaration" | "method" => {
            Some("function")
        }
        "class_definition" | "class_declaration" | "class" => Some("class"),
        "trait_item" | "interface_declaration" => Some("interface"),
        "enum_item" => Some("enum"),
        _ => None,
    }
}

fn is_call_kind(kind: &str) -> bool {
    matches!(kind, "call_expression" | "call" | "method_invocation")
}

fn call_target<'a>(node: &Node<'a>, source: &'a str) -> Option<&'a str> {
    ["function", "name"]
        .into_iter()
        .find_map(|f| node.child_by_field_name(f))
        .and_then(|t| node_text(&t, source))
}

fn push_pattern_node(
    node: Node,
    source: &str,
    signature: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    let (line_start, line_end) = node_lines(&node, source);
    if !seen.insert((signature.to_string(), line_start)) {
        return;
    }
    out.push(PatternNode {
        signature: signature.to_string(),
        line_start,
        line_end,
        excerpt: excerpt_for_node(&node, source, signature),
    });
}

fn push_match(node: &Node, source: &str, pattern: &str, out: &mut Vec<PatternMatch>) {
    let (line_start, line_end) = node_lines(node, source);
    let excerpt = excerpt_for_node(node, source, pattern);
    if out
        .iter()
        .any(|m| m.line_start == line_start && m.excerpt == excerpt)
    {
        return;
    }
    out.push(PatternMatch {
        line_start,
        line_end,
        excerpt,
    });
}

fn excerpt_for_node(node: &Node, source: &str, pattern: &str) -> String {
    if let Some(text) = node_text(node, source) {
        if text.lines().count() <= 6 {
            return text.to_string();
        }
    }
    let line = byte_to_line(source, node.start_byte());
    source
        .lines()
        .nth(line.saturating_sub(1) as usize)
        .unwrap_or(pattern)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_metavariable_shapes() {
        assert!(classify_native("fn $NAME($$$)").is_some());
        assert!(classify_native("def $NAME").is_some());
        assert!(classify_native("$OBJ.$METHOD($$$)").is_some());
        assert!(classify_native("foo($$$)").is_some());
        assert!(classify_native("process_request($$$)").is_some());
        // Nested / exotic → external
        assert!(classify_native("if ($COND) { $BODY }").is_none());
    }

    #[test]
    fn native_fn_meta_matches_rust() {
        let src = "fn process_request(x: i32) {}\nfn other() {}\n";
        let hits = match_pattern(Language::Rust, src, "fn $NAME($$$)").unwrap();
        assert!(hits.len() >= 2, "hits={hits:?}");
    }

    #[test]
    fn native_call_matches_exact_callee() {
        let src = "fn main() { process_request(1); other(2); }\n";
        let hits = match_pattern(Language::Rust, src, "process_request($$$)").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].excerpt.contains("process_request"));
    }
}
