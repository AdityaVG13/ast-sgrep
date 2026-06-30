use ast_sgrep_core::search::{HitKind, SearchHit, SearchResponse};

/// Minimal search response for plugin format smoke tests.
pub fn sample_search_response() -> SearchResponse {
    SearchResponse {
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
    }
}

/// Semantic embed hit sample for agent-format tests.
pub fn sample_embed_response() -> SearchResponse {
    SearchResponse {
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
    }
}
