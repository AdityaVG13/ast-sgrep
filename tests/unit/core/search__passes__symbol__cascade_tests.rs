use super::{def_hits_for_terms, symbol_pass_for_files};
use crate::query::ParsedQuery;
use crate::search::SearchOptions;
use crate::store::{IndexStore, SymbolRow, UpsertFileInput};
use std::collections::HashSet;
use tempfile::TempDir;

#[test]
fn survivor_file_filter_precedes_global_symbol_limit() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let symbol = SymbolRow {
        name: "target_symbol".into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        byte_start: 0,
        byte_end: 13,
    };
    for index in 0..=500 {
        let path = if index == 500 {
            "survivor.rs".to_string()
        } else {
            format!("decoy_{index:03}.rs")
        };
        let lines = [(1, "fn target_symbol() {}".to_string())];
        store
            .upsert_file(UpsertFileInput {
                rel_path: &path,
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: &format!("hash-{index}"),
                lines: &lines,
                eol: "\n",
                symbols: std::slice::from_ref(&symbol),
                callers: &[],
                imports: &[],
                pattern_nodes: &[],
                semantic_chunks: &[],
                embed_semantic: false,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();
    }
    let allowed = HashSet::from(["survivor.rs".to_string()]);
    let hits = symbol_pass_for_files(
        &store,
        &SearchOptions {
            root: temp.path().to_path_buf(),
            ..SearchOptions::default()
        },
        &ParsedQuery::parse("target_symbol"),
        &allowed,
    )
    .unwrap();
    assert!(
        hits.iter().any(|hit| hit.file == "survivor.rs"),
        "survivor after the global SQL ceiling was lost: {hits:#?}"
    );
    assert!(hits.iter().all(|hit| allowed.contains(&hit.file)));
}

#[test]
fn symbol_excerpts_are_read_only_for_retained_candidates() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    for (path, name) in [("discarded.rs", "target_suffix"), ("kept.rs", "target")] {
        let lines = [(1, format!("fn {name}() {{}}"))];
        let symbol = SymbolRow {
            name: name.into(),
            kind: "function".into(),
            line_start: 1,
            line_end: 1,
            byte_start: 0,
            byte_end: lines[0].1.len(),
        };
        store
            .upsert_file(UpsertFileInput {
                rel_path: path,
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: name,
                lines: &lines,
                eol: "\n",
                symbols: &[symbol],
                callers: &[],
                imports: &[],
                pattern_nodes: &[],
                semantic_chunks: &[],
                embed_semantic: false,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();
    }
    store
            .connection()
            .execute(
                "UPDATE lines SET content = x'ff' WHERE file_id = (SELECT id FROM files WHERE path = 'discarded.rs')",
                [],
            )
            .unwrap();
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 1,
        ..SearchOptions::default()
    };
    let parsed = ParsedQuery::parse("target");
    let hits = def_hits_for_terms(&store, &options, &parsed, super::SYMBOL_SQL_LIMIT).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].file, "kept.rs");
    assert_eq!(hits[0].excerpt, "fn target() {}");
}
