//! br-23f: finish.rs ranking must be a total order — MCP cross-process byte-stability.
//!
//! Contract (crates/ast-sgrep-mcp/src/lib.rs: "Search envelopes are
//! deterministic for the same query and index generation"): two hits tying on
//! score, coverage, file, and line_start but distinct in line_end/symbol must
//! serialize identically no matter what order the upstream channel fed them
//! in. cmp_ranked_ends_at_line_start historically stopped at line_start and
//! relied on input order for such pairs; that input comes from a randomly
//! seeded HashMap in lexical_from_fts, so tied pairs could flip between
//! processes. This test drives finish_response twice with the tied pair in
//! opposite orders and demands byte-identical JSON both times.
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::{finish_response, HitKind, HitSignal, SearchHit, SearchOptions};

fn tied_hit(symbol: &str, line_end: u32) -> SearchHit {
    SearchHit {
        kind: HitKind::Caller,
        file: "src/app.rs".into(),
        line_start: 81,
        line_end,
        symbol: Some(symbol.into()),
        caller: Some("run_pipeline".into()),
        callee: Some("refresh_token".into()),
        language: Some("rust".into()),
        score: 2.0,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Caller],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: "run_pipeline(); refresh_token();".into(),
    }
}

fn tie_pair() -> Vec<SearchHit> {
    // Two DISTINCT callers on the same source line: identical score,
    // coverage (single term, equal excerpts), file, line_start — differing
    // only in symbol/line_end/callee.
    vec![tied_hit("caller_one", 81), tied_hit("caller_two", 82)]
}

fn finished_json(hits: Vec<SearchHit>) -> String {
    let parsed = ParsedQuery::literal("refresh_token");
    let options = SearchOptions::default();
    let response = finish_response(&parsed, &options, hits, false);
    serde_json::to_string(&response).unwrap()
}

#[test]
fn tied_hits_serialize_identically_regardless_of_input_order() {
    let forward = finished_json(tie_pair());
    let mut reversed = tie_pair();
    reversed.reverse();
    let backward = finished_json(reversed);
    assert_eq!(
        forward, backward,
        "same query+index generation must produce byte-identical output \
         regardless of upstream channel order (br-23f)"
    );
}
