use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::tantivy_index::TantivySidecar;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
use std::path::Path;

fn pass17_indexer(root: &Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        use_tantivy: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap()
}

fn pass17_searcher(root: &Path) -> Searcher {
    let store = IndexStore::open(root, None).unwrap();
    Searcher::with_store(
        store,
        SearchOptions {
            root: root.to_path_buf(),
            use_tantivy: true,
            use_embed: false,
            ..SearchOptions::default()
        },
    )
}

/// Pass-17 H-PERF-002 lever: a noop refresh (zero changed files) must SKIP
/// the FTS5 sidecar rebuild while the sidecar's stored `source_generation`
/// equals the current `index_data_version`, and a real content change must
/// still rebuild (the mutation bumps the generation inside its transaction,
/// so the freshness equality breaks exactly when work happened).
///
/// Discriminator without timing: after a verified index, replace the sidecar
/// content in-band with an empty rebuild stamped at the CURRENT generation.
/// The freshness equality still holds, so a refresh that honors the skip
/// leaves the emptied sidecar alone; the old unconditional rebuild rewrites
/// it from the lines table and wipes the marker.
#[test]
fn noop_refresh_skips_fresh_sidecar_and_real_change_rebuilds() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/one.rs"), "fn alpha_token() {}\n").unwrap();
    std::fs::write(root.join("src/two.rs"), "fn second_thing() {}\n").unwrap();

    let stats = pass17_indexer(root).index_all().unwrap();
    assert_eq!(stats.files_indexed, 2);

    // Warm lexical search is served from the fresh sidecar.
    let warm = pass17_searcher(root)
        .search_lexical("alpha_token")
        .unwrap();
    assert!(warm.hits.iter().any(|hit| hit.excerpt.contains("alpha_token")));

    // In-band tamper: empty the sidecar but stamp it with the CURRENT
    // generation, so the skip predicate's freshness equality holds.
    {
        let store = IndexStore::open(root, None).unwrap();
        let generation = store.index_data_version().unwrap();
        drop(store);
        let sidecar = TantivySidecar::open(root).unwrap();
        sidecar
            .rebuild_from_lines_with_generation(&[], generation)
            .unwrap();
        assert!(!sidecar.is_search_ready().unwrap());
    }

    // Noop refresh: the walk finds both files unchanged.
    let stats = pass17_indexer(root).index_all().unwrap();
    assert_eq!(stats.files_indexed, 0, "noop refresh must re-index nothing");
    assert_eq!(stats.files_removed, 0);

    // THE SKIP: the emptied sidecar was not rewritten from the lines table.
    // (Failure-first: the unconditional rebuild restores the rows and this
    // assertion fails against pre-lever code.)
    let sidecar = TantivySidecar::open(root).unwrap();
    assert!(
        !sidecar.is_search_ready().unwrap(),
        "noop refresh must skip the sidecar rebuild while the stored generation is current"
    );

    // Mutation test: a real content change bumps the generation and must
    // force the rebuild, restoring sidecar rows from the new content.
    std::fs::write(root.join("src/one.rs"), "fn beta_replacement() {}\n").unwrap();
    let stats = pass17_indexer(root).index_all().unwrap();
    assert_eq!(stats.files_indexed, 1);
    let sidecar = TantivySidecar::open(root).unwrap();
    assert!(
        sidecar.is_search_ready().unwrap(),
        "a real file change must rebuild the sidecar"
    );
    let beta_hits = sidecar
        .search(&["beta_replacement".to_string()], 10)
        .unwrap();
    assert_eq!(beta_hits.len(), 1, "rebuilt sidecar must hold the new line");
    let searcher = pass17_searcher(root);
    let fresh = searcher.search_lexical("beta_replacement").unwrap();
    assert!(fresh.hits.iter().any(|hit| hit.excerpt.contains("beta_replacement")));
    let gone = searcher.search_lexical("alpha_token").unwrap();
    assert!(gone.hits.is_empty(), "replaced content must leave the index");
}

/// The lever's correctness predicate: search output is byte-identical
/// before and after a noop refresh (the skip changes no observable search
/// state).
#[test]
fn noop_refresh_keeps_search_output_byte_identical() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/one.rs"), "fn alpha_token() {}\n").unwrap();
    std::fs::write(root.join("src/two.rs"), "fn second_thing() {}\n").unwrap();

    pass17_indexer(root).index_all().unwrap();
    let before = pass17_searcher(root).search_lexical("alpha_token").unwrap();
    pass17_indexer(root).index_all().unwrap();
    let after = pass17_searcher(root).search_lexical("alpha_token").unwrap();
    assert_eq!(
        serde_json::to_string(&before).unwrap(),
        serde_json::to_string(&after).unwrap(),
        "noop refresh must not change search output"
    );
}

fn plain_input<'a>(
    path: &'a str,
    hash: &'a str,
    lines: &'a [(u32, String)],
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("rust"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
    }
}

/// Lexical sidecar identity: when the source generation advances, stale Tantivy
/// must miss and lexical search still returns fresh lines.
#[test]
fn lexical_sidecar_falls_back_when_source_generation_changes() {
    let temp = tempfile::tempdir().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let first = [(1, "alpha token".into())];
    store
        .upsert_file(plain_input("src/lib.rs", "one", &first))
        .unwrap();
    let generation = store.index_data_version().unwrap();
    let sidecar = TantivySidecar::open(temp.path()).unwrap();
    sidecar
        .rebuild_from_lines_with_generation(&store.all_indexed_lines().unwrap(), generation)
        .unwrap();
    assert!(sidecar.is_fresh(generation).unwrap());

    let second = [(1, "beta replacement".into())];
    store
        .upsert_file(plain_input("src/lib.rs", "two", &second))
        .unwrap();
    assert!(!sidecar
        .is_fresh(store.index_data_version().unwrap())
        .unwrap());
    let searcher = Searcher::with_store(
        store,
        SearchOptions {
            root: temp.path().to_path_buf(),
            use_tantivy: true,
            use_embed: false,
            ..SearchOptions::default()
        },
    );
    let response = searcher.search_lexical("beta").unwrap();
    assert!(response.hits.iter().any(|hit| hit.excerpt.contains("beta")));
}
