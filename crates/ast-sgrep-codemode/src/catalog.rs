//! Typed tool catalog with JSON Schema for PTC / Code Mode hosts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

const ROOT_ARG_DESC: &str =
    "Optional subdirectory under the session workspace root (foreign paths are refused)";

/// Full catalog exposed to Code Mode / PTC runtimes.
pub fn tool_catalog() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "search",
            description: "Hybrid code search (lexical + symbols + call graph + semantic). Supports defs:, callers:, imports:, pattern:, literal:, regex:, word: prefixes and natural-language queries.",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "format": {"type": "string", "enum": ["agent", "capsule"], "default": "capsule"},
                    "excerpt_lines": {"type": "integer", "minimum": 0, "description": "Inline up to N excerpt lines in capsule mode"},
                    "semantic_only": {"type": "boolean", "default": false}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "find",
            description: "Lexical / identifier lookup (word:). Faster than hybrid search when you already know the token. Prefixed queries (defs:, callers:, blast:, literal:, regex:, pattern:) pass through. blast:Symbol reverse-walks callers; blast:path uses imports.",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Exact token or prefixed query"},
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "format": {"type": "string", "enum": ["agent", "capsule"], "default": "capsule"},
                    "excerpt_lines": {"type": "integer", "minimum": 0}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "read",
            description: "Batched line windows from the index (file_lines), with disk fallback. Prefer one read({ refs }) over N calls. Caps at 32 windows.",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "start": {"type": "integer", "minimum": 1},
                    "end": {"type": "integer", "minimum": 1},
                    "ref": {"type": "string", "description": "file#Lstart-Lend"},
                    "refs": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                {"type": "string"},
                                {"type": "object", "properties": {
                                    "path": {"type": "string"},
                                    "start": {"type": "integer"},
                                    "end": {"type": "integer"},
                                    "ref": {"type": "string"}
                                }}
                            ]
                        }
                    },
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "context_lines": {"type": "integer", "minimum": 0},
                    "max_chars": {"type": "integer", "minimum": 1}
                },
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "edit",
            description: "Unique string replace jailed to the session root, then targeted reindex of touched paths. oldText must match exactly once. Serial with other mutations.",
            kind: ToolKind::Index,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "oldText": {"type": "string"},
                    "newText": {"type": "string"},
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"},
                                "oldText": {"type": "string"},
                                "newText": {"type": "string"}
                            },
                            "required": ["path", "oldText", "newText"]
                        }
                    },
                    "root": {"type": "string", "description": ROOT_ARG_DESC}
                },
                "additionalProperties": false
            }),
            capsule_default: false,
            read_only: false,
        },
        ToolDef {
            name: "semantic",
            description: "Semantic/embed pass only. Prefer when query words may not appear in source (e.g. credential renewal → auth_refresh).",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "format": {"type": "string", "enum": ["agent", "capsule"], "default": "capsule"},
                    "excerpt_lines": {"type": "integer", "minimum": 0}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "chain",
            description: "Expand a seed query into a callers/callees/imports neighborhood graph.",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "max_depth": {"type": "integer", "minimum": 1, "maximum": 8, "default": 2},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100},
                    "top_n": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            capsule_default: false,
            read_only: true,
        },
        ToolDef {
            name: "defs",
            description: "Definition lookup for a symbol (shorthand for search with defs: prefix).",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "symbol": {"type": "string"},
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "format": {"type": "string", "enum": ["agent", "capsule"], "default": "capsule"}
                },
                "required": ["symbol"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "callers",
            description: "Caller lookup for a symbol (shorthand for search with callers: prefix).",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "symbol": {"type": "string"},
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "format": {"type": "string", "enum": ["agent", "capsule"], "default": "capsule"}
                },
                "required": ["symbol"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "imports",
            description: "Import lookup (shorthand for search with imports: prefix).",
            kind: ToolKind::Search,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "module": {"type": "string"},
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "format": {"type": "string", "enum": ["agent", "capsule"], "default": "capsule"}
                },
                "required": ["module"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "index_status",
            description: "Show index statistics for a project root (files, symbols, embed backend).",
            kind: ToolKind::Index,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "root": {"type": "string", "description": ROOT_ARG_DESC}
                },
                "additionalProperties": false
            }),
            capsule_default: false,
            read_only: true,
        },
        ToolDef {
            name: "index_repo",
            description: "Build or incrementally update the .asgrep index. Pass known changed paths for a targeted update; use force=true for a full rebuild.",
            kind: ToolKind::Index,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "root": {"type": "string", "description": ROOT_ARG_DESC},
                    "force": {"type": "boolean", "default": false},
                    "paths": {
                        "type": "array",
                        "items": {"type": "string", "minLength": 1},
                        "minItems": 1,
                        "maxItems": 1024,
                        "description": "Known created, changed, or deleted paths under root"
                    }
                },
                "additionalProperties": false
            }),
            capsule_default: false,
            read_only: false,
        },
        ToolDef {
            name: "filter_hits",
            description: "Filter a previous search/capsule JSON by kind, path substring, or minimum score. Keeps intermediate work out of the model context.",
            kind: ToolKind::Transform,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "hits": {"type": "array", "description": "Hit array or full agent/capsule response"},
                    "kind": {"type": "string"},
                    "path_contains": {"type": "string"},
                    "min_score": {"type": "number"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500}
                },
                "required": ["hits"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "select",
            description: "Project fields from a JSON value (object or array of objects). Return only what the model needs.",
            kind: ToolKind::Transform,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "value": {"description": "Any JSON value"},
                    "fields": {"type": "array", "items": {"type": "string"}},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500}
                },
                "required": ["value", "fields"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "catalog_search",
            description: "Progressive discovery: find tools by keyword (Cloudflare-style codemode.search).",
            kind: ToolKind::Catalog,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Keyword(s) matched against name/description/kind"}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
        ToolDef {
            name: "catalog_describe",
            description: "Progressive discovery: return full schema for one tool (Cloudflare-style codemode.describe).",
            kind: ToolKind::Catalog,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            capsule_default: true,
            read_only: true,
        },
    ]
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
