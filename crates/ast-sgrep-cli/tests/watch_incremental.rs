//! Targeted watch updates: update_paths handles exact paths, removals prune, ignore rules hold, same-content no-ops.
use ast_sgrep_core::index::{IndexOptions, Indexer};
use std::fs;
use std::path::{Path, PathBuf};
fn temp_project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().expect("canonicalize");
    fs::write(
        root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { alpha_one() + 1 }\n",
    )
    .unwrap();
    fs::write(root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").unwrap();
    fs::create_dir_all(root.join("target")).unwrap();
    fs::write(
        root.join("target").join("gen.rs"),
        "pub fn generated() {}\n",
    )
    .unwrap();
    (dir, root)
}
fn indexer_for(root: &Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        respect_gitignore: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
}
#[test]
fn update_paths_handles_exact_targets_and_prunes_removals() {
    let (_dir, root) = temp_project();
    let mut indexer = indexer_for(&root);
    indexer.index_all().expect("initial index");
    let stats = indexer
        .update_paths(&[root.join("alpha.rs")])
        .expect("noop update");
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_skipped, 1);
    fs::write(
        root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_three() -> u32 { alpha_one() + 2 }\n",
    )
    .unwrap();
    let stats = indexer
        .update_paths(&[root.join("alpha.rs")])
        .expect("edit update");
    assert_eq!(stats.files_indexed, 1);
    let names: Vec<String> = indexer
        .store()
        .symbols_in_file("alpha.rs")
        .expect("symbols")
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(names.contains(&"alpha_three".to_string()), "got {names:?}");
    assert!(!names.contains(&"alpha_two".to_string()), "got {names:?}");
    assert!(!indexer
        .store()
        .symbols_in_file("beta.rs")
        .expect("beta symbols")
        .is_empty());
    fs::remove_file(root.join("beta.rs")).unwrap();
    let stats = indexer
        .update_paths(&[root.join("beta.rs")])
        .expect("removal update");
    assert_eq!(stats.files_removed, 1);
    assert!(indexer
        .store()
        .file_hash("beta.rs")
        .expect("hash lookup")
        .is_none());
    let stats = indexer
        .update_paths(&[root.join("target").join("gen.rs")])
        .expect("skip update");
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_skipped, 1);
}

#[test]
fn update_paths_is_bounded_and_prunes_newly_ignored_rows() {
    let (_dir, root) = temp_project();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        embed_semantic: false,
        respect_gitignore: true,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("initial index");
    assert!(indexer.store().file_hash("beta.rs").unwrap().is_some());

    fs::write(root.join(".gitignore"), "beta.rs\n").expect("ignore beta");
    let stats = indexer
        .update_paths(&[root.join("beta.rs"), root.join("alpha.rs")])
        .expect("targeted update");
    assert_eq!(stats.files_removed, 1);
    assert!(indexer.store().file_hash("beta.rs").unwrap().is_none());

    let too_many = vec![root.join("alpha.rs"); ast_sgrep_core::MAX_INCREMENTAL_PATHS + 1];
    let error = indexer
        .update_paths(&too_many)
        .expect_err("oversized update must be rejected");
    assert!(error.to_string().contains("exceeds max"));
}

#[test]
fn update_paths_reports_language_filter_removal_as_removed() {
    let (_dir, root) = temp_project();
    let mut initial = indexer_for(&root);
    initial.index_all().expect("initial index");
    drop(initial);

    let mut filtered = Indexer::new(IndexOptions {
        root: root.clone(),
        embed_semantic: false,
        lang_filter: Some("python".into()),
        ..IndexOptions::default()
    })
    .expect("filtered indexer");
    let stats = filtered
        .update_paths(&[root.join("alpha.rs")])
        .expect("targeted filtered update");
    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_indexed, 0);
    assert!(filtered.store().file_hash("alpha.rs").unwrap().is_none());
}

#[test]
fn update_paths_prunes_a_file_after_its_parent_directories_are_removed() {
    let (_dir, root) = temp_project();
    let nested = root.join("nested/inner/removed.rs");
    fs::create_dir_all(nested.parent().expect("nested parent")).expect("create nested parent");
    fs::write(&nested, "pub fn removed_with_parent() {}\n").expect("write nested source");
    let mut indexer = indexer_for(&root);
    indexer.index_all().expect("initial index");
    assert!(indexer
        .store()
        .file_hash("nested/inner/removed.rs")
        .expect("nested hash")
        .is_some());

    fs::remove_dir_all(root.join("nested")).expect("remove nested tree");
    let stats = indexer
        .update_paths(&[root.join("nested")])
        .expect("removed tree update");
    assert_eq!(stats.files_removed, 1);
    assert!(indexer
        .store()
        .file_hash("nested/inner/removed.rs")
        .expect("removed nested hash")
        .is_none());
}

#[test]
fn update_paths_prunes_descendants_when_a_directory_becomes_a_file() {
    let (_dir, root) = temp_project();
    let replaced = root.join("node.rs");
    fs::create_dir_all(&replaced).expect("create directory-shaped path");
    fs::write(replaced.join("old.rs"), "pub fn stale_descendant() {}\n").expect("write descendant");
    let mut indexer = indexer_for(&root);
    indexer.index_all().expect("initial index");
    assert!(indexer
        .store()
        .file_hash("node.rs/old.rs")
        .expect("descendant hash")
        .is_some());

    fs::remove_dir_all(&replaced).expect("remove old directory");
    fs::write(&replaced, "pub fn replacement_file() {}\n").expect("write replacement file");
    let stats = indexer
        .update_paths(std::slice::from_ref(&replaced))
        .expect("replacement update");

    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_indexed, 1);
    assert!(indexer
        .store()
        .file_hash("node.rs/old.rs")
        .expect("stale descendant hash")
        .is_none());
    assert!(indexer
        .store()
        .file_hash("node.rs")
        .expect("replacement hash")
        .is_some());
}

#[test]
fn update_paths_preserves_descendants_when_a_replacement_file_cannot_be_indexed() {
    let (_dir, root) = temp_project();
    let replaced = root.join("node.rs");
    fs::create_dir_all(&replaced).expect("create directory-shaped path");
    fs::write(replaced.join("old.rs"), "pub fn retained_descendant() {}\n")
        .expect("write descendant");
    let mut indexer = indexer_for(&root);
    indexer.index_all().expect("initial index");

    fs::remove_dir_all(&replaced).expect("remove old directory");
    fs::write(&replaced, [0xff]).expect("write invalid replacement");
    let stats = indexer
        .update_paths(std::slice::from_ref(&replaced))
        .expect("failed replacement is reported in stats");

    assert_eq!(stats.files_failed, 1);
    assert_eq!(stats.files_removed, 0);
    assert!(indexer
        .store()
        .file_hash("node.rs/old.rs")
        .expect("retained descendant hash")
        .is_some());
    assert!(indexer
        .store()
        .file_hash("node.rs")
        .expect("invalid replacement hash")
        .is_none());
}

#[cfg(unix)]
#[test]
fn update_paths_removes_replaced_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;

    let (_dir, root) = temp_project();
    let outside = tempfile::tempdir().expect("outside");
    let outside_source = outside.path().join("outside.rs");
    fs::write(&outside_source, "pub fn outside_secret() {}\n").expect("outside source");
    let alpha = root.join("alpha.rs");
    let mut indexer = indexer_for(&root);
    indexer.index_all().expect("initial index");

    fs::remove_file(&alpha).expect("remove alpha");
    symlink(&outside_source, &alpha).expect("outside symlink");
    let stats = indexer.update_paths(&[alpha]).expect("symlink update");
    assert_eq!(stats.files_removed, 1);
    assert!(indexer
        .store()
        .file_hash("alpha.rs")
        .expect("alpha hash")
        .is_none());
}

#[cfg(unix)]
#[test]
fn update_paths_rejects_intermediate_symlink_escape() {
    use std::os::unix::fs::symlink;

    let (_dir, root) = temp_project();
    let outside = tempfile::tempdir().expect("outside");
    let outside_source = outside.path().join("outside.rs");
    fs::write(&outside_source, "pub fn outside_secret() {}\n").expect("outside source");
    symlink(outside.path(), root.join("escaped")).expect("directory symlink");
    let mut indexer = indexer_for(&root);
    indexer.index_all().expect("initial index");

    let stats = indexer
        .update_paths(&[root.join("escaped/outside.rs")])
        .expect("escaped update is ignored");
    assert_eq!(stats.files_indexed, 0);
    assert!(indexer
        .store()
        .file_hash("escaped/outside.rs")
        .expect("escaped hash")
        .is_none());
}

#[cfg(unix)]
#[test]
fn update_paths_refuses_symlink_escape_into_index() {
    use std::os::unix::fs::symlink;

    let (_dir, root) = temp_project();
    let outside = tempfile::tempdir().expect("outside");
    let secret = outside.path().join("secret.rs");
    fs::write(&secret, "pub fn leaked_secret() {}\n").unwrap();
    let link = root.join("escape.rs");
    symlink(&secret, &link).expect("symlink");

    let mut indexer = indexer_for(&root);
    indexer.index_all().expect("initial index");
    assert!(
        indexer
            .store()
            .file_hash("escape.rs")
            .expect("hash")
            .is_none(),
        "full index must not follow symlinks"
    );

    let stats = indexer
        .update_paths(&[link])
        .expect("symlink update must not error");
    assert_eq!(
        stats.files_indexed, 0,
        "watch must not index through symlink escape"
    );
    assert!(
        indexer
            .store()
            .file_hash("escape.rs")
            .expect("hash")
            .is_none(),
        "symlink escape must not land in the index"
    );
    let leaked = indexer
        .store()
        .symbols_named("leaked_secret", 8)
        .expect("symbols");
    assert!(
        leaked.is_empty(),
        "outside content must not appear via watch symlink; got {leaked:?}"
    );
}
