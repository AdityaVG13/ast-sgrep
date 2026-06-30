//! Hardening tests from thermo-nuclear review rounds.

use std::path::PathBuf;

use ast_sgrep_core::store::IndexStore;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_embed::SemanticLocalEmbedding;
use tempfile::TempDir;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("fixture")
}

#[test]
fn semantic_chunks_map_to_correct_symbol_ids() {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let store = IndexStore::open(&root, Some(&index_path)).unwrap();
    let chunks = store.all_semantic_chunks(None).unwrap();
    assert!(!chunks.is_empty());
    for (file, _, _, symbol, text, _) in &chunks {
        assert!(!symbol.is_empty(), "chunk in {file} must have symbol_name");
        assert!(
            text.contains(symbol),
            "chunk text must reference its symbol {symbol}"
        );
    }
}

#[test]
fn incremental_skip_uses_content_hash_not_mtime() {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let mut opts = IndexOptions {
        root: root.clone(),
        index_path: Some(index_path),
        force_reindex: true,
        ..IndexOptions::default()
    };
    let mut indexer = Indexer::new(opts.clone()).unwrap();
    indexer.index_all().unwrap();

    opts.force_reindex = false;
    let mut indexer = Indexer::new(opts).unwrap();
    let stats = indexer.index_all().unwrap();
    assert_eq!(stats.files_indexed, 0, "unchanged files should not re-index");
    assert!(stats.files_skipped > 0);
}

#[test]
fn lexical_sidecar_follows_custom_index_path() {
    let temp = TempDir::new().unwrap();
    let custom_dir = temp.path().join("custom");
    std::fs::create_dir_all(&custom_dir).unwrap();
    let index_path = custom_dir.join("index.db");
    let root = fixture();

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        use_tantivy: true,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let lexical = custom_dir.join("lexical.db");
    assert!(
        lexical.exists(),
        "lexical sidecar must live beside custom index.db"
    );

    let searcher = Searcher::new(SearchOptions {
        root,
        index_path: Some(index_path),
        use_tantivy: true,
        limit: 8,
        ..SearchOptions::default()
    })
    .unwrap();
    let hits = searcher.search("process_request").unwrap();
    assert!(!hits.hits.is_empty());
}

#[test]
fn corrupt_ivf_sidecar_is_rejected() {
    use ast_sgrep_core::semantic_ivf::{compute_ann_fingerprint, load_semantic_ivf, save_semantic_ivf};
    use ast_sgrep_core::semantic_ann::SemanticAnnIndex;

    let temp = TempDir::new().unwrap();
    let path = temp.path().join("semantic.ivf");
    let dim = 16;
    let vectors = vec![0.0f32; 50 * dim];
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let fp = compute_ann_fingerprint(50, 50, dim, Some("semantic"));
    save_semantic_ivf(&path, fp, dim, &vectors, &index).unwrap();

    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&path, &bytes).unwrap();

    let loaded = load_semantic_ivf(&path, fp).unwrap();
    assert!(loaded.is_none(), "tampered IVF must not load");
}

#[test]
fn embedding_dim_mismatch_scores_zero() {
    let embedder = SemanticLocalEmbedding;
    let chunk_vec = embedder.embed_text("auth refresh");
    let wrong_dim_query = vec![1.0f32; chunk_vec.len() + 10];
    let sim = ast_sgrep_embed::rank_chunks_by_vector(
        &wrong_dim_query,
        &[(
            "a.rs".into(),
            1,
            1,
            "auth".into(),
            "excerpt".into(),
            chunk_vec,
        )],
        1,
    );
    assert!(sim.is_empty(), "dimension mismatch must not produce hits");
}
