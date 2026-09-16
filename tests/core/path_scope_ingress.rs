//! FB-80a-07/08/09 (owner-ruled 2026-09-15): `in:` path scopes are LOUD.
//!
//! - an unresolvable scope (bare `in:`, a `..` segment, an absolute path, a
//!   duplicate token, or a path that exists nowhere under the index root)
//!   refuses the search with a structured error instead of silently widening
//!   the search or answering silent-empty (07/08);
//! - an extension-less FILE scope pins that exact file instead of expanding
//!   to an impossible `file/**` glob (08);
//! - `in:`-looking text inside a double-quoted literal stays in the pattern
//!   (09) — scope tokens are only honored outside quotes.
//!
//! RED-first: every assertion here failed against the drop-silently splitter
//! (bare/`..`/duplicate scopes ran unscoped, exact files never matched, quoted
//! text was stolen into a bogus scope).
use ast_sgrep_core::{query::ParsedQuery, IndexOptions, SearchOptions, Searcher};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_src(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn indexed_scope_corpus() -> (TempDir, Searcher) {
    let temp = TempDir::new().unwrap();
    write_src(temp.path(), "calc.rs", "fn calc_one() { 1 }\n");
    write_src(temp.path(), "src/other.rs", "fn other_one() { 2 }\n");
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 12,
        ..SearchOptions::default()
    })
    .expect("searcher");
    (temp, searcher)
}

#[test]
fn bare_in_token_refuses_loudly() {
    let (_t, searcher) = indexed_scope_corpus();
    let err = searcher
        .search("pattern:fn $F() { $$$B } in:")
        .expect_err("bare `in:` must refuse, not run unscoped (FB-80a-07)");
    assert!(err.to_string().contains("in:"), "{err}");
}

#[test]
fn dotdot_scope_refuses_loudly() {
    let (_t, searcher) = indexed_scope_corpus();
    let err = searcher
        .search("pattern:fn $F() { $$$B } in:../outside")
        .expect_err("`..` scope must refuse, not silently widen (FB-80a-07)");
    assert!(err.to_string().contains(".."), "{err}");
}

#[test]
fn absolute_scope_refuses_loudly() {
    let (_t, searcher) = indexed_scope_corpus();
    let err = searcher
        .search("pattern:fn $F() { $$$B } in:/tmp")
        .expect_err("absolute scope must refuse: scopes are root-relative (FB-80a-08)");
    assert!(err.to_string().contains("/tmp"), "{err}");
}

#[test]
fn duplicate_scope_refuses_loudly() {
    let (_t, searcher) = indexed_scope_corpus();
    let err = searcher
        .search("pattern:fn $F() { $$$B } in:src in:calc.rs")
        .expect_err("a second `in:` value must refuse, not be dropped (FB-80a-07)");
    assert!(
        err.to_string().to_lowercase().contains("multiple"),
        "{err}"
    );
}

#[test]
fn unknown_scope_refuses_loudly() {
    let (_t, searcher) = indexed_scope_corpus();
    let err = searcher
        .search("pattern:fn $F() { $$$B } in:nosuchdir")
        .expect_err("scope matching nothing under the root must refuse, not silent-empty (FB-80a-08)");
    assert!(err.to_string().contains("nosuchdir"), "{err}");
}

#[test]
fn exact_file_scope_matches_only_that_file() {
    let (_t, searcher) = indexed_scope_corpus();
    let response = searcher
        .search("pattern:fn $F() { $$$B } in:calc.rs")
        .expect("an extension-less FILE scope matches that exact file (FB-80a-08)");
    assert!(
        !response.hits.is_empty(),
        "exact-file scope must keep calc.rs hits, not expand to an impossible calc.rs/** glob"
    );
    assert!(
        response
            .hits
            .iter()
            .all(|hit| hit.file.replace('\\', "/") == "calc.rs"),
        "every hit must come from calc.rs: {response:#?}"
    );
}

#[test]
fn dir_scope_still_filters() {
    let (_t, searcher) = indexed_scope_corpus();
    let response = searcher
        .search("pattern:fn $F() { $$$B } in:src")
        .expect("directory scope keeps working");
    assert!(!response.hits.is_empty());
    assert!(
        response
            .hits
            .iter()
            .all(|hit| hit.file.replace('\\', "/").starts_with("src/")),
        "every hit must come from src/: {response:#?}"
    );
}

#[test]
fn wildcard_scope_still_filters() {
    let (_t, searcher) = indexed_scope_corpus();
    let response = searcher
        .search("pattern:fn $F() { $$$B } in:src/*.rs")
        .expect("wildcard scope keeps working");
    assert!(!response.hits.is_empty());
    assert!(
        response
            .hits
            .iter()
            .all(|hit| hit.file.replace('\\', "/") == "src/other.rs"),
        "wildcard scope must keep matching semantics: {response:#?}"
    );
}

#[test]
fn quoted_in_text_stays_in_pattern() {
    // FB-80a-09: the `in:` token sits inside a double-quoted literal — the
    // splitter must not steal it into a bogus scope.
    let parsed = ParsedQuery::parse("x = \"a in:src\"");
    assert!(
        parsed.path_scope.is_none(),
        "quoted `in:` text must not become a path scope: {:?}",
        parsed.path_scope
    );
    assert!(
        parsed.raw.contains("in:src"),
        "the literal bytes must stay in the payload: {:?}",
        parsed.raw
    );
}
