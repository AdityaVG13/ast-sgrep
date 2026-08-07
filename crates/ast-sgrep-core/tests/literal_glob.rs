//! Regression for bead ast-sgrep-c2j5 (F-05): literal_sql GLOB/LIKE must treat
//! metacharacters in the needle as literals. Pre-fix, `literal:arr[0]` used
//! GLOB `*arr[0]*`, so `[0]` was a character class and matched `arr0`.
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{isolated_index_session, IsolatedIndexSession};

fn index_two_lines(a: &str, b: &str) -> IsolatedIndexSession {
    let session = isolated_index_session();
    session.write("f.rs", format!("{a}\n{b}\n"));
    session.index_all(IndexOptions {
        force_reindex: true,
        embed_semantic: false,
        ..session.index_options()
    });
    session
}

fn searcher(session: &IsolatedIndexSession) -> ast_sgrep_core::Searcher {
    session.searcher(SearchOptions {
        limit: 32,
        use_embed: false,
        ..session.search_options()
    })
}

#[test]
fn literal_bracket_metachar_matches_literally_not_as_glob_class() {
    let session = index_two_lines("let x = arr[0];", "let y = arr0;");
    let searcher = searcher(&session);

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
    let session = index_two_lines("token a[b] here", "token axb here");
    let searcher = searcher(&session);

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
