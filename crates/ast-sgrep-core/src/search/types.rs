use std::path::PathBuf; use crate::EmbedBackend; #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)] #[serde(rename_all = "lowercase")] pub enum HitKind {
    Asgrep, Def, Caller, Graph, Anchor, Import, Pattern, Embed,
} impl HitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HitKind::Asgrep => "asgrep", HitKind::Def => "def", HitKind::Caller => "caller", HitKind::Graph => "graph", HitKind::Anchor => "anchor", HitKind::Import => "import", HitKind::Pattern => "pattern", HitKind::Embed => "embed",
        }
    }
} #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] pub struct SearchHit {
    pub kind: HitKind, pub file: String, pub line_start: u32, pub line_end: u32, #[serde(skip_serializing_if = "Option::is_none")] pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub caller: Option<String>, #[serde(skip_serializing_if = "Option::is_none")] pub callee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub language: Option<String>, pub score: f64, pub excerpt: String,
} #[derive(Debug, Clone)] pub struct SpanHitInput {
    pub kind: HitKind, pub file: String, pub line_start: u32, pub line_end: u32, pub score: f64, pub excerpt: String, pub symbol: Option<String>, pub language: Option<String>,
} impl SearchHit {
    fn base(
        kind: HitKind, file: String, line_start: u32, line_end: u32, score: f64, excerpt: String,
    ) -> Self {
        Self {
            kind, file, line_start, line_end, symbol: None, caller: None, callee: None, language: None, score, excerpt,
        }
    }

    pub fn span(input: SpanHitInput) -> Self {
        Self {
            symbol: input.symbol, language: input.language, ..Self::base(
                input.kind, input.file, input.line_start, input.line_end, input.score, input.excerpt,
            )
        }
    }

    pub fn import(file: String, language: Option<String>, module_path: String, line_no: u32) -> Self {
        Self {
            symbol: Some(module_path.clone()), language, excerpt: format!("import {module_path}"), ..Self::base(HitKind::Import, file, line_no, line_no, 2.0, String::new())
        }
    }

    pub fn caller(
        file: String, language: Option<String>, caller: String, callee: String, line_no: u32, excerpt: String, score: f64,
    ) -> Self {
        Self {
            caller: Some(caller), callee: Some(callee), language, excerpt, ..Self::base(HitKind::Caller, file, line_no, line_no, score, String::new())
        }
    }

    pub fn graph_scored(
        file: String, language: Option<String>, caller: String, callee: String, line_no: u32, score: f64,
    ) -> Self {
        Self {
            symbol: Some(callee.clone()), caller: Some(caller.clone()), callee: Some(callee.clone()), language, excerpt: format!("{caller} calls {callee}"), score, ..Self::base(HitKind::Graph, file, line_no, line_no, score, String::new())
        }
    }
} #[derive(Debug, Clone)] pub struct SearchOptions {
    pub root: PathBuf, pub index_path: Option<PathBuf>, pub limit: usize, pub lang_filter: Option<String>, pub use_embed: bool, pub use_tantivy: bool, pub use_cloud_embed: bool, pub use_ollama_embed: bool, pub use_neural_embed: bool,
    pub use_semantic_only: bool, pub ann_threshold: Option<usize>,
    /// IVF clusters to probe (0/None = adaptive √k; ≥ n_clusters = exact).
    pub ann_probes: Option<usize>, pub use_rerank: bool, pub rerank_top_k: usize, pub case_insensitive: bool, pub context_before: usize, pub context_after: usize, pub count_only: bool, pub file_filter: Option<String>,
} impl Default for SearchOptions {
    fn default() -> Self {
        let env1 = |k: &str| std::env::var(k).ok().as_deref() == Some("1"); Self {
            root: PathBuf::from("."), index_path: None, limit: Self::default_limit(), lang_filter: None, use_embed: !env1("ASGREP_NO_EMBED"), use_tantivy: env1("ASGREP_TANTIVY"),
            use_cloud_embed: env1("ASGREP_CLOUD_EMBED"), use_ollama_embed: env1("ASGREP_OLLAMA_EMBED"), use_neural_embed: env1("ASGREP_NEURAL_EMBED"),
            use_semantic_only: env1("ASGREP_SEMANTIC_ONLY"), ann_threshold: std::env::var("ASGREP_ANN_THRESHOLD")
                .ok() .and_then(|v| v.parse().ok()),
            ann_probes: std::env::var("ASGREP_ANN_PROBES")
                .ok() .and_then(|v| v.parse().ok()),
            use_rerank: env1("ASGREP_RERANK"), rerank_top_k: std::env::var("ASGREP_RERANK_TOP_K")
                .ok() .and_then(|v| v.parse().ok()) .unwrap_or(20),
            case_insensitive: false, context_before: 0, context_after: 0, count_only: false, file_filter: None,
        }
    }
} impl SearchOptions {
    pub fn default_limit() -> usize {
        std::env::var("ASGREP_LIMIT")
            .ok() .and_then(|v| v.parse().ok()) .unwrap_or(16)
    }

    pub fn embed_preference(&self) -> ast_sgrep_embed::EmbedPreference {
        EmbedBackend::from_flags(
            self.use_cloud_embed, self.use_ollama_embed, self.use_neural_embed, self.use_semantic_only,
        ) .to_preference()
    }
} #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] pub struct SearchResponse {
    pub query: String, pub limit: usize,
    /// Ranked results after result gates. Hybrid search promotes at most three hits per
    /// file ahead of overflow before applying `limit`, so this is a diversity-aware
    /// ranking rather than a pure global score top-k.
    pub hits: Vec<SearchHit>, #[serde(default, skip_serializing_if = "Vec::is_empty")] pub counts: Vec<(String, u32)>, #[serde(default)] pub read_bytes_estimate: u64,
    #[serde(default)] pub returned_excerpt_bytes: u64, #[serde(default)] pub prevented_read_bytes: u64,
}

pub fn format_hit_line(hit: &SearchHit) -> String {
    let f = &hit.file; let (ls, le) = (hit.line_start, hit.line_end); let trunc = |s: &str| {
        if s.len() <= 120 { return s.to_string(); } let mut end = 120; while end > 0 && !s.is_char_boundary(end) { end -= 1; } format!("{}...", &s[..end])
    }; match hit.kind {
        HitKind::Asgrep => format!("ASGREP: {f}:{ls}-{le}: {}", hit.excerpt), HitKind::Def => format!(
            "DEF: {f}: {} span={ls}..{le} | {}", hit.symbol.as_deref().unwrap_or("?"), trunc(&hit.excerpt)
        ), HitKind::Caller => format!(
            "CALLER: {f}: {} -> {}", hit.caller.as_deref().unwrap_or("?"), hit.callee.as_deref().unwrap_or("?")
        ), HitKind::Graph => format!(
            "GRAPH: {f}: {} calls {}", hit.caller.as_deref().unwrap_or("?"), hit.callee.as_deref().unwrap_or("?")
        ), HitKind::Anchor => format!("ANCHOR: {f}:{ls}-{le}: {}", trunc(&hit.excerpt)),
        HitKind::Import => format!("IMPORT: {f}:{ls}: {}", hit.excerpt), HitKind::Pattern => format!("PATTERN: {f}:{ls}-{le}: {}", trunc(&hit.excerpt)), HitKind::Embed => {
            let sym = hit.symbol.as_deref().map(|s| format!("{s} | ")).unwrap_or_default(); format!("EMBED: {f}:{ls}-{le}: {sym}{}", trunc(&hit.excerpt))
        }
    }
}
