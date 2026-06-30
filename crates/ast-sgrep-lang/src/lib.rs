use std::collections::HashMap;
use std::path::Path;

/// Supported programming languages for AST extraction.
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
        ]
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A symbol definition extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SymbolDef {
    pub name: String,
    pub kind: SymbolKind,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

/// Kind of symbol definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
}

/// A function/method call extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CallSite {
    pub caller: String,
    pub callee: String,
    pub line: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

/// An import/use statement extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportSite {
    pub module_path: String,
    pub line: u32,
}

/// Full extraction result for a single file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExtractionResult {
    pub symbols: Vec<SymbolDef>,
    pub calls: Vec<CallSite>,
    pub imports: Vec<ImportSite>,
}

/// Detect language from file path and optional content sniffing.
pub fn detect_language(path: &Path, content: Option<&str>) -> Option<Language> {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_lowercase().as_str() {
            "rs" => return Some(Language::Rust),
            "ts" | "tsx" => return Some(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => return Some(Language::JavaScript),
            "py" | "pyi" => return Some(Language::Python),
            "go" => return Some(Language::Go),
            "java" => return Some(Language::Java),
            "cs" => return Some(Language::CSharp),
            "rb" => return Some(Language::Ruby),
            _ => {}
        }
    }

    if let Some(content) = content {
        let trimmed = content.trim_start();
        if trimmed.starts_with("package ") {
            let first_line = trimmed.lines().next().unwrap_or("");
            if first_line.starts_with("package ") {
                return Some(Language::Go);
            }
        }
        if trimmed.starts_with("#!/usr/bin/env ruby") || trimmed.starts_with("#!/usr/bin/ruby") {
            return Some(Language::Ruby);
        }
        if trimmed.starts_with("#!/usr/bin/env python") || trimmed.starts_with("#!/usr/bin/python") {
            return Some(Language::Python);
        }
    }

    None
}

/// Extension points for adding new language parsers.
pub trait LanguageParser: Send + Sync {
    fn language(&self) -> Language;
    fn parse(&self, source: &str) -> anyhow::Result<ExtractionResult>;
}

/// Registry of all supported language parsers.
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

    pub fn supported_languages(&self) -> Vec<Language> {
        Language::all().to_vec()
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

mod csharp;
mod extract;
mod go;
mod java;
mod python;
mod ruby;
mod rust;
mod typescript;

use csharp::CSharpParser;
use go::GoParser;
use java::JavaParser;
use python::PythonParser;
use ruby::RubyParser;
use rust::RustParser;
use typescript::{JavaScriptParser, TypeScriptParser};

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_by_extension() {
        assert_eq!(
            detect_language(Path::new("main.rs"), None),
            Some(Language::Rust)
        );
    }

    #[test]
    fn detects_python_by_shebang() {
        assert_eq!(
            detect_language(
                Path::new("script"),
                Some("#!/usr/bin/env python3\nprint('hi')")
            ),
            Some(Language::Python)
        );
    }

    #[test]
    fn detects_java_and_ruby() {
        assert_eq!(
            detect_language(Path::new("Main.java"), None),
            Some(Language::Java)
        );
        assert_eq!(
            detect_language(Path::new("app.rb"), None),
            Some(Language::Ruby)
        );
        assert_eq!(
            detect_language(Path::new("Program.cs"), None),
            Some(Language::CSharp)
        );
    }

    #[test]
    fn registry_lists_eight_languages() {
        let reg = ParserRegistry::new();
        assert_eq!(reg.supported_languages().len(), 8);
    }
}
