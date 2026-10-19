//! Topology CORE gates: construction surface for the single-engine build.
//!
//! This file previously pinned the 2x2 `neural-embed`/`rerank` feature-gate
//! matrix. Phase 2 (br-6oh) deleted both features and their flags, so the
//! matrix collapsed to one cell: every build has the same embed surface.
//! What remains worth pinning: hermetic options construct successfully over
//! a real index (embed on and off), and defaults stay on the documented
//! side. All options are built field by field (no `..Default`) where
//! hermeticity matters, so ambient `ASGREP_*` env cannot perturb them.

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};

/// Fully hermetic plain options: every field explicit, no `..Default`.
fn hermetic_plain() -> SearchOptions {
    SearchOptions {
        root: std::path::PathBuf::from("."),
        index_path: None,
        limit: 16,
        lang_filter: None,
        use_embed: true,
        use_tantivy: false,
        use_semantic_only: false,
        use_repository_vocabulary: true,
        ann_threshold: None,
        ann_probes: None,
        case_insensitive: false,
        context_before: 0,
        context_after: 0,
        count_only: false,
        file_filter: None,
    }
}

/// Tiny indexed corpus (no semantic rows): keeps construction tests offline.
fn indexed_corpus() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = ast_sgrep_testkit::file_tree(&[(
        "auth.rs",
        "fn refresh_token() {}\nfn caller() { refresh_token(); }\n",
    )]);
    let index_dir = tempfile::tempdir().unwrap();
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
    (corpus, index_dir, index_path)
}

/// INTENT: with no optional backends left to gate, construction accepts the
/// full hermetic option surface over a real index — embed on, embed off,
/// semantic-only on/off — and serves hits.
/// KILLS: construction-gate-divergence (a gate that outlives its feature).
#[test]
fn construction_accepts_hermetic_surface() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let mut embed_off = hermetic_plain();
    embed_off.use_embed = false;
    let mut semantic_only = hermetic_plain();
    semantic_only.use_semantic_only = true;
    for (opts, name) in [
        (hermetic_plain(), "plain"),
        (embed_off, "embed-off"),
        (semantic_only, "semantic-only"),
    ] {
        let mut opts = opts;
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        let searcher =
            Searcher::new(opts).unwrap_or_else(|e| panic!("{name} options must construct: {e}"));
        let resp = searcher.search("defs:refresh_token").unwrap();
        assert!(
            resp.hits
                .iter()
                .any(|h| h.symbol.as_deref() == Some("refresh_token")),
            "{name} construction must serve defs hits; got {:#?}",
            resp.hits
        );
    }
}

/// INTENT: default option values stay on the documented side of the
/// single-engine contract (embedding on unless disabled, no semantic-only
/// restriction by default).
/// KILLS: default-flip.
#[test]
fn default_option_semantics() {
    let opts = SearchOptions::default();
    assert!(opts.use_embed, "embedding is on by default");
    assert!(!opts.use_semantic_only);
    let index_opts = IndexOptions::default();
    assert!(index_opts.embed_semantic, "indexing embeds by default");
    // The embed identity is a compile-time constant, not an option.
    assert_eq!(
        ast_sgrep_embed::STATIC_MODEL_ID,
        "static:potion-code-16M-v2:256"
    );
    assert_eq!(ast_sgrep_embed::STATIC_DIM, 256);
}
