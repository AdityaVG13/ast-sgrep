use ast_sgrep_core::store::line_corpus::*;
use ast_sgrep_core::store::read_bundle::{self, ReadBundle};
use std::collections::HashSet;
use std::sync::Arc;

fn discovery<'a>(
    corpus: &'a LineCorpus,
    needle: &str,
    cap: usize,
    already: &HashSet<String>,
) -> Vec<LineHit<'a>> {
    let lower = needle.to_lowercase();
    corpus.scan_distinct_files(
        needle,
        cap,
        already,
        None,
        |_, _| true,
        |content| {
            ast_sgrep_core::search::passes::literal::content_matches_literal(
                content,
                needle,
                Some(&lower),
                false,
            )
        },
    )
}

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
fn bundle_roundtrip_matches_sql_corpus() {
    let rows: Vec<ast_sgrep_core::store::IndexedLineRow> = vec![
        (
            Arc::from("a.rs"),
            1,
            "alpha SearchHit".to_string(),
            Some(Arc::from("rust")),
        ),
        (
            Arc::from("a.rs"),
            2,
            "nope".to_string(),
            Some(Arc::from("rust")),
        ),
        (
            Arc::from("b.rs"),
            10,
            "SearchHit again".to_string(),
            Some(Arc::from("rust")),
        ),
        (
            Arc::from("docs/readme.md"),
            3,
            "snapshot notes".to_string(),
            None,
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    read_bundle::write_from_rows(&db, &rows, 7).unwrap();
    let bundle = ReadBundle::open_for_generation(&db, 7)
        .unwrap()
        .expect("bundle exists");
    let corpus = LineCorpus::from_bundle(std::sync::Arc::new(bundle), 7, 1).unwrap();
    let sql = pack(&[
        ("a.rs", Some("rust"), 1, "alpha SearchHit"),
        ("a.rs", Some("rust"), 2, "nope"),
        ("b.rs", Some("rust"), 10, "SearchHit again"),
        ("docs/readme.md", None, 3, "snapshot notes"),
    ]);
    for needle in ["SearchHit", "snapshot", "alpha", "absent"] {
        let expected: Vec<_> = sql
            .scan_cs(needle, false, None, 8, |_, _| true, |_, _, _| true)
            .into_iter()
            .map(|hit| (hit.path.to_string(), hit.line_no, hit.content.to_string()))
            .collect();
        let actual: Vec<_> = corpus
            .scan_cs(needle, false, None, 8, |_, _| true, |_, _, _| true)
            .into_iter()
            .map(|hit| (hit.path.to_string(), hit.line_no, hit.content.to_string()))
            .collect();
        assert_eq!(actual, expected, "scanned rows for {needle:?}");
        let expected_files: Vec<_> = discovery(&sql, needle, 8, &HashSet::new())
            .into_iter()
            .map(|hit| hit.path.to_string())
            .collect();
        let actual_files: Vec<_> = discovery(&corpus, needle, 8, &HashSet::new())
            .into_iter()
            .map(|hit| hit.path.to_string())
            .collect();
        assert_eq!(
            actual_files, expected_files,
            "file discovery for {needle:?}"
        );
    }
}

#[test]
fn bundle_generation_mismatch_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let rows: Vec<ast_sgrep_core::store::IndexedLineRow> =
        vec![(Arc::from("a.rs"), 1, "alpha".to_string(), None)];
    read_bundle::write_from_rows(&db, &rows, 3).unwrap();
    assert!(ReadBundle::open_for_generation(&db, 4).is_err());
    assert!(
        ReadBundle::open_for_generation(&dir.path().join("missing.db"), 3)
            .unwrap()
            .is_none()
    );
}

#[test]
fn ident_token_map_finds_camel_case_pieces() {
    // Assert the posting contract directly: residual full-file scanning can
    // conceal lost token seeds in a small corpus.
    let mut pairs = Vec::new();
    for (fi, text) in [
        (0, "snapshot snapshot 12345 snapshotstamp SnapshotStamp"),
        (0, "snapshot 12345 snake_case"),
        (1, "snapshot 12345 SnapshotStamp snake_case"),
    ] {
        push_ascii_tokens(&mut pairs, text, fi);
    }
    let postings = TokenCsr::from_pairs(pairs);
    for key in [
        "snapshot",
        "12345",
        "snapshotstamp",
        "stamp",
        "snake",
        "case",
        "snake_case",
    ] {
        assert_eq!(postings.get(key), &[0, 1], "posting for {key}");
    }
    let corpus = pack(&[
        ("types.rs", Some("rust"), 10, "pub struct SnapshotStamp {"),
        ("other.rs", Some("rust"), 1, "fn unrelated() {}"),
    ]);
    let already = HashSet::new();
    let hits = discovery(&corpus, "snapshot", 8, &already);
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
    let hits = discovery(&corpus, "snapshot", 1, &already);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "search/types.rs");
}

#[cfg(unix)]
#[test]
fn filename_backslashes_do_not_turn_source_files_into_test_directories() {
    let corpus = pack(&[
        (
            r"a\tests\source.rs",
            Some("rust"),
            1,
            "pub struct SnapshotStamp;",
        ),
        ("z.rs", Some("rust"), 1, "pub struct SnapshotStamp;"),
    ]);
    let hits = discovery(&corpus, "snapshot", 1, &HashSet::new());
    assert_eq!(hits[0].path, r"a\tests\source.rs");
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
    let corpus = pack(&[
        ("a.rs", None, 1, "Search"),
        ("a.rs", None, 2, "Hit"),
        ("b.rs", None, 1, "tail"),
        ("c.rs", None, 1, "head"),
        ("d.rs", None, 1, "embedded\nmarker"),
    ]);
    for needle in ["SearchHit", "Search\nHit", "Search\n", "tail\nhead"] {
        assert!(
            corpus
                .scan_cs(needle, false, None, 8, |_, _| true, |_, _, _| true)
                .is_empty(),
            "literal scan must not stitch records for {needle:?}"
        );
        assert!(
            discovery(&corpus, needle, 8, &HashSet::new()).is_empty(),
            "file discovery must not stitch records for {needle:?}"
        );
    }
    // A newline inside one stored record is data, unlike the packing separator.
    let needle = "embedded\nmarker";
    let literal = corpus.scan_cs(needle, false, None, 8, |_, _| true, |_, _, _| true);
    let discovery = discovery(&corpus, needle, 8, &HashSet::new());
    for hits in [literal, discovery] {
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].path, hits[0].line_no), ("d.rs", 1));
        assert_eq!(hits[0].content, needle);
    }
}

#[test]
fn packed_scan_emits_line_once() {
    let corpus = pack(&[("a.rs", None, 1, "SearchHit and SearchHit again")]);
    let hits = corpus.scan_cs("SearchHit", false, None, 8, |_, _| true, |_, _, _| true);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line_no, 1);
}

#[test]
fn bundle_empty_corpus_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let rows: Vec<ast_sgrep_core::store::IndexedLineRow> = Vec::new();
    read_bundle::write_from_rows(&db, &rows, 1).unwrap();
    let bundle = ReadBundle::open_for_generation(&db, 1)
        .unwrap()
        .expect("empty bundle exists");
    assert_eq!(bundle.line_count(), 0);
    let corpus = LineCorpus::from_bundle(std::sync::Arc::new(bundle), 1, 1).unwrap();
    assert!(corpus
        .scan_cs("anything", false, None, 8, |_, _| true, |_, _, _| true)
        .is_empty());
}

#[test]
fn bundle_truncation_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let rows: Vec<ast_sgrep_core::store::IndexedLineRow> =
        vec![(Arc::from("a.rs"), 1, "alpha".to_string(), None)];
    read_bundle::write_from_rows(&db, &rows, 2).unwrap();
    let path = read_bundle::path_for(&db);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..100.min(bytes.len())]).unwrap();
    let err = ReadBundle::open(&db).expect_err("truncated bundle must refuse");
    let text = err.to_string();
    assert!(
        text.contains("truncated") || text.contains("outside file") || text.contains("mismatch"),
        "{text}"
    );
}

#[test]
fn bundle_fingerprint_mismatch_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let rows: Vec<ast_sgrep_core::store::IndexedLineRow> =
        vec![(Arc::from("a.rs"), 1, "alpha".to_string(), None)];
    read_bundle::write_from_rows(&db, &rows, 5).unwrap();
    let path = read_bundle::path_for(&db);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[64] ^= 0xff;
    std::fs::write(&path, bytes).unwrap();
    let err = ReadBundle::open(&db).expect_err("flipped fingerprint must refuse");
    assert!(err.to_string().contains("fingerprint"), "{}", err);
}

#[test]
fn bundle_scan_is_path_sorted_regardless_of_input_order() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("index.db");
    let rows: Vec<ast_sgrep_core::store::IndexedLineRow> = vec![
        (Arc::from("z.rs"), 1, "SearchHit z".to_string(), None),
        (Arc::from("a.rs"), 1, "SearchHit a".to_string(), None),
        (Arc::from("m.rs"), 1, "SearchHit m".to_string(), None),
    ];
    read_bundle::write_from_rows(&db, &rows, 9).unwrap();
    let bundle = ReadBundle::open_for_generation(&db, 9)
        .unwrap()
        .expect("bundle exists");
    let corpus = LineCorpus::from_bundle(std::sync::Arc::new(bundle), 9, 1).unwrap();
    let hits = corpus.scan_cs("SearchHit", false, None, 8, |_, _| true, |_, _, _| true);
    let paths: Vec<_> = hits.iter().map(|hit| hit.path).collect();
    assert_eq!(paths, ["a.rs", "m.rs", "z.rs"]);
}
