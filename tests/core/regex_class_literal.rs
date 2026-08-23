//! Failure-first regression (regex class literal): `required_literal` must not
//! harvest character-class *content* as a required literal. In regex-syntax a
//! leading `]` inside a class is a literal member, so `[]abc]` is the class
//! {a,b,c,]} — no literal substring outside it is guaranteed. The pre-fix
//! scanner treated the first `]` as closing an empty class and harvested
//! `abc` (and `efg` from `[a\]bcd]efg`) as a trigram prefilter literal, so
//! lines that the regex genuinely matched were silently dropped by the FTS
//! prefilter — false negatives, never errors.

use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{isolated_index_session, IsolatedIndexSession};

fn session() -> IsolatedIndexSession {
    let session = isolated_index_session();
    session.write("r.rs", "let x = aefg();\nlet y = abc;\nlet z = plain;\n");
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
fn regex_leading_bracket_class_does_not_harvest_required_literal() {
    let searcher = searcher(&session());
    // `[]abc]` is a valid class {a,b,c,]}; it matches the line `let y = abc;`
    // (contains 'b'). No literal substring is required by the pattern.
    let resp = searcher.search("regex:[]abc]").unwrap();
    assert!(
        resp.hits.iter().any(|h| h.excerpt.contains("abc")),
        "regex:[]abc] must match the line containing 'abc'; got {:#?}",
        resp.hits
    );
}

#[test]
fn regex_escaped_bracket_class_does_not_harvest_required_literal() {
    let searcher = searcher(&session());
    // `[a\]bcd]efg` is the class {a,],b,c,d} followed by literal `efg`; it
    // matches `let x = aefg();` ('a' from the class + 'efg'). The pre-fix
    // scanner required literal `]efg` (class content + tail), which no
    // matching line contains, so the FTS prefilter dropped the hit.
    let resp = searcher.search(r"regex:[a\]bcd]efg").unwrap();
    assert!(
        resp.hits.iter().any(|h| h.excerpt.contains("aefg")),
        "regex:[a\\]bcd]efg must match the line containing 'aefg'; got {:#?}",
        resp.hits
    );
}
