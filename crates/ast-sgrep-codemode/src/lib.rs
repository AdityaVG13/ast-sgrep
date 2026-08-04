//! Code Mode / programmatic tool-calling for ast-sgrep.
//!
//! This crate is the **execution-oriented** agent surface: typed tools, a warm
//! session, progressive catalog discovery, and a JSON plan runner that composes
//! multiple search ops without a model round-trip between each call.
//!
//! It is intentionally separate from `ast-sgrep-mcp` (transport) and
//! `ast-sgrep-cli` (human/agent CLI). Those can consume this façade later.
//!
//! # Pattern
//!
//! Matches Cloudflare Code Mode, Anthropic programmatic tool calling, and
//! OpenAI programmatic tool calling: the model writes a compact plan (or code
//! that would call these tools); intermediate results stay in-process; only the
//! shaped return value needs to re-enter the model context.
//!
//! # Quick start
//!
//! ```ignore
//! use ast_sgrep_codemode::{CodeModeSession, SessionConfig};
//! use serde_json::json;
//!
//! let mut session = CodeModeSession::new(SessionConfig::default())?;
//! let hits = session.call("search", json!({
//!     "query": "auth refresh",
//!     "format": "capsule",
//!     "limit": 5
//! }))?;
//! ```

pub mod adapters;
pub mod catalog;
pub mod plan;
pub mod session;
pub mod tools;

pub use adapters::{
    anthropic_discovery_tools, anthropic_tools, cloudflare_connector, openai_tools, surface_manifest,
};
pub use catalog::{catalog_describe, catalog_search, tool_catalog, ToolDef, ToolKind};
pub use plan::{example_plan, parse_plan, run_plan, Plan, PlanResult, PlanStep};
pub use session::{CodeModeSession, SessionConfig};
pub use tools::{CallError, ToolName};

/// Crate version string (matches workspace package version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
