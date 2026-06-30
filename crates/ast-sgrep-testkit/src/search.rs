use ast_sgrep_core::search::{HitKind, SearchHit};
use ast_sgrep_core::{SearchResponse, Searcher};

pub fn top_symbols(response: &SearchResponse) -> Vec<String> {
    response
        .hits
        .iter()
        .filter_map(|h| h.symbol.clone())
        .collect()
}

pub fn assert_query_finds(searcher: &Searcher, query: &str, pred: impl Fn(&SearchHit) -> bool) {
    let response = searcher.search(query).expect("search");
    assert!(
        response.hits.iter().any(pred),
        "query {query:?} expected a matching hit; got {:?}",
        response.hits
    );
}

pub fn assert_excerpt_contains(searcher: &Searcher, query: &str, needle: &str) {
    assert_query_finds(searcher, query, |h| h.excerpt.contains(needle));
}

pub fn assert_has_embed_hits(searcher: &Searcher, query: &str) {
    let response = searcher.search(query).expect("search");
    assert!(
        response.hits.iter().any(|h| h.kind == HitKind::Embed)
            || response
                .hits
                .iter()
                .any(|h| h.excerpt.contains("auth_refresh") || h.excerpt.contains("authRefresh")),
        "expected embed or semantic excerpt hits for {query:?}"
    );
}

pub fn assert_no_embed_hits(searcher: &Searcher, query: &str) {
    let response = searcher.search(query).expect("search");
    assert!(
        !response.hits.iter().any(|h| h.kind == HitKind::Embed),
        "embed pass should be off for {query:?}"
    );
}
