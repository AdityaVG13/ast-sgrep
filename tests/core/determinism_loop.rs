//! Determinism regression (6ulo): identical no-embed searches must be stable.
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::isolated_index_session;

#[test]
fn fifty_identical_searches_are_byte_stable() {
    let session = isolated_index_session();
    session.write(
        "stable.rs",
        "fn auth_refresh() { renew_credentials(); }\nfn renew_credentials() {}\n",
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        force_reindex: true,
        ..session.index_options()
    });

    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 16,
        ..session.search_options()
    });

    let first = searcher.search("auth_refresh").unwrap();
    assert!(
        !first.hits.is_empty(),
        "expected non-empty hits for determinism baseline"
    );
    let first_json = serde_json::to_string(&first).unwrap();
    for i in 0..50 {
        let next = searcher.search("auth_refresh").unwrap();
        assert_eq!(
            next.hits.len(),
            first.hits.len(),
            "hit_count drifted on iteration {i}"
        );
        let next_json = serde_json::to_string(&next).unwrap();
        assert_eq!(
            next_json, first_json,
            "JSON identity drifted on iteration {i}"
        );
    }
}
