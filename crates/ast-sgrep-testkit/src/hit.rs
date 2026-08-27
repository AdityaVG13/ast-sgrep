use serde_json::Value;
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
