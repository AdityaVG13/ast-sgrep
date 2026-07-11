use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;
use crate::fixture::sample_root;
pub struct IndexedFixture {
    pub _temp: TempDir,
    pub indexer: Indexer,
}
pub fn reopen_indexer(indexed: &IndexedFixture, overrides: IndexOptions) -> Indexer {
    Indexer::new(IndexOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        ..overrides
    })
    .expect("indexer")
}
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
pub fn searcher_from(indexed: &IndexedFixture, mut opts: SearchOptions) -> Searcher {
    opts.root = indexed.indexer.store().root().to_path_buf();
    opts.index_path = Some(indexed.indexer.store().db_path().to_path_buf());
    Searcher::new(opts).expect("searcher")
}
