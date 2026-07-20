use ast_sgrep_core::search::HitKind; use ast_sgrep_core::{SearchHit, SearchResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum OutputFormat {
    Native, GitHub, GitLab, Agent, AgentCapsule,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "native" | "asgrep" => Some(Self::Native), "github" | "gh" => Some(Self::GitHub), "gitlab" | "gl" => Some(Self::GitLab),
            "agent" | "llm" | "ai" => Some(Self::Agent), "agent-capsule" | "capsule" => Some(Self::AgentCapsule), _ => None,
        }
    }
}

pub fn format_response(response: &SearchResponse, format: OutputFormat) -> serde_json::Value { format_response_with(response, format, 0) }

pub fn format_response_with(
    response: &SearchResponse, format: OutputFormat, excerpt_lines: usize,
) -> serde_json::Value {
    match format {
        OutputFormat::Native => serde_json::to_value(response)
            .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() })),
        OutputFormat::GitHub => to_github_json(response), OutputFormat::GitLab => to_gitlab_json(response), OutputFormat::Agent => to_agent_json(response), OutputFormat::AgentCapsule => to_agent_capsule_json(response, excerpt_lines),
    }
}

fn basename(file: &str) -> &str { file.rsplit('/').next().unwrap_or(file) }

fn hit_symbol(hit: &SearchHit) -> Option<&str> {
    hit.symbol
        .as_deref() .or(hit.callee.as_deref()) .or(hit.caller.as_deref())
}

/// GitHub-like page. `total_count` is the returned page size (no corpus-wide count).
pub fn to_github_json(response: &SearchResponse) -> serde_json::Value {
    let items: Vec<_> = response
        .hits .iter() .map(|h| {
            serde_json::json!({
                "name": basename(&h.file), "path": h.file, "score": h.score, "language": h.language, "text_matches": [{
                    "fragment": h.excerpt, "matches": [{"text": hit_symbol(h).unwrap_or(""), "indices": [0]}]
                }], "metadata": {
                    "kind": h.kind.as_str(), "line_start": h.line_start, "line_end": h.line_end, "symbol": h.symbol, "caller": h.caller, "callee": h.callee,
                }
            })
        }) .collect();
    serde_json::json!({
        "total_count": items.len(), "incomplete_results": response.limit > 0 && items.len() >= response.limit, "items": items, "query": response.query, "provider": "ast-sgrep"
    })
}

/// GitLab-like shape; `ref` is always HEAD, `project_id` always null.
pub fn to_gitlab_json(response: &SearchResponse) -> serde_json::Value {
    let data: Vec<_> = response
        .hits .iter() .map(|h| {
            serde_json::json!({
                "basename": basename(&h.file), "data": h.excerpt, "path": h.file, "filename": h.file, "ref": "HEAD", "startline": h.line_start, "project_id": null, "meta": {
                    "kind": h.kind.as_str(), "score": h.score, "language": h.language, "line_end": h.line_end, "symbol": h.symbol, "caller": h.caller, "callee": h.callee,
                }
            })
        }) .collect();
    serde_json::json!({ "data": data, "query": response.query, "provider": "ast-sgrep" })
}

pub fn to_agent_json(response: &SearchResponse) -> serde_json::Value {
    let hits: Vec<_> = response
        .hits .iter() .map(|h| {
            let mut follow_ups = Vec::new(); if let Some(sym) = hit_symbol(h) {
                follow_ups.push(format!("defs:{sym}")); follow_ups.push(format!("callers:{sym}"));
            } serde_json::json!({
                "kind": h.kind.as_str(), "semantic": h.kind == HitKind::Embed, "score": h.score, "file": h.file, "lines": { "start": h.line_start, "end": h.line_end },
                "symbol": h.symbol, "caller": h.caller, "callee": h.callee, "language": h.language, "excerpt": h.excerpt, "follow_up_queries": follow_ups,
            })
        }) .collect();
    let has_semantic = hits.iter().any(|h| h["semantic"] == true); let top_symbol = response
        .hits .first() .and_then(|h| h.symbol.clone().or(h.callee.clone()));
    let mut suggested = Vec::new(); if has_semantic {
        suggested.push(format!("asgrep semantic \"{}\"", response.query));
    } if let Some(sym) = top_symbol {
        suggested.push(format!("defs:{sym}")); suggested.push(format!("callers:{sym}"));
    } suggested.push("pattern: (delegate to ast-grep for structural search)".into()); suggested.push("rg (use ripgrep for raw text scan)".into()); serde_json::json!({
        "provider": "ast-sgrep", "version": env!("CARGO_PKG_VERSION"), "query": response.query, "limit": response.limit, "hit_count": hits.len(), "read_bytes_estimate": response.read_bytes_estimate,
        "returned_excerpt_bytes": response.returned_excerpt_bytes, "prevented_read_bytes": response.prevented_read_bytes, "has_semantic_hits": has_semantic,
        "stack_hint": "Use ast-sgrep for intent/navigation; ast-grep for patterns; ripgrep for grep.", "suggested_next": suggested, "hits": hits,
    })
}

const PREVIEW_MAX_CHARS: usize = 120;

pub fn to_agent_capsule_json(response: &SearchResponse, excerpt_lines: usize) -> serde_json::Value {
    let hits: Vec<_> = response
        .hits .iter() .map(|h| {
            let mut cap = serde_json::json!({
                "file": h.file, "lines": { "start": h.line_start, "end": h.line_end }, "symbol": h.symbol, "caller": h.caller, "callee": h.callee,
                "kind": h.kind.as_str(), "score": h.score, "preview": preview_line(&h.excerpt), "ref": format!("{}#L{}-L{}", h.file, h.line_start, h.line_end),
            }); if excerpt_lines > 0 {
                let body: Vec<_> = h.excerpt.lines().take(excerpt_lines).collect(); cap["excerpt"] = serde_json::Value::String(body.join("\n"));
            } cap
        }) .collect();
    let returned_excerpt_bytes: u64 = hits
        .iter() .filter_map(|h| {
            h.get("excerpt")
                .or_else(|| h.get("preview")) .and_then(serde_json::Value::as_str)
        }) .map(|e| e.len() as u64) .sum();
    serde_json::json!({
        "provider": "ast-sgrep", "mode": "capsule", "query": response.query, "limit": response.limit, "hit_count": hits.len(), "read_bytes_estimate": response.read_bytes_estimate,
        "returned_excerpt_bytes": returned_excerpt_bytes, "prevented_read_bytes": response.prevented_read_bytes,
        "expand_hint": "re-run with --excerpt-lines N for bodies, or read each ref span with your file reader (path + line window)", "hits": hits,
    })
}

fn preview_line(excerpt: &str) -> String {
    let line = excerpt
        .lines() .map(str::trim) .find(|l| !l.is_empty()) .unwrap_or("");
    if line.chars().count() <= PREVIEW_MAX_CHARS {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(PREVIEW_MAX_CHARS).collect::<String>())
    }
}

pub mod agent {
    pub use super::{to_agent_capsule_json, to_agent_json};
} pub mod github {
    pub use super::to_github_json;
} pub mod gitlab {
    pub use super::to_gitlab_json;
}
