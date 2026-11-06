//! F23: single path-identity helper kills H1/H2/S3.
//!
//! GitHub (`name`), GitLab (`basename`), and compact arms must share one
//! basename helper and one symbol-fallback chain. Regression pins:
//! - `hit_basename` handles trailing-slash keys (old inline
//!   `rsplit('/').next()` yielded `""` for `"src/"`);
//! - the GitHub text-match arm uses the full 3-chain
//!   (`symbol.or(callee).or(caller)`), so a caller-only hit surfaces the
//!   caller instead of `""` (old inline 2-chain dropped it).

use ast_sgrep_core::search::SnapshotStamp;
use ast_sgrep_core::{HitKind, HitSignal, SearchHit, SearchResponse};
use ast_sgrep_plugins::{
    hit_basename, hit_symbol, render_compact, to_github_json, to_gitlab_json, Budget,
};

fn hit_with(
    file: &str,
    symbol: Option<&str>,
    callee: Option<&str>,
    caller: Option<&str>,
) -> SearchHit {
    SearchHit {
        kind: HitKind::Def,
        file: file.to_owned(),
        line_start: 1,
        line_end: 2,
        symbol: symbol.map(str::to_owned),
        caller: caller.map(str::to_owned),
        callee: callee.map(str::to_owned),
        language: Some("rust".to_owned()),
        score: 1.0,
        signal: HitSignal::Structural,
        contributors: Vec::new(),
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: "fn caller_fn() {}".to_owned(),
        byte_span: None,
    }
}

fn response_with(hits: Vec<SearchHit>) -> SearchResponse {
    SearchResponse {
        query: "caller_fn".to_owned(),
        limit: 10,
        hits,
        counts: Vec::new(),
        read_bytes_estimate: 0,
        returned_excerpt_bytes: 0,
        prevented_read_bytes: 0,
        snapshot: SnapshotStamp::default(),
        truncated: false,
        query_expansions: Vec::new(),
    }
}

#[test]
fn basename_helper_covers_trailing_slash_variant() {
    assert_eq!(hit_basename("src/main.rs"), "main.rs");
    assert_eq!(hit_basename("main.rs"), "main.rs");
    // The old inline `rsplit('/').next()` returned `""` here.
    assert_eq!(hit_basename("src/"), "src");
}

#[test]
fn github_and_gitlab_share_one_basename() {
    let response = response_with(vec![hit_with("src/main.rs", None, None, None)]);
    let github = to_github_json(&response);
    let gitlab = to_gitlab_json(&response);
    assert_eq!(github["items"][0]["name"], gitlab["data"][0]["basename"]);
    assert_eq!(github["items"][0]["name"], "main.rs");
    // H1 through the adapters: the old inline `rsplit('/').next()` yielded `""`
    // for a trailing-slash key, so both arms must render the helper's `"src"`.
    // (The `""` value is pinned at helper level by
    // `basename_helper_covers_trailing_slash_variant`.)
    let trailing = response_with(vec![hit_with("src/", None, None, None)]);
    let trailing_github = to_github_json(&trailing);
    let trailing_gitlab = to_gitlab_json(&trailing);
    assert_eq!(trailing_github["items"][0]["name"], "src");
    assert_eq!(trailing_gitlab["data"][0]["basename"], "src");
}

#[test]
fn github_symbol_falls_back_through_caller_like_compact() {
    // H2/S3: a caller-only hit must surface the caller, not `""`.
    let response = response_with(vec![hit_with(
        "src/main.rs",
        None,
        None,
        Some("caller_fn"),
    )]);
    assert_eq!(hit_symbol(&response.hits[0]), Some("caller_fn"));
    let github = to_github_json(&response);
    let text = github["items"][0]["text_matches"][0]["matches"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(
        text, "caller_fn",
        "github arm must use the 3-chain helper, not an inline 2-chain"
    );
    // The compact row carries the same helper at index 3 (id, kind, signal,
    // symbol, snippet), so a caller-only hit must surface there too.
    let compact = render_compact(&response, &Budget::default());
    let rows = compact["h"].as_array().expect("compact rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0][3], "caller_fn",
        "compact arm must use the 3-chain helper, not an inline 2-chain"
    );
}
