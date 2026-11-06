//! OpenAI Responses API shapes for programmatic tool calling.

use crate::catalog::tool_catalog;
use serde_json::{json, Value};

/// Hosted programmatic tool-calling marker for the Responses API.
pub const PROGRAMMATIC_TOOL: &str = "programmatic_tool_calling";

/// Tools array fragment: hosted PTC enablement + function tools with
/// `allowed_callers` so generated JS can invoke them inside the V8 runtime.
pub fn openai_tools() -> Value {
    let mut tools = vec![json!({
        "type": PROGRAMMATIC_TOOL
    })];

    for def in tool_catalog() {
        if def.is_discovery() {
            continue;
        }
        let mut tool = json!({
            "type": "function",
            "name": def.name,
            "description": def.description,
            "parameters": def.input_schema.clone(),
            "strict": true,
        });
        if def.read_only {
            tool["allowed_callers"] = json!([PROGRAMMATIC_TOOL]);
        }
        tools.push(tool);
    }

    json!(tools)
}
