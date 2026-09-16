use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::tantivy_index::TantivySidecar;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
use std::path::Path;

fn pass17_indexer(root: &Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        use_tantivy: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap()
}

fn pass17_searcher(root: &Path) -> Searcher {
    let store = IndexStore::open(root, None).unwrap();
    Searcher::with_store(
        store,
        SearchOptions {
            root: root.to_path_buf(),
            use_tantivy: true,
            use_embed: false,
            ..SearchOptions::default()
        },
    )
}

/// Pass-17 H-PERF-002 lever: a noop refresh (zero changed files) must SKIP
/// the FTS5 sidecar rebuild while the sidecar's stored `source_generation`
/// equals the current `index_data_version`, and a real content change must
/// still rebuild (the mutation bumps the generation inside its transaction,
/// so the freshness equality breaks exactly when work happened).
///
/// Discriminator without timing: after a verified index, replace the sidecar
/// content in-band with an empty rebuild stamped at the CURRENT generation.
/// The freshness equality still holds, so a refresh that honors the skip
/// leaves the emptied sidecar alone; the old unconditional rebuild rewrites
/// it from the lines table and wipes the marker.
#[test]
fn noop_refresh_skips_fresh_sidecar_and_real_change_rebuilds() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/one.rs"), "fn alpha_token() {}\n").unwrap();
    std::fs::write(root.join("src/two.rs"), "fn second_thing() {}\n").unwrap();

    let stats = pass17_indexer(root).index_all().unwrap();
    assert_eq!(stats.files_indexed, 2);

    // Warm lexical search is served from the fresh sidecar.
    let warm = pass17_searcher(root)
        .search_lexical("alpha_token")
        .unwrap();
    assert!(warm.hits.iter().any(|hit| hit.excerpt.contains("alpha_token")));

    // In-band tamper: empty the sidecar but stamp it with the CURRENT
    // generation, so the skip predicate's freshness equality holds.
    {
        let store = IndexStore::open(root, None).unwrap();
        let generation = store.index_data_version().unwrap();
        drop(store);
        let sidecar = TantivySidecar::open(root).unwrap();
        sidecar
            .rebuild_from_lines_with_generation(&[], generation)
            .unwrap();
        assert!(!sidecar.is_search_ready().unwrap());
    }

    // Noop refresh: the walk finds both files unchanged.
    let stats = pass17_indexer(root).index_all().unwrap();
    assert_eq!(stats.files_indexed, 0, "noop refresh must re-index nothing");
    assert_eq!(stats.files_removed, 0);

    // THE SKIP: the emptied sidecar was not rewritten from the lines table.
    // (Failure-first: the unconditional rebuild restores the rows and this
    // assertion fails against pre-lever code.)
    let sidecar = TantivySidecar::open(root).unwrap();
    assert!(
        !sidecar.is_search_ready().unwrap(),
        "noop refresh must skip the sidecar rebuild while the stored generation is current"
    );

    // Mutation test: a real content change bumps the generation and must
    // force the rebuild, restoring sidecar rows from the new content.
    std::fs::write(root.join("src/one.rs"), "fn beta_replacement() {}\n").unwrap();
    let stats = pass17_indexer(root).index_all().unwrap();
    assert_eq!(stats.files_indexed, 1);
    let sidecar = TantivySidecar::open(root).unwrap();
    assert!(
        sidecar.is_search_ready().unwrap(),
        "a real file change must rebuild the sidecar"
    );
    let beta_hits = sidecar
        .search(&["beta_replacement".to_string()], 10)
        .unwrap();
    assert_eq!(beta_hits.len(), 1, "rebuilt sidecar must hold the new line");
    let searcher = pass17_searcher(root);
    let fresh = searcher.search_lexical("beta_replacement").unwrap();
    assert!(fresh.hits.iter().any(|hit| hit.excerpt.contains("beta_replacement")));
    let gone = searcher.search_lexical("alpha_token").unwrap();
    assert!(gone.hits.is_empty(), "replaced content must leave the index");
}

/// The lever's correctness predicate: search output is byte-identical
/// before and after a noop refresh (the skip changes no observable search
/// state).
#[test]
fn noop_refresh_keeps_search_output_byte_identical() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/one.rs"), "fn alpha_token() {}\n").unwrap();
    std::fs::write(root.join("src/two.rs"), "fn second_thing() {}\n").unwrap();

    pass17_indexer(root).index_all().unwrap();
    let before = pass17_searcher(root).search_lexical("alpha_token").unwrap();
    pass17_indexer(root).index_all().unwrap();
    let after = pass17_searcher(root).search_lexical("alpha_token").unwrap();
    assert_eq!(
        serde_json::to_string(&before).unwrap(),
        serde_json::to_string(&after).unwrap(),
        "noop refresh must not change search output"
    );
}

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
        depth_truncated: false,
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

/// The git_head probe-once cache served a stale HEAD for the whole Searcher
/// lifetime: a branch switch (or same-branch commit) after the first stamp
/// kept reporting the old head in every later response. The stamp-level memo
/// (`cached_stamp_parts`) already documents git_head as deliberately
/// uncached because `.git/HEAD` can move without any index write; the
/// per-Searcher probe-once cache contradicted that intent. Two capped file
/// reads per stamp are cheap; the value must be fresh.
#[test]
fn git_head_stamp_tracks_branch_switch_within_one_searcher() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let heads = root.join(".git").join("refs").join("heads");
    std::fs::create_dir_all(&heads).unwrap();
    std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        heads.join("main"),
        "1111111111111111111111111111111111111111\n",
    )
    .unwrap();
    std::fs::write(root.join("alpha_token.rs"), "fn alpha_token() {}\n").unwrap();
    pass17_indexer(root).index_all().unwrap();

    let searcher = pass17_searcher(root);
    let first = searcher.search("alpha_token").unwrap();
    assert_eq!(
        first.snapshot.git_head.as_deref(),
        Some("1111111111111111111111111111111111111111"),
        "first stamp must resolve the ref-form HEAD"
    );

    // Branch switch + commit land AFTER the first stamp. An index write bumps
    // the generation so the generation-keyed response cache cannot serve the
    // first response wholesale — the stamp path itself must re-execute, and
    // the probe-once git_head memo was precisely what then went stale.
    std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature\n").unwrap();
    std::fs::write(
        heads.join("feature"),
        "2222222222222222222222222222222222222222\n",
    )
    .unwrap();
    std::fs::write(root.join("alpha_token.rs"), "fn alpha_token() -> u32 { 1 }\n").unwrap();
    pass17_indexer(root).index_all().unwrap();
    let second = searcher.search("alpha_token").unwrap();
    assert_eq!(
        second.snapshot.git_head.as_deref(),
        Some("2222222222222222222222222222222222222222"),
        "stale probe-once cache served the old branch head"
    );
}

/// 80c (owner-ruled 2026-09-15): the `index_data_version` memo must not serve
/// a stale value after ANOTHER connection (a concurrent CLI reindex) commits a
/// bump. Same-connection writes already invalidate via `bump_index_data_
/// version`; the cross-process window is the registered 80c risk.
#[test]
fn index_data_version_memo_sees_cross_connection_bumps() {
    let root = tempfile::TempDir::new().unwrap();
    let store = IndexStore::open(root.path(), None).unwrap();
    let before = store.index_data_version().unwrap();

    // A separate indexer process/connection writes and bumps the version.
    std::fs::write(root.path().join("fresh.rs"), "fn fresh_marker() {}\n").unwrap();
    pass17_indexer(root.path()).index_all().unwrap();

    let after = store.index_data_version().unwrap();
    assert_eq!(
        after,
        before + 1,
        "memo must re-read after another connection commits; stale-fresh forever is the 80c defect"
    );
}
