//! Indexing lifecycle, IVF persistence, and scale regression tests.

use ast_sgrep_core::semantic_ann::{ann_threshold, should_use_ann};
use ast_sgrep_core::semantic_ivf::{load_semantic_ivf, semantic_ivf_path};
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{assert_semantic_finds, index_options_from, index_sample, reopen_indexer, searcher_from};

#[test]
fn incremental_reindex_is_stable() {
    let indexed = index_sample(IndexOptions::default());
    let first = indexed.indexer.store().status().unwrap().file_count;
    assert!(first >= 4);

    let mut second_pass = reopen_indexer(
        &indexed,
        IndexOptions::default(),
    );
    let stats = second_pass.index_all().unwrap();
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_removed, 0);
}

#[test]
fn force_reindex_reextracts_symbols() {
    let indexed = index_sample(IndexOptions::default());
    let opts = index_options_from(&indexed, IndexOptions::default());

    let mut indexer = reopen_indexer(&indexed, opts.clone());
    assert_eq!(indexer.index_all().unwrap().symbols_extracted, 0);

    let mut force = reopen_indexer(
        &indexed,
        IndexOptions {
            force_reindex: true,
            ..opts
        },
    );
    assert!(force.reindex_all().unwrap().symbols_extracted > 0);
}

#[test]
fn lang_filter_drops_other_languages() {
    let indexed = index_sample(IndexOptions::default());
    let full = indexed.indexer.store().status().unwrap().file_count;

    let rust_only = index_sample(IndexOptions {
        lang_filter: Some("rust".into()),
        force_reindex: true,
        ..IndexOptions::default()
    });
    let rust_count = rust_only.indexer.store().status().unwrap().file_count;
    assert!(rust_count < full);
    assert!(rust_count >= 1);
}

#[test]
fn lexical_tantivy_sidecar_search() {
    let indexed = index_sample(IndexOptions {
        use_tantivy: true,
        force_reindex: true,
        ..IndexOptions::default()
    });
    let db = indexed.indexer.store().db_path();
    let lexical = db.parent().unwrap().join("lexical.db");
    assert!(lexical.exists(), "lexical sidecar beside index.db");

    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 16,
            use_embed: false,
            use_tantivy: true,
            ..SearchOptions::default()
        },
    );
    assert!(!searcher.search("process_request").unwrap().hits.is_empty());
}

#[test]
fn semantic_chunks_indexed_by_default() {
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    assert!(indexed.indexer.store().status().unwrap().semantic_chunk_count > 0);
}

#[test]
fn ivf_sidecar_written_when_threshold_low() {
    let indexed = index_sample(IndexOptions {
        ann_threshold: Some(1),
        force_reindex: true,
        ..IndexOptions::default()
    });
    let index_path = indexed.indexer.store().db_path();
    let ivf_path = semantic_ivf_path(index_path);
    assert!(ivf_path.exists());

    let chunks = indexed.indexer.store().all_semantic_chunks(None).unwrap();
    let max_id = indexed
        .indexer
        .store()
        .semantic_chunk_max_id()
        .unwrap()
        .unwrap_or(0);
    let dim = chunks[0].5.len();
    let fp = ast_sgrep_core::semantic_ivf::compute_ann_fingerprint(
        chunks.len(),
        max_id,
        dim,
        Some("semantic"),
    );
    assert!(load_semantic_ivf(&ivf_path, fp).unwrap().is_some());
    assert!(should_use_ann(chunks.len(), Some(1)));
}

#[test]
fn ivf_backed_search_finds_synonyms() {
    let indexed = index_sample(IndexOptions {
        ann_threshold: Some(1),
        force_reindex: true,
        ..IndexOptions::default()
    });
    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 16,
            ann_threshold: Some(1),
            use_semantic_only: true,
            ..SearchOptions::default()
        },
    );
    assert_semantic_finds(&searcher, "credential renewal", &["auth_refresh"]);
}

#[test]
fn default_ann_threshold_is_two_thousand() {
    assert_eq!(ann_threshold(None), 2000);
}

#[test]
fn symbol_pass_stays_bounded_on_broad_query() {
    let indexed = index_sample(IndexOptions::default());
    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 64,
            ..SearchOptions::default()
        },
    );
    let response = searcher
        .search("how does auth refresh and process_request work")
        .unwrap();
    assert!(!response.hits.is_empty());
    assert!(response.hits.len() <= 64);
}
