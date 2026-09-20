use ast_sgrep_core::store::line_corpus::*;
use std::collections::HashSet;
use std::sync::Arc;

fn pack(lines: &[(&str, Option<&str>, u32, &str)]) -> Arc<LineCorpus> {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
            "CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT NOT NULL, language TEXT);
             CREATE TABLE lines(file_id INTEGER NOT NULL, line_no INTEGER NOT NULL, content TEXT NOT NULL);",
        )
        .unwrap();
    let mut file_ids: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for &(path, language, line_no, content) in lines {
        let id = match file_ids.get(path) {
            Some(id) => *id,
            None => {
                conn.execute(
                    "INSERT INTO files(path, language) VALUES (?1, ?2)",
                    rusqlite::params![path, language],
                )
                .unwrap();
                let id = conn.last_insert_rowid();
                file_ids.insert(path, id);
                id
            }
        };
        conn.execute(
            "INSERT INTO lines(file_id, line_no, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, line_no, content],
        )
        .unwrap();
    }
    LineCorpus::load(&conn, 1, 1)
        .unwrap()
        .expect("small fixture packs")
}

#[test]
fn ident_token_map_finds_camel_case_pieces() {
    let corpus = pack(&[
        ("types.rs", Some("rust"), 10, "pub struct SnapshotStamp {"),
        ("other.rs", Some("rust"), 1, "fn unrelated() {}"),
    ]);
    let already = HashSet::new();
    let hits = corpus.scan_distinct_files_cs("snapshot", 8, &already);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "types.rs");
    assert_eq!(hits[0].line_no, 10);
}

#[test]
fn ident_token_map_prefers_code_over_markdown() {
    let corpus = pack(&[
        ("README.md", None, 1, "snapshot generation notes"),
        (
            "search/types.rs",
            Some("rust"),
            10,
            "pub struct SnapshotStamp {",
        ),
    ]);
    let already = HashSet::new();
    let hits = corpus.scan_distinct_files_cs("snapshot", 1, &already);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "search/types.rs");
}

#[test]
fn packed_scan_is_path_sorted_and_respects_cap() {
    let corpus = pack(&[
        ("a.rs", Some("rust"), 1, "alpha SearchHit"),
        ("a.rs", Some("rust"), 2, "nope"),
        ("b.rs", Some("rust"), 10, "SearchHit again"),
        ("c.rs", Some("rust"), 3, "SearchHit third"),
    ]);
    let hits = corpus.scan_cs("SearchHit", false, None, 2, |_, _| true, |_, _, _| true);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].path, "a.rs");
    assert_eq!(hits[0].line_no, 1);
    assert_eq!(hits[1].path, "b.rs");
    assert_eq!(hits[1].line_no, 10);
}

#[test]
fn packed_scan_does_not_cross_newlines() {
    let corpus = pack(&[("a.rs", None, 1, "Search"), ("a.rs", None, 2, "Hit")]);
    let hits = corpus.scan_cs("SearchHit", false, None, 8, |_, _| true, |_, _, _| true);
    assert!(hits.is_empty());
}

#[test]
fn packed_scan_emits_line_once() {
    let corpus = pack(&[("a.rs", None, 1, "SearchHit and SearchHit again")]);
    let hits = corpus.scan_cs("SearchHit", false, None, 8, |_, _| true, |_, _, _| true);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line_no, 1);
}
