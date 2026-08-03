//! Structural and literal pattern matching over tree-sitter ASTs.
//!
//! **Why this exists (vs shelling out to ast-grep):**
//! - Indexed hybrid search needs a fast, in-process structural channel.
//! - Process spawn + JSON parsing is too heavy for tight loops and offline agents.
//! - Production search implements declarations, calls, kind predicates, and exact
//!   indexed signatures natively. Unsupported exotic rule syntax returns no hits.

use crate::extract::{
    byte_to_line, is_ident_kind, is_in_comment_or_string, is_member_expr_kind,
    last_identifier_in_chain, node_lines, node_text,
};
use crate::pattern_queries::{queries_for, CLASS_QUERY_TABLE, FUNCTION_QUERY_TABLE};
use crate::{Language, PatternNode};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
}

/// Declaration / type keyword prefixes used by native classification and prefilters.
///
/// `true` means class-like (`Class`); `false` means function-like (`Function`).
pub const DECL_PATTERN_PREFIXES: &[(&str, bool)] = &[
    ("fn ", false),
    ("def ", false),
    ("function ", false),
    ("func ", false),
    ("class ", true),
    ("struct ", true),
    ("interface ", true),
    ("type ", true),
];

/// True when syntax is outside the native structural subset.
///
/// Production callers use this for capability reporting only; they never spawn
/// an external matcher.
pub fn needs_ast_grep_fallback(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() || !p.contains('$') {
        return false;
    }
    // Native shapes we handle: fn/def/function/class $NAME, calls, member calls.
    classify_native(p).is_none()
}

pub(crate) fn tree_sitter_language(lang: Language) -> tree_sitter::Language {
    // Re-exports each grammar's LANGUAGE constant via `.into()`. CSharp currently
    // shares the Java grammar as a stand-in (documented limitation; see l115).
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
        None => Ok(Vec::new()),
    }
}

/// Matches identifier text exactly, including case.
///
/// This syntax-level policy intentionally differs from relevance ranking, where symbol
/// comparisons are case-folded. A pattern for `Foo` does not match an identifier `foo`.
pub(crate) fn match_literal_pattern(
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

/// Native structural pattern shapes handled in-process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeKind {
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

/// Classify a metavariable / structural pattern into the native subset.
pub fn classify_native(pattern: &str) -> Option<NativeKind> {
    let p = pattern.trim();
    for &(prefix, is_class) in DECL_PATTERN_PREFIXES {
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
            } else if is_pattern_ident(head) {
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
    // Allow trailing whitespace only after the closing paren.
    if close + 1 != p.len() && !p[close + 1..].trim().is_empty() {
        return None;
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

/// Identifier token check shared with index signature builders.
#[inline]
pub(crate) fn is_pattern_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

fn parse_call_path(callee: &str) -> Option<Vec<Option<String>>> {
    let mut segs = Vec::new();
    for part in callee.split(['.', ':']).filter(|s| !s.is_empty()) {
        let part = part.trim();
        if part.starts_with('$') {
            segs.push(None);
        } else if is_pattern_ident(part) {
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
    let (queries, name) = match kind {
        NativeKind::Function { name } => (queries_for(FUNCTION_QUERY_TABLE, lang), name.as_deref()),
        NativeKind::Class { name } => (queries_for(CLASS_QUERY_TABLE, lang), name.as_deref()),
        NativeKind::Call { path } => {
            walk_calls(tree.root_node(), source, path, &mut out);
            return Ok(out);
        }
    };
    run_queries(
        &language,
        tree.root_node(),
        source,
        queries,
        name,
        &mut out,
    )?;
    Ok(out)
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
    if let Some(callee) = call_match_path(&node, source, path) {
        push_match(&node, source, &callee.join("."), out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls(child, source, path, out);
    }
}

/// When `node` is a call outside trivia that matches `path`, return its callee segments.
fn call_match_path(node: &Node, source: &str, path: &[Option<String>]) -> Option<Vec<String>> {
    if is_in_comment_or_string(node) || !is_call_kind(node.kind()) {
        return None;
    }
    let callee = path_from_node(&call_field_node(node)?, source)?;
    path_matches(&callee, path).then_some(callee)
}

fn call_field_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    ["function", "name"]
        .into_iter()
        .find_map(|f| node.child_by_field_name(f))
}

fn path_from_node(node: &Node, source: &str) -> Option<Vec<String>> {
    if is_ident_kind(node.kind()) {
        return node_text(node, source).map(|t| vec![t.to_string()]);
    }
    if !is_member_expr_kind(node.kind()) {
        return last_identifier_in_chain(node, source).map(|s| vec![s]);
    }
    let mut segs = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(mut p) = path_from_node(&child, source) {
            segs.append(&mut p);
        }
    }
    (!segs.is_empty()).then_some(segs)
}

fn path_matches(actual: &[String], pattern: &[Option<String>]) -> bool {
    let segment_ok = |a: &String, p: &Option<String>| p.as_ref().is_none_or(|w| w == a);
    if actual.len() == pattern.len() {
        return actual.iter().zip(pattern.iter()).all(|(a, p)| segment_ok(a, p));
    }
    // Exact length only — except a single-segment pattern matches the last call segment.
    if pattern.len() != 1 {
        return false;
    }
    actual.last().is_some_and(|last| segment_ok(last, &pattern[0]))
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
    is_ident_kind(node.kind()) && node_text(node, source).is_some_and(|t| t == pattern)
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
        record_node_signatures(&node, source, out, seen);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_signatures(child, source, out, seen);
    }
}

fn record_node_signatures(
    node: &Node,
    source: &str,
    out: &mut Vec<PatternNode>,
    seen: &mut std::collections::HashSet<(String, u32)>,
) {
    if is_ident_kind(node.kind()) {
        if let Some(text) = node_text(node, source) {
            push_pattern_node(*node, source, text, out, seen);
        }
    }
    if let Some(prefix) = declaration_prefix(node.kind()) {
        push_pattern_node(*node, source, &format!("kind:{}", node.kind()), out, seen);
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|n| node_text(&n, source))
        {
            push_pattern_node(*node, source, &format!("{prefix} {name}"), out, seen);
            push_pattern_node(*node, source, &format!("decl:{prefix}:{name}"), out, seen);
        }
    }
    if !is_call_kind(node.kind()) {
        return;
    }
    push_pattern_node(*node, source, &format!("kind:{}", node.kind()), out, seen);
    let Some(callee) = call_target(node, source) else {
        return;
    };
    push_pattern_node(*node, source, &format!("call:{callee}"), out, seen);
    if let Some(name) = callee.rsplit(['.', ':']).find(|p| !p.is_empty()) {
        push_pattern_node(*node, source, &format!("call-name:{name}"), out, seen);
    }
}

/// Map a tree-sitter declaration kind to its indexed `decl:` / display prefix.
pub(crate) fn declaration_prefix(kind: &str) -> Option<&'static str> {
    DECL_KIND_PREFIXES
        .iter()
        .find_map(|&(node_kind, prefix)| (node_kind == kind).then_some(prefix))
}

/// AST node kind → short declaration prefix used in `decl:{prefix}:{name}` signatures.
pub const DECL_KIND_PREFIXES: &[(&str, &str)] = &[
    ("function_item", "fn"),
    ("struct_item", "struct"),
    ("function_definition", "def"),
    ("function_declaration", "function"),
    ("method_definition", "function"),
    ("method_declaration", "function"),
    ("method", "function"),
    ("class_definition", "class"),
    ("class_declaration", "class"),
    ("class", "class"),
    ("trait_item", "interface"),
    ("interface_declaration", "interface"),
    ("enum_item", "enum"),
];

const CALL_KINDS: &[&str] = &["call_expression", "call", "method_invocation"];

fn is_call_kind(kind: &str) -> bool {
    CALL_KINDS.contains(&kind)
}

fn call_target<'a>(node: &Node<'a>, source: &'a str) -> Option<&'a str> {
    call_field_node(node).and_then(|t| node_text(&t, source))
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
