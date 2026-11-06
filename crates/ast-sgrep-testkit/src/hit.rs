use ast_sgrep_core::{search::SpanHitInput, HitKind, SearchHit};
use serde_json::Value;

/// Build a single-line [`SearchHit`] span: `line_end = line`, excerpt
/// `excerpt {file}:{line}`, no symbol/language/byte-span. Pure constructor;
/// callers mutate the returned hit for symbol/contributor/margin variants.
pub fn mk_hit(kind: HitKind, file: &str, line: u32, score: f64) -> SearchHit {
    SearchHit::span(SpanHitInput {
        kind,
        file: file.to_string(),
        line_start: line,
        line_end: line,
        score,
        excerpt: format!("excerpt {file}:{line}"),
        symbol: None,
        language: None,
        byte_span: None,
    })
}
/// INTENT: full observable fused-row projection — `SearchHit` has no
/// `PartialEq`, so fusion/determinism asserts compare on this key: (kind,
/// file, line span, exact score bits, contributors). Pure projection.
pub fn fused_key(hit: &SearchHit) -> (HitKind, String, u32, u32, u64, Vec<HitKind>) {
    (
        hit.kind,
        hit.file.clone(),
        hit.line_start,
        hit.line_end,
        hit.score.to_bits(),
        hit.contributors.clone(),
    )
}

/// INTENT: order-sensitive hit identity (file, span, exact score and
/// confidence bits) for fixpoint/determinism asserts. Pure projection.
pub fn hit_key_bits(hit: &SearchHit) -> (String, u32, u32, u64, u64) {
    (
        hit.file.clone(),
        hit.line_start,
        hit.line_end,
        hit.score.to_bits(),
        hit.confidence.to_bits(),
    )
}

/// INTENT: order-free contributor comparison for merge asserts: contributor
/// kinds as sorted wire names. Pure projection.
pub fn sorted_contributors(hit: &SearchHit) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = hit.contributors.iter().map(|kind| kind.as_str()).collect();
    kinds.sort_unstable();
    kinds
}

/// Canonical cross-format hit identity: (file, line_start, kind, symbol, callee, caller).
///
/// The single hit-key type for every surface: core `SearchHit`s (via
/// [`HitKey::from_search_hit`]) and native/agent/capsule/GitHub/GitLab JSON
/// (via [`hit_keys`]). `line_start` is `u64` so core (`u32`) and wire
/// (`u64`) sources share one ordering without casts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HitKey {
    pub file: String,
    pub line_start: u64,
    pub kind: String,
    pub symbol: Option<String>,
    pub callee: Option<String>,
    pub caller: Option<String>,
}

impl HitKey {
    /// Project a core [`SearchHit`] to its cross-format identity.
    /// Pure projection.
    pub fn from_search_hit(hit: &SearchHit) -> Self {
        Self {
            file: hit.file.clone(),
            line_start: u64::from(hit.line_start),
            kind: hit.kind.as_str().to_owned(),
            symbol: hit.symbol.clone(),
            callee: hit.callee.clone(),
            caller: hit.caller.clone(),
        }
    }

    /// The basename of the served [`HitKey::file`] path: sessions rooted at
    /// distinct tempdirs stay comparable. Pure projection.
    pub fn name_key(&self) -> &str {
        Self::name_key_of(&self.file)
    }

    /// Basename projection over a raw file path. Pure projection.
    pub fn name_key_of(file: &str) -> &str {
        file.rsplit('/').next().unwrap_or(file)
    }

    /// Lenient file-only projection over a hit array: entries without a
    /// `file`/`path` are skipped (the file-set shape never asserts on them).
    /// Pure projection; backs the file-set projections.
    pub fn file_keys(value: &Value) -> Vec<String> {
        value["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|hit| {
                hit.get("file")
                    .or_else(|| hit.get("path"))
                    .and_then(|file| file.as_str())
                    .map(str::to_string)
            })
            .collect()
    }

    /// Suffix hit predicate for cross-tempdir assertions. Pure projection.
    pub fn has_file(value: &Value, name: &str) -> bool {
        Self::file_keys(value)
            .iter()
            .any(|file| file.ends_with(name))
    }

    /// Hit-count projection for exact serve / count-parity legs.
    /// Pure projection.
    pub fn hit_count(value: &Value) -> usize {
        value["hits"].as_array().map(Vec::len).unwrap_or(0)
    }

    /// Sorted (file, line_start, symbol) answer keys. Scores, excerpts, and
    /// ordering are excluded: only the served answer SET is compared across
    /// recovery. Missing fields degrade to defaults (recovery-tolerant by
    /// design — a renamed product field degrades instead of failing; the
    /// strict shape is [`hit_keys`]). Pure projection; backs
    /// `cli_recovery::search_answer_keys`.
    pub fn answer_keys(value: &Value) -> Vec<(String, u64, String)> {
        let mut keys: Vec<(String, u64, String)> = value["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|hit| {
                (
                    hit["file"].as_str().unwrap_or_default().to_owned(),
                    hit["line_start"].as_u64().unwrap_or(0),
                    hit["symbol"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        keys.sort();
        keys
    }
}

/// Order-sensitive scored identity of a ranked response row: the [`HitKey`]
/// plus the exact score bits (fused/determinism asserts compare on this;
/// [`crate::response_hit_keys`] has no score bits). Pure projection; backs
/// `core_oracle::response_hit_keys_with_scores` via [`ScoredHitKey::as_tuple`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoredHitKey {
    pub key: HitKey,
    pub line_end: u32,
    pub score_bits: u64,
}

impl ScoredHitKey {
    /// Project a core [`SearchHit`] to its scored identity. Pure projection.
    pub fn from_search_hit(hit: &SearchHit) -> Self {
        Self {
            key: HitKey::from_search_hit(hit),
            line_end: hit.line_end,
            score_bits: hit.score.to_bits(),
        }
    }

    /// Legacy tuple shape (file, line_start, line_end, kind, symbol,
    /// score bits) for whole-vec comparisons. Pure projection.
    pub fn as_tuple(&self) -> (String, u64, u32, String, Option<String>, u64) {
        (
            self.key.file.clone(),
            self.key.line_start,
            self.line_end,
            self.key.kind.clone(),
            self.key.symbol.clone(),
            self.score_bits,
        )
    }
}
/// Extract canonical hit identities from native, agent, capsule, GitHub, or GitLab JSON.
pub fn hit_keys(value: &Value) -> Result<Vec<HitKey>, String> {
    let hits = value
        .get("hits")
        .or_else(|| value.get("items"))
        .or_else(|| value.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| "response has no hit array".to_string())?;
    hits.iter().map(hit_key).collect()
}
fn hit_key(hit: &Value) -> Result<HitKey, String> {
    let meta = hit.get("metadata").or_else(|| hit.get("meta"));
    let field = |name: &str| {
        hit.get(name)
            .or_else(|| meta.and_then(|v| v.get(name)))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let file = field("file")
        .or_else(|| field("path"))
        .ok_or_else(|| "hit has no file/path".to_string())?;
    let line_start = hit
        .get("line_start")
        .or_else(|| hit.get("startline"))
        .or_else(|| hit.get("lines").and_then(|l| l.get("start")))
        .or_else(|| meta.and_then(|v| v.get("line_start")))
        .and_then(Value::as_u64)
        .ok_or_else(|| "hit has no line_start".to_string())?;
    Ok(HitKey {
        file,
        line_start,
        kind: field("kind").ok_or_else(|| "hit has no kind".to_string())?,
        symbol: field("symbol"),
        callee: field("callee"),
        caller: field("caller"),
    })
}
