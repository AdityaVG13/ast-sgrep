//! Structural and literal pattern matching over tree-sitter ASTs.
//!
//! **Why this exists (vs shelling out to ast-grep):**
//! - Indexed hybrid search needs a fast, in-process structural channel.
//! - External `ast-grep` is excellent for full metavariable rules, but process
//!   spawn + JSON parse is too heavy for tight loops and offline agents.
//! - We implement the common ~80% of patterns natively (function/method/class
//!   decls and calls with `$NAME` / `$$$` holes). Complex rules still fall
//!   through to external ast-grep when installed.

use crate::extract::{
    byte_to_line, is_ident_kind, is_in_comment_or_string, is_member_expr_kind,
    last_identifier_in_chain, node_lines, node_text,
};
use crate::pattern_queries::{class_queries_for, queries_for, FUNCTION_QUERY_TABLE};
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

/// True when the pattern needs external ast-grep (we cannot handle it natively).
///
/// Patterns without `$` always run in-process. Patterns with `$`/`$$$` use the
/// native structural matcher when they fit a known shape; only exotic shapes
/// still require the external binary. A `$`-pattern with no structural syntax
/// (no `(`/`{`/`;`/`=`/`:` — e.g. bare `$$$word<<<` garbage) is not a real rule
/// and never needs the fallback: native match-none is honest for it.
pub fn needs_ast_grep_fallback(pattern: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() || !p.contains('$') {
        return false;
    }
    let has_structure = p
        .chars()
        .any(|c| matches!(c, '(' | '{' | ';' | '=' | ':' | '['));
    if !has_structure {
        return false;
    }
    classify_native(p).is_none()
}

pub fn tree_sitter_language(lang: Language) -> tree_sitter::Language {
    match lang {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        // e2hc/difu.5: C# patterns were parsed with the Java grammar, causing
        // misparses of C#-specific syntax. Use the real C# grammar so the
        // pattern channel agrees with the extraction channel.
        Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Language::Swift => tree_sitter_swift::LANGUAGE.into(),
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
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

/// Native structural pattern shapes handled in-process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeKind {
    /// Function-like declaration; `name == None` means any name (`$NAME`).
    Function { name: Option<String> },
    /// Class/struct/type declaration. `keyword` is the pattern prefix
    /// (`class`, `struct`, `interface`, or `type`) so queries stay kind-specific.
    Class {
        keyword: &'static str,
        name: Option<String>,
    },
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
                NativeKind::Class {
                    keyword: prefix.trim(),
                    name,
                }
            } else {
                NativeKind::Function { name }
            });
        }
    }

    // Calls: $F($$$), foo($$$), $O.$M($$$), a.b.$$$c($$$)
    let open = p.find('(')?;
    let close = p.rfind(')')?;
    if close <= open {
        return None;
    }
    // Allow trailing whitespace only after the closing paren.
    if close + 1 != p.len() && !p[close + 1..].trim().is_empty() {
        return None;
    }
    let args = p[open + 1..close].trim();
    // Args must be empty, $$$, or pure metavars separated by commas.
    if !args.is_empty() && args != "$$$" && !args.split(',').all(is_pure_metavariable) {
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

fn is_pure_metavariable(arg: &str) -> bool {
    let arg = arg.trim();
    arg.strip_prefix("$$$")
        .or_else(|| arg.strip_prefix('$'))
        .is_some_and(is_pattern_ident)
}

fn parse_call_path(callee: &str) -> Option<Vec<Option<String>>> {
    let callee = callee.strip_prefix("::").unwrap_or(callee);
    let normalized = callee.replace("::", ".");
    if normalized.is_empty()
        || normalized.starts_with('.')
        || normalized.ends_with('.')
        || normalized.contains("..")
    {
        return None;
    }
    let mut segments = Vec::new();
    for part in normalized.split('.') {
        let part = part.trim();
        if is_pure_metavariable(part) {
            segments.push(None);
        } else if is_pattern_ident(part) {
            segments.push(Some(part.to_string()));
        } else {
            return None;
        }
    }
    (!segments.is_empty()).then_some(segments)
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
            run_queries(
                &language,
                tree.root_node(),
                source,
                queries_for(FUNCTION_QUERY_TABLE, lang),
                name.as_deref(),
                None,
                &mut out,
            )?;
        }
        NativeKind::Class { keyword, name } => {
            run_queries(
                &language,
                tree.root_node(),
                source,
                class_queries_for(lang, keyword),
                name.as_deref(),
                Some((lang, *keyword)),
                &mut out,
            )?;
        }
        NativeKind::Call { path } => {
            walk_calls(tree.root_node(), source, path, &mut out);
        }
    }
    Ok(out)
}

fn run_queries(
    language: &tree_sitter::Language,
    root: Node,
    source: &str,
    queries: &[&str],
    name_filter: Option<&str>,
    class_filter: Option<(Language, &str)>,
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
            if let Some((lang, keyword)) = class_filter {
                if !class_keyword_matches(lang, &node, source, keyword) {
                    continue;
                }
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

fn class_keyword_matches(lang: Language, node: &Node, source: &str, keyword: &str) -> bool {
    if lang != Language::Kotlin {
        return true;
    }
    let kind = kotlin_class_keyword(node, source);
    match keyword {
        "class" => kind == "class",
        "interface" => kind == "interface",
        "type" => true,
        _ => false,
    }
}

fn kotlin_class_keyword(node: &Node, source: &str) -> &'static str {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "modifiers" | "class_modifier")
            && kotlin_class_keyword(&child, source) == "enum"
        {
            return "enum";
        }
        if let Some(text) = node_text(&child, source) {
            match text.trim() {
                "enum" => return "enum",
                "interface" => return "interface",
                _ => {}
            }
        }
    }
    "class"
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
    let callee = call_target_path(node, source)?;
    path_matches(&callee, path).then_some(callee)
}

fn call_field_node<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    ["function", "name"]
        .into_iter()
        .find_map(|f| node.child_by_field_name(f))
        // Swift/Kotlin call_expression has no function/name fields; callee is the
        // first named child. Safe for other langs: Ruby uses `call`, C# uses
        // `invocation_expression`, and field-bearing grammars hit find_map first.
        .or_else(|| {
            (node.kind() == "call_expression")
                .then(|| node.named_child(0))
                .flatten()
        })
}

fn call_target_path(node: &Node, source: &str) -> Option<Vec<String>> {
    path_from_node(&call_field_node(node)?, source)
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
        return actual
            .iter()
            .zip(pattern.iter())
            .all(|(a, p)| segment_ok(a, p));
    }
    // Exact length only — except a single-segment pattern matches the last call segment.
    pattern.len() == 1
        && actual
            .last()
            .is_some_and(|last| segment_ok(last, &pattern[0]))
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
    if let Some(prefix) = declaration_prefix(node, source) {
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

/// Map a tree-sitter declaration node to its indexed `decl:` / display prefix.
///
/// Most kinds are table-driven; `class_declaration` inspects Swift
/// `declaration_kind` / Kotlin keyword tokens so singleton forms stay exact.
pub fn declaration_prefix(node: &Node, source: &str) -> Option<&'static str> {
    let kind = node.kind();
    if kind == "class_declaration" {
        return class_declaration_prefix(node, source);
    }
    DECL_KIND_PREFIXES
        .iter()
        .find_map(|&(node_kind, prefix)| (node_kind == kind).then_some(prefix))
}

fn class_declaration_prefix(node: &Node, source: &str) -> Option<&'static str> {
    match node
        .child_by_field_name("declaration_kind")
        .and_then(|kind| node_text(&kind, source))
    {
        Some("struct" | "actor") => Some("struct"),
        Some("enum") => Some("enum"),
        Some("extension") => Some("type"),
        // No Swift declaration_kind (or unrecognised) — Kotlin reuses class_declaration.
        _ => match kotlin_class_keyword(node, source) {
            "interface" => Some("interface"),
            "enum" => Some("enum"),
            _ => Some("class"),
        },
    }
}

/// AST node kind → short declaration prefix used in `decl:{prefix}:{name}` signatures.
pub const DECL_KIND_PREFIXES: &[(&str, &str)] = &[
    ("function_item", "fn"),
    ("struct_item", "struct"),
    ("struct_declaration", "struct"),
    ("struct_specifier", "struct"),
    ("function_definition", "def"),
    ("function_declaration", "function"),
    ("protocol_function_declaration", "function"),
    ("method_definition", "function"),
    ("method_declaration", "function"),
    ("method", "function"),
    ("singleton_method", "function"),
    ("local_function_statement", "function"),
    ("class_definition", "class"),
    ("class", "class"),
    ("record_declaration", "class"),
    ("class_specifier", "class"),
    ("trait_item", "interface"),
    ("interface_declaration", "interface"),
    ("protocol_declaration", "interface"),
    ("enum_item", "enum"),
    ("enum_declaration", "enum"),
    ("enum_specifier", "enum"),
];

// e2hc/difu.5: invocation_expression is the C# tree-sitter grammar's call node.
const CALL_KINDS: &[&str] = &[
    "call_expression",
    "call",
    "method_invocation",
    "invocation_expression",
    "function_call_expression",
    "member_call_expression",
    "scoped_call_expression",
];

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
#[path = "../../../tests/unit/lang/pattern.rs"]
mod tests;
