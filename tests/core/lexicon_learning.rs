//! ufk7: the engine learns this repository's vocabulary instead of relying on
//! hand-written global concept groups.
use ast_sgrep_core::lexicon::{
    explain, prose_terms, subtokens, Association, Lexicon, LexiconBuilder, Observation, MIN_SUPPORT,
};
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};

#[test]
fn identifiers_split_into_subtokens() {
    assert_eq!(subtokens("refresh_token"), vec!["refresh", "token"]);
    assert_eq!(subtokens("refreshToken"), vec!["refresh", "token"]);
    assert_eq!(subtokens("HTTPStatusCode"), vec!["httpstatus", "code"]);
    assert_eq!(subtokens("Store::open"), vec!["store", "open"]);
    // Generic terms carry no repository meaning and are dropped.
    assert!(subtokens("self").is_empty());
    assert!(subtokens("a_b").is_empty(), "sub-3-char tokens dropped");
}

#[test]
fn ppmi_prefers_distinctive_pairs_over_frequent_ones() {
    let mut builder = LexiconBuilder::new();
    // `rotate` always appears with `credentials`; both are otherwise rare.
    // `handler` appears everywhere, so it should NOT win on association.
    for _ in 0..MIN_SUPPORT + 2 {
        builder.observe(&Observation {
            identifier_terms: vec!["rotate".into()],
            prose_terms: vec!["credentials".into(), "handler".into()],
        });
    }
    for index in 0..20 {
        builder.observe(&Observation {
            identifier_terms: vec![format!("unrelated{index}")],
            prose_terms: vec!["handler".into()],
        });
    }

    let associations = builder.finish();
    let rotate: Vec<_> = associations.iter().filter(|a| a.term == "rotate").collect();
    assert!(!rotate.is_empty(), "rotate must learn something");
    let top = rotate[0];
    assert_eq!(
        top.related, "credentials",
        "the distinctive pair must outrank the ubiquitous one: {rotate:?}"
    );
    assert!(top.ppmi > 0.0);
    assert!(top.support >= MIN_SUPPORT);
}

#[test]
fn pairs_below_the_support_floor_are_rejected() {
    let mut builder = LexiconBuilder::new();
    // Seen together only twice: below MIN_SUPPORT, so it is noise.
    for _ in 0..(MIN_SUPPORT - 1) {
        builder.observe(&Observation {
            identifier_terms: vec!["lonely".into()],
            prose_terms: vec!["coincidence".into()],
        });
    }
    let associations = builder.finish();
    assert!(
        !associations.iter().any(|a| a.term == "lonely"),
        "a pair under the support floor must not enter the lexicon"
    );
}

#[test]
fn learning_is_deterministic() {
    let build = || {
        let mut builder = LexiconBuilder::new();
        for _ in 0..5 {
            builder.observe(&Observation {
                identifier_terms: vec!["rotate".into(), "session".into()],
                prose_terms: vec!["refresh".into(), "credentials".into()],
            });
        }
        builder.finish()
    };
    let first = build();
    for _ in 0..5 {
        let again = build();
        assert_eq!(first.len(), again.len());
        for (a, b) in first.iter().zip(again.iter()) {
            assert_eq!(
                (&a.term, &a.related, a.support),
                (&b.term, &b.related, b.support)
            );
        }
    }
}

#[test]
fn expansion_carries_checkable_evidence() {
    let mut builder = LexiconBuilder::new();
    // PPMI measures co-occurrence ABOVE chance, so it needs contrast: if two
    // terms are the only vocabulary in the corpus they always co-occur, their
    // PMI is exactly 0, and no association is learned. That is correct
    // behavior, so the fixture supplies background vocabulary.
    for _ in 0..6 {
        builder.observe(&Observation {
            identifier_terms: vec!["rotate".into()],
            prose_terms: vec!["credentials".into()],
        });
    }
    for index in 0..30 {
        builder.observe(&Observation {
            identifier_terms: vec![format!("other{index}")],
            prose_terms: vec![format!("topic{index}"), "common".into()],
        });
    }
    let lexicon = Lexicon::from_associations(builder.finish());
    let added = lexicon.expand(&["rotate".to_string()], 5);
    assert!(!added.is_empty(), "expansion must fire");
    assert_eq!(added[0].related, "credentials");

    let reverse = lexicon.expand(&["credentials".to_string()], 5);
    assert!(
        reverse
            .iter()
            .any(|association| association.related == "rotate"),
        "symmetric PPMI must let repository prose recover its identifier: {reverse:?}"
    );

    let reason = explain(&added[0]);
    assert!(reason.contains("rotate"), "{reason}");
    assert!(reason.contains("credentials"), "{reason}");
    assert!(
        reason.contains(&added[0].support.to_string()),
        "explanation must quote the checkable support count: {reason}"
    );
}

/// End to end: indexing a repository learns its vocabulary, with no network.
#[test]
fn indexing_builds_a_lexicon_from_the_corpus() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    // A repository where `rotate` consistently means refreshing credentials,
    // against a background of unrelated vocabulary. The contrast matters:
    // PPMI scores co-occurrence above chance, so a corpus with one uniform
    // vocabulary correctly yields no associations at all.
    for index in 0..8 {
        std::fs::write(
            src.join(format!("auth{index}.rs")),
            format!(
                "/// Rotate the credentials for an expired session.\n\
                 fn rotate_credentials_{index}(session: &Session) {{}}\n"
            ),
        )
        .unwrap();
    }
    for index in 0..24 {
        std::fs::write(
            src.join(format!("misc{index}.rs")),
            format!(
                "/// Compute a geometry bounding volume for mesh {index}.\n\
                 fn compute_bounds_{index}(mesh: &Mesh) {{}}\n"
            ),
        )
        .unwrap();
    }
    Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");

    let store = IndexStore::open(temp.path(), None).expect("store");
    let rows = store.all_lexicon_rows().expect("lexicon rows");
    assert!(
        !rows.is_empty(),
        "indexing must learn associations from the corpus"
    );
    assert!(
        rows.iter().any(|a| a.term == "rotate"),
        "the repository's own vocabulary must be learned: {rows:?}"
    );

    // And a search reports the expansion as auditable evidence.
    let response = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher")
    .search("rotate")
    .expect("search");
    assert!(
        !response.query_expansions.is_empty(),
        "an expanded query must say so: {:?}",
        response.query_expansions
    );
    let first = &response.query_expansions[0];
    assert!(first.support > 0);
    assert!(first.because.contains("repository association"));
}

#[test]
fn targeted_mutations_clear_then_rebuild_the_lexicon() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("terms.rs");
    let corpus = |left: &str, right: &str| {
        let related = (0..8)
            .map(|index| format!("/// {right} domain relation\nfn {left}_{index}() {{}}\n"))
            .collect::<String>();
        let background = (0..24)
            .map(|index| {
                format!("/// unrelated geometry topic {index}\nfn background_mesh_{index}() {{}}\n")
            })
            .collect::<String>();
        related + &background
    };
    std::fs::write(&source, corpus("rotate_credentials", "renewal")).unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    assert!(!indexer.store().all_lexicon_rows().unwrap().is_empty());

    std::fs::write(&source, corpus("compute_geometry", "bounding")).unwrap();
    indexer.update_paths(std::slice::from_ref(&source)).unwrap();
    assert!(
        indexer.store().all_lexicon_rows().unwrap().is_empty(),
        "source mutation must never leave stale associations visible"
    );
    assert!(indexer.deferred_rebuilds_pending());
    indexer.flush_deferred_rebuilds().unwrap();
    assert!(indexer
        .store()
        .all_lexicon_rows()
        .unwrap()
        .iter()
        .all(|association| association.term != "rotate"));
}

#[test]
fn learning_bounds_pathological_terms_and_observations() {
    use ast_sgrep_core::lexicon::MAX_TERM_CHARS;

    assert!(subtokens(&"x".repeat(MAX_TERM_CHARS + 1)).is_empty());
    let mut builder = LexiconBuilder::new();
    for _ in 0..MIN_SUPPORT {
        builder.observe(&Observation {
            identifier_terms: vec!["x".repeat(MAX_TERM_CHARS + 1)],
            prose_terms: vec!["bounded".into()],
        });
    }
    assert!(
        builder.finish().is_empty(),
        "direct builder inputs must enforce the same term-size bound as tokenization"
    );
}

#[test]
fn persisted_lexicon_rejects_oversized_rows_and_terms() {
    use ast_sgrep_core::lexicon::{load_lexicon, MAX_PAIRS, MAX_TERM_CHARS};

    let temp = tempfile::tempdir().unwrap();
    let store = ast_sgrep_core::IndexStore::open(temp.path(), None).unwrap();
    store
        .connection()
        .execute_batch(&format!(
            "WITH RECURSIVE counter(value) AS (
               VALUES(0)
               UNION ALL
               SELECT value + 1 FROM counter WHERE value < {MAX_PAIRS}
             )
             INSERT INTO lexicon(term, related, ppmi, support)
             SELECT printf('term%06d', value), 'related', 1.0, 3 FROM counter;"
        ))
        .unwrap();
    let error = load_lexicon(&store).expect_err("row cap must fail closed");
    assert!(error.to_string().contains("exceeds maximum"), "{error}");

    store
        .connection()
        .execute("DELETE FROM lexicon", [])
        .unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO lexicon(term, related, ppmi, support) VALUES(?1, 'related', 1.0, 3)",
            rusqlite::params!["x".repeat(MAX_TERM_CHARS + 1)],
        )
        .unwrap();
    let error = load_lexicon(&store).expect_err("term cap must fail closed");
    assert!(
        error.to_string().contains("term exceeds maximum"),
        "{error}"
    );

    let invalid = Association {
        term: "credential".into(),
        related: "renewal".into(),
        ppmi: f64::NAN,
        support: MIN_SUPPORT,
    };
    let error = store
        .replace_lexicon(&[invalid])
        .expect_err("non-finite first-party scores must be rejected before storage");
    assert!(error.to_string().contains("non-finite"), "{error}");
}

#[test]
fn prose_terms_survive_punctuation() {
    let terms = prose_terms("Rotate the credentials, then renew_session().");
    assert!(terms.contains(&"rotate".to_string()), "{terms:?}");
    assert!(terms.contains(&"credentials".to_string()), "{terms:?}");
    assert!(terms.contains(&"renew".to_string()), "{terms:?}");
    assert!(!terms.contains(&"the".to_string()), "stop terms dropped");
}
