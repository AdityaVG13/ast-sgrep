use std::path::PathBuf;

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tempfile::TempDir;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sample")
}

fn setup_index() -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture_root().canonicalize().unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    (temp, root, index_path)
}

fn bench_search(c: &mut Criterion) {
    let (_temp, root, index_path) = setup_index();
    let searcher = Searcher::new(SearchOptions {
        root,
        index_path: Some(index_path),
        limit: 16,
        lang_filter: None,
        use_embed: false,
        use_tantivy: false,
        use_cloud_embed: false,
        use_ollama_embed: false,
        use_semantic_only: false,
        ann_threshold: None,
    })
    .unwrap();

    c.bench_function("search_process_request", |b| {
        b.iter(|| {
            black_box(searcher.search("process_request").unwrap());
        });
    });

    c.bench_function("search_auth_refresh_nl", |b| {
        b.iter(|| {
            black_box(searcher.search("how does auth refresh work").unwrap());
        });
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
