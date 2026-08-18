use crate::index::{index_sample, json_hit_keys, HitKey, IndexedFixture};
use ast_sgrep_core::IndexOptions;
use ast_sgrep_lsp::{settings::AsgrepSettings, LspBackend};
use std::path::Path;
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
/// LSP in-process search → surface hit keys.
///
/// `use_embed` aligns with core/CLI. Soft-skip when embed is requested but the
/// surface cannot emit embed hits is forbidden for mock-free e2e (lbx1.13).
pub fn lsp_search_hit_keys(
    root: &Path,
    index_path: &Path,
    query: &str,
    limit: usize,
    use_embed: bool,
) -> Vec<HitKey> {
    let mut backend = LspBackend::new(root.to_path_buf());
    backend.set_index_path(index_path.to_path_buf());
    backend
        .apply_settings(AsgrepSettings {
            // Product: no_embed=true disables embed; no_embed=false enables it.
            no_embed: Some(!use_embed),
            ..AsgrepSettings::default()
        })
        .expect("apply LSP settings");
    json_hit_keys(&backend.search(query, false, limit).expect("LSP search"))
}
