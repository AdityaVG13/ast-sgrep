#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::Path;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    Swift,
    C,
    Cpp,
    Kotlin,
    Php,
}
impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Java => "java",
            Language::CSharp => "csharp",
            Language::Ruby => "ruby",
            Language::Swift => "swift",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Kotlin => "kotlin",
            Language::Php => "php",
        }
    }
    pub fn all() -> &'static [Language] {
        &[
            Language::Rust,
            Language::TypeScript,
            Language::JavaScript,
            Language::Python,
            Language::Go,
            Language::Java,
            Language::CSharp,
            Language::Ruby,
            Language::Swift,
            Language::C,
            Language::Cpp,
            Language::Kotlin,
            Language::Php,
        ]
    }

    /// Indexed source extensions and the language they store as.
    ///
    /// Shared by [`Language::parse`] / `--lang` filters and [`detect_language`]
    /// so an alias like `h` or `hpp` cannot drift from on-disk ids.
    pub const SOURCE_EXTENSIONS: &[(&str, Language)] = &[
        ("rs", Language::Rust),
        ("ts", Language::TypeScript),
        ("tsx", Language::TypeScript),
        ("js", Language::JavaScript),
        ("jsx", Language::JavaScript),
        ("mjs", Language::JavaScript),
        ("cjs", Language::JavaScript),
        ("py", Language::Python),
        ("pyi", Language::Python),
        ("go", Language::Go),
        ("java", Language::Java),
        ("cs", Language::CSharp),
        ("rb", Language::Ruby),
        ("swift", Language::Swift),
        ("c", Language::C),
        ("h", Language::C),
        ("cpp", Language::Cpp),
        ("cc", Language::Cpp),
        ("cxx", Language::Cpp),
        ("hpp", Language::Cpp),
        ("hxx", Language::Cpp),
        ("hh", Language::Cpp),
        ("ipp", Language::Cpp),
        ("kt", Language::Kotlin),
        ("kts", Language::Kotlin),
        ("php", Language::Php),
    ];

    /// Language for a file extension (`ts`, `hpp`, `pyi`, …). Case-insensitive.
    pub fn from_extension(ext: &str) -> Option<Language> {
        let lower = ext.trim().to_ascii_lowercase();
        Self::SOURCE_EXTENSIONS
            .iter()
            .find(|(candidate, _)| *candidate == lower)
            .map(|(_, lang)| *lang)
    }

    /// Parse a language id into a `Language`, accepting `Language::as_str` forms,
    /// indexed file extensions, and common name aliases (including Title Case).
    pub fn parse(raw: &str) -> Option<Language> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(lang) = Self::from_extension(&lower) {
            return Some(lang);
        }
        match lower.as_str() {
            "rust" => Some(Language::Rust),
            "typescript" => Some(Language::TypeScript),
            "javascript" => Some(Language::JavaScript),
            "python" => Some(Language::Python),
            "golang" => Some(Language::Go),
            "csharp" | "c#" | "c-sharp" => Some(Language::CSharp),
            "ruby" => Some(Language::Ruby),
            "c++" => Some(Language::Cpp),
            "kotlin" => Some(Language::Kotlin),
            _ => None,
        }
    }
    /// Normalize an external language label to `Language::as_str` casing.
    /// Unknown labels are lowercased so case-sensitive filters stay consistent.
    pub fn normalize_id(raw: &str) -> String {
        Self::parse(raw)
            .map(|lang| lang.as_str().to_string())
            .unwrap_or_else(|| raw.trim().to_ascii_lowercase())
    }

    /// Canonical language id for index storage and SQL filters.
    ///
    /// Known aliases (`ts`, `hpp`, `py`, `rs`, `h`, …) map to [`Language::as_str`].
    /// Blank input is no filter. Unknown labels are lowercased.
    pub fn canonical_filter(raw: Option<&str>) -> Option<String> {
        let trimmed = raw?.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self::normalize_id(trimmed))
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SymbolDef {
    pub name: String,
    pub kind: SymbolKind,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Type,
    Interface,
    Enum,
    Doc,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CallSite {
    pub caller: String,
    pub callee: String,
    pub line: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportSite {
    pub module_path: String,
    pub line: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PatternNode {
    pub signature: String,
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExtractionResult {
    pub symbols: Vec<SymbolDef>,
    pub calls: Vec<CallSite>,
    pub imports: Vec<ImportSite>,
    pub pattern_nodes: Vec<PatternNode>,
}
pub fn detect_language(path: &Path, content: Option<&str>) -> Option<Language> {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if let Some(lang) = Language::from_extension(ext) {
            return Some(lang);
        }
    }
    let trimmed = content?.trim_start();
    if trimmed.starts_with("package ") {
        return Some(Language::Go);
    }
    if trimmed.starts_with("#!/usr/bin/env ruby") || trimmed.starts_with("#!/usr/bin/ruby") {
        return Some(Language::Ruby);
    }
    if trimmed.starts_with("#!/usr/bin/env python") || trimmed.starts_with("#!/usr/bin/python") {
        return Some(Language::Python);
    }
    if trimmed.starts_with("#!/usr/bin/env php") || trimmed.starts_with("<?php") {
        return Some(Language::Php);
    }
    None
}
pub trait LanguageParser: Send + Sync {
    fn language(&self) -> Language;
    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult>;
}
pub struct ParserRegistry {
    parsers: HashMap<Language, Box<dyn LanguageParser>>,
}
impl ParserRegistry {
    pub fn new() -> Self {
        Self {
            parsers: Language::all()
                .iter()
                .map(|&lang| (lang, make_parser(lang)))
                .collect(),
        }
    }
    pub fn parse(&self, language: Language, source: &str) -> anyhow::Result<ExtractionResult> {
        self.parsers
            .get(&language)
            .ok_or_else(|| anyhow::anyhow!("no parser registered for language {language}"))?
            .parse(source)
    }
}
impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}
mod extract;
mod langs;
mod pattern;
mod pattern_queries;
mod signature;
use langs::{
    CParser, CSharpParser, CppParser, GoParser, JavaParser, JavaScriptParser, KotlinParser,
    PhpParser, PythonParser, RubyParser, RustParser, SwiftParser, TypeScriptParser,
};
pub use pattern::{
    classify_native, declaration_prefix, match_literal_pattern, match_pattern,
    needs_ast_grep_fallback, tree_sitter_language, BodyTemplate, NativeKind, PatternMatch,
    DECL_KIND_PREFIXES, DECL_PATTERN_PREFIXES,
};
pub use signature::{
    cached_pattern_signatures, required_pattern_literal, structural_term_signatures, DECL_PREFIXES,
};
fn make_parser(lang: Language) -> Box<dyn LanguageParser> {
    match lang {
        Language::Rust => Box::new(RustParser),
        Language::TypeScript => Box::new(TypeScriptParser),
        Language::JavaScript => Box::new(JavaScriptParser),
        Language::Python => Box::new(PythonParser),
        Language::Go => Box::new(GoParser),
        Language::Java => Box::new(JavaParser),
        Language::CSharp => Box::new(CSharpParser),
        Language::Ruby => Box::new(RubyParser),
        Language::Swift => Box::new(SwiftParser),
        Language::C => Box::new(CParser),
        Language::Cpp => Box::new(CppParser),
        Language::Kotlin => Box::new(KotlinParser),
        Language::Php => Box::new(PhpParser),
    }
}

#[cfg(test)]
mod canonical_filter_tests {
    use super::{detect_language, Language};
    use std::path::Path;

    const NAME_ALIASES: &[(&str, &str)] = &[
        ("rust", "rust"),
        ("typescript", "typescript"),
        ("javascript", "javascript"),
        ("python", "python"),
        ("golang", "go"),
        ("csharp", "csharp"),
        ("c#", "csharp"),
        ("c-sharp", "csharp"),
        ("ruby", "ruby"),
        ("c++", "cpp"),
        ("kotlin", "kotlin"),
        ("TypeScript", "typescript"),
    ];

    #[test]
    fn every_source_extension_canonicalizes_and_detects() {
        for (ext, lang) in Language::SOURCE_EXTENSIONS {
            assert_eq!(
                Language::canonical_filter(Some(ext)).as_deref(),
                Some(lang.as_str()),
                "extension {ext}"
            );
            let rel = format!("n.{ext}");
            let path = Path::new(&rel);
            assert_eq!(
                detect_language(path, None),
                Some(*lang),
                "detect_language({ext})"
            );
            assert_eq!(Language::from_extension(ext), Some(*lang));
            assert_eq!(
                Language::from_extension(&ext.to_ascii_uppercase()),
                Some(*lang)
            );
        }
    }

    #[test]
    fn stored_ids_and_name_aliases_parse() {
        for lang in Language::all() {
            assert_eq!(Language::parse(lang.as_str()), Some(*lang));
        }
        for (raw, stored) in NAME_ALIASES {
            assert_eq!(
                Language::canonical_filter(Some(raw)).as_deref(),
                Some(*stored),
                "alias {raw}"
            );
        }
    }

    #[test]
    fn aliases_map_to_stored_ids() {
        assert_eq!(
            Language::canonical_filter(Some("ts")).as_deref(),
            Some("typescript")
        );
        assert_eq!(
            Language::canonical_filter(Some("hpp")).as_deref(),
            Some("cpp")
        );
        assert_eq!(Language::canonical_filter(Some("h")).as_deref(), Some("c"));
        assert_eq!(
            Language::canonical_filter(Some("c#")).as_deref(),
            Some("csharp")
        );
    }

    #[test]
    fn blank_is_no_filter() {
        assert_eq!(Language::canonical_filter(None), None);
        assert_eq!(Language::canonical_filter(Some("")), None);
        assert_eq!(Language::canonical_filter(Some("  ")), None);
    }

    #[test]
    fn unknown_labels_lowercase() {
        assert_eq!(
            Language::canonical_filter(Some("Fortran")).as_deref(),
            Some("fortran")
        );
    }
}
