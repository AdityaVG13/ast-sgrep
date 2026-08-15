//! End-to-end smoke (renamed from parity.rs — e9qc). External oracle compare lives elsewhere.
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::store::IndexStore;
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_embed::EmbedPreference;
use ast_sgrep_testkit::{index_sample, reopen_indexer, searcher_from};
use std::fs;
use std::path::Path;

fn stored_text_column(root: &Path, index_path: &Path, sql: &str) -> Vec<String> {
    let store = IndexStore::open(root, Some(index_path)).unwrap();
    let mut statement = store.connection().prepare(sql).unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Regression for Issue #12 / F-01: prefixed callers:/defs: must return hits even
/// when the query casing differs from the stored symbol casing. Pre-fix, the raw
/// mixed-case target was scored against a lowercased symbol, yielding score 0 and
/// dropping every caller row.
#[test]
fn prefixed_modes_are_case_insensitive_on_mixed_case_symbols() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("auth.rs"),
        "fn RefreshToken() {}\nfn caller() { RefreshToken(); }\n",
    )
    .unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let stored_callees = stored_text_column(
        corpus.path(),
        &index_path,
        "SELECT callee FROM callers ORDER BY callee",
    );
    assert_eq!(stored_callees, vec!["RefreshToken"]);
    let queried_callees = [
        "callers:RefreshToken",
        "callers:refreshtoken",
        "callers:REFRESHTOKEN",
    ];
    eprintln!(
        "normalization evidence: stored callers.callee={stored_callees:?}; queried={queried_callees:?}; comparison=lower(c.callee)=lower(?)"
    );

    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        limit: 16,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();

    // Query casing differs from stored casing; each must still return caller hits.
    for q in queried_callees {
        let resp = searcher.search(q).unwrap();
        let caller_hit = resp
            .hits
            .iter()
            .find(|h| h.kind == HitKind::Caller && h.callee.as_deref() == Some("RefreshToken"));
        assert!(
            caller_hit.is_some(),
            "{q} must return a caller hit; got {:#?}",
            resp.hits
        );
        assert!(
            caller_hit.unwrap().score > 0.0,
            "{q} caller hit must have a positive score"
        );
    }

    let defs = searcher.search("defs:RefreshToken").unwrap();
    assert!(
        defs.hits
            .iter()
            .any(|h| h.kind == HitKind::Def && h.symbol.as_deref() == Some("RefreshToken")),
        "defs:RefreshToken must return a Def hit; got {:#?}",
        defs.hits
    );
}

/// Regression for Issue #12 / oxbj: `imports:` must return hits when the query
/// casing differs from the stored module_path casing. `query_imports` uses
/// `like_terms_filter` (SQLite LIKE, ASCII case-insensitive), so a mixed-case
/// module path must match case-variant queries. Pre-evidence, `imports:` had no
/// mixed-case coverage at all.
#[test]
fn imports_mode_is_case_insensitive_on_mixed_case_module_path() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("app.ts"),
        "import { Bar } from './Utils';\n",
    )
    .unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let stored_modules = stored_text_column(
        corpus.path(),
        &index_path,
        "SELECT module_path FROM imports ORDER BY module_path",
    );
    assert_eq!(stored_modules, vec!["./Utils"]);
    let queried_modules = ["imports:./Utils", "imports:./utils", "imports:./UTILS"];
    eprintln!(
        "normalization evidence: stored imports.module_path={stored_modules:?}; queried={queried_modules:?}; comparison=lower(module_path) LIKE escaped lower substring"
    );

    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        limit: 16,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();

    // Query casing differs from stored casing; each must still return an import hit.
    for q in queried_modules {
        let resp = searcher.search(q).unwrap();
        let import_hit = resp
            .hits
            .iter()
            .find(|h| h.kind == HitKind::Import && h.symbol.as_deref() == Some("./Utils"));
        assert!(
            import_hit.is_some(),
            "{q} must return an import hit for module_path './Utils'; got {:#?}",
            resp.hits
        );
    }
}

#[test]
fn literal_and_regex_context_is_targeted_bounded_and_file_diverse() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let mut crowded = String::new();
    for index in 0..200 {
        crowded.push_str(&format!("let needle_{index} = true;\n"));
    }
    fs::write(corpus.path().join("a.rs"), crowded).unwrap();
    fs::write(
        corpus.path().join("b.rs"),
        format!(
            "fn giant_symbol() {{\nlet before = 1;\nlet needle_other = \"{}\";\nlet after = 2;\n}}\n",
            "🦀".repeat(20_000)
        ),
    )
    .unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        limit: 4,
        use_embed: false,
        context_before: 1,
        context_after: 1,
        ..SearchOptions::default()
    })
    .unwrap();

    let literal = searcher.search("literal:needle_other").unwrap();
    let excerpt = &literal.hits[0].excerpt;
    assert!(excerpt.contains("let before = 1;"));
    assert!(excerpt.contains("let needle_other"));
    assert!(excerpt.len() <= ast_sgrep_core::MAX_SEARCH_HIT_EXCERPT_BYTES);
    assert!(excerpt.ends_with('…'));

    let definition = searcher.search("defs:giant_symbol").unwrap();
    let excerpt = &definition.hits[0].excerpt;
    assert!(excerpt.contains("fn giant_symbol()"));
    assert!(excerpt.len() <= ast_sgrep_core::MAX_SEARCH_HIT_EXCERPT_BYTES);
    assert!(excerpt.ends_with('…'));

    let regex = searcher.search("regex:needle_").unwrap();
    assert_eq!(regex.hits.len(), 4);
    assert!(
        regex.hits.iter().any(|hit| hit.file == "b.rs"),
        "per-file preference must retain later files: {:?}",
        regex.hits.iter().map(|hit| &hit.file).collect::<Vec<_>>()
    );
}
#[test]
#[ignore = "requires ASGREP_REAL_PI_FIXTURE archive"]
fn archived_pi_fixture_graph_modes_match_indexed_keys() {
    let root = std::env::var_os("ASGREP_REAL_PI_FIXTURE")
        .map(std::path::PathBuf::from)
        .expect("ASGREP_REAL_PI_FIXTURE must name the archived Pi corpus");
    let index_dir = tempfile::tempdir().unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    let indexed = indexer.index_all().unwrap();
    let stats = indexer.store().status().unwrap();
    eprintln!(
        "archived Pi corpus: indexed={} skipped={} files={} symbols={} callers={} imports={}",
        indexed.files_indexed,
        indexed.files_skipped,
        stats.file_count,
        stats.symbol_count,
        stats.caller_count,
        stats.import_count
    );
    assert!(
        stats.file_count >= 3_000,
        "archive is unexpectedly incomplete"
    );
    assert!(
        stats.caller_count >= 100_000,
        "archive must contain the large indexed call graph"
    );
    assert!(
        stats.import_count >= 10_000,
        "archive must contain the large indexed import graph"
    );

    let store = IndexStore::open(&root, Some(&index_path)).unwrap();
    let defined_names = {
        let mut statement = store
            .connection()
            .prepare("SELECT DISTINCT lower(name) FROM symbols")
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .unwrap()
    };
    let caller_keys = {
        let mut statement = store
            .connection()
            .prepare(
                "SELECT callee, COUNT(*) AS n FROM callers \
                 GROUP BY callee HAVING n BETWEEN 2 AND 20 \
                 ORDER BY n DESC, callee LIMIT 200",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .filter(|(name, _)| defined_names.contains(&name.to_lowercase()))
            .take(3)
            .collect::<Vec<_>>()
    };
    let import_keys = {
        let mut statement = store
            .connection()
            .prepare(
                "SELECT module_path, COUNT(*) AS n FROM imports \
                 GROUP BY module_path ORDER BY n DESC, module_path LIMIT 3",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(
        !caller_keys.is_empty(),
        "no defined callees found in real corpus"
    );
    assert!(
        !import_keys.is_empty(),
        "no import keys found in real corpus"
    );
    eprintln!("defined caller keys={caller_keys:?}");
    eprintln!("import keys={import_keys:?}");

    let searcher = Searcher::new(SearchOptions {
        root: root.clone(),
        index_path: Some(index_path),
        limit: 500,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    let reported_defs = searcher.search("defs:refreshToken").unwrap();
    let reported_callers = searcher.search("callers:refreshToken").unwrap();
    let reported_callers_lower = searcher.search("callers:refreshtoken").unwrap();
    assert!(
        reported_defs
            .hits
            .iter()
            .any(|hit| hit.kind == HitKind::Def),
        "the issue's refreshToken definition must remain in the real corpus"
    );
    let reported_count = reported_callers
        .hits
        .iter()
        .filter(|hit| hit.kind == HitKind::Caller)
        .count();
    assert!(
        reported_count > 0,
        "callers:refreshToken reproduced issue #12"
    );
    assert_eq!(
        reported_count,
        reported_callers_lower
            .hits
            .iter()
            .filter(|hit| hit.kind == HitKind::Caller)
            .count(),
        "the reported caller changes across casing"
    );
    let reported_chain = expand_chain(
        &store,
        "refreshToken",
        &ChainConfig {
            top_n: 5,
            max_depth: 1,
            limit: 64,
            ..ChainConfig::default()
        },
    )
    .unwrap();
    assert!(
        !reported_chain.seeds.is_empty() || !reported_chain.nodes.is_empty(),
        "chain refreshToken returned no graph evidence"
    );
    eprintln!(
        "refreshToken evidence: defs={} callers={} lowercase_callers={} chain_seeds={} chain_nodes={}",
        reported_defs.hits.iter().filter(|hit| hit.kind == HitKind::Def).count(),
        reported_count,
        reported_callers_lower
            .hits
            .iter()
            .filter(|hit| hit.kind == HitKind::Caller)
            .count(),
        reported_chain.seeds.len(),
        reported_chain.nodes.len()
    );

    for (symbol, _) in &caller_keys {
        let mixed = searcher.search(&format!("callers:{symbol}")).unwrap();
        let lower = searcher
            .search(&format!("callers:{}", symbol.to_lowercase()))
            .unwrap();
        let mixed_count = mixed
            .hits
            .iter()
            .filter(|hit| hit.kind == HitKind::Caller)
            .count();
        let lower_count = lower
            .hits
            .iter()
            .filter(|hit| hit.kind == HitKind::Caller)
            .count();
        assert!(mixed_count > 0, "callers:{symbol} returned no hits");
        assert_eq!(
            mixed_count, lower_count,
            "caller casing changed hit count for {symbol}"
        );
        let defs = searcher.search(&format!("defs:{symbol}")).unwrap();
        assert!(
            defs.hits.iter().any(|hit| hit.kind == HitKind::Def),
            "defs:{symbol} returned no definition"
        );
    }
    for (module, _) in &import_keys {
        let mixed = searcher.search(&format!("imports:{module}")).unwrap();
        let lower = searcher
            .search(&format!("imports:{}", module.to_lowercase()))
            .unwrap();
        let mixed_count = mixed
            .hits
            .iter()
            .filter(|hit| hit.kind == HitKind::Import)
            .count();
        let lower_count = lower
            .hits
            .iter()
            .filter(|hit| hit.kind == HitKind::Import)
            .count();
        assert!(mixed_count > 0, "imports:{module} returned no hits");
        assert_eq!(
            mixed_count, lower_count,
            "import casing changed hit count for {module}"
        );
    }
}

#[test]
fn parity_embed_backend_and_search_option_wiring() {
    assert_eq!(EmbedBackend::from_flags(true, false), EmbedBackend::Neural);
    assert_eq!(
        EmbedBackend::Neural.to_preference(),
        EmbedPreference::Neural
    );
    assert_eq!(EmbedBackend::Neural.to_preference_str(), "neural");
    assert_eq!(EmbedBackend::parse("neural"), EmbedBackend::Neural);
    assert_eq!(EmbedBackend::parse("fastembed"), EmbedBackend::Neural);
    let opts = SearchOptions {
        use_neural_embed: true,
        ann_probes: Some(4),
        use_rerank: true,
        rerank_top_k: 5,
        ..SearchOptions::default()
    };
    assert_eq!(opts.embed_preference(), EmbedPreference::Neural);
    assert_eq!(opts.ann_probes, Some(4));
    assert!(opts.use_rerank);
    assert_eq!(opts.rerank_top_k, 5);
    let _indexed = index_sample(IndexOptions {
        force_reindex: true,
        embed_backend: EmbedBackend::Semantic,
        ..IndexOptions::default()
    });
    // Fail-closed contract (parity_search_option_wiring): Searcher::new rejects
    // the flags when the features are not compiled; with them, the wiring must
    // still surface defs hits.
    #[cfg(not(all(feature = "neural-embed", feature = "rerank")))]
    assert!(
        ast_sgrep_core::Searcher::new(opts.clone()).is_err(),
        "neural/rerank flags must fail closed when features are off"
    );
    #[cfg(all(feature = "neural-embed", feature = "rerank"))]
    {
        let searcher = searcher_from(&_indexed, opts.clone());
        let resp = searcher.search("defs:auth_refresh").unwrap();
        assert!(
            resp.hits
                .iter()
                .any(|h| h.symbol.as_deref() == Some("auth_refresh")),
            "wired options must still return defs hits; got {:#?}",
            resp.hits
        );
    }
}
#[test]
fn index_all_preserves_semantic_ivf_on_noop_and_file_failure() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("lib.rs"),
        "fn alpha() { beta(); }\nfn beta() {} ",
    )
    .unwrap();
    let index_path = index_dir.path().join("index.db");
    let options = IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_backend: EmbedBackend::Semantic,
        ann_threshold: Some(1),
        force_reindex: false,
        ..IndexOptions::default()
    };
    let mut indexer = Indexer::new(options.clone()).unwrap();
    assert_eq!(indexer.index_all().unwrap().files_indexed, 1);
    let sidecar = ast_sgrep_core::semantic_ivf::semantic_ivf_path(&index_path);
    let original = fs::read(&sidecar).expect("semantic IVF sidecar built");
    let no_op = indexer.index_all().unwrap();
    assert_eq!(no_op.files_indexed, 0);
    assert_eq!(fs::read(&sidecar).unwrap(), original);
    fs::write(corpus.path().join("broken.rs"), [0xff]).unwrap();
    let failed = indexer.index_all().unwrap();
    assert_eq!(failed.files_failed, 1);
    assert_eq!(failed.files_indexed, 0);
    assert_eq!(fs::read(&sidecar).unwrap(), original);
}

#[test]
fn binary_assets_with_text_extensions_are_skipped_and_stale_rows_removed() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let source = corpus.path().join("records.json");
    fs::write(&source, "{\"name\":\"searchable_record\"}\n").unwrap();

    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_dir.path().join("index.db")),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    assert_eq!(indexer.index_all().unwrap().files_indexed, 1);

    // Zstandard frame magic followed by non-UTF-8 payload, matching generated
    // artifacts that retain a `.json` suffix.
    fs::write(&source, [0x28, 0xb5, 0x2f, 0xfd, 0xff]).unwrap();
    let updated = indexer.update_paths(std::slice::from_ref(&source)).unwrap();
    assert_eq!(updated.files_failed, 0);
    assert_eq!(updated.files_removed, 1);
    assert_eq!(indexer.store().status().unwrap().file_count, 0);

    let scanned = indexer.index_all().unwrap();
    assert_eq!(scanned.files_failed, 0);
    assert_eq!(scanned.files_skipped, 1);
}

#[test]
fn failed_file_preparation_preserves_prior_rows_and_aborts_strict_reindex() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let source = corpus.path().join("lib.rs");
    let index_path = index_dir.path().join("index.db");
    fs::write(&source, "fn durable_symbol() {}\n").unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    fs::write(&source, [0xff]).unwrap();
    let partial = indexer.index_all().unwrap();
    assert_eq!(partial.files_failed, 1);
    assert_eq!(indexer.store().status().unwrap().file_count, 1);
    assert!(indexer.reindex_all().is_err());

    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    assert!(!searcher.search("durable_symbol").unwrap().hits.is_empty());
}
#[test]
fn parity_index_defs_hybrid_chain() {
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let stats = indexed.indexer.store().status().unwrap();
    assert!(
        stats.file_count >= 4,
        "sample fixture should index multiple files"
    );
    assert!(stats.symbol_count > 0, "symbols must be extracted");
    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 16,
            use_embed: true,
            ..SearchOptions::default()
        },
    );
    let defs = searcher.search("defs:auth_refresh").unwrap();
    assert!(
        defs.hits
            .iter()
            .any(|h| h.kind == HitKind::Def && h.symbol.as_deref() == Some("auth_refresh")),
        "defs:auth_refresh must return Def hit; got {:#?}",
        defs.hits
    );
    let callers = searcher.search("callers:process_request").unwrap();
    assert!(
        callers
            .hits
            .iter()
            .any(|h| h.kind == HitKind::Caller && h.callee.as_deref() == Some("process_request")),
        "callers:process_request; got {:#?}",
        callers.hits
    );
    let nl = searcher.search_semantic("credential renewal").unwrap();
    // e2hc.19(b): The old oracle accepted ANY Embed hit via
    // `|| h.kind == HitKind::Embed`, making the assertion vacuous — an
    // irrelevant semantic chunk would satisfy it. Removed that clause so the
    // oracle requires an actually-relevant hit: either the symbol is
    // auth_refresh or the excerpt mentions it.
    assert!(
        !nl.hits.is_empty()
            && nl
                .hits
                .iter()
                .any(|h| h.symbol.as_deref() == Some("auth_refresh")
                    || h.excerpt.contains("auth_refresh")),
        "semantic search should surface auth_refresh; got {:#?}",
        nl.hits
    );
    let root = indexed.indexer.store().root().to_path_buf();
    let db = indexed.indexer.store().db_path().to_path_buf();
    let store = IndexStore::open(&root, Some(&db)).unwrap();
    let chain = expand_chain(
        &store,
        "process_request",
        &ChainConfig {
            top_n: 5,
            max_depth: 1,
            limit: 16,
            ..ChainConfig::default()
        },
    )
    .unwrap();
    assert!(
        !chain.seeds.is_empty() || !chain.nodes.is_empty(),
        "chain must produce seeds or nodes"
    );
    for n in &chain.nodes {
        assert!(n.depth <= 1);
    }
    let stored_backend = indexed
        .indexer
        .store()
        .get_meta("embed_backend")
        .unwrap()
        .expect("sample index stores concrete embedding backend");
    let mut again = reopen_indexer(
        &indexed,
        IndexOptions {
            embed_backend: EmbedBackend::parse(&stored_backend),
            ..IndexOptions::default()
        },
    );
    assert_eq!(again.index_all().unwrap().files_indexed, 0);
}
