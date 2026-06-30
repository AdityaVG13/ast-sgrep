use std::path::PathBuf;

use crate::EmbedBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HitKind {
    Asgrep,
    Def,
    Caller,
    Graph,
    Anchor,
    Import,
    Pattern,
    Embed,
}

impl HitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HitKind::Asgrep => "asgrep",
            HitKind::Def => "def",
            HitKind::Caller => "caller",
            HitKind::Graph => "graph",
            HitKind::Anchor => "anchor",
            HitKind::Import => "import",
            HitKind::Pattern => "pattern",
            HitKind::Embed => "embed",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub kind: HitKind,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub score: f64,
    pub excerpt: String,
}

impl SearchHit {
    fn base(kind: HitKind, file: String, line_start: u32, line_end: u32, score: f64, excerpt: String) -> Self {
        Self {
            kind,
            file,
            line_start,
            line_end,
            symbol: None,
            caller: None,
            callee: None,
            language: None,
            score,
            excerpt,
        }
    }

    pub fn span(
        kind: HitKind,
        file: String,
        line_start: u32,
        line_end: u32,
        score: f64,
        excerpt: String,
        symbol: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self { symbol, language, ..Self::base(kind, file, line_start, line_end, score, excerpt) }
    }

    pub fn import(file: String, language: Option<String>, module_path: String, line_no: u32) -> Self {
        Self {
            symbol: Some(module_path.clone()),
            language,
            excerpt: format!("import {module_path}"),
            ..Self::base(HitKind::Import, file, line_no, line_no, 2.0, String::new())
        }
    }

    pub fn caller(
        file: String,
        language: Option<String>,
        caller: String,
        callee: String,
        line_no: u32,
        excerpt: String,
        score: f64,
    ) -> Self {
        Self {
            caller: Some(caller),
            callee: Some(callee),
            language,
            excerpt,
            ..Self::base(HitKind::Caller, file, line_no, line_no, score, String::new())
        }
    }

    pub fn graph(
        file: String,
        language: Option<String>,
        caller: String,
        callee: String,
        line_no: u32,
    ) -> Self {
        let excerpt = format!("{caller} calls {callee}");
        Self {
            symbol: Some(callee.clone()),
            caller: Some(caller),
            callee: Some(callee),
            language,
            excerpt,
            score: crate::rank::SCORE_GRAPH,
            ..Self::base(HitKind::Graph, file, line_no, line_no, crate::rank::SCORE_GRAPH, String::new())
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub limit: usize,
    pub lang_filter: Option<String>,
    pub use_embed: bool,
    pub use_tantivy: bool,
    pub use_cloud_embed: bool,
    pub use_ollama_embed: bool,
    pub use_semantic_only: bool,
    pub ann_threshold: Option<usize>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        let no_embed = std::env::var("ASGREP_NO_EMBED").ok().as_deref() == Some("1");
        Self {
            root: PathBuf::from("."),
            index_path: None,
            limit: Self::default_limit(),
            lang_filter: None,
            use_embed: !no_embed,
            use_tantivy: std::env::var("ASGREP_TANTIVY").ok().as_deref() == Some("1"),
            use_cloud_embed: std::env::var("ASGREP_CLOUD_EMBED").ok().as_deref() == Some("1"),
            use_ollama_embed: std::env::var("ASGREP_OLLAMA_EMBED").ok().as_deref() == Some("1"),
            use_semantic_only: std::env::var("ASGREP_SEMANTIC_ONLY").ok().as_deref() == Some("1"),
            ann_threshold: std::env::var("ASGREP_ANN_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok()),
        }
    }
}

impl SearchOptions {
    pub fn default_limit() -> usize {
        std::env::var("ASGREP_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16)
    }

    pub fn embed_preference(&self) -> ast_sgrep_embed::EmbedPreference {
        EmbedBackend::from_flags(
            self.use_cloud_embed,
            self.use_ollama_embed,
            self.use_semantic_only,
        )
        .to_preference()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub limit: usize,
    pub hits: Vec<SearchHit>,
}
