use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::semantic_chunk::SemanticChunkInput;
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{IndexStore, SearchOptions, Searcher};
use tempfile::TempDir;

fn chunk(name: &str, excerpt: &str) -> SemanticChunkInput {
    SemanticChunkInput {
        symbol_name: name.into(),
        kind: "function".into(),
        line_start: 1,
        line_end: 1,
        excerpt: excerpt.into(),
        callers: Vec::new(),
        callees: Vec::new(),
        doc: String::new(),
        scope: String::new(),
    }
}

fn upsert<'a>(
    path: &'a str,
    hash: &'a str,
    lines: &'a [(u32, String)],
    chunks: &'a [SemanticChunkInput],
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("python"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        semantic_chunks: chunks,
        embed_semantic: true,
        embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
    }
}

#[test]
fn same_connection_delete_readd_invalidates_semantic_cache() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let searcher = Searcher::with_store(
        store,
        SearchOptions {
            root: temp.path().to_path_buf(),
            use_embed: true,
            ann_threshold: Some(usize::MAX),
            ..SearchOptions::default()
        },
    );

    let old_lines = [(1, "def legacy_handler(): return 'obsolete'".into())];
    let old_chunks = [chunk(
        "legacy_handler",
        "credential legacy obsolete handler",
    )];
    searcher
        .store()
        .upsert_file(upsert("a.py", "old", &old_lines, &old_chunks))
        .unwrap();
    let old = searcher
        .search_semantic("credential legacy obsolete")
        .unwrap();
    assert!(old.hits.iter().any(|hit| {
        hit.kind == HitKind::Embed && hit.symbol.as_deref() == Some("legacy_handler")
    }));

    searcher.store().remove_file("a.py").unwrap();
    let fresh_lines = [(1, "def fresh_handler(): return 'renewed'".into())];
    let fresh_chunks = [chunk("fresh_handler", "payment renewal fresh handler")];
    searcher
        .store()
        .upsert_file(upsert("a.py", "fresh", &fresh_lines, &fresh_chunks))
        .unwrap();

    let fresh = searcher.search_semantic("payment renewal fresh").unwrap();
    assert!(fresh.hits.iter().any(|hit| {
        hit.kind == HitKind::Embed && hit.symbol.as_deref() == Some("fresh_handler")
    }));
    assert!(!fresh
        .hits
        .iter()
        .any(|hit| hit.symbol.as_deref() == Some("legacy_handler")));
}
