//! Provider-shaped tool definitions for Anthropic, OpenAI, and Cloudflare hosts.

mod anthropic;
mod cloudflare;
mod openai;

pub use anthropic::{anthropic_discovery_tools, anthropic_tools};
pub use cloudflare::cloudflare_connector;
pub use openai::openai_tools;

use crate::catalog::{tool_catalog, ToolDef};
use serde_json::{json, Value};

/// Shared envelope describing this Code Mode surface for any host.
pub fn surface_manifest() -> Value {
    json!({
        "provider": "ast-sgrep",
        "surface": "codemode",
        "version": env!("CARGO_PKG_VERSION"),
        "pattern": {
            "cloudflare": "https://developers.cloudflare.com/agents/tools/codemode/",
            "anthropic": "https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling",
            "openai": "https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling"
        },
        "defaults": {
            "format": "capsule",
            "progressive_discovery": ["catalog_search", "catalog_describe"]
        },
        "tool_count": tool_catalog().len(),
    })
}

pub(crate) fn schema_for(def: &ToolDef) -> Value {
    def.input_schema.clone()
}
