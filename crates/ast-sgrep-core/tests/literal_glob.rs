//! Regression for bead ast-sgrep-c2j5 (F-05): literal_sql GLOB/LIKE must treat
//! metacharacters in the needle as literals. Pre-fix, `literal:arr[0]` used
//! GLOB `*arr[0]*`, so `[0]` was a character class and matched `arr0`.
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;

fn index_two_lines(a: &str, b: &str) -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(corpus.path().join("f.rs"), format!("{a}\n{b}\n")).unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    (corpus, index_dir, index_path)
}

fn searcher(root: &std::path::Path, index_path: &std::path::Path) -> Searcher {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.to_path_buf()),
        limit: 32,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap()
}

#[test]
fn literal_bracket_metachar_matches_literally_not_as_glob_class() {
    let (corpus, _idx, index_path) = index_two_lines("let x = arr[0];", "let y = arr0;");
    let searcher = searcher(corpus.path(), &index_path);

    let resp = searcher.search("literal:arr[0]").unwrap();
    assert!(
        resp.hits.iter().any(|h| h.excerpt.contains("arr[0]")),
        "literal:arr[0] must match the bracketed line; got {:#?}",
        resp.hits
    );
    assert!(
        !resp
            .hits
            .iter()
            .any(|h| h.excerpt.contains("arr0") && !h.excerpt.contains("arr[0]")),
        "literal:arr[0] must not match arr0 via GLOB character class; got {:#?}",
        resp.hits
    );
}

#[test]
fn literal_a_bracket_b_matches_literally_not_axb() {
    let (corpus, _idx, index_path) = index_two_lines("token a[b] here", "token axb here");
    let searcher = searcher(corpus.path(), &index_path);

    let resp = searcher.search("literal:a[b]").unwrap();
    assert!(
        resp.hits.iter().any(|h| h.excerpt.contains("a[b]")),
        "literal:a[b] must match literally; got {:#?}",
        resp.hits
    );
    assert!(
        !resp
            .hits
            .iter()
            .any(|h| h.excerpt.contains("axb") && !h.excerpt.contains("a[b]")),
        "literal:a[b] must not match axb; got {:#?}",
        resp.hits
    );
}
