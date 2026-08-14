//! dvc4: a name-only guess must never be presented as an exact call edge.
use ast_sgrep_core::resolution::{Resolution, ResolvedEdge, SymbolId};
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};

#[test]
fn symbol_identity_is_more_than_a_name() {
    let a = SymbolId::new("src/client.rs", "send").with_owner("HttpClient");
    let b = SymbolId::new("src/queue.rs", "send").with_owner("Queue");
    assert_ne!(a, b, "same name on unrelated owners must not be one symbol");
    assert_eq!(a.qualified(), "src/client.rs::HttpClient::send");
    assert_ne!(a.qualified(), b.qualified());
}

#[test]
fn only_disambiguated_resolutions_are_precise() {
    for precise in [
        Resolution::CompilerExact,
        Resolution::ScipExact,
        Resolution::ImportResolved,
        Resolution::FileLocalUnique,
    ] {
        assert!(precise.is_precise(), "{precise:?} should be precise");
    }
    // The honesty gate: these are guesses.
    assert!(!Resolution::NameOnly.is_precise());
    assert!(!Resolution::RepositoryUnique.is_precise());
    assert!(!Resolution::Ambiguous {
        candidates: vec![SymbolId::new("a.rs", "send"), SymbolId::new("b.rs", "send"),],
    }
    .is_precise());
}

#[test]
fn resolution_strength_is_ordered() {
    let ordered = [
        Resolution::CompilerExact,
        Resolution::ScipExact,
        Resolution::ImportResolved,
        Resolution::FileLocalUnique,
        Resolution::RepositoryUnique,
        Resolution::NameOnly,
    ];
    for pair in ordered.windows(2) {
        assert!(
            pair[0].rank() < pair[1].rank(),
            "{:?} must outrank {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn candidate_counts_classify_the_match() {
    // The only definition in the referencing file.
    assert_eq!(
        Resolution::from_candidates(1, 5, std::iter::empty()),
        Resolution::FileLocalUnique
    );
    // Exactly one in the whole repository.
    assert_eq!(
        Resolution::from_candidates(0, 1, std::iter::empty()),
        Resolution::RepositoryUnique
    );
    // Nothing known at all: a bare name.
    assert_eq!(
        Resolution::from_candidates(0, 0, std::iter::empty()),
        Resolution::NameOnly
    );
    // Several candidates, and they are carried so a consumer can see them.
    let ambiguous = Resolution::from_candidates(
        0,
        3,
        [SymbolId::new("a.rs", "send"), SymbolId::new("b.rs", "send")],
    );
    match &ambiguous {
        Resolution::Ambiguous { candidates } => assert_eq!(candidates.len(), 2),
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    assert!(!ambiguous.is_precise());
}

#[test]
fn an_imprecise_edge_is_never_described_as_a_call() {
    let guess = ResolvedEdge {
        caller: SymbolId::new("src/login.rs", "handle_login"),
        callee: SymbolId::new("", "send"),
        resolution: Resolution::NameOnly,
    };
    let (label, precise) = guess.describe();
    assert!(!precise);
    assert!(
        label.contains("may call"),
        "a guess must be hedged, got: {label}"
    );
    assert!(
        label.contains("name_only"),
        "the label must name the weak resolution: {label}"
    );

    let known = ResolvedEdge {
        caller: SymbolId::new("src/login.rs", "handle_login"),
        callee: SymbolId::new("src/auth.rs", "refresh_token"),
        resolution: Resolution::FileLocalUnique,
    };
    let (label, precise) = known.describe();
    assert!(precise);
    assert!(label.contains("calls"), "{label}");
    assert!(!label.contains("may call"), "{label}");
}

/// End to end: real caller hits carry a resolution tier, and an ambiguous
/// name does not claim precision.
#[test]
fn caller_hits_carry_a_resolution_tier() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    // `send` is defined twice on unrelated types: the classic collision.
    std::fs::write(
        src.join("client.rs"),
        "fn send() {}\nfn handle_login() { send(); }\n",
    )
    .unwrap();
    std::fs::write(src.join("queue.rs"), "fn send() {}\n").unwrap();
    // `only_here` is defined exactly once repository-wide.
    std::fs::write(
        src.join("unique.rs"),
        "fn only_here() {}\nfn caller_of_unique() { only_here(); }\n",
    )
    .unwrap();

    Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");

    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher");

    let ambiguous = searcher.search("callers:send").expect("search");
    let resolved: Vec<_> = ambiguous
        .hits
        .iter()
        .filter_map(|hit| hit.resolution.clone())
        .collect();
    assert!(
        !resolved.is_empty(),
        "caller hits must carry a resolution tier: {:?}",
        ambiguous.hits
    );
    // Two same-named definitions exist, so nothing here may claim precision
    // through repository uniqueness.
    assert!(
        resolved
            .iter()
            .all(|r| !matches!(r, Resolution::RepositoryUnique)),
        "a duplicated name must not resolve as repository-unique: {resolved:?}"
    );

    let unique = searcher.search("callers:only_here").expect("search");
    let unique_resolutions: Vec<_> = unique
        .hits
        .iter()
        .filter_map(|hit| hit.resolution.clone())
        .collect();
    assert!(
        unique_resolutions.iter().any(|r| matches!(
            r,
            Resolution::FileLocalUnique | Resolution::RepositoryUnique
        )),
        "a uniquely-named callee must resolve better than name-only: {unique_resolutions:?}"
    );
}

#[test]
fn scip_upgrades_an_ambiguous_call_without_inventing_edges() {
    use ast_sgrep_core::scip::{ScipDocument, ScipIndex, ScipOccurrence, SCIP_ROLE_DEFINITION};

    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("client.rs"), "fn send() {}\n").unwrap();
    std::fs::write(src.join("queue.rs"), "fn send() {}\n").unwrap();
    std::fs::write(src.join("login.rs"), "fn handle_login() { send(); }\n").unwrap();

    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");

    let before = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher")
    .search("callers:send")
    .expect("search");
    let before_tiers: Vec<_> = before
        .hits
        .iter()
        .filter_map(|hit| hit.resolution.clone())
        .collect();
    assert!(
        before_tiers.iter().all(|r| !r.is_precise()),
        "cross-file collision must stay imprecise before SCIP: {before_tiers:?}"
    );

    let applied = indexer
        .store()
        .apply_scip(&ScipIndex {
            documents: vec![
                ScipDocument {
                    relative_path: "src/login.rs".into(),
                    occurrences: vec![ScipOccurrence {
                        symbol: "rust+crate+send().".into(),
                        symbol_roles: 0,
                        range: vec![0, 20, 0, 24],
                    }],
                },
                ScipDocument {
                    relative_path: "src/client.rs".into(),
                    occurrences: vec![ScipOccurrence {
                        symbol: "rust+crate+send().".into(),
                        symbol_roles: SCIP_ROLE_DEFINITION,
                        range: vec![0, 3, 0, 7],
                    }],
                },
                ScipDocument {
                    relative_path: "src/missing.rs".into(),
                    occurrences: vec![ScipOccurrence {
                        symbol: "rust+crate+ghost().".into(),
                        symbol_roles: 0,
                        range: vec![0, 0, 0, 5],
                    }],
                },
            ],
        })
        .expect("apply scip");
    assert!(
        applied.refs_upgraded >= 1,
        "login.rs send() ref must match: {applied:?}"
    );
    assert!(
        applied.defs_upgraded >= 1,
        "client.rs send def must match: {applied:?}"
    );
    assert!(
        applied.skipped >= 1,
        "missing.rs must not invent an edge: {applied:?}"
    );

    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher");
    let after = searcher.search("callers:send").expect("search");
    let login = after
        .hits
        .iter()
        .find(|hit| hit.file.ends_with("login.rs"))
        .expect("login.rs caller hit");
    assert_eq!(login.resolution, Some(Resolution::ScipExact));
    assert!(login.resolution.as_ref().unwrap().is_precise());

    let defs = searcher.search("defs:send").expect("defs");
    assert!(
        defs.hits.iter().any(|hit| {
            hit.file.ends_with("client.rs") && hit.resolution == Some(Resolution::ScipExact)
        }),
        "SCIP def must upgrade client.rs send: {:?}",
        defs.hits
            .iter()
            .map(|h| (&h.file, h.resolution.clone()))
            .collect::<Vec<_>>()
    );

    let ghost = searcher.search("callers:ghost").expect("ghost");
    assert!(
        ghost.hits.is_empty(),
        "SCIP must not invent callers:ghost hits: {:?}",
        ghost.hits
    );
}

#[test]
fn scip_never_downgrades_a_stronger_tier() {
    assert_eq!(
        Resolution::CompilerExact.upgrade(Resolution::ScipExact),
        Resolution::CompilerExact
    );
    assert_eq!(
        Resolution::NameOnly.upgrade(Resolution::ScipExact),
        Resolution::ScipExact
    );
    assert_eq!(
        Resolution::ScipExact.upgrade(Resolution::FileLocalUnique),
        Resolution::ScipExact
    );
}
