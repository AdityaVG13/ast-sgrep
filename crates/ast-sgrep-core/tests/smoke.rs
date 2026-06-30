use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{index_sample, reopen_indexer, searcher_from};

#[test]
fn index_and_search_smoke() {
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let stats = indexed.indexer.store().status().unwrap();
    assert!(stats.file_count >= 4);
    assert!(stats.symbol_count > 0);
    assert!(stats.semantic_chunk_count > 0);

    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 16,
            ..SearchOptions::default()
        },
    );
    let callers = searcher.search("callers:process_request").unwrap();
    assert!(callers.hits.iter().any(|h| {
        h.kind == HitKind::Caller && h.callee.as_deref() == Some("process_request")
    }));
    assert!(!searcher.search("defs:auth_refresh").unwrap().hits.is_empty());
    assert!(searcher
        .search("credential renewal")
        .unwrap()
        .hits
        .iter()
        .any(|h| h.kind == HitKind::Embed));

    let mut again = reopen_indexer(&indexed, IndexOptions::default());
    assert_eq!(again.index_all().unwrap().files_indexed, 0);
}
