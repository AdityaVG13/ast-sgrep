use ast_sgrep_core::SearchResponse;

/// Convert ast-sgrep hits to GitLab code search API JSON shape.
pub fn to_gitlab_json(response: &SearchResponse) -> serde_json::Value {
    let data: Vec<serde_json::Value> = response
        .hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "basename": hit.file.rsplit('/').next().unwrap_or(&hit.file),
                "data": hit.excerpt,
                "path": hit.file,
                "filename": hit.file,
                "ref": "HEAD",
                "startline": hit.line_start,
                "project_id": null,
                "meta": {
                    "kind": hit.kind.as_str(),
                    "score": hit.score,
                    "language": hit.language,
                    "line_end": hit.line_end,
                    "symbol": hit.symbol,
                    "caller": hit.caller,
                    "callee": hit.callee,
                }
            })
        })
        .collect();

    serde_json::json!({
        "data": data,
        "query": response.query,
        "provider": "ast-sgrep"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_sgrep_testkit::sample_search_response;

    #[test]
    fn gitlab_shape_has_data_array() {
        let json = to_gitlab_json(&sample_search_response());
        assert_eq!(json["data"].as_array().unwrap().len(), 1);
        assert_eq!(json["data"][0]["startline"], 1);
    }
}
