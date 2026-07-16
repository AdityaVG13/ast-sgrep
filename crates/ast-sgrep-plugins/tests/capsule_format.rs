//! Capsule format: refs + previews by default, bodies only on request,
//! hit order identical to the full agent format.

use ast_sgrep_core::search::{HitKind, SearchHit};
use ast_sgrep_core::SearchResponse;
use ast_sgrep_plugins::{format_response_with, to_github_json, OutputFormat};

fn sample_response() -> SearchResponse {
    let long_line = "x".repeat(300);
    SearchResponse {
        query: "renewal flow".into(),
        limit: 5,
        hits: vec![
            SearchHit {
                kind: HitKind::Def,
                file: "src/auth.rs".into(),
                line_start: 10,
                line_end: 42,
                symbol: Some("auth_refresh".into()),
                caller: None,
                callee: None,
                language: Some("rust".into()),
                score: 5.5,
                excerpt: "fn auth_refresh() {
    renew_token();
    log();
}".into(),
            },
            SearchHit {
                kind: HitKind::Caller,
                file: "src/session.rs".into(),
                line_start: 7,
                line_end: 7,
                symbol: None,
                caller: Some("open_session".into()),
                callee: Some("auth_refresh".into()),
                language: Some("rust".into()),
                score: 3.2,
                excerpt: format!("   
{long_line}"),
            },
        ],
        counts: Vec::new(),
        read_bytes_estimate: 1_000,
        returned_excerpt_bytes: 350,
        prevented_read_bytes: 650,
    }
}

#[test]
fn capsule_hits_carry_refs_and_previews_without_bodies() {
    let response = sample_response();
    let capsule = format_response_with(&response, OutputFormat::AgentCapsule, 0);

    assert_eq!(capsule["mode"], "capsule");
    assert_eq!(capsule["hit_count"], 2);
    let hits = capsule["hits"].as_array().expect("hits array");

    assert_eq!(hits[0]["ref"], "src/auth.rs#L10-L42");
    assert_eq!(hits[0]["symbol"], "auth_refresh");
    assert_eq!(hits[0]["preview"], "fn auth_refresh() {");
    assert!(hits[0].get("excerpt").is_none(), "no body by default");

    // Preview skips blank lines and truncates long ones.
    assert_eq!(hits[1]["symbol"], serde_json::Value::Null);
    assert_eq!(hits[1]["caller"], "open_session");
    assert_eq!(hits[1]["callee"], "auth_refresh");
    let preview = hits[1]["preview"].as_str().expect("preview");
    assert!(preview.chars().count() <= 121, "len {}", preview.len());
    assert!(preview.starts_with('x'));

    // Capsule excerpts have format-specific byte counts, but prevented bytes remain
    // the query-level metric shared with the full agent response.
    let agent = format_response_with(&response, OutputFormat::Agent, 0);
    assert_ne!(capsule["returned_excerpt_bytes"], 350);
    assert_eq!(agent["prevented_read_bytes"], 650);
    assert_eq!(capsule["prevented_read_bytes"], 650);
}


#[test]
fn github_page_at_limit_is_marked_incomplete() {
    let mut response = sample_response();
    response.limit = response.hits.len();

    let github = to_github_json(&response);

    assert_eq!(github["total_count"], response.hits.len());
    assert_eq!(github["incomplete_results"], true);
}
