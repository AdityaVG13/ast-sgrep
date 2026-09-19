use crate::index::{index_sample, json_hit_keys, HitKey, IndexedFixture};
use ast_sgrep_core::IndexOptions;
use ast_sgrep_lsp::types::{Position, Range, TextDocumentContentChangeEvent};
use ast_sgrep_lsp::{settings::AsgrepSettings, LspBackend};
use std::path::Path;

/// INTENT: full-document replace edit event (no range): the literal struct
/// would bury the text intent in noise. Pure constructor.
pub fn edit_full_replace(text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: text.to_string(),
    }
}

/// INTENT: ranged edit event without `rangeLength`: delegates to
/// [`edit_ranged_len`] so the two ranged builders cannot drift apart. Pure
/// constructor.
pub fn edit_ranged(
    start: (u32, u32),
    end: (u32, u32),
    text: &str,
) -> TextDocumentContentChangeEvent {
    edit_ranged_len(start, end, None, text)
}

/// INTENT: ranged edit event with an explicit `rangeLength`: the
/// rangeLength-bearing form the precedence facet pins. Pure constructor.
pub fn edit_ranged_len(
    start: (u32, u32),
    end: (u32, u32),
    range_length: Option<u32>,
    text: &str,
) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: Some(Range {
            start: Position {
                line: start.0,
                character: start.1,
            },
            end: Position {
                line: end.0,
                character: end.1,
            },
        }),
        range_length,
        text: text.to_string(),
    }
}
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
