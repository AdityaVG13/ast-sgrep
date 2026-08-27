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
fn pattern_callers_join_excludes_non_calling_functions_in_the_same_file() {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "src/app.rs",
        "fn target() {\n    helper();\n}\n\nfn false_positive() {\n    unrelated();\n}\n\nfn helper() {}\nfn unrelated() {}\n",
    );
    let searcher = indexed_searcher(temp.path());

    let response = searcher
        .search("pattern:fn $NAME($$$) AND callers:helper")
        .unwrap();
    assert_eq!(
        response.hits.len(),
        1,
        "span join must remove same-file noise"
    );
    assert_eq!(response.hits[0].kind, HitKind::Pattern);
    assert!(response.hits[0].excerpt.contains("fn target()"));
    assert!(!response.hits[0].excerpt.contains("false_positive"));
    assert!(response.hits[0].contributors.contains(&HitKind::Caller));
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

// --- Quoted payloads containing " AND " must not split or bail (br-9kb) ---

/// Fixture for quote-awareness: `app.rs` carries callers of `helper` plus the
/// byte-exact source line `"cats AND dogs"` (double quotes included — literal
/// payloads are byte-exact, quotes are not stripped). `other.rs` holds a
/// caller of `helper` and a `frobnicate17` token but no `cats AND dogs`, so
/// intersections are decidable by construction.
fn quoted_and_root() -> TempDir {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "src/app.rs",
        "fn helper() {}\nfn caller_one() {\n    helper();\n}\nlet note = \"cats AND dogs\";\n",
    );
    write_src(
        temp.path(),
        "src/other.rs",
        "fn unrelated() {\n    helper();\n}\nlet flag = frobnicate17;\n",
    );
    temp
}

#[test]
fn quoted_and_payload_still_forms_a_two_channel_conjunction() {
    let temp = quoted_and_root();
    let searcher = indexed_searcher(temp.path());
    // The only ` AND ` outside quotes separates the two channels; the one
    // inside `literal:"cats AND dogs"` is payload and must not split or bail.
    let response = searcher
        .search("word:helper AND literal:\"cats AND dogs\"")
        .unwrap();
    assert!(
        !response.hits.is_empty(),
        "word:helper AND literal:\"cats AND dogs\" must execute as a \
         conjunction (word channel ∩ literal channel); falling back to \
         ordinary search silently drops the intersection — got {} hits",
        response.hits.len()
    );
    assert!(
        response.hits.iter().all(|hit| hit.file == "src/app.rs"),
        "intersection must keep only files matched by BOTH channels \
         (other.rs lacks \"cats AND dogs\"): {:?}",
        response
            .hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn quoted_and_not_right_channel_still_subtracts() {
    let temp = quoted_and_root();
    let searcher = indexed_searcher(temp.path());
    let response = searcher
        .search("word:frobnicate17 AND NOT literal:\"skip AND me\"")
        .unwrap();
    assert!(
        response.hits.iter().any(|h| h.file == "src/other.rs"),
        "AND NOT with a quoted right payload must still subtract at file \
         scope and keep the unmatched left side — got {} hits",
        response.hits.len()
    );
}

#[test]
fn single_channel_quoted_and_is_not_a_conjunction() {
    let temp = quoted_and_root();
    let searcher = indexed_searcher(temp.path());
    // Zero unquoted separators: the entire string is one literal payload.
    // Must resolve through the literal channel (byte-exact), not degrade.
    let response = searcher.search("literal:\"cats AND dogs\"").unwrap();
    assert!(
        response
            .hits
            .iter()
            .any(|h| h.file == "src/app.rs" && h.excerpt.contains("\"cats AND dogs\"")),
        "literal:\"cats AND dogs\" must return the byte-exact source line"
    );
    assert!(
        !response.hits.iter().any(|h| h.file == "src/other.rs"),
        "literal channel must stay byte-exact: other.rs has no such bytes"
    );
}
