//! Named tool dispatch and in-plan transforms.

use crate::catalog::{catalog_describe, catalog_search, catalog_summary};
use crate::session::CodeModeSession;
use serde_json::{json, Value};
use thiserror::Error;

/// Known tool names (stringly matched at the boundary; catalog is source of truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolName {
    Search,
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
            "semantic" => Self::Semantic,
            "chain" => Self::Chain,
            "defs" => Self::Defs,
            "callers" => Self::Callers,
            "imports" => Self::Imports,
            "index_status" => Self::IndexStatus,
            "index_repo" => Self::IndexRepo,
            "filter_hits" => Self::FilterHits,
            "select" => Self::Select,
            "catalog_search" => Self::CatalogSearch,
            "catalog_describe" => Self::CatalogDescribe,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
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
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("{0}")]
    InvalidArgs(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Dispatch a single tool call against a session.
pub fn call_tool(session: &mut CodeModeSession, name: &str, args: Value) -> Result<Value, CallError> {
    let tool = ToolName::parse(name).ok_or_else(|| CallError::UnknownTool(name.to_string()))?;
    match tool {
        ToolName::Search => session.search(&args).map_err(CallError::from),
        ToolName::Semantic => {
            let mut a = args;
            if let Some(obj) = a.as_object_mut() {
                obj.insert("semantic_only".into(), json!(true));
            }
            session.search(&a).map_err(CallError::from)
        }
        ToolName::Chain => session.chain(&args).map_err(CallError::from),
        ToolName::Defs => {
            let symbol = require_str(&args, "symbol")?;
            let mut a = args.clone();
            if let Some(obj) = a.as_object_mut() {
                obj.insert("query".into(), json!(format!("defs:{symbol}")));
            }
            session.search(&a).map_err(CallError::from)
        }
        ToolName::Callers => {
            let symbol = require_str(&args, "symbol")?;
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
                None => Err(CallError::InvalidArgs(format!("unknown tool in catalog: {name}"))),
            }
        }
    }
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, CallError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| CallError::InvalidArgs(format!("{key} is required")))
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
        .unwrap_or(usize::MAX);

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
