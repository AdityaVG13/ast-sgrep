//! Thin end-to-end parity: one sample index, real search/chain entry points.
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::store::IndexStore;
use ast_sgrep_core::{EmbedBackend, IndexOptions, SearchOptions};
use ast_sgrep_embed::EmbedPreference;
use ast_sgrep_testkit::{index_sample, reopen_indexer, searcher_from};

#[test]
fn parity_embed_backend_and_search_option_wiring() {
    assert_eq!(
        EmbedBackend::from_flags(false, false, true, false),
        EmbedBackend::Neural
    );
    assert_eq!(EmbedBackend::Neural.to_preference(), EmbedPreference::Neural);
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

    // Live search path accepts rerank/ann_probes without panic (rerank may no-op if feature off).
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        embed_backend: EmbedBackend::Semantic,
        ..IndexOptions::default()
    });
    let searcher = searcher_from(&indexed, opts);
    let resp = searcher.search("defs:auth_refresh").unwrap();
    assert!(
        resp.hits.iter().any(|h| h.symbol.as_deref() == Some("auth_refresh")),
        "wired options must still return defs hits; got {:#?}",
        resp.hits
    );
}

#[test]
fn parity_index_defs_hybrid_chain() {
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let stats = indexed.indexer.store().status().unwrap();
    assert!(stats.file_count >= 4, "sample fixture should index multiple files");
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
        defs.hits.iter().any(|h| {
            h.kind == HitKind::Def && h.symbol.as_deref() == Some("auth_refresh")
        }),
        "defs:auth_refresh must return Def hit; got {:#?}",
        defs.hits
    );

    let callers = searcher.search("callers:process_request").unwrap();
    assert!(
        callers.hits.iter().any(|h| {
            h.kind == HitKind::Caller && h.callee.as_deref() == Some("process_request")
        }),
        "callers:process_request; got {:#?}",
        callers.hits
    );

    let nl = searcher.search("credential renewal").unwrap();
    assert!(
        !nl.hits.is_empty()
            && nl.hits.iter().any(|h| {
                h.symbol.as_deref() == Some("auth_refresh")
                    || h.excerpt.contains("auth_refresh")
                    || h.kind == HitKind::Embed
            }),
        "NL/hybrid should surface auth_refresh; got {:#?}",
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

    let mut again = reopen_indexer(&indexed, IndexOptions::default());
    assert_eq!(again.index_all().unwrap().files_indexed, 0);
}
