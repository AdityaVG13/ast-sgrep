//! Page composition: Hybrid pages emit each file's best hit before any
//! file's second hit (first-occurrence order, overflow backfilled, then
//! truncated to the limit).
//!
//! Contract (2026-10-29 evidence class #3): the frozen-suite RepoQA pages
//! were 53% duplicate-file slots (file_div@50 = .469 at cap 3); capping at
//! one hit per file compresses gold's rank whenever another file's
//! duplicate rows sat above it (offline: repoqa MRR@10 .7219 -> .7421,
//! div@50 .469 -> 1.000, cosqa bit-identical). This test pins the ORDER
//! SEMANTICS of the cap: first occurrences keep rank order, overflow rows
//! backfill after all first occurrences, and the page truncates to the
//! limit. Written failure-first against the shipped cap of 3.

use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::{finish_response, HitKind, HitSignal, SearchHit, SearchOptions};

fn hit(file: &str, line_start: u32, score: f64) -> SearchHit {
    SearchHit {
        kind: HitKind::Asgrep,
        file: file.into(),
        line_start,
        line_end: line_start + 3,
        symbol: None,
        caller: None,
        callee: None,
        language: Some("rust".into()),
        score,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Asgrep],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: "let x = token();".into(),
        byte_span: None,
    }
}

fn default_options() -> SearchOptions {
    SearchOptions::default()
}

/// A page whose ranked order is a,b,a,c,a,b must emit a,b,c,a,b: every
/// file's first occurrence, in rank order, before any second occurrence.
#[test]
fn hybrid_page_emits_file_firsts_before_second_hits() {
    let options = default_options();
    let parsed = ParsedQuery::parse("token");
    // Ranked input: three files, with a and b repeating above c.
    let hits = vec![
        hit("src/a.rs", 10, 5.0),
        hit("src/b.rs", 20, 4.0),
        hit("src/a.rs", 40, 3.0),
        hit("src/c.rs", 30, 2.0),
        hit("src/a.rs", 70, 1.5),
        hit("src/b.rs", 60, 1.0),
    ];
    let out = finish_response(&parsed, &options, hits, false);
    let order: Vec<(String, u32)> = out
        .hits
        .iter()
        .map(|h| (h.file.clone(), h.line_start))
        .collect();
    let expected = vec![
        ("src/a.rs".to_string(), 10),
        ("src/b.rs".to_string(), 20),
        ("src/c.rs".to_string(), 30),
        ("src/a.rs".to_string(), 40),
        ("src/a.rs".to_string(), 70),
        ("src/b.rs".to_string(), 60),
    ];
    assert_eq!(order, expected, "first occurrences in rank order, then overflow");
}

/// A gold file ranked under another file's duplicates must compress: with
/// a,a,a,b the gold b enters the page at position 2, not 4.
#[test]
fn duplicate_rows_above_gold_compress_gold_rank() {
    let options = default_options();
    let parsed = ParsedQuery::parse("token");
    let hits = vec![
        hit("src/noise.rs", 10, 5.0),
        hit("src/noise.rs", 20, 4.0),
        hit("src/noise.rs", 30, 3.0),
        hit("src/gold.rs", 40, 2.0),
        hit("src/other.rs", 50, 1.0),
        hit("src/other.rs", 60, 0.5),
    ];
    let out = finish_response(&parsed, &options, hits, false);
    let gold_pos = out
        .hits
        .iter()
        .position(|h| h.file == "src/gold.rs")
        .expect("gold present");
    assert_eq!(gold_pos, 1, "gold is the page's 2nd row: noise first, duplicates dropped below it");
}
