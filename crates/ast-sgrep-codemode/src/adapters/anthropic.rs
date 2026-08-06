//! Anthropic Messages API tool shapes for programmatic tool calling.
//!
//! Eligible tools set `allowed_callers` to the code-execution tool so Claude can
//! invoke them from the sandbox instead of via model round-trips.

use crate::adapters::schema_for;
use crate::catalog::tool_catalog;
use serde_json::{json, Value};

/// Default code-execution tool type Claude PTC expects.
pub const CODE_EXECUTION_CALLER: &str = "code_execution_20260120";

/// Tool list suitable for `tools` in a Messages request, including the
/// code_execution tool and ast-sgrep tools marked for programmatic callers.
pub fn anthropic_tools() -> Value {
    let mut tools = vec![json!({
        "type": CODE_EXECUTION_CALLER,
        "name": "code_execution"
    })];

    for def in tool_catalog() {
        // Skip meta discovery tools from the always-on list; hosts can add them
        // when progressive discovery is desired.
        if def.name.starts_with("catalog_") {
            continue;
        }
        let mut tool = json!({
            "name": def.name,
            "description": def.description,
            "input_schema": schema_for(&def),
        });
        if def.read_only {
            tool["allowed_callers"] = json!([CODE_EXECUTION_CALLER]);
        }
        tools.push(tool);
    }

    json!(tools)
}

/// Progressive-discovery tools only (catalog_search / catalog_describe).
pub fn anthropic_discovery_tools() -> Value {
    let tools: Vec<Value> = tool_catalog()
        .into_iter()
        .filter(|d| d.name.starts_with("catalog_"))
        .map(|def| {
            json!({
                "name": def.name,
                "description": def.description,
                "input_schema": schema_for(&def),
                "allowed_callers": [CODE_EXECUTION_CALLER],
            })
        })
        .collect();
    json!(tools)
}
