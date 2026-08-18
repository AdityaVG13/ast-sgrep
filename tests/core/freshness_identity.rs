use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::tantivy_index::TantivySidecar;
use ast_sgrep_core::{IndexStore, SearchOptions, Searcher};

fn plain_input<'a>(
    path: &'a str,
    hash: &'a str,
    lines: &'a [(u32, String)],
) -> UpsertFileInput<'a> {
    UpsertFileInput {
        rel_path: path,
        language: Some("rust"),
        mtime_secs: 1,
        mtime_nanos: 0,
        content_hash: hash,
        lines,
        eol: "\n",
        symbols: &[],
        callers: &[],
        imports: &[],
        pattern_nodes: &[],
        semantic_chunks: &[],
        embed_semantic: false,
        embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
    }
}

/// Lexical sidecar identity: when the source generation advances, stale Tantivy
/// must miss and lexical search still returns fresh lines.
#[test]
fn lexical_sidecar_falls_back_when_source_generation_changes() {
    let temp = tempfile::tempdir().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    let first = [(1, "alpha token".into())];
    store
        .upsert_file(plain_input("src/lib.rs", "one", &first))
        .unwrap();
    let generation = store.index_data_version().unwrap();
    let sidecar = TantivySidecar::open(temp.path()).unwrap();
    sidecar
        .rebuild_from_lines_with_generation(&store.all_indexed_lines().unwrap(), generation)
        .unwrap();
    assert!(sidecar.is_fresh(generation).unwrap());

    let second = [(1, "beta replacement".into())];
    store
        .upsert_file(plain_input("src/lib.rs", "two", &second))
        .unwrap();
    assert!(!sidecar
        .is_fresh(store.index_data_version().unwrap())
        .unwrap());
    let searcher = Searcher::with_store(
        store,
        SearchOptions {
            root: temp.path().to_path_buf(),
            use_tantivy: true,
            use_embed: false,
            ..SearchOptions::default()
        },
    );
    let response = searcher.search_lexical("beta").unwrap();
    assert!(response.hits.iter().any(|hit| hit.excerpt.contains("beta")));
}
