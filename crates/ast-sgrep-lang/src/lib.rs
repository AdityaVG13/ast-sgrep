use std::collections::HashMap; use std::path::Path; #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)] #[serde(rename_all = "lowercase")] pub enum Language {
    Rust, TypeScript, JavaScript, Python, Go, Java, CSharp, Ruby,
} impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Language::Rust => "rust", Language::TypeScript => "typescript", Language::JavaScript => "javascript",
            Language::Python => "python", Language::Go => "go", Language::Java => "java", Language::CSharp => "csharp", Language::Ruby => "ruby",
        }
    }

    pub fn all() -> &'static [Language] {
        &[
            Language::Rust, Language::TypeScript, Language::JavaScript, Language::Python, Language::Go, Language::Java, Language::CSharp, Language::Ruby,
        ]
    }
} impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
} #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)] pub struct SymbolDef {
    pub name: String, pub kind: SymbolKind, pub line_start: u32, pub line_end: u32, pub byte_start: usize, pub byte_end: usize,
} #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)] #[serde(rename_all = "lowercase")] pub enum SymbolKind {
    Function, Method, Class, Type, Interface, Enum, Doc,
} #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)] pub struct CallSite {
    pub caller: String, pub callee: String, pub line: u32, pub byte_start: usize, pub byte_end: usize,
} #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)] pub struct ImportSite {
    pub module_path: String, pub line: u32,
} #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)] pub struct PatternNode {
    pub signature: String, pub line_start: u32, pub line_end: u32, pub excerpt: String,
} #[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)] pub struct ExtractionResult {
    pub symbols: Vec<SymbolDef>, pub calls: Vec<CallSite>, pub imports: Vec<ImportSite>, pub pattern_nodes: Vec<PatternNode>,
} pub fn detect_language(path: &Path, content: Option<&str>) -> Option<Language> {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lang = match ext.to_lowercase().as_str() {
            "rs" => Some(Language::Rust), "ts" | "tsx" => Some(Language::TypeScript), "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript), "py" | "pyi" => Some(Language::Python),
            "go" => Some(Language::Go), "java" => Some(Language::Java), "cs" => Some(Language::CSharp), "rb" => Some(Language::Ruby), _ => None,
        }; if lang.is_some() { return lang; }
    } let trimmed = content?.trim_start(); if trimmed.starts_with("package ") { return Some(Language::Go); } if trimmed.starts_with("#!/usr/bin/env ruby") || trimmed.starts_with("#!/usr/bin/ruby") { return Some(Language::Ruby); }
    if trimmed.starts_with("#!/usr/bin/env python") || trimmed.starts_with("#!/usr/bin/python") { return Some(Language::Python); } None
} pub trait LanguageParser: Send + Sync {
    fn language(&self) -> Language; fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult>;
} pub struct ParserRegistry {
    parsers: HashMap<Language, Box<dyn LanguageParser>>,
} impl ParserRegistry {
    pub fn new() -> Self {
        Self {
            parsers: Language::all()
                .iter() .map(|&lang| (lang, make_parser(lang))) .collect(),
        }
    }

    pub fn parse(&self, language: Language, source: &str) -> anyhow::Result<ExtractionResult> {
        self.parsers
            .get(&language) .ok_or_else(|| anyhow::anyhow!("no parser registered for language {language}"))? .parse(source)
    }
} impl Default for ParserRegistry {
    fn default() -> Self { Self::new() }
} mod extract; mod langs; mod pattern; pub use pattern::{
    match_literal_pattern, needs_ast_grep_fallback, tree_sitter_language, PatternMatch,
}; use langs::{
    CSharpParser, GoParser, JavaParser, JavaScriptParser, PythonParser, RubyParser, RustParser, TypeScriptParser,
}; fn make_parser(lang: Language) -> Box<dyn LanguageParser> {
    match lang {
        Language::Rust => Box::new(RustParser), Language::TypeScript => Box::new(TypeScriptParser), Language::JavaScript => Box::new(JavaScriptParser), Language::Python => Box::new(PythonParser), Language::Go => Box::new(GoParser),
        Language::Java => Box::new(JavaParser), Language::CSharp => Box::new(CSharpParser), Language::Ruby => Box::new(RubyParser),
    }
}
