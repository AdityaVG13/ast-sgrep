//! vvpk: identifiers must not be stemmed. Porter is right for prose and wrong
//! for code, and one analyzer cannot serve both.
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};

fn build(root: &std::path::Path) {
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    // `indexing` vs `index`: porter folds these together, so a query for one
    // pulls the other. The code field must keep them distinct.
    std::fs::write(
        src.join("lib.rs"),
        "fn start_indexing(store: &Store) {}\n\
         fn index(store: &Store) {}\n\
         fn refresh_token(session: &Session) {}\n\
         fn refreshing_tokens(session: &Session) {}\n\
         /// Renew an expired login for the current session.\n\
         fn renew(session: &Session) {}\n",
    )
    .expect("write");
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
}

fn search(root: &std::path::Path, query: &str) -> Vec<String> {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        use_embed: false,
        ..SearchOptions::default()
    })
    .expect("searcher")
    .search(query)
    .expect("search")
    .hits
    .into_iter()
    .map(|hit| hit.excerpt)
    .collect()
}

#[test]
fn the_code_field_exists_and_is_populated() {
    let temp = tempfile::tempdir().unwrap();
    build(temp.path());
    let store = IndexStore::open(temp.path(), None).expect("store");
    let rows: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_code_fts", [], |row| row.get(0))
        .expect("code field must exist");
    assert!(rows > 0, "code field must be populated during indexing");

    // Both fields index the same lines; only the analyzer differs.
    let prose: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM lines_fts", [], |row| row.get(0))
        .expect("prose field");
    assert_eq!(rows, prose, "code and prose fields must stay in lockstep");
}

#[test]
fn the_two_analyzers_genuinely_differ() {
    let temp = tempfile::tempdir().unwrap();
    build(temp.path());
    let store = IndexStore::open(temp.path(), None).expect("store");
    let count = |table: &str, term: &str| -> i64 {
        store
            .connection()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {table} MATCH ?1"),
                [term],
                |row| row.get(0),
            )
            .expect("fts query")
    };

    // Porter conflates `indexing` with `index`, so the prose field matches
    // lines that do not contain the queried word at all.
    let prose_indexing = count("lines_fts", "indexing");
    assert!(
        prose_indexing >= 2,
        "porter should conflate indexing/index, got {prose_indexing}"
    );

    // The code field treats `start_indexing` as ONE term, which is the point:
    // an identifier means itself. The trade-off is that a bare substring no
    // longer matches an identifier through this field -- substring search is
    // what the trigram field is for.
    assert_eq!(count("lines_code_fts", "indexing"), 0);
    assert_eq!(count("lines_code_fts", "start_indexing"), 1);
    assert_eq!(
        count("lines_code_fts", "index"),
        1,
        "`index` matches only its own line"
    );

    // And the prose field cannot make that distinction at all.
    assert!(
        count("lines_fts", "index") >= 2,
        "porter cannot separate `index` from `start_indexing`/`indexing`"
    );
}

#[test]
fn underscore_identifiers_stay_one_term_in_the_code_field() {
    let temp = tempfile::tempdir().unwrap();
    build(temp.path());
    let store = IndexStore::open(temp.path(), None).expect("store");
    let hits: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM lines_code_fts WHERE lines_code_fts MATCH 'refresh_token'",
            [],
            |row| row.get(0),
        )
        .expect("code query");
    assert_eq!(
        hits, 1,
        "`refresh_token` must match its own line only, not every line with `token`"
    );
}

#[test]
fn identifier_search_returns_the_identifier_not_its_stem() {
    let temp = tempfile::tempdir().unwrap();
    build(temp.path());
    let excerpts = search(temp.path(), "refresh_token");
    assert!(
        excerpts.iter().any(|e| e.contains("refresh_token")),
        "identifier query must find its own definition: {excerpts:?}"
    );
}

#[test]
fn prose_queries_still_reach_the_stemmed_field() {
    let temp = tempfile::tempdir().unwrap();
    build(temp.path());
    // A natural-language question keeps the porter analyzer, so `expired`
    // still reaches the doc comment that says `expired`.
    let excerpts = search(temp.path(), "renew an expired login");
    assert!(
        !excerpts.is_empty(),
        "prose query must still return results"
    );
}
