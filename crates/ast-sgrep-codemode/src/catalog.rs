//! Typed tool catalog with JSON Schema for PTC / Code Mode hosts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;

/// High-level tool roles for progressive discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Retrieval / navigation over the index.
    Search,
    /// Index lifecycle (status / build).
    Index,
    /// In-plan transforms that never touch the index.
    Transform,
    /// Meta: discover tools themselves.
    Catalog,
}

/// One callable tool: name, docs, JSON Schema, and PTC-oriented metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub kind: ToolKind,
    /// JSON Schema object for arguments.
    pub input_schema: Value,
    /// Whether hosts should prefer capsule-sized outputs by default.
    pub capsule_default: bool,
    /// Safe to call from code-execution sandboxes without human approval.
    pub read_only: bool,
}

/// Owned shadow of [`ToolDef`] for one-time JSON deserialization: `&'static str`
/// fields cannot borrow from a parsed buffer, so the const below is parsed
/// once, the two strings per tool are leaked once, and calls clone the static.
#[derive(Deserialize)]
struct OwnedToolDef {
    name: String,
    description: String,
    kind: ToolKind,
    input_schema: Value,
    capsule_default: bool,
    read_only: bool,
}

/// Checked-in catalog. This was 300 lines of `json!` macro expansion (53KiB of
/// .text for static data); the same bytes as JSON cost ~12KiB of rodata plus
/// one parse. To regenerate after editing, dump from a trusted binary:
/// `codemode-batch --requests <one catalog_describe per tool>`, then take
/// `[.results[].value]`. Key order is irrelevant (serde_json::Map is
/// BTreeMap-backed: keys always serialize sorted).
const CATALOG_JSON: &str = include_str!("catalog_data.json");

static CATALOG: OnceLock<Vec<ToolDef>> = OnceLock::new();

/// Full catalog exposed to Code Mode / PTC runtimes.
pub fn tool_catalog() -> Vec<ToolDef> {
    CATALOG
        .get_or_init(|| {
            let owned: Vec<OwnedToolDef> = serde_json::from_str(CATALOG_JSON)
                .expect("checked-in catalog_data.json must parse");
            owned
                .into_iter()
                .map(|t| ToolDef {
                    name: Box::leak(t.name.into_boxed_str()),
                    description: Box::leak(t.description.into_boxed_str()),
                    kind: t.kind,
                    input_schema: t.input_schema,
                    capsule_default: t.capsule_default,
                    read_only: t.read_only,
                })
                .collect()
        })
        .clone()
}

/// Keyword search over the catalog (progressive discovery).
pub fn catalog_search(query: &str) -> Vec<ToolDef> {
    let q = query.to_ascii_lowercase();
    let terms: Vec<&str> = q.split_whitespace().filter(|t| !t.is_empty()).collect();
    tool_catalog()
        .into_iter()
        .filter(|t| {
            if terms.is_empty() {
                return true;
            }
            let hay = format!(
                "{} {} {:?}",
                t.name,
                t.description.to_ascii_lowercase(),
                t.kind
            )
            .to_ascii_lowercase();
            terms.iter().any(|term| hay.contains(term))
        })
        .collect()
}

/// Full definition for one tool, if present.
pub fn catalog_describe(name: &str) -> Option<ToolDef> {
    tool_catalog().into_iter().find(|t| t.name == name)
}

/// Compact catalog listing for prompts / adapters.
pub fn catalog_summary() -> Value {
    json!({
        "provider": "ast-sgrep",
        "surface": "codemode",
        "version": env!("CARGO_PKG_VERSION"),
        "tools": tool_catalog().iter().map(|t| json!({
            "name": t.name,
            "kind": t.kind,
            "read_only": t.read_only,
            "capsule_default": t.capsule_default,
            "description": t.description,
        })).collect::<Vec<_>>(),
    })
}
