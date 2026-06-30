use ast_sgrep_core::IndexOptions;
use ast_sgrep_lsp::LspBackend;

use crate::index::index_sample;

/// LSP backend backed by an indexed sample fixture.
pub fn sample_backend() -> (IndexedFixture, LspBackend) {
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let root = indexed.indexer.store().root().to_path_buf();
    let index_path = indexed.indexer.store().db_path().to_path_buf();
    let mut backend = LspBackend::new(root);
    backend.set_index_path(index_path);
    backend.ensure_index().expect("ensure index");
    (indexed, backend)
}

// Re-export so callers can keep temp dirs alive via IndexedFixture.
pub use crate::index::IndexedFixture;
