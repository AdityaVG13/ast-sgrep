use super::*;
fn hit(file: &str, line: u32, score: f64) -> SearchHit {
    SearchHit {
        kind: HitKind::Asgrep,
        file: file.to_owned(),
        line_start: line,
        line_end: line,
        symbol: None,
        caller: None,
        callee: None,
        language: None,
        score,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Asgrep],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        excerpt: String::new(),
    }
}

#[test]
fn git_head_reads_only_bounded_in_repository_object_ids() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".git/refs/heads")).unwrap();
    std::fs::write(root.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let object_id = "A".repeat(40);
    std::fs::write(root.path().join(".git/refs/heads/main"), &object_id).unwrap();
    assert_eq!(
        read_git_head(root.path()),
        Some(object_id.to_ascii_lowercase())
    );

    std::fs::write(root.path().join(".git/HEAD"), "ref: ../../outside\n").unwrap();
    assert_eq!(read_git_head(root.path()), None);
    std::fs::write(root.path().join(".git/HEAD"), "not a commit id\n").unwrap();
    assert_eq!(read_git_head(root.path()), None);
    std::fs::write(root.path().join(".git/HEAD"), "x".repeat(4 * 1024 + 1)).unwrap();
    assert_eq!(read_git_head(root.path()), None);
}

#[cfg(unix)]
#[test]
fn git_head_refuses_symlinked_git_metadata() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("HEAD"), "a".repeat(40)).unwrap();
    symlink(outside.path(), root.path().join(".git")).unwrap();
    assert_eq!(read_git_head(root.path()), None);
}

#[test]
fn searcher_remaps_zero_and_oversize_limit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    // Minimal empty root is not a valid index; use with_store path via open after index.
    // Indexer creates the db so Searcher::new can open it.
    {
        let mut indexer = crate::Indexer::new(crate::IndexOptions {
            root: root.clone(),
            embed_semantic: false,
            ..crate::IndexOptions::default()
        })
        .unwrap();
        let _ = indexer.index_all();
    }
    let zero = Searcher::new(SearchOptions {
        root: root.clone(),
        limit: 0,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    assert_eq!(zero.options().limit, 16);
    let huge = Searcher::new(SearchOptions {
        root: root.clone(),
        limit: 50_000,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    assert_eq!(huge.options().limit, crate::limits::MAX_OUTPUT_RESULTS);
}

#[test]
fn rejects_oversize_query() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    {
        let mut indexer = crate::Indexer::new(crate::IndexOptions {
            root: root.clone(),
            embed_semantic: false,
            ..crate::IndexOptions::default()
        })
        .unwrap();
        let _ = indexer.index_all();
    }
    let searcher = Searcher::new(SearchOptions {
        root,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    let q = "a".repeat(crate::limits::MAX_QUERY_CHARS + 1);
    let err = searcher.search(&q).unwrap_err();
    assert!(err.to_string().contains("query exceeds maximum"), "{err}");
}

#[test]
fn lexicon_replacement_invalidates_long_lived_search_caches() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let store = IndexStore::open(&root, None).unwrap();
    store
        .replace_lexicon(&[crate::lexicon::Association {
            term: "refresh".into(),
            related: "token".into(),
            ppmi: 1.0,
            support: 3,
        }])
        .unwrap();
    let searcher = Searcher::with_store(
        store,
        SearchOptions {
            root,
            use_embed: false,
            ..SearchOptions::default()
        },
    );

    let first = searcher.search("refresh").unwrap();
    assert_eq!(first.query_expansions[0].related, "token");

    searcher
        .store()
        .replace_lexicon(&[crate::lexicon::Association {
            term: "refresh".into(),
            related: "session".into(),
            ppmi: 1.0,
            support: 4,
        }])
        .unwrap();
    let second = searcher.search("refresh").unwrap();
    assert_eq!(second.query_expansions[0].related, "session");
}

#[test]
fn append_ledger_entry_errors_when_parent_dir_missing() {
    let temp = tempfile::tempdir().unwrap();
    let missing_parent = temp.path().join("no_such_dir").join("ledger.jsonl");
    let response = SearchResponse {
        query: "q".into(),
        limit: 16,
        hits: vec![],
        counts: vec![],
        read_bytes_estimate: 0,
        returned_excerpt_bytes: 0,
        prevented_read_bytes: 0,
        snapshot: SnapshotStamp::default(),
        query_expansions: Vec::new(),
    };
    let err = append_ledger_entry(&missing_parent, &response).expect_err("missing parent");
    assert!(
        err.kind() == std::io::ErrorKind::NotFound
            || err.to_string().to_lowercase().contains("no such file")
            || err.raw_os_error().is_some(),
        "unexpected err: {err}"
    );
}

#[test]
fn append_ledger_entry_writes_json_line() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ledger.jsonl");
    let response = SearchResponse {
        query: "hello".into(),
        limit: 16,
        hits: vec![],
        counts: vec![],
        read_bytes_estimate: 10,
        returned_excerpt_bytes: 2,
        prevented_read_bytes: 8,
        snapshot: SnapshotStamp::default(),
        query_expansions: Vec::new(),
    };
    append_ledger_entry(&path, &response).expect("write");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("\"query\":\"hello\""), "{body}");
    assert!(body.ends_with('\n'), "{body:?}");
}

#[test]
fn excerpt_coverage_respects_term_casing() {
    let mut h = hit("a.rs", 1, 1.0);
    h.excerpt = "AuthRefresh token".into();
    assert_eq!(excerpt_term_coverage(&["AuthRefresh".into()], &h), 1);
    // Lowercase terms are case-insensitive and match the lowered excerpt.
    assert_eq!(excerpt_term_coverage(&["authrefresh".into()], &h), 1);
    // Mixed/upper terms stay case-sensitive and miss wrong casing.
    assert_eq!(excerpt_term_coverage(&["AUTHREFRESH".into()], &h), 0);
    assert_eq!(excerpt_term_coverage(&["token".into()], &h), 1);
}

#[test]
fn pretruncate_keeps_high_coverage_lower_score() {
    let parsed = ParsedQuery::parse("alpha beta gamma");
    let mut low = hit("low.rs", 1, 0.1);
    low.excerpt = "alpha beta gamma present".into();
    let mut highs: Vec<_> = (0..40)
        .map(|i| {
            let mut h = hit(&format!("high-{i}.rs"), 1, 1.0);
            h.excerpt = "alpha only".into();
            h
        })
        .collect();
    highs.push(low);
    let options = SearchOptions {
        limit: 5,
        ..SearchOptions::default()
    };
    let response = finish_response(&parsed, &options, highs, false);
    assert!(
        response.hits.iter().any(|h| h.file == "low.rs"),
        "high-coverage lower-score hit must survive pre-truncate"
    );
}

#[test]
fn finish_response_assigns_confidence_when_dedup_false() {
    // Regression for pass5 / ast-sgrep-d2a1.7: search_semantic finishes with
    // dedup=false and used to leave confidence at 0.0 forever.
    let parsed = ParsedQuery::parse("credential renewal");
    let mut embed = hit("auth.rs", 10, 3.2);
    embed.kind = HitKind::Embed;
    embed.signal = HitSignal::Semantic;
    embed.contributors = vec![HitKind::Embed];
    let options = SearchOptions {
        limit: 8,
        use_embed: false,
        ..SearchOptions::default()
    };
    let response = finish_response(&parsed, &options, vec![embed], false);
    assert_eq!(response.hits.len(), 1);
    assert!(
        response.hits[0].confidence > 0.0,
        "dedup=false path must still assign confidence"
    );
    assert!((response.hits[0].confidence - 0.35).abs() < 1e-12);
}

#[test]
fn definition_affinity_prefers_phrase_boundary_spelling() {
    let parsed = ParsedQuery::parse("how does auth refresh work");
    let mut snake = hit("snake.rs", 1, 1.0);
    snake.kind = HitKind::Def;
    snake.symbol = Some("auth_refresh".into());
    let mut camel = hit("camel.rs", 1, 1.0);
    camel.kind = HitKind::Def;
    camel.symbol = Some("authRefresh".into());
    assert!(
        definition_query_affinity(&parsed, &snake) > definition_query_affinity(&parsed, &camel)
    );

    let unrelated = ParsedQuery::parse("authorization workflow");
    let mut short = hit("short.rs", 1, 1.0);
    short.kind = HitKind::Def;
    short.symbol = Some("auth".into());
    assert_eq!(definition_query_affinity(&unrelated, &short), 0);

    let suffix = ParsedQuery::parse("refreshable token");
    short.symbol = Some("refresh".into());
    assert_eq!(definition_query_affinity(&suffix, &short), 0);
}

#[test]
fn hybrid_window_retains_definition_evidence() {
    let mut hits = vec![
        hit("embed-a.rs", 1, 1.0),
        hit("embed-b.rs", 1, 0.9),
        hit("def.rs", 1, 0.2),
    ];
    hits[0].kind = HitKind::Embed;
    hits[1].kind = HitKind::Embed;
    hits[2].kind = HitKind::Def;
    let gated = enforce_result_gates(hits, QueryMode::Hybrid, 2);
    assert_eq!(gated.len(), 2);
    assert_eq!(gated[0].kind, HitKind::Embed);
    assert_eq!(gated[1].kind, HitKind::Def);
}

#[test]
fn rerank_can_promote_candidate_beyond_final_limit() {
    let options = SearchOptions {
        limit: 16,
        use_rerank: true,
        rerank_top_k: 20,
        ..SearchOptions::default()
    };
    let hits: Vec<_> = (0..20)
        .map(|i| {
            hit(
                &format!("candidate-{i}.rs"),
                i + 1,
                1.0 - f64::from(i) / 100.0,
            )
        })
        .collect();
    let candidates =
        enforce_result_gates(hits, QueryMode::Literal, rerank_candidate_limit(&options));
    assert_eq!(candidates.len(), 20);
    let reranked = apply_rerank_order(candidates, options.rerank_top_k, [(16, 1.0)]);
    let final_hits = enforce_result_gates(reranked, QueryMode::Literal, options.limit);
    assert_eq!(final_hits.len(), options.limit);
    assert_eq!(final_hits[0].file, "candidate-16.rs");
}
#[test]
fn rerank_reorders_prefix_without_overwriting_fused_scores() {
    let hits = vec![
        hit("a.rs", 1, 0.9),
        hit("b.rs", 2, 0.8),
        hit("c.rs", 3, 0.7),
        hit("tail.rs", 4, 0.6),
    ];
    let reranked = apply_rerank_order(
        hits,
        3,
        [(2, 0.99), (0, 0.5), (7, 1.0), (2, 0.2), (1, f32::NAN)],
    );
    let identity: Vec<_> = reranked
        .iter()
        .map(|h| (h.file.as_str(), h.score))
        .collect();
    assert_eq!(
        identity,
        vec![
            ("c.rs", 0.7),
            ("a.rs", 0.9),
            ("b.rs", 0.8),
            ("tail.rs", 0.6)
        ]
    );
}
#[test]
fn literal_prefilter_handles_trigram_casefold_short_terms_and_bounds() {
    use crate::store::UpsertFileInput;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let mut lines = (1..=1_000)
        .map(|line| (line, format!("filler line {line}")))
        .collect::<Vec<_>>();
    lines.push((1_001, "NeedleCase id".to_string()));
    store
        .upsert_file(UpsertFileInput {
            rel_path: "large.rs",
            language: Some("rust"),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: "large",
            lines: &lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &[],
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
        })
        .unwrap();
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        ..SearchOptions::default()
    };
    let hits =
        literal_prefilter_pass(&store, &options, &ParsedQuery::parse("needlecase id")).unwrap();
    assert!(hits.iter().any(|hit| hit.excerpt == "NeedleCase id"));

    for index in 0..120 {
        let path = format!("bound-{index:03}.rs");
        let term = if index < 60 {
            "alphauniqueterm"
        } else {
            "betauniqueterm"
        };
        let bound_lines = [(1, term.to_string())];
        store
            .upsert_file(UpsertFileInput {
                rel_path: &path,
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: &path,
                lines: &bound_lines,
                eol: "\n",
                symbols: &[],
                callers: &[],
                imports: &[],
                pattern_nodes: &[],
                semantic_chunks: &[],
                embed_semantic: false,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();
    }
    let bounded = literal_prefilter_pass(
        &store,
        &options,
        &ParsedQuery::parse("alphauniqueterm betauniqueterm"),
    )
    .unwrap();
    let files = bounded
        .iter()
        .map(|hit| hit.file.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(files.len(), CASCADE_PREFILTER_FILE_LIMIT);
}

#[test]
fn hybrid_cap_and_limit_are_reapplied_after_rerank() {
    let hits = vec![
        hit("a.rs", 1, 0.9),
        hit("a.rs", 2, 0.8),
        hit("a.rs", 3, 0.7),
        hit("a.rs", 4, 0.6),
        hit("b.rs", 1, 0.5),
    ];
    let reranked = apply_rerank_order(hits, 5, [(3, 1.0), (2, 0.9), (1, 0.8), (0, 0.7), (4, 0.1)]);
    let gated = enforce_result_gates(reranked, QueryMode::Hybrid, 4);
    let identity: Vec<_> = gated
        .iter()
        .map(|h| (h.file.as_str(), h.line_start, h.score))
        .collect();
    assert_eq!(
        identity,
        vec![
            ("a.rs", 4, 0.6),
            ("a.rs", 3, 0.7),
            ("a.rs", 2, 0.8),
            ("b.rs", 1, 0.5)
        ]
    );
}

#[test]
fn regex_cap_and_limit_are_reapplied_after_rerank() {
    let hits = vec![
        hit("a.rs", 1, 0.9),
        hit("a.rs", 2, 0.8),
        hit("a.rs", 3, 0.7),
        hit("a.rs", 4, 0.6),
        hit("b.rs", 1, 0.5),
    ];
    let reranked = apply_rerank_order(hits, 5, [(3, 1.0), (2, 0.9), (1, 0.8), (0, 0.7), (4, 0.1)]);
    let gated = enforce_result_gates(reranked, QueryMode::Regex, 4);
    assert_eq!(
        gated
            .iter()
            .map(|hit| (hit.file.as_str(), hit.line_start))
            .collect::<Vec<_>>(),
        vec![("a.rs", 4), ("a.rs", 3), ("a.rs", 2), ("b.rs", 1)]
    );
}

#[test]
fn lock_clear_on_poison_resets_state() {
    let mutex = Mutex::new(vec![1, 2, 3]);
    let _ = std::panic::catch_unwind(|| {
        let _guard = mutex.lock().unwrap();
        panic!("inject poison");
    });
    assert!(mutex.is_poisoned());
    let guard = lock_clear_on_poison(&mutex, |v| v.clear());
    assert!(guard.is_empty());
    assert!(!mutex.is_poisoned());
}
