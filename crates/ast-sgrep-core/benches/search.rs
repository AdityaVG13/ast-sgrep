use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use ast_sgrep_testkit::index_sample;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_search(c: &mut Criterion) {
    let indexed = index_sample(IndexOptions::default());
    let searcher = Searcher::new(SearchOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        limit: 16,
        use_embed: false,
        ..SearchOptions::default()
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
