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
    // No Embed-contributor assertion here: since ba3fa030 the semantic
    // channel is a fallback that stays silent when structural evidence
    // answers (as it does for this exact-symbol query). The semantic stage
    // is pinned below on a zero-overlap query instead.
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

    // Semantic stage: a zero-overlap conceptual query must surface embed
    // evidence (the fallback-open path). No lexical containment here — a
    // semantic hit outside the lexical file set is the channel working.
    let semantic = searcher.search("credential renewal").unwrap();
    assert!(
        !semantic.hits.is_empty(),
        "zero-overlap query must return hits"
    );
    assert!(
        semantic.hits.iter().any(|hit| hit
            .contributors
            .contains(&ast_sgrep_core::search::HitKind::Embed)),
        "fallback-open query must surface embed evidence: {:#?}",
        semantic.hits
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

    // Prefixed literal stays fail-closed on empty discovery (no conceptual
    // semantic escape). Use a single absent token — underscore phrases can
    // split into terms that match imports under the hybrid path.
    let no_lexical_survivors = searcher.search("literal:zzzabsentphraseyyy").unwrap();
    assert!(
        no_lexical_survivors.hits.is_empty(),
        "literal empty discovery must stay empty: {:#?}",
        no_lexical_survivors.hits
    );

    let lexical_only = searcher.search_literal("processed").unwrap();
    assert!(
        !lexical_only.hits.is_empty(),
        "fixture must reach the structural stage"
    );
    let no_structural_survivors = searcher.search("processed").unwrap();
    // ht1h.3/parity: no structural survivors must fall back to the lexical
    // survivors (plain-content files stay findable) and the semantic stage
    // then runs on those lexical files — NL queries surface semantically
    // related symbols even without structural signals.
    assert!(
        !no_structural_survivors.hits.is_empty(),
        "lexical survivors must be returned when the structural stage is empty: {:#?}",
        no_structural_survivors.hits
    );
    let lexical_files: HashSet<_> = lexical_only.hits.iter().map(|h| h.file.clone()).collect();
    assert!(
        no_structural_survivors
            .hits
            .iter()
            .all(|hit| lexical_files.contains(&hit.file)),
        "later stages leaked outside lexical survivors: {:#?}",
        no_structural_survivors.hits
    );
}

#[test]
fn conceptual_empty_lexical_without_embed_stays_empty() {
    let indexed = index_sample(IndexOptions {
        embed_semantic: false,
        ..IndexOptions::default()
    });
    let searcher = Searcher::new(SearchOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        limit: 8,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();

    // Conceptual nonsense with embed disabled must not invent hits. Escape is
    // gated on use_embed; identifier/literal empty paths stay fail-closed.
    let response = searcher.search("zzzabsentphraseyyy").unwrap();
    assert!(
        response.hits.is_empty(),
        "no-embed conceptual empty must stay empty: {:#?}",
        response.hits
    );
}
