use ast_sgrep_core::SearchResponse;

/// Convert ast-sgrep hits to GitHub code search API JSON shape.
pub fn to_github_json(response: &SearchResponse) -> serde_json::Value {
    let items: Vec<serde_json::Value> = response
        .hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "name": hit.file.rsplit('/').next().unwrap_or(&hit.file),
                "path": hit.file,
                "score": hit.score,
                "language": hit.language,
                "text_matches": [{
                    "fragment": hit.excerpt,
                    "matches": [{
                        "text": hit.symbol.as_deref()
                            .or(hit.callee.as_deref())
                            .unwrap_or(""),
                        "indices": [0]
                    }]
                }],
                "metadata": {
                    "kind": hit.kind.as_str(),
                    "line_start": hit.line_start,
                    "line_end": hit.line_end,
                    "caller": hit.caller,
                    "callee": hit.callee,
                }
            })
        })
        .collect();

    serde_json::json!({
        "total_count": items.len(),
        "incomplete_results": false,
        "items": items,
        "query": response.query,
        "provider": "ast-sgrep"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_sgrep_core::search::{HitKind, SearchHit};

    #[test]
    fn github_shape_has_items() {
        let response = SearchResponse {
            query: "auth".into(),
            limit: 16,
            hits: vec![SearchHit {
                kind: HitKind::Def,
                file: "src/main.rs".into(),
                line_start: 1,
                line_end: 3,
                symbol: Some("auth_refresh".into()),
                caller: None,
                callee: None,
                language: Some("rust".into()),
                score: 10.0,
                excerpt: "fn auth_refresh() {}".into(),
            }],
        };
        let json = to_github_json(&response);
        assert_eq!(json["total_count"], 1);
        assert_eq!(json["items"][0]["path"], "src/main.rs");
    }
}
