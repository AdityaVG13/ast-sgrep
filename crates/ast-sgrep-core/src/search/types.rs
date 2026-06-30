use std::path::PathBuf;

/// Kind of search hit in output.
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

/// A single search result.
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

/// Search options.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub limit: usize,
    pub lang_filter: Option<String>,
    /// Run semantic embedding pass (default on).
    pub use_embed: bool,
    pub use_tantivy: bool,
    pub use_cloud_embed: bool,
    pub use_ollama_embed: bool,
    pub use_semantic_only: bool,
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
}

/// Search response wrapper for JSON output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub limit: usize,
    pub hits: Vec<SearchHit>,
}
