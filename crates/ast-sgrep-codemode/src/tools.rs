//! Named tool dispatch and in-plan transforms.

use crate::catalog::{catalog_describe, catalog_search, catalog_summary};
use crate::session::CodeModeSession;
use serde_json::{json, Value};
use thiserror::Error;

/// Known tool names (stringly matched at the boundary; catalog is source of truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolName {
    Search,
    Find,
    Read,
    Edit,
    Semantic,
    Chain,
    Defs,
    Callers,
    Imports,
    IndexStatus,
    IndexRepo,
    FilterHits,
    Select,
    CatalogSearch,
    CatalogDescribe,
}

impl ToolName {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "search" | "code_search" => Self::Search,
            "find" | "grep" | "keyword" => Self::Find,
            "read" | "code_read" => Self::Read,
            "edit" | "code_edit" => Self::Edit,
            "semantic" => Self::Semantic,
            "chain" => Self::Chain,
            "defs" | "define" | "definition" | "definitions" => Self::Defs,
            "callers" | "references" => Self::Callers,
            "imports" => Self::Imports,
            "index_status" | "indexStatus" => Self::IndexStatus,
            "index_repo" | "indexRepo" => Self::IndexRepo,
            "filter_hits" => Self::FilterHits,
            "select" => Self::Select,
            "catalog_search" | "catalogSearch" => Self::CatalogSearch,
            "catalog_describe" | "catalogDescribe" => Self::CatalogDescribe,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Find => "find",
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Semantic => "semantic",
            Self::Chain => "chain",
            Self::Defs => "defs",
            Self::Callers => "callers",
            Self::Imports => "imports",
            Self::IndexStatus => "index_status",
            Self::IndexRepo => "index_repo",
            Self::FilterHits => "filter_hits",
            Self::Select => "select",
            Self::CatalogSearch => "catalog_search",
            Self::CatalogDescribe => "catalog_describe",
        }
    }
}

#[derive(Debug, Error)]
pub enum CallError {
    #[error("{0}")]
    UnknownTool(String),
    #[error("{0}")]
    InvalidArgs(String),
    /// The sticky session's call budget is exhausted (br-r49). Serve must
    /// answer once and stop instead of flooding identical per-call errors.
    #[error("codemode call budget exceeded (max_calls={0})")]
    BudgetExhausted(usize),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Dispatch a single tool call against a session.
pub fn call_tool(
    session: &mut CodeModeSession,
    name: &str,
    args: Value,
) -> Result<Value, CallError> {
    let tool = ToolName::parse(name).ok_or_else(|| unknown_tool(name))?;
    match tool {
        ToolName::Search => session.search(&args).map_err(CallError::from),
        ToolName::Find => session.find(&args).map_err(CallError::from),
        ToolName::Read => session.read_windows(&args).map_err(CallError::from),
        ToolName::Edit => session.edit_files(&args).map_err(CallError::from),
        ToolName::Semantic => {
            let mut a = args;
            if let Some(obj) = a.as_object_mut() {
                obj.insert("semantic_only".into(), json!(true));
            }
            session.search(&a).map_err(CallError::from)
        }
        ToolName::Chain => session.chain(&args).map_err(CallError::from),
        ToolName::Defs => {
            let symbol = require_symbol(&args)?;
            let mut a = args.clone();
            if let Some(obj) = a.as_object_mut() {
                obj.insert("query".into(), json!(format!("defs:{symbol}")));
            }
            session.search(&a).map_err(CallError::from)
        }
        ToolName::Callers => {
            let symbol = require_symbol(&args)?;
            let mut a = args.clone();
            if let Some(obj) = a.as_object_mut() {
                obj.insert("query".into(), json!(format!("callers:{symbol}")));
            }
            session.search(&a).map_err(CallError::from)
        }
        ToolName::Imports => {
            let module = require_str(&args, "module")?;
            let mut a = args.clone();
            if let Some(obj) = a.as_object_mut() {
                obj.insert("query".into(), json!(format!("imports:{module}")));
            }
            session.search(&a).map_err(CallError::from)
        }
        ToolName::IndexStatus => session.index_status(&args).map_err(CallError::from),
        ToolName::IndexRepo => session.index_repo(&args).map_err(CallError::from),
        ToolName::FilterHits => filter_hits(&args),
        ToolName::Select => select_fields(&args),
        ToolName::CatalogSearch => {
            let query = require_str(&args, "query")?;
            Ok(json!({
                "tools": catalog_search(query),
                "summary": catalog_summary(),
            }))
        }
        ToolName::CatalogDescribe => {
            let name = require_str(&args, "name")?;
            match catalog_describe(name) {
                Some(def) => Ok(serde_json::to_value(def)?),
                None => Err(CallError::InvalidArgs(format!(
                    "unknown tool in catalog: {name}"
                ))),
            }
        }
    }
}

fn require_symbol<'a>(args: &'a Value) -> Result<&'a str, CallError> {
    args.get("symbol")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("query").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CallError::InvalidArgs(
                "symbol is required. Call asgrep.defs(\"Name\") or asgrep.defs({ symbol: \"Name\" })"
                    .into(),
            )
        })
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, CallError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| match key {
            "module" => CallError::InvalidArgs(
                "module is required. Call asgrep.imports(\"os\") or asgrep.imports({ module: \"os\" })"
                    .into(),
            ),
            "query" => CallError::InvalidArgs(
                "query is required. Call asgrep.search(\"text\") or asgrep.search({ query: \"text\" })"
                    .into(),
            ),
            "name" => CallError::InvalidArgs(
                "name is required. Call asgrep.catalogDescribe(\"search\")".into(),
            ),
            _ => CallError::InvalidArgs(format!("{key} is required")),
        })
}

const KNOWN_TOOLS: &[&str] = &[
    "search",
    "find",
    "read",
    "edit",
    "semantic",
    "chain",
    "defs",
    "callers",
    "imports",
    "index_status",
    "index_repo",
    "filter_hits",
    "select",
    "catalog_search",
    "catalog_describe",
];

fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let rows = a_chars.len() + 1;
    let cols = b_chars.len() + 1;
    let mut prev = vec![0; cols];
    let mut cur = vec![0; cols];
    for (j, cell) in prev.iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..rows {
        cur[0] = i;
        for j in 1..cols {
            let cost = usize::from(a_chars[i - 1] != b_chars[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev.copy_from_slice(&cur);
    }
    prev[b_chars.len()]
}

fn suggest_tool(name: &str) -> Option<&'static str> {
    let needle = name.to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    let max_distance = (needle.len() / 3).max(1);
    let mut best: Option<(&'static str, usize)> = None;
    for candidate in KNOWN_TOOLS {
        let lower = candidate.to_ascii_lowercase();
        let distance = if lower.contains(&needle) || needle.contains(&lower) {
            1.min(edit_distance(&needle, &lower))
        } else {
            edit_distance(&needle, &lower)
        };
        if distance > max_distance {
            continue;
        }
        match best {
            Some((_, best_distance)) if distance >= best_distance => {}
            _ => best = Some((*candidate, distance)),
        }
    }
    best.map(|(name, _)| name)
}

fn unknown_tool(name: &str) -> CallError {
    let extra = suggest_tool(name)
        .map(|hint| format!(" Did you mean {hint}?"))
        .unwrap_or_default();
    CallError::UnknownTool(format!(
        "unknown tool: {name}.{extra} Use search, find, defs, callers, read, edit."
    ))
}

fn hit_array(value: &Value) -> Result<Vec<Value>, CallError> {
    if let Some(arr) = value.as_array() {
        return Ok(arr.clone());
    }
    if let Some(arr) = value.get("hits").and_then(|h| h.as_array()) {
        return Ok(arr.clone());
    }
    Err(CallError::InvalidArgs(
        "hits must be an array or an agent/capsule response with hits".into(),
    ))
}

fn filter_hits(args: &Value) -> Result<Value, CallError> {
    let hits_val = args
        .get("hits")
        .ok_or_else(|| CallError::InvalidArgs("hits is required".into()))?;
    let kind = args.get("kind").and_then(|v| v.as_str());
    let path_contains = args.get("path_contains").and_then(|v| v.as_str());
    let min_score = args.get("min_score").and_then(|v| v.as_f64());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .map(|n| n.clamp(1, ast_sgrep_core::MAX_OUTPUT_RESULTS))
        .unwrap_or(ast_sgrep_core::MAX_OUTPUT_RESULTS);

    let out: Vec<Value> = hit_array(hits_val)?
        .into_iter()
        .filter(|hit| {
            if let Some(k) = kind {
                if hit.get("kind").and_then(|v| v.as_str()) != Some(k) {
                    return false;
                }
            }
            if let Some(sub) = path_contains {
                let file = hit.get("file").and_then(|v| v.as_str()).unwrap_or("");
                if !file.contains(sub) {
                    return false;
                }
            }
            if let Some(min) = min_score {
                let score = hit.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if score < min {
                    return false;
                }
            }
            true
        })
        .take(limit)
        .collect();

    Ok(json!({
        "provider": "ast-sgrep",
        "surface": "codemode",
        "tool": "filter_hits",
        "hit_count": out.len(),
        "hits": out,
    }))
}

fn select_fields(args: &Value) -> Result<Value, CallError> {
    let value = args
        .get("value")
        .ok_or_else(|| CallError::InvalidArgs("value is required".into()))?;
    let fields = args
        .get("fields")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CallError::InvalidArgs("fields must be an array of strings".into()))?;
    let field_names: Vec<&str> = fields.iter().filter_map(|f| f.as_str()).collect();
    if field_names.is_empty() {
        return Err(CallError::InvalidArgs("fields must be non-empty".into()));
    }
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    let project = |obj: &Value| -> Value {
        let mut out = serde_json::Map::new();
        for name in &field_names {
            if let Some(v) = obj.get(*name) {
                out.insert((*name).to_string(), v.clone());
            }
        }
        Value::Object(out)
    };

    if let Some(arr) = value.as_array() {
        let mut projected: Vec<Value> = arr.iter().map(project).collect();
        if let Some(n) = limit {
            projected.truncate(n);
        }
        return Ok(json!(projected));
    }
    if value.is_object() {
        return Ok(project(value));
    }
    Err(CallError::InvalidArgs(
        "value must be an object or array of objects".into(),
    ))
}
