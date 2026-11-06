//! F6: one `core::paths` canonicalization helper with an explicit
//! follow-symlinks yes/no parameter; the historical variants route through
//! it and agree.
//!
//! KILLS: helper-missing (fails to compile before F6 lands), variant
//! divergence (follow=true must equal `std::fs::canonicalize`; follow=false
//! must preserve a missing leaf instead of erroring), and root-identity
//! drift (Indexer + Searcher built through a symlinked root must share the
//! canonical identity and still serve hits).

use ast_sgrep_core::paths::canonicalize_path;
use ast_sgrep_core::{indexed_rel_path, IndexOptions, Indexer, SearchOptions, Searcher};
use std::path::Path;

#[test]
fn paths_helper_unifies_canonicalization_variants() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("corpus");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src").join("main.rs"), "pub fn path_needle() {}\n").unwrap();

    // follow=true is exactly std canonicalize (the 7 follow variants).
    let std_canon = root.canonicalize().unwrap();
    assert_eq!(
        canonicalize_path(&root, true).unwrap(),
        std_canon,
        "follow=true must equal std::fs::canonicalize"
    );

    // follow=false preserves a missing leaf (the index_watch affected-path
    // variant); follow=true refuses it exactly like std does.
    let missing = root.join("not_yet_indexed.rs");
    assert!(missing.canonicalize().is_err());
    assert!(canonicalize_path(&missing, true).is_err());
    assert_eq!(
        canonicalize_path(&missing, false).unwrap(),
        std_canon.join("not_yet_indexed.rs"),
        "follow=false must preserve the missing leaf on the canonical parent"
    );

    // Both policies agree on an existing file.
    let existing = root.join("src").join("main.rs");
    assert_eq!(
        canonicalize_path(&existing, true).unwrap(),
        canonicalize_path(&existing, false).unwrap(),
    );

    // `indexed_rel_path` still guards identity keys through the same module.
    assert!(indexed_rel_path(Path::new("src/main.rs")).is_ok());
    assert!(indexed_rel_path(Path::new("/abs/main.rs")).is_err());
    assert!(indexed_rel_path(Path::new("../escape.rs")).is_err());

    // Indexer + Searcher built through a symlinked root share identity
    // (index.rs:324 and search/mod.rs:178-179 route through the helper).
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link = tmp.path().join("corpus_link");
        symlink(&root, &link).unwrap();
        let db = tmp.path().join("index.db");
        let mut indexer = Indexer::new(IndexOptions {
            root: link.clone(),
            index_path: Some(db.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap();
        indexer.index_all().unwrap();
        assert_eq!(
            indexer.store().root(),
            &std_canon,
            "indexer root through a symlink must be the canonical root"
        );
        let searcher = Searcher::new(SearchOptions {
            root: link,
            index_path: Some(db),
            ..SearchOptions::default()
        })
        .unwrap();
        assert_eq!(
            searcher.options().root,
            std_canon,
            "searcher root through a symlink must be the canonical root"
        );
        let hits = searcher.search("path_needle").unwrap().hits;
        assert!(
            hits.iter().any(|h| h.file.ends_with("main.rs")),
            "symlink-rooted search must serve hits from the canonical identity"
        );
    }
}
