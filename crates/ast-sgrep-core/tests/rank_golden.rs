use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{index_sample, load_ranking_fixture, searcher_from};

#[test]
fn ranking_golden_cases() {
    let fixture = load_ranking_fixture();
    assert_eq!(fixture.fixture, "sample");

    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 16,
            use_embed: true,
            ..SearchOptions::default()
        },
    );

    for case in &fixture.cases {
        let response = searcher
            .search(&case.query)
            .unwrap_or_else(|e| panic!("search {:?} failed: {e}", case.query));
        ast_sgrep_testkit::assert_ranking_case(case, &response);
    }
}
