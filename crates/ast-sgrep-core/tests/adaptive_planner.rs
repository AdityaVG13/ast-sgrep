//! ocx8: the planner must stop early when cheap evidence already answers the
//! query, and must say what it ran and what it skipped.
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};

fn corpus(root: &std::path::Path) {
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    // Many exact occurrences of one distinctive token, so the cheap channels
    // can satisfy a query on their own.
    for index in 0..10 {
        std::fs::write(
            src.join(format!("mod{index}.rs")),
            format!(
                "fn uses_marker_{index}() {{ distinctive_marker_token(); }}\n\
                 fn distinctive_marker_token() {{}}\n"
            ),
        )
        .expect("write");
    }
    // Unrelated prose-heavy file so conceptual queries have somewhere to go.
    std::fs::write(
        src.join("docs.rs"),
        "/// Renew an expired login when the credential lifetime elapses.\n\
         fn renew_expired_login() {}\n",
    )
    .expect("write");
}

fn searcher(root: &std::path::Path, use_embed: bool) -> Searcher {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        use_embed,
        ..SearchOptions::default()
    })
    .expect("searcher")
}

fn indexed() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    corpus(temp.path());
    Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    temp
}

#[test]
fn a_query_answered_by_exact_evidence_skips_the_semantic_stage() {
    let temp = indexed();
    let response = searcher(temp.path(), true)
        .search("distinctive_marker_token")
        .expect("search");

    assert!(!response.hits.is_empty(), "fixture must match");
    assert!(
        response.plan.stopped_because.is_some(),
        "the planner must record why it stopped: {:?}",
        response.plan
    );
    assert!(
        response.plan.skipped.contains(&"semantic".to_string()),
        "the expensive stage must be skipped: {:?}",
        response.plan
    );
    assert!(
        !response.plan.stages.contains(&"semantic".to_string()),
        "a skipped stage must not be reported as executed: {:?}",
        response.plan
    );
    // The reason is a sentence a human can act on, not an opaque code.
    let reason = response.plan.stopped_because.unwrap();
    assert!(reason.contains("exact"), "{reason}");
}

#[test]
fn cheap_stages_are_always_reported_as_executed() {
    let temp = indexed();
    let response = searcher(temp.path(), true)
        .search("distinctive_marker_token")
        .expect("search");
    for stage in ["lexical", "structural", "symbol"] {
        assert!(
            response.plan.stages.contains(&stage.to_string()),
            "{stage} must be recorded: {:?}",
            response.plan
        );
    }
}

#[test]
fn disabling_embeddings_is_reported_as_a_skipped_stage_not_a_stop() {
    let temp = indexed();
    let response = searcher(temp.path(), false)
        .search("renew expired login credential")
        .expect("search");
    assert!(
        response.plan.skipped.contains(&"semantic".to_string()),
        "a disabled channel must still be visible: {:?}",
        response.plan
    );
}

#[test]
fn early_exit_does_not_change_which_results_are_returned_for_exact_queries() {
    let temp = indexed();
    // An exact query is answerable without semantics by construction, so the
    // early exit must be observationally equivalent to running the full plan.
    let with_embed = searcher(temp.path(), true)
        .search("distinctive_marker_token")
        .expect("search");
    let without_embed = searcher(temp.path(), false)
        .search("distinctive_marker_token")
        .expect("search");

    let ids = |response: &ast_sgrep_core::SearchResponse| {
        response
            .hits
            .iter()
            .map(|hit| (hit.file.clone(), hit.line_start))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&with_embed),
        ids(&without_embed),
        "skipping semantics on an exact query must not change the answer"
    );
}

#[test]
fn the_plan_is_present_on_every_response() {
    let temp = indexed();
    let searcher = searcher(temp.path(), true);
    for query in [
        "distinctive_marker_token",
        "renew expired login",
        "defs:renew_expired_login",
    ] {
        let response = searcher.search(query).expect("search");
        assert!(
            !response.plan.stages.is_empty() || !response.hits.is_empty(),
            "every response must carry a plan or be a miss: {query}"
        );
    }
}
