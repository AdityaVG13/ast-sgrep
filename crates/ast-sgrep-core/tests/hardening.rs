use ast_sgrep_core::store::IndexStore;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_embed::SemanticLocalEmbedding;
use ast_sgrep_testkit::{index_repo, index_sample, reopen_indexer, sample_root, temp_repo};

#[test]
fn semantic_chunks_reference_their_symbols() {
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let store = IndexStore::open(
        indexed.indexer.store().root(),
        Some(indexed.indexer.store().db_path()),
    )
    .unwrap();
    for (file, _, _, symbol, text, _) in store.all_semantic_chunks(None).unwrap() {
        assert!(!symbol.is_empty(), "chunk in {file} must have symbol_name");
        assert!(text.contains(&symbol), "chunk text must reference {symbol}");
    }
}

#[test]
fn incremental_skip_uses_content_hash() {
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let mut indexer = reopen_indexer(
        &indexed,
        IndexOptions {
            force_reindex: false,
            ..IndexOptions::default()
        },
    );
    let stats = indexer.index_all().unwrap();
    assert_eq!(stats.files_indexed, 0);
    assert!(stats.files_skipped > 0);
}

#[test]
fn lexical_sidecar_follows_custom_index_path() {
    let temp = tempfile::TempDir::new().unwrap();
    let custom_dir = temp.path().join("custom");
    std::fs::create_dir_all(&custom_dir).unwrap();
    let index_path = custom_dir.join("index.db");

    let mut indexer = Indexer::new(IndexOptions {
        root: sample_root(),
        index_path: Some(index_path.clone()),
        use_tantivy: true,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    assert!(custom_dir.join("lexical.db").exists());

    let searcher = Searcher::new(SearchOptions {
        root: sample_root(),
        index_path: Some(index_path),
        use_tantivy: true,
        limit: 8,
        ..SearchOptions::default()
    })
    .unwrap();
    assert!(!searcher.search("process_request").unwrap().hits.is_empty());
}

#[test]
fn corrupt_ivf_sidecar_is_rejected() {
    use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
    use ast_sgrep_core::semantic_ivf::{compute_ann_fingerprint, load_semantic_ivf, save_semantic_ivf};

    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("semantic.ivf");
    let dim = 16;
    let vectors = vec![0.0f32; 50 * dim];
    let index = SemanticAnnIndex::build_from_flat(&vectors, dim);
    let fp = compute_ann_fingerprint(50, 50, dim, Some("semantic"));
    save_semantic_ivf(&path, fp, dim, &vectors, &index).unwrap();

    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&path, &bytes).unwrap();
    assert!(load_semantic_ivf(&path, fp).unwrap().is_none());
}

#[test]
fn embedding_dim_mismatch_scores_zero() {
    let chunk_vec = SemanticLocalEmbedding.embed_text("auth refresh");
    let wrong_dim_query = vec![1.0f32; chunk_vec.len() + 10];
    assert!(ast_sgrep_embed::rank_chunks_by_vector(
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
    )
    .is_empty());
}

#[test]
fn embed_from_bytes_rejects_trailing_bytes() {
    assert!(ast_sgrep_embed::embed_from_bytes(&[0u8, 0u8, 0u8]).is_err());
}

#[test]
fn crlf_file_text_round_trips() {
    let repo = temp_repo(&[(
        "main.rs",
        "fn main() {\r\n    println!(\"hi\");\r\n}\r\n",
    )]);
    let (_temp, indexer) = index_repo(
        &repo,
        IndexOptions {
            force_reindex: true,
            embed_semantic: false,
            ..IndexOptions::default()
        },
    );
    let original = "fn main() {\r\n    println!(\"hi\");\r\n}\r\n";
    assert_eq!(indexer.store().file_text("main.rs").unwrap().unwrap(), original);
}
