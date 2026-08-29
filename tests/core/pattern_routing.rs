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
fn rust_function_body_template_matches_without_external_ast_grep() {
    let session = indexed_rs("fn alpha() { beta(); }\nfn beta() {}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher
        .search("pattern:fn $NAME() { $$$BODY }")
        .expect("native pattern search");
    assert!(
        response
            .hits
            .iter()
            .any(|hit| hit.excerpt.contains("fn alpha")),
        "native function template must find alpha: {:?}",
        response.hits
    );
}

#[test]
fn malformed_function_tail_does_not_use_broad_cached_signature() {
    let session = indexed_rs("fn first() {}\nfn second() {}\n");
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let result = searcher.search("pattern:fn $NAME($$$) trailing garbage");
    assert!(
        result.is_err() || result.is_ok_and(|response| response.hits.is_empty()),
        "malformed pattern must not return broad cached matches"
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
    let session = indexed_rs("fn main() {\n    let msg = \"foo bar unique_phrase\";\n}\n");
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

#[test]
fn ident_pattern_is_served_from_index() {
    let session = indexed_rs(
        "pub struct SearchHit { pub file: String }\nfn other() { let _ = SearchHit { file: String::new() }; }\n",
    );
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        limit: 32,
        ..session.search_options()
    });
    let response = searcher.search("pattern:SearchHit").unwrap();
    assert!(
        response
            .hits
            .iter()
            .any(|hit| hit.file.ends_with("mod.rs") && hit.line_start >= 1),
        "ident pattern must hit indexed identifier nodes: {:?}",
        response.hits
    );
}
