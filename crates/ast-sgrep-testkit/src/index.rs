use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;

use crate::fixture::sample_root;

/// Indexed sample repo kept alive for the duration of a test.
pub struct IndexedFixture {
    pub _temp: TempDir,
    pub indexer: Indexer,
}

/// Build index options that reopen an existing indexed fixture.
pub fn index_options_from(indexed: &IndexedFixture, overrides: IndexOptions) -> IndexOptions {
    IndexOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        ..overrides
    }
}

/// Reopen an indexer against a previously indexed fixture.
pub fn reopen_indexer(indexed: &IndexedFixture, overrides: IndexOptions) -> Indexer {
    Indexer::new(index_options_from(indexed, overrides)).expect("indexer")
}

/// Index the sample fixture with optional overrides.
pub fn index_sample(mut opts: IndexOptions) -> IndexedFixture {
    let temp = TempDir::new().expect("tempdir");
    opts.index_path = Some(temp.path().join("index.db"));
    if opts.root.as_os_str() == "." {
        opts.root = sample_root();
    }
    let mut indexer = Indexer::new(opts).expect("indexer");
    indexer.index_all().expect("index");
    IndexedFixture {
        _temp: temp,
        indexer,
    }
}

/// Build a searcher from an indexed fixture.
pub fn searcher_from(indexed: &IndexedFixture, mut opts: SearchOptions) -> Searcher {
    opts.root = indexed.indexer.store().root().to_path_buf();
    opts.index_path = Some(indexed.indexer.store().db_path().to_path_buf());
    Searcher::new(opts).expect("searcher")
}

/// Hybrid searcher over a freshly indexed sample fixture.
pub fn hybrid_searcher(no_embed: bool) -> (IndexedFixture, Searcher) {
    searcher_with(no_embed, false)
}

/// Semantic-first searcher over a freshly indexed sample fixture.
pub fn semantic_searcher(no_embed: bool) -> (IndexedFixture, Searcher) {
    searcher_with(no_embed, true)
}

fn searcher_with(no_embed: bool, semantic_only: bool) -> (IndexedFixture, Searcher) {
    let indexed = index_sample(IndexOptions {
        embed_semantic: !no_embed,
        force_reindex: true,
        ..IndexOptions::default()
    });
    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 32,
            use_embed: !no_embed,
            use_semantic_only: semantic_only,
            ..SearchOptions::default()
        },
    );
    (indexed, searcher)
}
