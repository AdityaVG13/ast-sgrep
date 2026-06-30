//! LLM/agent-optimized JSON — compact, actionable, with follow-up query hints.

use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::SearchResponse;

/// Convert hits to an agent-friendly JSON shape for tool-calling pipelines.
pub fn to_agent_json(response: &SearchResponse) -> serde_json::Value {
    let hits: Vec<serde_json::Value> = response
        .hits
        .iter()
        .map(|hit| {
            let semantic = hit.kind == HitKind::Embed;
            let symbol = hit
                .symbol
                .as_deref()
                .or(hit.callee.as_deref())
                .or(hit.caller.as_deref());

            let mut follow_ups: Vec<String> = Vec::new();
            if let Some(sym) = symbol {
                follow_ups.push(format!("defs:{sym}"));
                follow_ups.push(format!("callers:{sym}"));
            }

            serde_json::json!({
                "kind": hit.kind.as_str(),
                "semantic": semantic,
                "score": hit.score,
                "file": hit.file,
                "lines": { "start": hit.line_start, "end": hit.line_end },
                "symbol": hit.symbol,
                "caller": hit.caller,
                "callee": hit.callee,
                "language": hit.language,
                "excerpt": hit.excerpt,
                "follow_up_queries": follow_ups,
            })
        })
        .collect();

    let has_semantic = hits.iter().any(|h| h["semantic"] == true);
    let top_symbol = response
        .hits
        .first()
        .and_then(|h| h.symbol.clone().or(h.callee.clone()));

    let mut suggested: Vec<String> = Vec::new();
    if has_semantic {
        suggested.push(format!("asgrep semantic \"{}\"", response.query));
    }
    if let Some(sym) = top_symbol {
        suggested.push(format!("defs:{sym}"));
        suggested.push(format!("callers:{sym}"));
    }
    suggested.push("pattern: (delegate to ast-grep for structural search)".into());
    suggested.push("rg (use ripgrep for raw text scan)".into());

    serde_json::json!({
        "provider": "ast-sgrep",
        "version": "1.0.0-alpha",
        "query": response.query,
        "limit": response.limit,
        "hit_count": hits.len(),
        "has_semantic_hits": has_semantic,
        "stack_hint": "Use ast-sgrep for intent/navigation; ast-grep for patterns; ripgrep for grep.",
        "suggested_next": suggested,
        "hits": hits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_sgrep_core::search::{HitKind, SearchHit};

    #[test]
    fn agent_json_includes_follow_ups_and_semantic_flag() {
        let response = SearchResponse {
            query: "credential renewal".into(),
            limit: 16,
            hits: vec![SearchHit {
                kind: HitKind::Embed,
                file: "src/main.rs".into(),
                line_start: 19,
                line_end: 22,
                symbol: Some("auth_refresh".into()),
                caller: None,
                callee: None,
                language: Some("rust".into()),
                score: 3.5,
                excerpt: "fn auth_refresh() {}".into(),
            }],
        };
        let json = to_agent_json(&response);
        assert_eq!(json["has_semantic_hits"], true);
        assert_eq!(json["hits"][0]["semantic"], true);
        assert!(json["hits"][0]["follow_up_queries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q.as_str() == Some("defs:auth_refresh")));
    }
}
