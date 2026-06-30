//! Persistence tests for `.asgrep/semantic.ivf` sidecar.

use std::path::PathBuf;

use ast_sgrep_core::semantic_ann::{ann_threshold, should_use_ann};
use ast_sgrep_core::semantic_ivf::{load_semantic_ivf, semantic_ivf_path};
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("fixture")
}

#[test]
fn index_builds_semantic_ivf_sidecar_when_threshold_low() {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        ann_threshold: Some(1),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let ivf_path = semantic_ivf_path(&index_path);
    assert!(
        ivf_path.exists(),
        "semantic.ivf should be written when chunk count >= threshold"
    );

    let status = indexer.store().status().unwrap();
    assert!(status.semantic_chunk_count >= 1);
    assert!(should_use_ann(status.semantic_chunk_count, Some(1)));

    let chunks = indexer.store().all_semantic_chunks(None).unwrap();
    let max_id = indexer.store().semantic_chunk_max_id().unwrap().unwrap_or(0);
    let dim = chunks[0].5.len();
    let fp = ast_sgrep_core::semantic_ivf::compute_ann_fingerprint(
        chunks.len(),
        max_id,
        dim,
        Some("semantic"),
    );
    let loaded = load_semantic_ivf(&ivf_path, fp).unwrap();
    assert!(loaded.is_some());
}

#[test]
fn search_uses_persisted_ivf_on_second_process() {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let opts = IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        ann_threshold: Some(1),
        force_reindex: true,
        ..IndexOptions::default()
    };

    let mut indexer = Indexer::new(opts.clone()).unwrap();
    indexer.index_all().unwrap();

    let searcher = Searcher::new(SearchOptions {
        root,
        index_path: Some(index_path),
        limit: 16,
        ann_threshold: Some(1),
        use_semantic_only: true,
        ..SearchOptions::default()
    })
    .unwrap();

    let response = searcher.search("credential renewal").unwrap();
    assert!(
        response.hits.iter().any(|h| {
            h.symbol.as_deref() == Some("auth_refresh")
                || h.excerpt.contains("auth_refresh")
        }),
        "IVF-backed search should still find auth_refresh"
    );
}

#[test]
fn default_ann_threshold_is_two_thousand() {
    assert_eq!(ann_threshold(None), 2000);
}
