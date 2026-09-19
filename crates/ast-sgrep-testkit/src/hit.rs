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
    let mut kinds: Vec<&'static str> =
        hit.contributors.iter().map(|kind| kind.as_str()).collect();
    kinds.sort_unstable();
    kinds
}

/// Canonical cross-format hit identity: (file, line_start, kind, symbol, callee, caller).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HitKey {
    pub file: String,
    pub line_start: u64,
    pub kind: String,
    pub symbol: Option<String>,
    pub callee: Option<String>,
    pub caller: Option<String>,
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
