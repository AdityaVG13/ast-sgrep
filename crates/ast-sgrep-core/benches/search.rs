use ast_sgrep_core::{rank::coverage_symbol_score, IndexOptions, SearchOptions, Searcher};
use ast_sgrep_testkit::index_sample;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_search(c: &mut Criterion) {
    let indexed = index_sample(IndexOptions::default());
    let searcher = Searcher::new(SearchOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        limit: 16,
        use_embed: true,
        ..SearchOptions::default()
    }).unwrap();
    let lexical_searcher = Searcher::new(SearchOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        limit: 16,
        use_embed: false,
        ..SearchOptions::default()
    }).unwrap();

    c.bench_function("search_process_request", |b| {
        b.iter(|| {
            black_box(searcher.search("process_request").unwrap());
        });
    });

    c.bench_function("search_auth_refresh_nl", |b| { b.iter(|| {
        black_box(searcher.search("how does auth refresh work").unwrap());
    }); });
    c.bench_function("search_auth_refresh_nl_lexical_only", |b| {
        b.iter(|| black_box(lexical_searcher.search("how does auth refresh work").unwrap()));
    });

    let symbol_terms = vec![
        "auth".to_owned(),
        "refresh".to_owned(),
        "token".to_owned(),
        "cache".to_owned(),
    ];
    let symbol_candidates = [
        "auth_refresh_token",
        "refresh_auth_cache",
        "token_cache",
        "authenticator",
        "refresh_session",
        "cached_token",
        "authorize_request",
        "session_store",
    ];
    c.bench_function("rank_symbol_candidates_multi_term", |b| {
        b.iter(|| {
            let score = black_box(symbol_candidates)
                .iter()
                .map(|symbol| coverage_symbol_score(black_box(&symbol_terms), black_box(symbol)))
                .sum::<f64>();
            black_box(score);
        });
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
