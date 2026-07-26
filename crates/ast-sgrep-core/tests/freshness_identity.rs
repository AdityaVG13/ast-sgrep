use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_chunk::SemanticChunkInput;
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::tantivy_index::TantivySidecar;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};

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
        .rebuild_from_lines(&store.all_indexed_lines().unwrap(), generation)
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

#[test]
fn default_watch_refreshes_an_existing_lexical_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("lib.rs");
    std::fs::write(&source, "fn old_token() {}\n").unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        use_tantivy: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let sidecar = TantivySidecar::open(temp.path()).unwrap();
    sidecar
        .rebuild_from_lines(
            &indexer.store().all_indexed_lines().unwrap(),
            indexer.store().index_data_version().unwrap(),
        )
        .unwrap();

    std::fs::write(&source, "fn new_token() {}\n").unwrap();
    indexer.update_paths(&[source]).unwrap();
    assert!(indexer.deferred_rebuilds_pending());
    indexer.flush_deferred_rebuilds().unwrap();
    assert!(sidecar
        .is_fresh(indexer.store().index_data_version().unwrap())
        .unwrap());
}

#[test]
fn unchanged_bulk_files_skip_but_force_reindexes() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("lib.rs"), "fn stable() {}\n").unwrap();
    let mut normal = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        ..IndexOptions::default()
    })
    .unwrap();
    assert_eq!(normal.index_all().unwrap().files_indexed, 1);
    let second = normal.index_all().unwrap();
    assert_eq!(second.files_indexed, 0);
    assert_eq!(second.files_skipped, 1);

    let mut forced = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    assert_eq!(forced.index_all().unwrap().files_indexed, 1);
}

#[test]
fn embedding_preference_change_bypasses_structure_fast_path() {
    let temp = tempfile::tempdir().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let lines = [(1, "fn identity() {}".into())];
    let chunks = [SemanticChunkInput {
        symbol_name: "identity".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        excerpt: "fn identity() {}".into(),
        callers: vec![],
        callees: vec![],
        doc: String::new(),
        scope: String::new(),
    }];
    let mut first = plain_input("lib.rs", "same", &lines);
    first.semantic_chunks = &chunks;
    first.embed_semantic = true;
    first.embed_backend = ast_sgrep_embed::EmbedPreference::Semantic;
    store.upsert_file(first).unwrap();
    let before = store.index_data_version().unwrap();

    let mut changed = plain_input("lib.rs", "same", &lines);
    changed.semantic_chunks = &chunks;
    changed.embed_semantic = true;
    changed.embed_backend = ast_sgrep_embed::EmbedPreference::Neural;
    store.upsert_file(changed).unwrap();
    assert!(store.index_data_version().unwrap() > before);
}

#[test]
fn reassign_all_reclusters_centroids_from_current_vectors() {
    let old = (0..256)
        .flat_map(|index| [index as f32, 0.0])
        .collect::<Vec<_>>();
    let current = (0..256)
        .flat_map(|index| [0.0, (255 - index) as f32])
        .collect::<Vec<_>>();
    let mut reassigned = SemanticAnnIndex::build_from_flat(&old, 2);
    reassigned.reassign_all(&current, 2);
    let rebuilt = SemanticAnnIndex::build_from_flat(&current, 2);
    let mut reassigned_bytes = Vec::new();
    let mut rebuilt_bytes = Vec::new();
    reassigned.write_to(&mut reassigned_bytes, 2).unwrap();
    rebuilt.write_to(&mut rebuilt_bytes, 2).unwrap();
    assert_eq!(reassigned_bytes, rebuilt_bytes);
}
