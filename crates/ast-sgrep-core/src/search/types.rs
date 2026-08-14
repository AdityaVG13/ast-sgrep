use crate::EmbedBackend;
use std::path::PathBuf;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HitSignal {
    Exact,
    Structural,
    Semantic,
}
impl HitSignal {
    pub const ALL: [Self; 3] = [Self::Exact, Self::Structural, Self::Semantic];

    /// Evidence strength ordering: exact beats structural beats semantic (vh65).
    pub fn rank(self) -> u8 {
        match self {
            Self::Exact => 2,
            Self::Structural => 1,
            Self::Semantic => 0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Structural => "structural",
            Self::Semantic => "semantic",
        }
    }
}
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
    pub fn signal(self) -> HitSignal {
        match self {
            HitKind::Asgrep => HitSignal::Exact,
            HitKind::Embed => HitSignal::Semantic,
            HitKind::Def
            | HitKind::Caller
            | HitKind::Graph
            | HitKind::Anchor
            | HitKind::Import
            | HitKind::Pattern => HitSignal::Structural,
        }
    }
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
#[derive(Debug, Clone, serde::Serialize)]
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
    pub signal: HitSignal,
    pub contributors: Vec<HitKind>,
    pub margin: f64,
    /// Calibrated-independent relevance estimate, deliberately separate from
    #[serde(default)]
    pub confidence: f64,
    /// How a graph edge was resolved, when this hit came from one (dvc4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<crate::resolution::Resolution>,
    /// Per-field embed similarities when multi-field vectors were used (7d5x.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_fields: Option<super::field_weight::EmbedFieldScores>,
    /// Deterministic critic annotations (P0 critic-on-shortlist). Engine-derived,
    /// never trusted from the wire (same policy as `resolution`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub critic: Vec<super::critic::CriticNote>,
    #[serde(serialize_with = "serialize_excerpt")]
    pub excerpt: String,
}
#[derive(serde::Deserialize)]
struct SearchHitWire {
    kind: HitKind,
    file: String,
    line_start: u32,
    line_end: u32,
    symbol: Option<String>,
    caller: Option<String>,
    callee: Option<String>,
    language: Option<String>,
    score: f64,
    #[serde(default)]
    signal: Option<HitSignal>,
    #[serde(default)]
    contributors: Vec<HitKind>,
    #[serde(default)]
    margin: f64,
    /// Preserved on JSON round-trip so agents that cache/re-parse hits keep trust.
    /// Non-finite wire values sanitize to 0.0 (same policy as `margin`).
    #[serde(default)]
    confidence: f64,
    excerpt: String,
}
impl<'de> serde::Deserialize<'de> for SearchHit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <SearchHitWire as serde::Deserialize>::deserialize(deserializer)?;
        let _untrusted_signal = wire.signal;
        let _untrusted_contributors = wire.contributors;
        Ok(Self {
            kind: wire.kind,
            file: wire.file,
            line_start: wire.line_start,
            line_end: wire.line_end,
            symbol: wire.symbol,
            caller: wire.caller,
            callee: wire.callee,
            language: wire.language,
            score: wire.score,
            signal: wire.kind.signal(),
            contributors: vec![wire.kind],
            confidence: if wire.confidence.is_finite() {
                wire.confidence
            } else {
                0.0
            },
            margin: if wire.margin.is_finite() {
                wire.margin.max(0.0)
            } else {
                0.0
            },
            // dvc4: resolution is engine-derived, never trusted from the wire.
            resolution: None,
            embed_fields: None,
            critic: Vec::new(),
            excerpt: bound_excerpt(wire.excerpt),
        })
    }
}
#[derive(Debug, Clone)]
pub struct SpanHitInput {
    pub kind: HitKind,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub score: f64,
    pub excerpt: String,
    pub symbol: Option<String>,
    pub language: Option<String>,
}
impl SearchHit {
    fn base(
        kind: HitKind,
        file: String,
        line_start: u32,
        line_end: u32,
        score: f64,
        excerpt: String,
    ) -> Self {
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
            signal: kind.signal(),
            contributors: vec![kind],
            margin: 0.0,
            confidence: 0.0,
            resolution: None,
            embed_fields: None,
            critic: Vec::new(),
            excerpt: bound_excerpt(excerpt),
        }
    }
    pub fn span(input: SpanHitInput) -> Self {
        Self {
            symbol: input.symbol,
            language: input.language,
            ..Self::base(
                input.kind,
                input.file,
                input.line_start,
                input.line_end,
                input.score,
                input.excerpt,
            )
        }
    }
    pub fn import(
        file: String,
        language: Option<String>,
        module_path: String,
        line_no: u32,
    ) -> Self {
        Self {
            symbol: Some(module_path.clone()),
            language,
            excerpt: bound_excerpt(format!("import {module_path}")),
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
            excerpt: bound_excerpt(excerpt),
            ..Self::base(
                HitKind::Caller,
                file,
                line_no,
                line_no,
                score,
                String::new(),
            )
        }
    }
    pub fn graph_scored(
        file: String,
        language: Option<String>,
        caller: String,
        callee: String,
        line_no: u32,
        score: f64,
    ) -> Self {
        Self {
            symbol: Some(callee.clone()),
            caller: Some(caller.clone()),
            callee: Some(callee.clone()),
            language,
            excerpt: bound_excerpt(format!("{caller} calls {callee}")),
            score,
            ..Self::base(HitKind::Graph, file, line_no, line_no, score, String::new())
        }
    }
}

pub(crate) fn bound_excerpt(mut excerpt: String) -> String {
    const MARKER: &str = "\n…";
    let max = crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES;
    if excerpt.len() <= max {
        return excerpt;
    }
    let mut end = max.saturating_sub(MARKER.len());
    while end > 0 && !excerpt.is_char_boundary(end) {
        end -= 1;
    }
    excerpt.truncate(end);
    excerpt.push_str(MARKER);
    excerpt
}

pub(crate) fn bound_excerpt_ref(excerpt: &str) -> String {
    const MARKER: &str = "\n…";
    let max = crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES;
    if excerpt.len() <= max {
        return excerpt.to_owned();
    }
    let mut end = max.saturating_sub(MARKER.len());
    while end > 0 && !excerpt.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = String::with_capacity(end + MARKER.len());
    bounded.push_str(&excerpt[..end]);
    bounded.push_str(MARKER);
    bounded
}

fn serialize_excerpt<S>(excerpt: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if excerpt.len() <= crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES {
        return serializer.serialize_str(excerpt);
    }
    serializer.serialize_str(&bound_excerpt_ref(excerpt))
}
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub limit: usize,
    pub lang_filter: Option<String>,
    pub use_embed: bool,
    pub use_tantivy: bool,
    /// Adapter flags for `embed_backend()`. Concurrent trues collapse
    /// Neural > Semantic > Auto.
    pub use_neural_embed: bool,
    pub use_semantic_only: bool,
    pub ann_threshold: Option<usize>,
    /// IVF clusters to probe (0/None = adaptive, at most 90% populated; ≥ n_clusters = exact).
    pub ann_probes: Option<usize>,
    pub use_rerank: bool,
    pub rerank_top_k: usize,
    pub case_insensitive: bool,
    pub context_before: usize,
    pub context_after: usize,
    pub count_only: bool,
    pub file_filter: Option<String>,
}
impl Default for SearchOptions {
    fn default() -> Self {
        use crate::env_flag::env_flag;
        Self {
            root: PathBuf::from("."),
            index_path: None,
            limit: Self::default_limit(),
            lang_filter: None,
            use_embed: !env_flag("ASGREP_NO_EMBED"),
            use_tantivy: env_flag("ASGREP_TANTIVY"),
            use_neural_embed: env_flag("ASGREP_NEURAL_EMBED"),
            use_semantic_only: env_flag("ASGREP_SEMANTIC_ONLY"),
            ann_threshold: std::env::var("ASGREP_ANN_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok()),
            ann_probes: std::env::var("ASGREP_ANN_PROBES")
                .ok()
                .and_then(|v| v.parse().ok()),
            use_rerank: env_flag("ASGREP_RERANK"),
            rerank_top_k: std::env::var("ASGREP_RERANK_TOP_K")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            case_insensitive: false,
            context_before: 0,
            context_after: 0,
            count_only: false,
            file_filter: None,
        }
    }
}
impl SearchOptions {
    pub fn default_limit() -> usize {
        crate::limits::clamp_output_limit(
            std::env::var("ASGREP_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok()),
            16,
        )
    }
    pub fn embed_preference(&self) -> ast_sgrep_embed::EmbedPreference {
        self.embed_backend().to_preference()
    }

    /// Canonical embed backend for these options. `use_*` flags remain
    /// public adapters (Neural > Semantic > Auto).
    pub fn embed_backend(&self) -> EmbedBackend {
        EmbedBackend::from_flags(self.use_neural_embed, self.use_semantic_only)
    }

    /// Set the backend and sync the `use_*` adapter flags.
    pub fn set_embed_backend(&mut self, backend: EmbedBackend) {
        let (neural, semantic_only) = backend.to_flags();
        self.use_neural_embed = neural;
        self.use_semantic_only = semantic_only;
    }

    /// Hard-error text when a non-hashed backend is requested but cannot run.
    ///
    /// Hashed (`Semantic`) and `Auto` stay available. Neural must not silently
    /// swap to hashed hits unless `ASGREP_NEURAL_FALLBACK=1`.
    pub fn unavailable_non_hashed_embed(&self) -> Option<String> {
        match self.embed_preference() {
            ast_sgrep_embed::EmbedPreference::Neural => {
                #[cfg(not(feature = "neural-embed"))]
                {
                    Some(
                        "semantic_search requested neural embed but this build has no neural-embed feature; refusing hashed fallback"
                            .into(),
                    )
                }
                #[cfg(feature = "neural-embed")]
                {
                    if ast_sgrep_embed::NeuralEmbeddingConfig::from_env().is_none() {
                        Some(
                            "semantic_search requested neural embed but it is not configured; refusing hashed fallback"
                                .into(),
                        )
                    } else {
                        None
                    }
                }
            }
            _ => None,
        }
    }
    /// Stable fingerprint of options that affect search results (nyui).
    pub fn cache_identity(&self) -> String {
        format!(
            "root={}\0idx={:?}\0lim={}\0lang={:?}\0embed={}\0tantivy={}\0neural={}\0sem={}\0ann_t={:?}\0ann_p={:?}\0rerank={}\0rk={}\0ci={}\0cb={}\0ca={}\0co={}\0ff={:?}",
            self.root.display(),
            self.index_path,
            self.limit,
            self.lang_filter,
            self.use_embed,
            self.use_tantivy,
            self.use_neural_embed,
            self.use_semantic_only,
            self.ann_threshold,
            self.ann_probes,
            self.use_rerank,
            self.rerank_top_k,
            self.case_insensitive,
            self.context_before,
            self.context_after,
            self.count_only,
            self.file_filter,
        )
    }
}
/// Identity of the index snapshot a response was built from (d3l5).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotStamp {
    /// Monotonic index generation every indexing transaction bumps.
    pub generation: i64,
    /// Database schema version the response was read under.
    pub schema_version: i64,
    /// Highest indexed file mtime: what the index believes about the worktree.
    pub worktree_revision: i64,
    /// Resolved git HEAD, when the root is a git worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_head: Option<String>,
    /// Semantic sidecar fingerprint, when a semantic sidecar was consulted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_manifest: Option<String>,
    /// Channels that could not run, or ran against a mismatched sidecar.
    /// A degraded channel must be visible, never silently dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_channels: Vec<DegradedChannel>,
}

/// One learned association applied to a query (ufk7).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueryExpansion {
    pub term: String,
    pub related: String,
    /// Co-occurrence count behind the association: the checkable number.
    pub support: u32,
    /// Human-readable justification.
    pub because: String,
}

/// A channel that failed or was skipped, and why (d3l5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DegradedChannel {
    pub channel: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub limit: usize,
    /// Ranked results after result gates. Hybrid search promotes at most three hits per
    pub hits: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counts: Vec<(String, u32)>,
    #[serde(default)]
    pub read_bytes_estimate: u64,
    #[serde(default)]
    pub returned_excerpt_bytes: u64,
    #[serde(default)]
    pub prevented_read_bytes: u64,
    /// Index snapshot this response was built from (d3l5).
    #[serde(default)]
    pub snapshot: SnapshotStamp,
    /// Repository associations that widened this query (ufk7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_expansions: Vec<QueryExpansion>,
}
pub fn format_hit_line(hit: &SearchHit) -> String {
    let f = &hit.file;
    let (ls, le) = (hit.line_start, hit.line_end);
    let trunc = |s: &str| {
        if s.len() <= 120 {
            return s.to_string();
        }
        let mut end = 120;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    };
    match hit.kind {
        HitKind::Asgrep => format!("ASGREP: {f}:{ls}-{le}: {}", hit.excerpt),
        HitKind::Def => format!(
            "DEF: {f}: {} span={ls}..{le} | {}",
            hit.symbol.as_deref().unwrap_or("?"),
            trunc(&hit.excerpt)
        ),
        HitKind::Caller => format!(
            "CALLER: {f}: {} -> {}",
            hit.caller.as_deref().unwrap_or("?"),
            hit.callee.as_deref().unwrap_or("?")
        ),
        HitKind::Graph => format!(
            "GRAPH: {f}: {} calls {}",
            hit.caller.as_deref().unwrap_or("?"),
            hit.callee.as_deref().unwrap_or("?")
        ),
        HitKind::Anchor => format!("ANCHOR: {f}:{ls}-{le}: {}", trunc(&hit.excerpt)),
        HitKind::Import => format!("IMPORT: {f}:{ls}: {}", hit.excerpt),
        HitKind::Pattern => format!("PATTERN: {f}:{ls}-{le}: {}", trunc(&hit.excerpt)),
        HitKind::Embed => {
            let sym = hit
                .symbol
                .as_deref()
                .map(|s| format!("{s} | "))
                .unwrap_or_default();
            format!("EMBED: {f}:{ls}-{le}: {sym}{}", trunc(&hit.excerpt))
        }
    }
}
pub fn matches_lang(language: Option<&str>, filter: Option<&str>) -> bool {
    filter.is_none_or(|want| {
        language.is_some_and(|have| {
            // Compare on Language::as_str forms so Title Case external labels
            // (e.g. ast-grep "Rust") still match `--lang rust`.
            let have_n = ast_sgrep_lang::Language::normalize_id(have);
            let want_n = ast_sgrep_lang::Language::normalize_id(want);
            have_n == want_n
        })
    })
}
pub fn assign_signal_margins(hits: &mut [SearchHit]) {
    for hit in hits.iter_mut() {
        hit.signal = hit.kind.signal();
        hit.margin = 0.0;
    }
    for signal in HitSignal::ALL {
        let mut indices = hits
            .iter()
            .enumerate()
            .filter_map(|(index, hit)| {
                (hit.signal == signal && hit.score.is_finite()).then_some(index)
            })
            .collect::<Vec<_>>();
        indices.sort_unstable_by(|left, right| hits[*right].score.total_cmp(&hits[*left].score));
        let mut start = 0;
        while start < indices.len() {
            let score = hits[indices[start]].score;
            let mut end = start + 1;
            while end < indices.len() && hits[indices[end]].score == score {
                end += 1;
            }
            let margin = if end - start > 1 || end == indices.len() {
                0.0
            } else {
                let next = hits[indices[end]].score;
                let delta = score - next;
                if delta.is_finite() {
                    delta.max(0.0)
                } else if score > next {
                    f64::MAX
                } else {
                    0.0
                }
            };
            for index in &indices[start..end] {
                hits[*index].margin = margin;
            }
            start = end;
        }
    }
}

/// Assign heuristic confidence on every hit (vh65 / pass5).
pub fn assign_hit_confidence(hits: &mut [SearchHit]) {
    for hit in hits.iter_mut() {
        hit.confidence = estimate_confidence(hit);
    }
}

/// Fold a duplicate observation of the same location into the kept hit (vh65).
pub(super) fn merge_channel_evidence(kept: &mut SearchHit, other: SearchHit) {
    for contributor in other
        .contributors
        .iter()
        .copied()
        .chain(std::iter::once(other.kind))
    {
        if !kept.contributors.contains(&contributor) {
            kept.contributors.push(contributor);
        }
    }
    // Strongest signal observed for this location wins.
    if other.signal.rank() > kept.signal.rank() {
        kept.signal = other.signal;
    }
    if other.score > kept.score {
        // Preserve the previous best-score semantics: the higher-scoring row
        // supplies score, kind, and excerpt.
        kept.score = other.score;
        kept.kind = other.kind;
        if !other.excerpt.is_empty() {
            kept.excerpt = bound_excerpt(other.excerpt);
        }
        kept.margin = other.margin;
    } else if kept.excerpt.is_empty() && !other.excerpt.is_empty() {
        kept.excerpt = bound_excerpt(other.excerpt);
    }
    // Fill in identity details the kept row happened to lack.
    kept.symbol = kept.symbol.take().or(other.symbol);
    kept.caller = kept.caller.take().or(other.caller);
    kept.callee = kept.callee.take().or(other.callee);
    kept.language = kept.language.take().or(other.language);
    // Critic annotations are evidence about the location, not the row.
    for note in other.critic {
        if !kept.critic.contains(&note) {
            kept.critic.push(note);
        }
    }
}

/// Heuristic confidence from channel agreement and signal strength (vh65).
fn estimate_confidence(hit: &SearchHit) -> f64 {
    let strongest = hit
        .contributors
        .iter()
        .map(|kind| kind.signal())
        .chain(std::iter::once(hit.kind.signal()))
        .max_by_key(|signal| signal.rank())
        .unwrap_or(hit.signal);
    let base = match strongest {
        HitSignal::Exact => 0.75,
        HitSignal::Structural => 0.60,
        HitSignal::Semantic => 0.35,
    };
    let agreement = hit.contributors.len().saturating_sub(1).min(3) as f64 * 0.08;
    (base + agreement).clamp(0.0, 0.99)
}

/// Human- and agent-readable reasons this location was returned (vh65).
pub fn hit_why(hit: &SearchHit) -> Vec<String> {
    let mut why = Vec::new();
    for contributor in &hit.contributors {
        why.push(match contributor {
            HitKind::Asgrep => "exact_text".to_owned(),
            HitKind::Def => "exact_symbol".to_owned(),
            HitKind::Caller => match &hit.caller {
                Some(caller) => format!("called_by:{caller}"),
                None => "caller_edge".to_owned(),
            },
            HitKind::Graph => "graph_edge".to_owned(),
            HitKind::Anchor => "anchor".to_owned(),
            HitKind::Import => "import_edge".to_owned(),
            HitKind::Pattern => "structural_pattern".to_owned(),
            HitKind::Embed => "semantic_similarity".to_owned(),
        });
    }
    if let Some(fields) = &hit.embed_fields {
        why.extend(fields.why_terms());
    }
    for note in &hit.critic {
        why.push(format!("critic:{}", note.as_str()));
    }
    why.dedup();
    why
}

#[cfg(test)]
#[path = "../../../../tests/unit/core/search__types.rs"]
mod tests;
