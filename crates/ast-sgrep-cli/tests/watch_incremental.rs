//! Targeted watch updates: update_paths handles exact paths, removals prune, ignore rules hold, same-content no-ops.
use std::fs; use std::path::{Path, PathBuf}; use ast_sgrep_core::index::{IndexOptions, Indexer};
fn temp_project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir"); let root = dir.path().canonicalize().expect("canonicalize");
    fs::write(root.join("alpha.rs"), "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { alpha_one() + 1 }\n").unwrap();
    fs::write(root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").unwrap();
    fs::create_dir_all(root.join("target")).unwrap(); fs::write(root.join("target").join("gen.rs"), "pub fn generated() {}\n").unwrap();
    (dir, root)
}
fn indexer_for(root: &Path) -> Indexer {
    Indexer::new(IndexOptions { root: root.to_path_buf(), embed_semantic: false, respect_gitignore: false, ..IndexOptions::default() }).expect("indexer")
}
#[test] fn update_paths_handles_exact_targets_and_prunes_removals() {
    let (_dir, root) = temp_project(); let mut indexer = indexer_for(&root); indexer.index_all().expect("initial index");
    let stats = indexer.update_paths(&[root.join("alpha.rs")]).expect("noop update"); assert_eq!(stats.files_indexed, 0); assert_eq!(stats.files_skipped, 1);
    fs::write(root.join("alpha.rs"), "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_three() -> u32 { alpha_one() + 2 }\n").unwrap();
    let stats = indexer.update_paths(&[root.join("alpha.rs")]).expect("edit update"); assert_eq!(stats.files_indexed, 1);
    let names: Vec<String> = indexer.store().symbols_in_file("alpha.rs").expect("symbols").into_iter().map(|s| s.name).collect();
    assert!(names.contains(&"alpha_three".to_string()), "got {names:?}"); assert!(!names.contains(&"alpha_two".to_string()), "got {names:?}");
    assert!(!indexer.store().symbols_in_file("beta.rs").expect("beta symbols").is_empty());
    fs::remove_file(root.join("beta.rs")).unwrap();
    let stats = indexer.update_paths(&[root.join("beta.rs")]).expect("removal update");
    assert_eq!(stats.files_removed, 1); assert!(indexer.store().file_hash("beta.rs").expect("hash lookup").is_none());
    let stats = indexer.update_paths(&[root.join("target").join("gen.rs")]).expect("skip update");
    assert_eq!(stats.files_indexed, 0); assert_eq!(stats.files_skipped, 1);
}
