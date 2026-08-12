//! Code Mode / programmatic tool-calling for ast-sgrep (Rust side).
//!
//! # What this crate is
//!
//! Catalog + in-process session + plan runner + host adapters for the Code Mode
//! pattern (model writes code that orchestrates search). Sibling to MCP — this
//! crate must **never** depend on `ast-sgrep-mcp`, and MCP must never depend on
//! this crate. Both sit on `ast-sgrep-core` only.
//!
//! # Root policy (R-CM-ROOT-POLICY option A)
//!
//! Tool `root` args are jailed under `SessionConfig.root` (canonicalize +
//! `Path::starts_with`), matching MCP `sandbox_root`. Foreign roots fail closed
//! with `escapes configured workspace`. NAPI inherits the same Session contract.
//!
//! Pi's primary agent surface is the **JS sandbox** in
//! `packages/pi/extension/src/codemode/` (`asgrep` tool). This Rust
//! crate serves Rust hosts and emits Anthropic/OpenAI/Cloudflare-shaped tool
//! definitions for hosts that already provide a code-execution sandbox.
//!
//! # Pattern
//!
//! Matches Cloudflare Code Mode, Anthropic programmatic tool calling, and
//! OpenAI programmatic tool calling: compose tools in executable code; keep
//! intermediates out of the model; return a shaped result.
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
pub mod batch;
pub mod catalog;
pub mod plan;
pub mod session;
pub mod tools;

pub use adapters::{
    anthropic_discovery_tools, anthropic_tools, cloudflare_connector, openai_tools, surface_manifest,
};
pub use batch::{
    run_batch, run_serve, BatchCall, BatchCallResult, BatchRequest, BatchResponse, ParallelMode,
    ServeRequest, ServeResponse, MAX_BATCH_CALLS,
};
pub use catalog::{catalog_describe, catalog_search, tool_catalog, ToolDef, ToolKind};
pub use plan::{example_plan, parse_plan, run_plan, Plan, PlanResult, PlanStep};
pub use session::{CodeModeSession, SessionConfig};
pub use tools::{CallError, ToolName};

/// Crate version string (matches workspace package version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
