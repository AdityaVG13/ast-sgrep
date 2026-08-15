//! End-to-end evidence for two-channel conjunction queries
//! (P0 channel-conjunction): `<channel> AND [NOT] <channel>` through
//! `Searcher::search` against a real index.
use ast_sgrep_core::search::{HitKind, SearchOptions, Searcher};
use ast_sgrep_core::{IndexOptions, Indexer};
use std::fs;
use tempfile::TempDir;

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn indexed_searcher(root: &std::path::Path) -> Searcher {
    indexed_searcher_with_limit(root, SearchOptions::default().limit)
}

fn indexed_searcher_with_limit(root: &std::path::Path, limit: usize) -> Searcher {
    let index_path = root.join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path),
        limit,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap()
}

fn sample_root() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_src(
        root,
        "src/app.rs",
        "fn helper() {}\nfn caller_one() {\n    helper();\n}\n",
    );
    write_src(root, "src/other.rs", "fn unrelated() {\n    helper();\n}\n");
    temp
}

#[test]
fn and_intersects_two_channels_by_file() {
    let temp = sample_root();
    let searcher = indexed_searcher(temp.path());
    // Callers of helper exist in both files; only src/app.rs contains the
    // literal caller_one, so the conjunction must narrow to that file.
    let response = searcher
        .search("callers:helper AND literal:caller_one")
        .unwrap();
    assert!(!response.hits.is_empty(), "conjunction must hit");
    assert!(
        response.hits.iter().all(|hit| hit.file == "src/app.rs"),
        "AND must keep only files matched by both channels: {:?}",
        response
            .hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        response
            .hits
            .iter()
            .any(|hit| hit.contributors.contains(&HitKind::Caller)),
        "left channel identity must be caller evidence"
    );
    assert_eq!(response.query, "callers:helper AND literal:caller_one");
}

#[test]
fn and_not_subtracts_the_right_channel() {
    let temp = sample_root();
    let searcher = indexed_searcher(temp.path());
    let response = searcher
        .search("callers:helper AND NOT literal:caller_one")
        .unwrap();
    assert!(!response.hits.is_empty(), "negated conjunction must hit");
    assert!(
        response.hits.iter().all(|hit| hit.file == "src/other.rs"),
        "AND NOT must drop files matched by the right channel: {:?}",
        response
            .hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn conjunction_with_pattern_channel_joins_graph_and_structure() {
    let temp = sample_root();
    let searcher = indexed_searcher(temp.path());
    let response = searcher
        .search("callers:helper AND pattern:fn $NAME($$$)")
        .unwrap();
    assert!(
        !response.hits.is_empty(),
        "caller + pattern conjunction must hit"
    );
    for hit in &response.hits {
        assert!(
            hit.contributors
                .iter()
                .any(|kind| matches!(kind, HitKind::Caller | HitKind::Graph)),
            "hits keep left-channel identity: {:?}",
            hit.contributors
        );
    }
}

#[test]
fn plain_english_and_still_searches_hybrid() {
    let temp = sample_root();
    let searcher = indexed_searcher(temp.path());
    // Unprefixed sides: AND is plain text, not an operator. Must not error.
    let response = searcher.search("helper AND caller_one").unwrap();
    assert_eq!(response.query, "helper AND caller_one");
}

#[test]
fn conjunction_results_are_deterministic() {
    let temp = sample_root();
    let searcher = indexed_searcher(temp.path());
    let first = searcher
        .search("callers:helper AND pattern:fn $NAME($$$)")
        .unwrap();
    let second = searcher
        .search("callers:helper AND pattern:fn $NAME($$$)")
        .unwrap();
    let key = |response: &ast_sgrep_core::SearchResponse| {
        response
            .hits
            .iter()
            .map(|hit| (hit.file.clone(), hit.line_start, hit.line_end))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&first), key(&second));
}

#[test]
fn conjunction_finds_intersection_beyond_normal_channel_page() {
    let temp = TempDir::new().unwrap();
    for index in 0..205 {
        let marker = if index == 204 {
            "late_intersection();"
        } else {
            ""
        };
        write_src(
            temp.path(),
            &format!("src/caller_{index:03}.rs"),
            &format!("fn caller_{index:03}() {{ helper(); {marker} }}\n"),
        );
    }
    let searcher = indexed_searcher_with_limit(temp.path(), 1);
    let response = searcher
        .search("callers:helper AND literal:late_intersection")
        .unwrap();
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.hits[0].file, "src/caller_204.rs");
}

#[test]
fn and_not_removes_right_match_beyond_normal_channel_page() {
    let temp = TempDir::new().unwrap();
    for index in 0..205 {
        let marker = if index == 204 {
            "late_left_marker();"
        } else {
            ""
        };
        write_src(
            temp.path(),
            &format!("src/caller_{index:03}.rs"),
            &format!("fn caller_{index:03}() {{ helper(); {marker} }}\n"),
        );
    }
    let searcher = indexed_searcher_with_limit(temp.path(), 1);
    let response = searcher
        .search("literal:late_left_marker AND NOT callers:helper")
        .unwrap();
    assert!(
        response.hits.is_empty(),
        "late right match must subtract left"
    );
}
