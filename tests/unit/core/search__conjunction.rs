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
    let combined = combine(left, right, false, false);
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
    let combined = combine(left, right, true, false);
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].file, "src/handle.rs");
}

#[test]
fn empty_right_channel_is_honest() {
    let left = vec![hit(HitKind::Def, "src/a.rs", (1, 2), 0.9)];
    assert!(combine(left.clone(), Vec::new(), false, false).is_empty());
    assert_eq!(combine(left, Vec::new(), true, false).len(), 1);
}

#[test]
fn pattern_callers_join_requires_span_overlap() {
    let patterns = vec![
        hit(HitKind::Pattern, "src/app.rs", (1, 3), 0.9),
        hit(HitKind::Pattern, "src/app.rs", (5, 7), 0.8),
    ];
    let callers = vec![hit(HitKind::Caller, "src/app.rs", (2, 2), 0.7)];

    let combined = combine(patterns, callers, false, true);
    assert_eq!(combined.len(), 1);
    assert_eq!((combined[0].line_start, combined[0].line_end), (1, 3));
    assert!(combined[0].contributors.contains(&HitKind::Caller));
}

#[test]
fn pattern_callers_join_rejects_same_line_non_overlap() {
    let mut pattern = hit(HitKind::Pattern, "src/app.rs", (1, 1), 0.9);
    pattern.excerpt = "fn compact() {}".into();
    let mut caller = hit(HitKind::Caller, "src/app.rs", (1, 1), 0.7);
    caller.callee = Some("helper".into());
    caller.excerpt = "fn compact() {} helper();".into();

    assert!(combine(vec![pattern], vec![caller], false, true).is_empty());
}

#[test]
fn pattern_callers_join_checks_multiline_boundary_columns() {
    let mut pattern = hit(HitKind::Pattern, "src/app.rs", (1, 3), 0.9);
    pattern.excerpt = "fn target() {\n    inside();\n}".into();
    let mut outside = hit(HitKind::Caller, "src/app.rs", (1, 1), 0.7);
    outside.callee = Some("outside".into());
    outside.excerpt = "outside(); fn target() {".into();
    let mut inside = hit(HitKind::Caller, "src/app.rs", (2, 2), 0.7);
    inside.callee = Some("inside".into());
    inside.excerpt = "    inside();".into();

    assert!(
        combine(vec![pattern.clone()], vec![outside], false, true).is_empty(),
        "a call before the opening boundary must not join"
    );
    let combined = combine(vec![pattern], vec![inside], false, true);
    assert_eq!(
        combined.len(),
        1,
        "the interior call must retain the pattern"
    );
    assert_eq!(
        combined[0].contributors,
        vec![HitKind::Pattern, HitKind::Caller]
    );
}

#[test]
fn negated_pattern_callers_join_subtracts_only_overlapping_spans() {
    let patterns = vec![
        hit(HitKind::Pattern, "src/app.rs", (1, 3), 0.9),
        hit(HitKind::Pattern, "src/app.rs", (5, 7), 0.8),
    ];
    let callers = vec![hit(HitKind::Caller, "src/app.rs", (2, 2), 0.7)];

    let combined = combine(patterns, callers, true, true);
    assert_eq!(combined.len(), 1);
    assert_eq!((combined[0].line_start, combined[0].line_end), (5, 7));
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
