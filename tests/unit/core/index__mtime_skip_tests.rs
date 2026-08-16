use super::{IndexOptions, Indexer};
use std::fs;

#[test]
fn second_index_all_skips_unchanged_files_via_mtime() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(corpus.path().join("main.ts"), "export const value = 1;\n").unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_dir.path().join("index.db")),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    let first = indexer.index_all().unwrap();
    assert_eq!(first.files_indexed, 1);
    assert_eq!(first.files_skipped, 0);

    let second = indexer.index_all().unwrap();
    assert_eq!(second.files_indexed, 0);
    assert_eq!(second.files_skipped, 1);

    fs::write(corpus.path().join("main.ts"), "export const value = 2;\n").unwrap();
    let third = indexer.index_all().unwrap();
    assert_eq!(third.files_indexed, 1);
    assert_eq!(third.files_skipped, 0);
}
