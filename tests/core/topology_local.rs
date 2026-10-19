//! Topology CORE local path: the single native embed surface as intent
//! suites. As of Phase 2 (br-6oh) there is exactly one engine — the static
//! table — and exactly one stored identity; these tests pin that surface.
//!
//! OFFLINE POLICY: every test is pure/local. Only the static engine and
//! `use_embed = false` searches run here; nothing loads a model or touches
//! the network under any feature set (there are no embed feature sets left).

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Fully hermetic local options: every field explicit, no `..Default`, so no
/// ambient `ASGREP_*` env var can perturb the test. `use_embed = false` keeps
/// every search on the lexical path.
fn hermetic_local_options() -> SearchOptions {
    SearchOptions {
        root: std::path::PathBuf::from("."),
        index_path: None,
        limit: 16,
        lang_filter: None,
        use_embed: false,
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

/// Tiny indexed corpus with NO semantic rows (`embed_semantic: false`):
/// keeps every search offline. Corpus dir via `testkit::file_tree`.
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

fn searcher_for(
    corpus: &std::path::Path,
    index_path: &std::path::Path,
    opts: SearchOptions,
) -> Searcher {
    let mut opts = opts;
    opts.root = corpus.to_path_buf();
    opts.index_path = Some(index_path.to_path_buf());
    Searcher::new(opts).unwrap()
}

// ---------------------------------------------------------------------------
// Single-engine surface
// ---------------------------------------------------------------------------

/// INTENT: the one embed engine produces the BIT-IDENTICAL vector through
/// every entry point (direct, batch, free-function facade), deterministically.
/// Facets: dim/model-id constants; empty input yields a full-dim vector,
/// never an error; batch == single elementwise; distinct inputs discriminate.
/// KILLS: vector-divergence.
#[test]
fn local_embed_vectors_bit_identical_across_entry_points() {
    use ast_sgrep_embed::{embed_text, embed_texts, StaticEmbedder, STATIC_DIM, STATIC_MODEL_ID};
    assert_eq!(STATIC_DIM, 256);
    assert_eq!(STATIC_MODEL_ID, "static:potion-code-16M-v2:256");

    let engine = StaticEmbedder::new();
    // Empty input still yields a full-dim vector, never an error.
    assert_eq!(engine.embed("").len(), STATIC_DIM);

    let inputs = [
        "refresh token authentication",
        "database migration rollback",
        "",
        "héllo wörld 🦀",
    ];
    let refs: Vec<&str> = inputs.to_vec();
    let batched = engine.embed_batch(&refs);
    assert_eq!(batched.len(), inputs.len());
    let facade_batched = embed_texts(&refs);
    let mut seen = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let direct = engine.embed(input);
        assert_eq!(direct.len(), STATIC_DIM);
        // Determinism: same input, same vector, every call.
        assert_eq!(direct, engine.embed(input));
        // Batch and free-function entry points agree exactly.
        assert_eq!(batched[i], direct);
        assert_eq!(embed_text(input), direct);
        assert_eq!(facade_batched[i], direct);
        // The engine discriminates distinct inputs.
        for prev in &seen {
            assert_ne!(&direct, prev, "inputs must map distinctly");
        }
        seen.push(direct);
    }
}

/// INTENT: option normalization clamps identically, and repeat searches over
/// the same index are byte-stable.
/// Facets: limit/context clamps (zero/oversize/in-range); whole-response
/// bytes identical across repeat searches.
/// KILLS: clamp-divergence, nondeterminism.
#[test]
fn normalization_clamps_and_repeat_stability() {
    use ast_sgrep_core::limits::{MAX_EXCERPT_LINES, MAX_OUTPUT_RESULTS};
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let build = |opts: SearchOptions| searcher_for(corpus.path(), &index_path, opts);

    let mut zero = hermetic_local_options();
    zero.limit = 0;
    assert_eq!(build(zero).options().limit, 16);

    let mut huge = hermetic_local_options();
    huge.limit = usize::MAX;
    huge.context_before = usize::MAX;
    huge.context_after = usize::MAX;
    let s = build(huge);
    assert_eq!(s.options().limit, MAX_OUTPUT_RESULTS);
    assert_eq!(s.options().context_before, MAX_EXCERPT_LINES);
    assert_eq!(s.options().context_after, MAX_EXCERPT_LINES);

    assert_eq!(build(hermetic_local_options()).options().limit, 16);

    // Repeat searches return identical whole-response bytes.
    let searcher = build(hermetic_local_options());
    for query in [
        "defs:refresh_token",
        "callers:refresh_token",
        "refresh_token",
    ] {
        let a = searcher.search(query).unwrap();
        assert!(!a.hits.is_empty(), "stability probe must hit for {query:?}");
        let b = searcher.search(query).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "repeat search must be byte-stable for {query:?}"
        );
    }
}
