use ast_sgrep_core::search::HitSignal;
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use ast_sgrep_testkit::index_sample;
use std::collections::HashSet;

#[test]
fn hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages() {
    let indexed = index_sample(IndexOptions {
        embed_semantic: true,
        ..IndexOptions::default()
    });
    let searcher = Searcher::new(SearchOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        limit: 32,
        use_embed: true,
        case_insensitive: true,
        ..SearchOptions::default()
    })
    .unwrap();

    let mut lexical_files = HashSet::new();
    for term in ["process_request", "process", "request"] {
        lexical_files.extend(
            searcher
                .search_literal(term)
                .unwrap()
                .hits
                .into_iter()
                .map(|hit| hit.file),
        );
    }
    assert!(!lexical_files.is_empty());
    let response = searcher.search("process_request").unwrap();
    assert!(!response.hits.is_empty());
    let signals = response
        .hits
        .iter()
        .map(|hit| hit.signal)
        .collect::<HashSet<_>>();
    assert!(signals.contains(&HitSignal::Structural));
    let identities = response
        .hits
        .iter()
        .map(|hit| (hit.file.as_str(), hit.line_start))
        .collect::<HashSet<_>>();
    assert_eq!(identities.len(), response.hits.len());
    assert!(response.hits.iter().all(|hit| !hit.contributors.is_empty()));
    assert!(response.hits.iter().any(|hit| hit
        .contributors
        .contains(&ast_sgrep_core::search::HitKind::Embed)));
    assert!(
        response.hits.iter().any(|hit| hit.contributors.len() > 1),
        "fixture must exercise multi-channel fusion: {:#?}",
        response.hits
    );
    assert!(
        response
            .hits
            .iter()
            .all(|hit| lexical_files.contains(&hit.file)),
        "later stages leaked outside lexical survivors: {:#?}",
        response.hits
    );
}

#[test]
fn cascade_stops_when_a_stage_has_no_survivors() {
    let indexed = index_sample(IndexOptions {
        embed_semantic: true,
        ..IndexOptions::default()
    });
    let searcher = Searcher::new(SearchOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        limit: 32,
        use_embed: true,
        ..SearchOptions::default()
    })
    .unwrap();

    let no_lexical_survivors = searcher.search("phrase_absent_from_every_file").unwrap();
    assert!(no_lexical_survivors.hits.is_empty());

    let lexical_only = searcher.search_literal("processed").unwrap();
    assert!(
        !lexical_only.hits.is_empty(),
        "fixture must reach the structural stage"
    );
    let no_structural_survivors = searcher.search("processed").unwrap();
    assert!(
        no_structural_survivors.hits.is_empty(),
        "semantic stage ran without structural survivors: {:#?}",
        no_structural_survivors.hits
    );
}
