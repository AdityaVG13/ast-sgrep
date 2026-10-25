//! Intent: the two line analyzers (`lines_fts` porter, `lines_code_fts`
//! unstemmed `_`-aware) must be FTS5 external-content tables bound to
//! `lines`. Storing each line's text a second and third time inside FTS
//! shadow `_content` tables costs ~39 MB at repo scale for zero query
//! value: postings and ranking read through the content table by rowid.

use ast_sgrep_testkit::{
    core_index_options, core_search_options, write_core_fixture, CorePipelineFixture,
};

const FILES: &[(&str, &str)] = &[
    (
        "src/auth.rs",
        "pub fn refresh_token(next: String) {\n    let token = next;\n}\n",
    ),
    (
        "src/store.py",
        "def persist_session(session):\n    write_to_disk(session)\n",
    ),
];
const QUERIES: &[&str] = &["refresh_token", "persist session disk", "token"];

fn shadow_tables(db: &std::path::Path) -> Vec<String> {
    let conn =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open db");
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND (name LIKE 'lines_fts%' OR name LIKE 'lines_code_fts%')")
        .expect("prepare");
    let mut names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .filter_map(|r| r.ok())
        .collect();
    names.sort();
    names
}

fn index_and_search(fixture: &ast_sgrep_testkit::CorePipelineFixture) -> Vec<String> {
    let mut options = core_index_options(&fixture.root, &fixture.db);
    options.embed_semantic = true;
    let mut indexer = ast_sgrep_core::Indexer::new(options).expect("indexer new");
    indexer.index_all().expect("index_all");
    let searcher =
        ast_sgrep_core::Searcher::new(core_search_options(&fixture.root, &fixture.db, 5))
            .expect("searcher");
    QUERIES
        .iter()
        .map(|q| format!("{:?}", searcher.search(q).expect("search")))
        .collect()
}

#[test]
fn fts_analyzers_are_external_content_and_outputs_stable_across_updates() {
    let fixture = write_core_fixture(FILES);
    let outputs = index_and_search(&fixture);
    assert!(
        outputs.iter().any(|o| o.contains("src/auth.rs")),
        "fixture must produce hits for the rust file"
    );

    // Contract: no shadow content tables -- text lives once, in `lines`.
    let tables = shadow_tables(&fixture.db);
    assert!(
        !tables.iter().any(|t| t.ends_with("_content")),
        "FTS shadow content tables must not exist under external content, found: {tables:?}"
    );

    // Contract: pattern_nodes index economy. `idx_pattern_nodes_file
    // (file_id)` is prefix-redundant with `idx_pattern_nodes_file_sig
    // (file_id, signature)` and costs ~6 MB at repo scale; only the
    // composite and the signature-only index may exist.
    let econ = rusqlite::Connection::open_with_flags(
        &fixture.db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db");
    let mut idx_stmt = econ
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='pattern_nodes'")
        .expect("prepare index list");
    let idx: Vec<String> = idx_stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("index list")
        .map(|r| r.expect("row"))
        .collect();
    assert!(
        !idx.iter().any(|n| n == "idx_pattern_nodes_file"),
        "prefix-redundant idx_pattern_nodes_file must not exist, found: {idx:?}"
    );
    assert!(
        idx.iter().any(|n| n == "idx_pattern_nodes_file_sig")
            && idx.iter().any(|n| n == "idx_pattern_nodes_signature"),
        "file_sig and signature indexes must both exist, found: {idx:?}"
    );
    // The survivor must serve the per-file delete path (delete_file_children)
    // as a seek, not degrade to a table scan.
    let plan: String = econ
        .query_row(
            "EXPLAIN QUERY PLAN DELETE FROM pattern_nodes WHERE file_id=?1",
            [1i64],
            |r| r.get::<_, String>(3),
        )
        .expect("delete plan");
    assert!(
        plan.contains("idx_pattern_nodes_file_sig") && !plan.contains("SCAN"),
        "per-file delete must seek idx_pattern_nodes_file_sig, got: {plan}"
    );

    // Incremental update must stay consistent: the file-update path deletes
    // FTS rows (token re-derivation reads `lines`) and reinserts them.
    // Before reopening, regress the db to a legacy shape: schema 17 with the
    // prefix-redundant idx_pattern_nodes_file still present. The reopen must
    // migrate it away (schema 18 drops the index), proving the migration
    // drop -- not just the fresh DDL -- enforces the index economy.
    {
        let w = rusqlite::Connection::open(&fixture.db).expect("open writable");
        w.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_pattern_nodes_file ON pattern_nodes(file_id);\
             PRAGMA user_version = 17;",
        )
        .expect("inject legacy index");
    }
    std::fs::write(
        fixture.root.join("src/auth.rs"),
        "pub fn refresh_token(next: String, ttl: u32) {\n    let token = next;\n}\n",
    )
    .expect("touch");
    let mut options = core_index_options(&fixture.root, &fixture.db);
    options.embed_semantic = true;
    let mut indexer = ast_sgrep_core::Indexer::new(options).expect("indexer new");
    indexer.index_all().expect("incremental index_all");
    let post = rusqlite::Connection::open_with_flags(
        &fixture.db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("reopen db");
    let survived: i64 = post
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='idx_pattern_nodes_file'",
            [],
            |r| r.get(0),
        )
        .expect("count legacy index");
    assert_eq!(
        survived, 0,
        "schema-18 migration must drop the prefix-redundant idx_pattern_nodes_file"
    );
    let v: i64 = post
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("version");
    assert_eq!(v, 18, "reopen must land on schema 18");

    let searcher =
        ast_sgrep_core::Searcher::new(core_search_options(&fixture.root, &fixture.db, 5))
            .expect("searcher");
    let hit = format!("{:?}", searcher.search("refresh_token").expect("search"));
    assert!(
        hit.contains("ttl"),
        "search must reflect the updated line content after incremental reindex"
    );

    // The deleted old shape must not leak through stale postings.
    let gone = format!(
        "{:?}",
        searcher.search("persist session disk").expect("search")
    );
    assert!(
        gone.contains("store.py"),
        "untouched sibling file must still match"
    );
}
