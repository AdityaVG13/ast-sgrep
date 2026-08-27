//! Failure-first regression (word-LIMIT-window): a `word:` query must surface
//! whole-word matches even when the SQL LIMIT window fills with substring-only
//! rows first.
//!
//! Contract: `asgrep 'word:t'` returns up to `options.limit` WHOLE-WORD matches
//! regardless of how many substring-only rows precede them in `(path, line_no)`
//! order. The pre-fix SQL path applies its `LIMIT max(limit,100)` before the
//! word-boundary postfilter, so qualifying rows beyond the window were silently
//! dropped — a false negative, never an error.
//!
//! Fixture design: one file whose FIRST 150 lines each contain the substring
//! `alpha` only inside longer identifiers (`alphabetic`), then a line containing
//! the standalone token `alpha`. With limit < 150, the whole-word row sits
//! beyond every SQL window; the contract says it must still be returned.

use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{isolated_index_session, IsolatedIndexSession};

fn session() -> IsolatedIndexSession {
    let session = isolated_index_session();
    let mut body = String::new();
    // 150 substring-only lines: `alphabetic` contains `alpha` but never as a
    // standalone token (followed by `b`, a word character).
    for i in 0..150 {
        body.push_str(&format!("let value_{i} = \"alphabetic text\";\n"));
    }
    // The only whole-word `alpha` in the corpus, at line 151 — beyond any
    // max(limit,100) window taken over the preceding substring-only rows.
    body.push_str("let target = alpha;\n");
    session.write("w.rs", body);
    session.index_all(IndexOptions {
        force_reindex: true,
        embed_semantic: false,
        ..session.index_options()
    });
    session
}

#[test]
fn word_query_returns_whole_word_match_beyond_substring_window() {
    let session = session();
    let searcher = session.searcher(SearchOptions {
        limit: 10,
        use_embed: false,
        ..session.search_options()
    });

    let resp = searcher.search("word:alpha").unwrap();
    assert!(
        resp.hits
            .iter()
            .any(|h| h.file == "w.rs" && h.line_start == 151),
        "word:alpha must return the standalone-token line 151 even though 150 \
         substring-only ('alphabetic') lines precede it; got {} hits: {:#?}",
        resp.hits.len(),
        resp.hits
    );
}
