use tree_sitter::{Node, Parser}; use crate::extract::{byte_to_line, is_in_comment_or_string, node_lines, node_text}; use crate::{Language, PatternNode}; #[derive(Debug, Clone, PartialEq, Eq)] pub struct PatternMatch { pub line_start: u32, pub line_end: u32, pub excerpt: String, } pub fn needs_ast_grep_fallback(pattern: &str) -> bool { pattern.contains('$') } pub fn tree_sitter_language(lang: Language) -> tree_sitter::Language {
    match lang {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(), Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), Language::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(), Language::Go => tree_sitter_go::LANGUAGE.into(), Language::Java | Language::CSharp => tree_sitter_java::LANGUAGE.into(), Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
    }
}
/// Matches identifier text exactly, including case.
///
/// This syntax-level policy intentionally differs from relevance ranking, where symbol
/// comparisons are case-folded. A pattern
/// for `Foo` does not match an identifier `foo`.
pub fn match_literal_pattern( lang: Language, source: &str, pattern: &str, ) -> anyhow::Result<Vec<PatternMatch>> {
    if pattern.is_empty() { return Ok(Vec::new()); } let mut parser = Parser::new(); parser
        .set_language(&tree_sitter_language(lang)) .map_err(|e| anyhow::anyhow!("failed to set language: {e}"))?;
    let tree = parser
        .parse(source, None) .ok_or_else(|| anyhow::anyhow!("failed to parse source"))?;
    let mut matches = Vec::new(); walk_pattern_nodes(tree.root_node(), source, pattern, &mut matches); Ok(matches)
} fn walk_pattern_nodes(node: Node, source: &str, pattern: &str, out: &mut Vec<PatternMatch>) {
    if !is_in_comment_or_string(&node) {
        if identifier_matches(&node, source, pattern) { push_match(&node, source, pattern, out); } if let Some(name_node) = node.child_by_field_name("name") { if identifier_matches(&name_node, source, pattern) { push_match(&node, source, pattern, out); } }
    } let mut cursor = node.walk(); for child in node.children(&mut cursor) { walk_pattern_nodes(child, source, pattern, out); }
} fn identifier_matches(node: &Node, source: &str, pattern: &str) -> bool { is_identifier_kind(node.kind()) && node_text(node, source).is_some_and(|t| t == pattern) } fn is_identifier_kind(kind: &str) -> bool {
    matches!( kind, "identifier"
            | "type_identifier" | "field_identifier" | "property_identifier" | "package_identifier"
    )
} pub(crate) fn collect_pattern_nodes(root: Node, source: &str) -> Vec<PatternNode> { let mut out = Vec::new(); let mut seen = std::collections::HashSet::new(); collect_node_signatures(root, source, &mut out, &mut seen); out } fn collect_node_signatures(
    node: Node, source: &str, out: &mut Vec<PatternNode>, seen: &mut std::collections::HashSet<(String, u32)>, ) {
    if !is_in_comment_or_string(&node) {
        if is_identifier_kind(node.kind()) { if let Some(text) = node_text(&node, source) { push_pattern_node(node, source, text, out, seen); } } if let Some(prefix) = declaration_prefix(node.kind()) {
            push_pattern_node(node, source, &format!("kind:{}", node.kind()), out, seen); if let Some(name) = node
                .child_by_field_name("name") .and_then(|n| node_text(&n, source))
            {
                push_pattern_node(node, source, &format!("{prefix} {name}"), out, seen); push_pattern_node(node, source, &format!("decl:{prefix}:{name}"), out, seen);
            }
        } if is_call_kind(node.kind()) {
            push_pattern_node(node, source, &format!("kind:{}", node.kind()), out, seen); if let Some(callee) = call_target(&node, source) {
                push_pattern_node(node, source, &format!("call:{callee}"), out, seen); if let Some(name) = callee.rsplit(['.', ':']).find(|p| !p.is_empty()) { push_pattern_node(node, source, &format!("call-name:{name}"), out, seen); }
            }
        }
    } let mut cursor = node.walk(); for child in node.children(&mut cursor) { collect_node_signatures(child, source, out, seen); }
} fn declaration_prefix(kind: &str) -> Option<&'static str> {
    match kind {
        "function_item" => Some("fn"), "struct_item" => Some("struct"), "function_definition" => Some("def"),
        "function_declaration" | "method_definition" | "method_declaration" => Some("function"), "class_definition" | "class_declaration" => Some("class"), _ => None,
    }
} fn is_call_kind(kind: &str) -> bool { matches!(kind, "call_expression" | "call" | "method_invocation") } fn call_target<'a>(node: &Node<'a>, source: &'a str) -> Option<&'a str> {
    ["function", "name"]
        .into_iter() .find_map(|f| node.child_by_field_name(f)) .and_then(|t| node_text(&t, source))
} fn push_pattern_node( node: Node, source: &str, signature: &str, out: &mut Vec<PatternNode>, seen: &mut std::collections::HashSet<(String, u32)>, ) {
    let (line_start, line_end) = node_lines(&node, source); if !seen.insert((signature.to_string(), line_start)) { return; } out.push(PatternNode { signature: signature.to_string(), line_start, line_end, excerpt: excerpt_for_node(&node, source, signature), });
} fn push_match(node: &Node, source: &str, pattern: &str, out: &mut Vec<PatternMatch>) {
    let (line_start, line_end) = node_lines(node, source); let excerpt = excerpt_for_node(node, source, pattern); if out
        .iter() .any(|m| m.line_start == line_start && m.excerpt == excerpt)
    {
        return;
    } out.push(PatternMatch { line_start, line_end, excerpt, });
} fn excerpt_for_node(node: &Node, source: &str, pattern: &str) -> String {
    if let Some(text) = node_text(node, source) { if text.lines().count() <= 6 { return text.to_string(); } } let line = byte_to_line(source, node.start_byte()); source
        .lines() .nth(line.saturating_sub(1) as usize) .unwrap_or(pattern) .to_string()
}
