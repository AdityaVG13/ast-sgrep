use ast_sgrep_core::search::{HitKind, SearchHit, SearchResponse};

pub fn sample_search_response() -> SearchResponse {
    SearchResponse {
        query: "auth".into(),
        limit: 16,
        hits: vec![SearchHit::span(
            HitKind::Def,
            "src/main.rs".into(),
            1,
            3,
            10.0,
            "fn auth_refresh() {}".into(),
            Some("auth_refresh".into()),
            Some("rust".into()),
        )],
    }
}

pub fn sample_embed_response() -> SearchResponse {
    SearchResponse {
        query: "credential renewal".into(),
        limit: 16,
        hits: vec![SearchHit::span(
            HitKind::Embed,
            "src/main.rs".into(),
            19,
            22,
            3.5,
            "fn auth_refresh() {}".into(),
            Some("auth_refresh".into()),
            None,
        )],
    }
}
