//! JSON plan runner: compose tools in one shot without model round-trips.
//!
//! Plans are intentionally small and JSON-native so hosts that cannot embed a
//! full JS sandbox (or prefer deterministic replay) can still get Code Mode
//! benefits. Hosted PTC runtimes (Claude / OpenAI) can instead call the same
//! tools from generated code; this runner is the local / MCP-adjacent path.

use crate::session::CodeModeSession;
use crate::tools::CallError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Step id used in `$id` / `$id.path` references.
    pub id: String,
    /// Catalog tool name.
    pub tool: String,
    /// Arguments; string values may be `$step.path` references.
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<PlanStep>,
    /// Optional return expression: `$step` or `$step.path`. Defaults to last step.
    #[serde(default, rename = "return")]
    pub return_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResult {
    pub ok: bool,
    pub steps: HashMap<String, Value>,
    #[serde(rename = "return")]
    pub return_value: Value,
    pub call_count: usize,
}

/// Execute a plan against a session. Step outputs are available to later steps
/// via `$id` or `$id.dotted.path` string refs inside args.
pub fn run_plan(session: &mut CodeModeSession, plan: &Plan) -> Result<PlanResult, CallError> {
    if plan.steps.is_empty() {
        return Err(CallError::InvalidArgs("plan.steps must be non-empty".into()));
    }
    let mut outputs: HashMap<String, Value> = HashMap::new();
    for step in &plan.steps {
        if step.id.is_empty() {
            return Err(CallError::InvalidArgs("plan step id must be non-empty".into()));
        }
        if outputs.contains_key(&step.id) {
            return Err(CallError::InvalidArgs(format!(
                "duplicate plan step id: {}",
                step.id
            )));
        }
        let args = resolve_value(&step.args, &outputs)?;
        let out = session.call(&step.tool, args)?;
        outputs.insert(step.id.clone(), out);
    }

    let return_value = if let Some(r) = &plan.return_ref {
        resolve_ref(r, &outputs)?
    } else {
        let last = plan.steps.last().expect("non-empty steps");
        outputs
            .get(&last.id)
            .cloned()
            .unwrap_or(Value::Null)
    };

    Ok(PlanResult {
        ok: true,
        steps: outputs,
        return_value,
        call_count: session.call_count(),
    })
}

/// Parse a plan from JSON.
pub fn parse_plan(value: &Value) -> Result<Plan, CallError> {
    serde_json::from_value(value.clone())
        .map_err(|e| CallError::InvalidArgs(format!("invalid plan: {e}")))
}

fn resolve_value(value: &Value, outputs: &HashMap<String, Value>) -> Result<Value, CallError> {
    match value {
        Value::String(s) if s.starts_with('$') => resolve_ref(s, outputs),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_value(item, outputs)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), resolve_value(v, outputs)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

fn resolve_ref(expr: &str, outputs: &HashMap<String, Value>) -> Result<Value, CallError> {
    let expr = expr.strip_prefix('$').unwrap_or(expr);
    if expr.is_empty() {
        return Err(CallError::InvalidArgs("empty $ref".into()));
    }
    let mut parts = expr.split('.');
    let id = parts.next().unwrap();
    let mut cur = outputs
        .get(id)
        .cloned()
        .ok_or_else(|| CallError::InvalidArgs(format!("unknown step ref: ${id}")))?;
    for part in parts {
        cur = match &cur {
            Value::Object(map) => map
                .get(part)
                .cloned()
                .ok_or_else(|| CallError::InvalidArgs(format!("missing path .{part} in ${expr}")))?,
            Value::Array(arr) => {
                let idx: usize = part.parse().map_err(|_| {
                    CallError::InvalidArgs(format!("array index required at .{part} in ${expr}"))
                })?;
                arr.get(idx).cloned().ok_or_else(|| {
                    CallError::InvalidArgs(format!("index {idx} out of range in ${expr}"))
                })?
            }
            _ => {
                return Err(CallError::InvalidArgs(format!(
                    "cannot index into non-container at .{part} in ${expr}"
                )));
            }
        };
    }
    Ok(cur)
}

/// Convenience: wrap a single tool call as a one-step plan result.
pub fn single_result(_tool: &str, value: Value) -> PlanResult {
    let mut steps = HashMap::new();
    steps.insert("main".into(), value.clone());
    PlanResult {
        ok: true,
        steps,
        return_value: value,
        call_count: 1,
    }
}

/// Example plan JSON for docs / robot discovery.
pub fn example_plan() -> Value {
    json!({
        "steps": [
            {
                "id": "seed",
                "tool": "search",
                "args": {"query": "auth refresh", "format": "capsule", "limit": 5}
            },
            {
                "id": "narrow",
                "tool": "filter_hits",
                "args": {"hits": "$seed", "path_contains": "src/", "limit": 3}
            },
            {
                "id": "graph",
                "tool": "chain",
                "args": {"query": "$narrow.hits.0.symbol", "max_depth": 2}
            },
            {
                "id": "out",
                "tool": "select",
                "args": {
                    "value": "$graph",
                    "fields": ["query", "node_count", "edge_count", "nodes", "edges"]
                }
            }
        ],
        "return": "$out"
    })
}
