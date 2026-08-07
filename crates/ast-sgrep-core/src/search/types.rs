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
    /// `score` (vh65).
    ///
    /// `score` orders results; `confidence` states how much to trust the top
    /// one. A single weak lexical hit and a hit confirmed by three channels can
    /// rank adjacently, and only `confidence` can say they are not equally
    /// trustworthy. This is a documented heuristic over channel agreement and
    /// signal strength, NOT a probability calibrated against held-out data --
    /// calibration is separate work and is not claimed here.
    #[serde(default)]
    pub confidence: f64,
    /// How a graph edge was resolved, when this hit came from one (dvc4).
    ///
    /// `None` for non-graph hits. A hit whose resolution is not precise must
    /// never be rendered as an exact call edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<crate::resolution::Resolution>,
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
            excerpt: wire.excerpt,
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
            excerpt,
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
            excerpt: format!("{caller} calls {callee}"),
            score,
            ..Self::base(HitKind::Graph, file, line_no, line_no, score, String::new())
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
            use_cloud_embed: env_flag("ASGREP_CLOUD_EMBED"),
            use_ollama_embed: env_flag("ASGREP_OLLAMA_EMBED"),
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
        EmbedBackend::from_flags(
            self.use_cloud_embed,
            self.use_ollama_embed,
            self.use_neural_embed,
            self.use_semantic_only,
        )
        .to_preference()
    }
    /// Stable fingerprint of options that affect search results (nyui).
    pub fn cache_identity(&self) -> String {
        format!(
            "root={}\0idx={:?}\0lim={}\0lang={:?}\0embed={}\0tantivy={}\0cloud={}\0ollama={}\0neural={}\0sem={}\0ann_t={:?}\0ann_p={:?}\0rerank={}\0rk={}\0ci={}\0cb={}\0ca={}\0co={}\0ff={:?}",
            self.root.display(),
            self.index_path,
            self.limit,
            self.lang_filter,
            self.use_embed,
            self.use_tantivy,
            self.use_cloud_embed,
            self.use_ollama_embed,
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
///
/// The hard invariant: a `SearchResponse` may carry evidence from exactly one
/// index generation. Without this, a multi-pass search running in autocommit
/// could take its definition from generation `g`, its callers from `g + 1`, and
/// its semantic sidecar from `g - 1`, and report the mixture as one coherent
/// answer.
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
    /// file ahead of overflow before applying `limit`, so this is a diversity-aware
    /// ranking rather than a pure global score top-k.
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
    ///
    /// Query expansion changes what the user asked for, so the engine states
    /// which terms it added and on what evidence rather than silently
    /// broadening the search.
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
///
/// Call after channel merge and after [`assign_signal_margins`]: margins rewrite
/// display `signal` from `kind`, but confidence must reflect contributor
/// agreement and the strongest observed channel, not the post-margin display
/// field alone.
pub fn assign_hit_confidence(hits: &mut [SearchHit]) {
    for hit in hits.iter_mut() {
        hit.confidence = estimate_confidence(hit);
    }
}

pub fn dedup_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut best: Vec<SearchHit> = Vec::with_capacity(hits.len());
    let mut positions: std::collections::HashMap<_, usize> = std::collections::HashMap::new();
    for hit in hits {
        // vh65: identity is the LOCATION. Channel kind is evidence about a
        // location, not part of what a location is -- keying on it let one
        // physical span survive three times as three near-identical rows with
        // three opaque scores.
        let key = (
            hit.file.clone(),
            hit.line_start,
            hit.line_end,
            hit.symbol.clone(),
            hit.caller.clone(),
            hit.callee.clone(),
        );
        if let Some(&index) = positions.get(&key) {
            merge_channel_evidence(&mut best[index], hit);
        } else {
            positions.insert(key, best.len());
            best.push(hit);
        }
    }
    assign_hit_confidence(&mut best);
    best
}

/// Fold a duplicate observation of the same location into the kept hit (vh65).
///
/// Best score wins the ordering, exactly as before, so ranking behavior is
/// preserved; what changes is that the losing row's channel is retained as
/// evidence instead of being dropped or duplicated.
fn merge_channel_evidence(kept: &mut SearchHit, other: SearchHit) {
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
            kept.excerpt = other.excerpt;
        }
        kept.margin = other.margin;
    } else if kept.excerpt.is_empty() && !other.excerpt.is_empty() {
        kept.excerpt = other.excerpt;
    }
    // Fill in identity details the kept row happened to lack.
    kept.symbol = kept.symbol.take().or(other.symbol);
    kept.caller = kept.caller.take().or(other.caller);
    kept.callee = kept.callee.take().or(other.callee);
    kept.language = kept.language.take().or(other.language);
}

/// Heuristic confidence from channel agreement and signal strength (vh65).
///
/// Deliberately simple and inspectable. Exact signals start high; semantic-only
/// evidence starts low; each additional independent channel that agrees adds a
/// bounded increment. This is not a calibrated probability and must not be
/// reported as one.
///
/// Base strength is the **strongest contributor channel** (and `kind`), not
/// solely `hit.signal`. `assign_signal_margins` rewrites display `signal` from
/// `kind` for within-channel margins; confidence must not collapse to that
/// display field after multi-channel merges (pass5).
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
///
/// Derived from the merged evidence, so it cannot drift from the evidence that
/// actually produced the hit.
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
    why.dedup();
    why
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
        SearchHit {
            kind,
            file: file.into(),
            line_start: line,
            line_end: line,
            symbol: None,
            caller: None,
            callee: None,
            language: None,
            score,
            signal: kind.signal(),
            contributors: vec![kind],
            margin: 0.0,
            confidence: 0.0,
            excerpt: String::new(),
        }
    }

    #[test]
    fn confidence_uses_strongest_contributor_not_display_signal() {
        // Higher-scoring Embed wins kind/score; lower-scoring Asgrep still contributes
        // exact evidence. After margins rewrite display signal to Semantic, confidence
        // must keep Exact base + one agreement step (0.75 + 0.08).
        let mut merged = dedup_hits(vec![
            hit(HitKind::Embed, "a.rs", 1, 0.9),
            hit(HitKind::Asgrep, "a.rs", 1, 0.4),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].kind, HitKind::Embed);
        assert!(merged[0].contributors.contains(&HitKind::Asgrep));
        assert!(merged[0].contributors.contains(&HitKind::Embed));

        assign_signal_margins(&mut merged);
        assert_eq!(merged[0].signal, HitSignal::Semantic);
        // Re-assign as finish_response does after margins (pass5).
        assign_hit_confidence(&mut merged);
        let expected = 0.75 + 0.08;
        assert!(
            (merged[0].confidence - expected).abs() < 1e-12,
            "confidence={} expected {expected}",
            merged[0].confidence
        );
    }

    #[test]
    fn semantic_only_confidence_is_nonzero_without_dedup() {
        // search_semantic uses dedup=false; confidence must still be populated.
        let mut hits = vec![hit(HitKind::Embed, "sem.rs", 3, 2.5)];
        assign_signal_margins(&mut hits);
        assign_hit_confidence(&mut hits);
        assert!((hits[0].confidence - 0.35).abs() < 1e-12);
        assert!(hits[0].confidence > 0.0);
    }

    #[test]
    fn empty_hits_confidence_assign_is_noop() {
        let mut hits: Vec<SearchHit> = vec![];
        assign_hit_confidence(&mut hits);
        assert!(hits.is_empty());
    }

    #[test]
    fn search_hit_json_round_trip_preserves_confidence() {
        // d2a1.8: custom Deserialize used SearchHitWire without confidence, so
        // round-trip always forced 0.0 even when finish_response had assigned it.
        let mut original = hit(HitKind::Asgrep, "lib.rs", 10, 1.0);
        original.confidence = 0.83;
        original.excerpt = "fn foo() {}".into();
        original.symbol = Some("foo".into());

        let json = serde_json::to_string(&original).expect("serialize");
        assert!(
            json.contains("\"confidence\""),
            "serialized JSON must emit confidence: {json}"
        );
        let back: SearchHit = serde_json::from_str(&json).expect("deserialize");
        assert!(
            (back.confidence - 0.83).abs() < 1e-12,
            "round-trip confidence={} expected 0.83",
            back.confidence
        );
        assert_eq!(back.file, "lib.rs");
        assert_eq!(back.kind, HitKind::Asgrep);
        assert_eq!(back.symbol.as_deref(), Some("foo"));
    }

    #[test]
    fn search_hit_json_missing_confidence_defaults_zero() {
        let json = r#"{
            "kind": "embed",
            "file": "a.rs",
            "line_start": 1,
            "line_end": 1,
            "score": 0.5,
            "excerpt": "x"
        }"#;
        let hit: SearchHit = serde_json::from_str(json).expect("deserialize without confidence");
        assert_eq!(hit.confidence, 0.0);
        assert_eq!(hit.kind, HitKind::Embed);
    }
}
