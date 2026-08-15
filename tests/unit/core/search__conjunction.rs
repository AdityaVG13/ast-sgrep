use super::*;
use crate::query::QueryMode;
use crate::search::types::{HitKind, SearchHit};

fn hit(kind: HitKind, file: &str, lines: (u32, u32), score: f64) -> SearchHit {
    SearchHit {
        kind,
        file: file.into(),
        line_start: lines.0,
        line_end: lines.1,
        symbol: None,
        caller: None,
        callee: None,
        language: None,
        score,
        signal: kind.signal(),
        contributors: vec![kind],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: String::new(),
    }
}

#[test]
fn parses_two_prefixed_channels() {
    let conj = parse("callers:process_request AND pattern:fn $NAME($$$)").expect("conjunction");
    assert!(!conj.negated);
    match (&conj.left, &conj.right) {
        (ChannelQuery::Mode(left), ChannelQuery::Mode(right)) => {
            assert_eq!(left.mode, QueryMode::Callers);
            assert_eq!(left.target.as_deref(), Some("process_request"));
            assert_eq!(right.mode, QueryMode::Pattern);
            assert_eq!(right.target.as_deref(), Some("fn $NAME($$$)"));
        }
        other => panic!("unexpected channels: {other:?}"),
    }
}

#[test]
fn parses_semantic_channel_with_quotes() {
    let conj =
        parse("imports: rusqlite AND semantic:\"parameterized query\"").expect("conjunction");
    match (&conj.left, &conj.right) {
        (ChannelQuery::Mode(left), ChannelQuery::Semantic(query)) => {
            assert_eq!(left.mode, QueryMode::Imports);
            assert_eq!(left.target.as_deref(), Some("rusqlite"));
            assert_eq!(query, "parameterized query");
        }
        other => panic!("unexpected channels: {other:?}"),
    }
}

#[test]
fn parses_and_not_in_both_cases() {
    for raw in [
        "defs:handle AND not callers:test_",
        "defs:handle AND NOT callers:test_",
    ] {
        let conj = parse(raw).expect("conjunction");
        assert!(conj.negated, "{raw} must negate");
        match &conj.right {
            ChannelQuery::Mode(right) => {
                assert_eq!(right.mode, QueryMode::Callers);
                assert_eq!(right.target.as_deref(), Some("test_"));
            }
            other => panic!("unexpected right channel: {other:?}"),
        }
    }
}

#[test]
fn plain_english_and_falls_through() {
    // Unprefixed sides: "AND" keeps its English meaning in hybrid search.
    assert!(parse("sessions AND cookies").is_none());
    assert!(parse("defs:handle AND cleanup logic").is_none());
    assert!(parse("error handling AND callers:retry").is_none());
}

#[test]
fn more_than_two_channels_falls_through() {
    assert!(parse("defs:a AND callers:b AND imports:c").is_none());
}

#[test]
fn empty_channel_targets_fall_through() {
    assert!(parse("defs: AND callers:b").is_none());
    assert!(parse("defs:a AND semantic:\"\"").is_none());
    // A lone quote must not slice out of bounds (it is a 1-byte payload).
    let _ = parse("defs:a AND semantic:'");
}

#[test]
fn and_intersects_by_file_and_merges_overlapping_evidence() {
    let left = vec![
        hit(HitKind::Caller, "src/auth.rs", (10, 20), 0.9),
        hit(HitKind::Caller, "src/other.rs", (1, 5), 0.8),
    ];
    let right = vec![
        hit(HitKind::Pattern, "src/auth.rs", (12, 18), 0.7),
        hit(HitKind::Pattern, "src/unrelated.rs", (1, 3), 0.6),
    ];
    let combined = combine(left, right, false);
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].file, "src/auth.rs");
    assert!(combined[0].contributors.contains(&HitKind::Caller));
    assert!(
        combined[0].contributors.contains(&HitKind::Pattern),
        "overlapping right evidence must merge into the kept hit"
    );
}

#[test]
fn and_not_subtracts_right_channel_files() {
    let left = vec![
        hit(HitKind::Def, "src/handle.rs", (1, 10), 0.9),
        hit(HitKind::Def, "tests/handle_test.rs", (1, 10), 0.8),
    ];
    let right = vec![hit(HitKind::Caller, "tests/handle_test.rs", (5, 5), 0.7)];
    let combined = combine(left, right, true);
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].file, "src/handle.rs");
}

#[test]
fn empty_right_channel_is_honest() {
    let left = vec![hit(HitKind::Def, "src/a.rs", (1, 2), 0.9)];
    assert!(combine(left.clone(), Vec::new(), false).is_empty());
    assert_eq!(combine(left, Vec::new(), true).len(), 1);
}

#[test]
fn response_query_keeps_full_raw_and_left_mode() {
    let raw = "callers:process_request AND pattern:fn $NAME($$$)";
    let conj = parse(raw).expect("conjunction");
    let parsed = response_query(raw, &conj);
    assert_eq!(parsed.raw, raw);
    assert_eq!(parsed.mode, QueryMode::Callers);
    assert_eq!(parsed.target.as_deref(), Some("process_request"));
}

#[test]
fn semantic_left_side_ranks_as_hybrid_text() {
    let raw = "semantic:\"token renewal\" AND imports:rusqlite";
    let conj = parse(raw).expect("conjunction");
    let parsed = response_query(raw, &conj);
    assert_eq!(parsed.raw, raw);
    assert_eq!(parsed.mode, QueryMode::Hybrid);
}
