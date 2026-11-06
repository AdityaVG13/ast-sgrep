//! Cloudflare Agents Code Mode connector-style descriptor.
//!
//! Cloudflare Code Mode exposes configured tools as typed methods and supports
//! progressive discovery via search/describe. This adapter emits a JSON
//! connector manifest hosts can load; it does not embed the Agents SDK.

use crate::catalog::tool_catalog;
use serde_json::{json, Value};

/// Connector manifest: methods map 1:1 to catalog tools.
pub fn cloudflare_connector() -> Value {
    let methods: Vec<Value> = tool_catalog()
        .into_iter()
        .map(|def| {
            json!({
                "name": def.name,
                "description": def.description,
                "kind": def.kind,
                "readOnly": def.read_only,
                "parameters": def.input_schema.clone(),
                "returns": {
                    "description": "JSON value (agent, capsule, chain, status, or transform result)"
                }
            })
        })
        .collect();

    json!({
        "name": "ast-sgrep",
        "version": env!("CARGO_PKG_VERSION"),
        "mode": "codemode",
        "progressiveDiscovery": {
            "search": "catalog_search",
            "describe": "catalog_describe"
        },
        "methods": methods,
    })
}
