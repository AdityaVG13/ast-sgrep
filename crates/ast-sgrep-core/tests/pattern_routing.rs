//! Pattern routing tests (e9qc) — native union / prefix routing without external ast-grep.
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{isolated_index_session, IsolatedIndexSession};

fn indexed_rs(body: &str) -> IsolatedIndexSession {
    let session = isolated_index_session();
    session.write("mod.rs", body);
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    session
}

#[test]
fn pattern_prefix_routes_to_native_or_index_hits() {
    let session = indexed_rs("fn greet_user() {}\nfn other() { greet_user(); }\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher.search("pattern: greet_user").unwrap();
    assert!(
        !response.hits.is_empty(),
        "pattern: greet_user should hit via index signatures and/or native matcher"
    );
}

#[test]
fn exotic_pattern_without_ast_grep_is_structured_empty_not_panic() {
    let session = indexed_rs("fn alpha() {}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 8,
        ..session.search_options()
    });
    // Deliberately exotic rule syntax — must not panic; empty or structured error via Result.
    let result = searcher.search("pattern: $$$UNLIKELY_EXOTIC_RULE<<<");
    assert!(result.is_ok(), "exotic pattern must not panic: {result:?}");
}

#[test]
fn hybrid_quoted_literal_intent_hits_phrase_line() {
    let session =
        indexed_rs("fn main() {\n    let msg = \"foo bar unique_phrase\";\n}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 16,
        ..session.search_options()
    });
    let hybrid = searcher.search("\"foo bar unique_phrase\"").unwrap();
    let literal = searcher.search("literal:foo bar unique_phrase").unwrap();
    assert!(
        !literal.hits.is_empty(),
        "literal phrase must hit: {:?}",
        literal.hits
    );
    let lit_line = literal.hits[0].line_start;
    assert!(
        hybrid.hits.iter().any(|h| h.line_start == lit_line),
        "quoted hybrid Literal intent must hit same line as literal: (50hx); hybrid={:?} literal={:?}",
        hybrid.hits,
        literal.hits
    );
}
