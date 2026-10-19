use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::tantivy_index::TantivySidecar;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
use std::path::Path;

#[test]
fn relative_index_path_writer_stamp_is_next_to_the_actual_database() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let relative = Path::new("private-index/index.db");
    let absolute = ast_sgrep_core::try_index_db_path(root, Some(relative)).unwrap();
    assert_eq!(
        ast_sgrep_core::store::writer_generation_path(root, Some(relative)),
        ast_sgrep_core::store::writer_generation_path(root, Some(&absolute)),
        "relative and absolute spellings of the same database must share an epoch"
    );
}

#[test]
fn adjacent_explicit_databases_do_not_serve_each_others_lexical_rows() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let first_db = temp.path().join("indexes/first.db");
    let second_db = temp.path().join("indexes/second.db");
    for (root, db, source) in [
        (&first, &first_db, "fn foreignneedle() {}\n"),
        (&second, &second_db, "fn localneedle() {}\n"),
    ] {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("a.rs"), source).unwrap();
        Indexer::new(IndexOptions {
            root: root.clone(),
            index_path: Some(db.clone()),
            use_tantivy: true,
            embed_semantic: false,
            ..Default::default()
        })
        .unwrap()
        .index_all()
        .unwrap();
    }
    for (root, db, absent, present) in [
        (first, first_db, "localneedle", "foreignneedle"),
        (second, second_db, "foreignneedle", "localneedle"),
    ] {
        let searcher = Searcher::new(SearchOptions {
            root,
            index_path: Some(db),
            use_tantivy: true,
            use_embed: false,
            ..Default::default()
        })
        .unwrap();
        assert!(searcher.search_lexical(absent).unwrap().hits.is_empty());
        assert!(!searcher.search_lexical(present).unwrap().hits.is_empty());
    }
}

#[test]
fn aborted_cross_root_indexer_preserves_the_committed_root_binding() {
    use std::sync::{atomic::AtomicBool, Arc};

    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let db = temp.path().join("shared/index.db");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("a.rs"), "fn first_root_only() {}\n").unwrap();
    std::fs::write(second.join("a.rs"), "fn second_root_only() {}\n").unwrap();
    let options = |root: &Path| IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.clone()),
        embed_semantic: false,
        ..Default::default()
    };
    Indexer::new(options(&first)).unwrap().index_all().unwrap();
    let before = IndexStore::open_readonly(&first, Some(&db))
        .unwrap()
        .get_meta("root")
        .unwrap();

    let mut aborted = Indexer::new(options(&second)).unwrap();
    aborted.set_cancel(Arc::new(AtomicBool::new(true)));
    assert!(aborted.index_all().is_err());
    drop(aborted);
    let store = IndexStore::open_readonly(&first, Some(&db)).unwrap();
    assert_eq!(store.get_meta("root").unwrap(), before);
    let wrong_root = Searcher::new(SearchOptions {
        root: second,
        index_path: Some(db),
        use_embed: false,
        ..Default::default()
    })
    .unwrap();
    assert!(
        !wrong_root.store().file_exists("a.rs").unwrap(),
        "foreign-root search must use its inert stand-in, not certify old rows for the new root"
    );
}

#[test]
fn trailing_block_comment_prefix_does_not_hide_executable_declarations() {
    let temp = tempfile::tempdir().unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..Default::default()
    })
    .unwrap();
    indexer
        .index_content("a.rs", "fn stable() {}\n/* note */ fn old_name() {}\n")
        .unwrap();
    indexer
        .index_content("a.rs", "fn stable() {}\n/* note */ fn new_name() {}\n")
        .unwrap();
    let mut names: Vec<String> = indexer
        .store()
        .symbols_in_file("a.rs")
        .unwrap()
        .into_iter()
        .map(|symbol| symbol.name)
        .collect();
    names.sort();
    assert_eq!(names, vec!["new_name", "stable"]);
}

#[test]
fn root_binding_requires_complete_cross_root_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let db = temp.path().join("shared/index.db");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("a.rs"), "fn first_only() {}\n").unwrap();
    std::fs::write(first.join("old.rs"), "fn old_only() {}\n").unwrap();
    std::fs::write(second.join("a.rs"), "fn second_only() {}\n").unwrap();
    let options = |root: &Path| IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.clone()),
        embed_semantic: false,
        ..Default::default()
    };
    Indexer::new(options(&first)).unwrap().index_all().unwrap();
    let mut indexer = Indexer::new(options(&second)).unwrap();
    let before = indexer.store().get_meta("root").unwrap();
    assert!(indexer
        .index_content("a.rs", "fn second_only() {}\n")
        .is_err());
    assert!(indexer.index_file(&second.join("a.rs"), "a.rs").is_err());
    assert!(indexer.update_paths(&[second.join("old.rs")]).is_err());
    assert_eq!(indexer.store().get_meta("root").unwrap(), before);
    assert_eq!(
        indexer.store().symbols_in_file("a.rs").unwrap()[0].name,
        "first_only"
    );
    assert!(indexer.store().file_exists("old.rs").unwrap());
    drop(indexer);

    let mut filtered = options(&second);
    filtered.lang_filter = Some("rust".into());
    assert!(Indexer::new(filtered).unwrap().index_all().is_err());
    // A preparation failure is not permission to certify surviving old rows.
    std::fs::write(second.join("a.rs"), [0xff]).unwrap();
    let mut indexer = Indexer::new(options(&second)).unwrap();
    assert!(indexer.index_all().is_err());
    assert_eq!(indexer.store().get_meta("root").unwrap(), before);
    assert!(indexer.store().file_exists("old.rs").unwrap());
    std::fs::write(second.join("a.rs"), "fn second_only() {}\n").unwrap();
    indexer.index_all().unwrap();
    assert_eq!(
        indexer.store().get_meta("root").unwrap(),
        Some(second.canonicalize().unwrap().display().to_string())
    );
    assert!(!indexer.store().file_exists("old.rs").unwrap());
    assert_eq!(
        indexer.store().symbols_in_file("a.rs").unwrap()[0].name,
        "second_only"
    );
}

#[test]
fn initial_partial_root_binding_rolls_back_with_failed_rows() {
    let temp = tempfile::tempdir().unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(indexer.store().get_meta("root").unwrap(), None);
    indexer.store().connection().execute_batch(
        "CREATE TEMP TRIGGER refuse_file BEFORE INSERT ON files BEGIN SELECT RAISE(ABORT, 'injected row failure'); END;"
    ).unwrap();
    assert!(indexer.index_content("a.rs", "fn initial() {}\n").is_err());
    assert_eq!(indexer.store().get_meta("root").unwrap(), None);
    assert!(!indexer.store().file_exists("a.rs").unwrap());
    indexer
        .store()
        .connection()
        .execute_batch("DROP TRIGGER refuse_file")
        .unwrap();
    indexer.index_content("a.rs", "fn initial() {}\n").unwrap();
    assert_eq!(
        indexer.store().get_meta("root").unwrap(),
        Some(temp.path().canonicalize().unwrap().display().to_string())
    );
    assert_eq!(
        indexer.store().symbols_in_file("a.rs").unwrap()[0].name,
        "initial"
    );
}

#[test]
fn copied_or_legacy_lexical_sidecar_is_not_owned_by_the_target_database() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let first = root.join(".asgrep/first.db");
    let second = root.join(".asgrep/second.db");
    for (db, source) in [
        (&first, "fn foreignneedle() {}\n"),
        (&second, "fn localneedle() {}\n"),
    ] {
        std::fs::write(root.join("a.rs"), source).unwrap();
        Indexer::new(IndexOptions {
            root: root.to_path_buf(),
            index_path: Some(db.clone()),
            embed_semantic: false,
            use_tantivy: true,
            ..Default::default()
        })
        .unwrap()
        .index_all()
        .unwrap();
    }
    let generation = IndexStore::open_readonly(root, Some(&second))
        .unwrap()
        .index_data_version()
        .unwrap();
    assert_eq!(
        IndexStore::open_readonly(root, Some(&first))
            .unwrap()
            .index_data_version()
            .unwrap(),
        generation
    );
    let first_sidecar = ast_sgrep_core::tantivy_index::sidecar_path(root, Some(&first));
    let second_sidecar = ast_sgrep_core::tantivy_index::sidecar_path(root, Some(&second));
    std::fs::copy(first_sidecar, &second_sidecar).unwrap();
    for legacy in [false, true] {
        if legacy {
            rusqlite::Connection::open(&second_sidecar)
                .unwrap()
                .execute("DELETE FROM meta WHERE key = 'source_identity'", [])
                .unwrap();
        }
        assert!(!TantivySidecar::open_for_index(root, Some(&second))
            .unwrap()
            .is_fresh(generation)
            .unwrap());
        let searcher = Searcher::new(SearchOptions {
            root: root.to_path_buf(),
            index_path: Some(second.clone()),
            use_tantivy: true,
            use_embed: false,
            ..Default::default()
        })
        .unwrap();
        assert!(searcher
            .search_lexical("foreignneedle")
            .unwrap()
            .hits
            .is_empty());
        assert!(!searcher
            .search_lexical("localneedle")
            .unwrap()
            .hits
            .is_empty());
    }
}

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
    let warm = pass17_searcher(root).search_lexical("alpha_token").unwrap();
    assert!(warm
        .hits
        .iter()
        .any(|hit| hit.excerpt.contains("alpha_token")));

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
        .search(&["beta_replacement".to_string()], 10, None)
        .unwrap();
    assert_eq!(beta_hits.len(), 1, "rebuilt sidecar must hold the new line");
    let searcher = pass17_searcher(root);
    let fresh = searcher.search_lexical("beta_replacement").unwrap();
    assert!(fresh
        .hits
        .iter()
        .any(|hit| hit.excerpt.contains("beta_replacement")));
    let gone = searcher.search_lexical("alpha_token").unwrap();
    assert!(
        gone.hits.is_empty(),
        "replaced content must leave the index"
    );
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
    std::fs::write(
        root.join("alpha_token.rs"),
        "fn alpha_token() -> u32 { 1 }\n",
    )
    .unwrap();
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
