//! Core stage oracles (consolidated).
//!
//! Consolidates the CAT=other stage facets of `oracle_foundry_pass3` plus the
//! pass-1 store-boundary facets (schema-mismatch, mmap) into 5 intent-grouped
//! tests. Expectations are hand-computed; failures assert discriminants,
//! never message text.

use ast_sgrep_core::gitignore::{
    is_ignored, is_indexable_extension, should_skip_dir, should_skip_file, IgnoreMatcher,
    DEFAULT_SKIP_DIR_NAMES, DOCUMENT_EXTENSIONS,
};
use ast_sgrep_core::index::{indexed_rel_path, split_content_lines};
use ast_sgrep_core::io_bounds::{read_bounded_line, read_text_capped, BoundedLine};
use ast_sgrep_core::lexicon::{
    prose_terms, subtokens, Association, Lexicon, LexiconBuilder, Observation, MAX_PER_TERM,
    MIN_SUPPORT,
};
use ast_sgrep_core::resolution::{Resolution, SymbolId};
use ast_sgrep_core::scip::{
    load_scip_index, normalize_scip_path, scip_symbol_ident, ScipLoad, ScipOccurrence,
    SCIP_CHANNEL, SCIP_ROLE_DEFINITION,
};
use ast_sgrep_core::search::passes::lexical::{lexical_pool_limit, LEXICAL_POOL_FLOOR};
use ast_sgrep_core::search::{finish_response, SnapshotStamp, SpanHitInput};
use ast_sgrep_core::{
    follow_ups_for_hit, margin_is_decisive, plan_suggested_next, CriticNote, HitKind, HitSignal,
    ParsedQuery, SearchHit, SearchOptions, SearchResponse, StoreError,
    MAX_SEARCH_HIT_EXCERPT_BYTES,
};
use ast_sgrep_mmap::map_readonly;
use ast_sgrep_testkit::{finish_options, hit_key_bits, mk_hit, write_temp};

/// INTENT: bounded IO ingress — stdin lines (CRLF, at/over-limit, TooLong
/// drain, zero-limit), capped reads (exact boundary, binary/dir refusal),
/// indexed rel-path accept/refuse, and CRLF line splitting with 1-based
/// numbering and round-trip.
/// KILLS: boundary-flip, CRLF-strip, drain, cap-flip, lossy-decode,
/// dir-accept, traversal-accept, numbering, and strip mutants.
/// ABSORBS: stdin_bounded_line_table_crlf_and_edges,
/// capped_read_boundary_and_fail_closed, indexed_rel_path_accepts_and_refuses,
/// split_lines_eol_numbering_and_roundtrip.
#[test]
fn bounded_io_paths_and_line_splitting() {
    // Facet 1: bounded stdin line table — CRLF strip, boundaries, drain, edges.
    #[derive(Debug, PartialEq)]
    enum Outcome {
        Line(Vec<u8>),
        TooLong,
    }
    fn drain(mut data: &[u8], limit: usize) -> Vec<Outcome> {
        let mut out = Vec::new();
        loop {
            match read_bounded_line(&mut data, limit).expect("in-memory io") {
                None => return out,
                Some(BoundedLine::Line(bytes)) => out.push(Outcome::Line(bytes)),
                Some(BoundedLine::TooLong) => out.push(Outcome::TooLong),
            }
        }
    }

    // Empty input yields absence (None), not an empty line.
    assert_eq!(drain(b"", 8), vec![]);
    // Newline-terminated, unterminated, and bare-newline shapes.
    assert_eq!(drain(b"abc\n", 8), vec![Outcome::Line(b"abc".to_vec())]);
    assert_eq!(drain(b"abc", 8), vec![Outcome::Line(b"abc".to_vec())]);
    assert_eq!(drain(b"\n", 8), vec![Outcome::Line(vec![])]);
    // CRLF: the carriage return is framing, not payload.
    assert_eq!(drain(b"a\r\n", 8), vec![Outcome::Line(b"a".to_vec())]);
    // Unterminated CRLF keeps a lone CR (only newline-adjacent CR strips).
    assert_eq!(drain(b"a\r", 8), vec![Outcome::Line(b"a\r".to_vec())]);
    // Boundary: exactly-at-limit is a line; one byte over is TooLong.
    assert_eq!(drain(b"abc\n", 3), vec![Outcome::Line(b"abc".to_vec())]);
    assert_eq!(drain(b"abcd\n", 3), vec![Outcome::TooLong]);
    // Zero limit: only the empty line fits.
    assert_eq!(drain(b"\n", 0), vec![Outcome::Line(vec![])]);
    assert_eq!(drain(b"a\n", 0), vec![Outcome::TooLong]);
    // TooLong drains through the newline: the next record is the next line.
    assert_eq!(
        drain(b"ok\ntoolonggggg\nok2", 4),
        vec![
            Outcome::Line(b"ok".to_vec()),
            Outcome::TooLong,
            Outcome::Line(b"ok2".to_vec()),
        ]
    );
    // Multibyte payload passes through uninterpreted.
    assert_eq!(
        drain("é\n".as_bytes(), 8),
        vec![Outcome::Line("é".as_bytes().to_vec())]
    );
    // Discriminant spot-check via matches!: not-None, not-Line.
    let mut data: &[u8] = b"toolong\n";
    assert!(matches!(
        read_bounded_line(&mut data, 3).expect("io"),
        Some(BoundedLine::TooLong)
    ));
    // Determinism under repetition.
    assert_eq!(drain(b"a\nbb\nccc", 2), drain(b"a\nbb\nccc", 2));

    // Facet 2: capped reads — exact boundary, fail-closed on binary/dir/oversize.
    let empty = write_temp(b"");
    assert_eq!(read_text_capped(empty.path(), 64).expect("empty ok"), "");
    let hello = write_temp(b"hello");
    assert_eq!(
        read_text_capped(hello.path(), 64).expect("small ok"),
        "hello"
    );
    // Exact-cap content is accepted; one byte over is refused.
    let at_cap = write_temp(&[b'a'; 8]);
    assert_eq!(
        read_text_capped(at_cap.path(), 8).expect("at-cap ok").len(),
        8
    );
    let over_cap = write_temp(&[b'a'; 9]);
    assert!(read_text_capped(over_cap.path(), 8).is_err());
    // Invalid UTF-8 fails closed (binary), never lossy-decoded.
    let binary = write_temp(&[0xff, 0xfe, 0x00, b'a']);
    assert!(read_text_capped(binary.path(), 64).is_err());
    // A directory is not a regular file.
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(read_text_capped(dir.path(), 64).is_err());
    // Unicode and CRLF survive byte-exact.
    let uni = write_temp("hélloµ\r\nworld\n".as_bytes());
    assert_eq!(
        read_text_capped(uni.path(), 64).expect("unicode ok"),
        "hélloµ\r\nworld\n"
    );

    // Facet 3: indexed rel paths — accept table, refuse table, determinism.
    use std::path::Path;
    assert_eq!(
        indexed_rel_path(Path::new("src/main.rs")).expect("ok"),
        "src/main.rs"
    );
    assert_eq!(indexed_rel_path(Path::new("a.rs")).expect("ok"), "a.rs");
    assert_eq!(
        indexed_rel_path(Path::new("src/µ.rs")).expect("unicode ok"),
        "src/µ.rs"
    );
    for bad in [
        "",
        "/abs/x.rs",
        "../up.rs",
        "a/../b.rs",
        "a/../../b.rs",
        "a\0b.rs",
    ] {
        assert!(
            indexed_rel_path(Path::new(bad)).is_err(),
            "must refuse {bad:?}"
        );
    }
    // A backslash is an ordinary filename byte on Unix: accepted, unrewritten.
    let backslash = indexed_rel_path(Path::new("a\\b.rs")).expect("backslash ok");
    #[cfg(not(windows))]
    assert_eq!(backslash, "a\\b.rs");
    #[cfg(windows)]
    assert_eq!(backslash, "a/b.rs");
    // Determinism under repetition.
    assert_eq!(
        indexed_rel_path(Path::new("src/main.rs")).expect("ok"),
        indexed_rel_path(Path::new("src/main.rs")).expect("ok")
    );

    // Facet 4: line splitting — EOL detection, 1-based numbering, round-trip.
    // Empty content is one empty first line, LF by default.
    let empty = split_content_lines("");
    assert_eq!(empty.eol, "lf");
    assert_eq!(empty.lines, vec![(1u32, String::new())]);
    // LF shape: 1-based, no stripping beyond the newline itself.
    let lf = split_content_lines("a\nb");
    assert_eq!(lf.eol, "lf");
    assert_eq!(lf.lines, vec![(1, "a".to_string()), (2, "b".to_string())]);
    // Trailing newline yields a final empty line.
    assert_eq!(
        split_content_lines("a\n").lines,
        vec![(1, "a".to_string()), (2, String::new())]
    );
    // CRLF: marker detected, carriage returns stripped from payload.
    let crlf = split_content_lines("a\r\nb\r\n");
    assert_eq!(crlf.eol, "crlf");
    assert_eq!(
        crlf.lines,
        vec![
            (1, "a".to_string()),
            (2, "b".to_string()),
            (3, String::new())
        ]
    );
    // A lone CR is content, not framing: no CRLF marker, CR preserved.
    let lone_cr = split_content_lines("a\rb");
    assert_eq!(lone_cr.eol, "lf");
    assert_eq!(lone_cr.lines, vec![(1, "a\rb".to_string())]);
    // Unicode payload preserved verbatim.
    let uni = split_content_lines("héllo\nµ");
    assert_eq!(
        uni.lines,
        vec![(1, "héllo".to_string()), (2, "µ".to_string())]
    );
    // Metamorphic: LF content round-trips through join; line numbers are 1..=n.
    for content in ["a\nb\n", "x", "l1\nl2\nl3"] {
        let split = split_content_lines(content);
        let joined = split
            .lines
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(joined, content, "round-trip for {content:?}");
        for (index, (number, _)) in split.lines.iter().enumerate() {
            assert_eq!(*number, (index + 1) as u32);
        }
    }
    // Differential: the `text::` re-export agrees with the canonical path.
    let via_text = ast_sgrep_core::text::split_content_lines("a\r\nb");
    let via_index = split_content_lines("a\r\nb");
    assert_eq!(via_text.eol, via_index.eol);
    assert_eq!(via_text.lines, via_index.lines);
}

/// INTENT: response shaping — excerpt bounding (marker, at-cap passthrough,
/// idempotence, char-boundary safety), wire-hit distrust (signal/contributors
/// re-derived, margin sanitized, excerpt bounded, serialize fixpoint), and
/// finish gates (limits/order/bytes, count-only, dedup flag, filter compat,
/// scope differential, def promotion).
/// KILLS: marker-drop, cap-flip, mid-char-cut, trust-wire-field,
/// unbounded-excerpt, limit-off-by-one, order, filter-fail-open, and
/// promotion-drop mutants.
/// ABSORBS: excerpt_bound_is_idempotent_and_marked,
/// wire_hits_distrust_signal_margin_contributors,
/// finish_response_gates_limits_and_filters.
#[test]
fn excerpt_wire_and_finish_gates() {
    // Facet 1: excerpt bounding — marker, idempotence, char-boundary safety.
    const MAX: usize = MAX_SEARCH_HIT_EXCERPT_BYTES;
    assert_eq!(MAX, 65_536);
    fn excerpt_of(excerpt: String) -> String {
        SearchHit::span(SpanHitInput {
            kind: HitKind::Asgrep,
            file: "a.rs".to_string(),
            line_start: 1,
            line_end: 1,
            score: 1.0,
            excerpt,
            symbol: None,
            language: None,
            byte_span: None,
        })
        .excerpt
    }

    assert_eq!(excerpt_of(String::new()), "");
    assert_eq!(excerpt_of("hello".to_string()), "hello");
    // Exactly-at-cap passes through untouched: no marker, same length.
    let at_cap = "a".repeat(MAX);
    let bounded = excerpt_of(at_cap.clone());
    assert_eq!(bounded.len(), MAX);
    assert_eq!(bounded, at_cap);
    // One byte over: truncated with the "\n…" marker, total stays within cap.
    let bounded = excerpt_of("a".repeat(MAX + 1));
    assert!(bounded.ends_with("\n…"), "marker required");
    assert_eq!(bounded.len(), MAX);
    // Idempotence: bounding an already-bounded excerpt is a fixpoint.
    let huge = "b".repeat(MAX * 2);
    let once = excerpt_of(huge);
    let twice = excerpt_of(once.clone());
    assert_eq!(once, twice);
    // Char-boundary walk-back: "x" + é-run puts cut point 65532 mid-char,
    // so the cut retreats one byte to 65531 and the total is 65535.
    let mixed = format!("x{}", "é".repeat(MAX));
    let bounded = excerpt_of(mixed);
    assert_eq!(bounded.len(), MAX - 1);
    assert!(bounded.ends_with("\n…"));
    assert!(bounded.starts_with('x'));
    assert!(bounded.is_char_boundary(bounded.len()));

    // Facet 2: wire distrust — signal/contributors/margin/excerpt sanitized.
    fn decode(value: serde_json::Value) -> SearchHit {
        serde_json::from_value(value).expect("wire hit decodes")
    }
    fn wire(excerpt: &str, margin: f64) -> serde_json::Value {
        serde_json::json!({
            "kind": "asgrep",
            "file": "a.rs",
            "line_start": 1,
            "line_end": 2,
            "symbol": null,
            "caller": null,
            "callee": null,
            "language": null,
            "score": 3.0,
            "signal": "semantic",
            "contributors": ["embed", "def"],
            "margin": margin,
            "excerpt": excerpt,
        })
    }

    let hit = decode(wire("short", -5.0));
    // Signal is re-derived from kind; the wire claim is ignored.
    assert_eq!(hit.signal, HitSignal::Exact);
    // Contributors reset to the row's own kind.
    assert_eq!(hit.contributors, vec![HitKind::Asgrep]);
    // Negative margin sanitizes to zero; engine-derived fields stay empty.
    assert_eq!(hit.margin, 0.0);
    assert_eq!(hit.confidence, 0.0);
    assert!(hit.resolution.is_none());
    assert!(hit.embed_fields.is_none());
    assert!(hit.critic.is_empty());
    assert!(hit.byte_span.is_none());
    // A finite non-negative margin survives decode.
    assert_eq!(decode(wire("short", 2.5)).margin, 2.5);
    // Differential: wire-ingress bounding agrees with constructor bounding.
    let huge = "a".repeat(MAX_SEARCH_HIT_EXCERPT_BYTES + 4096);
    let via_wire = decode(wire(&huge, 0.0)).excerpt;
    let via_span = SearchHit::span(SpanHitInput {
        kind: HitKind::Asgrep,
        file: "a.rs".to_string(),
        line_start: 1,
        line_end: 2,
        score: 3.0,
        excerpt: huge,
        symbol: None,
        language: None,
        byte_span: None,
    })
    .excerpt;
    assert_eq!(via_wire, via_span);
    assert!(via_wire.ends_with("\n…"));
    // Fixpoint: serialize → decode is stable on the derived fields.
    let round = decode(wire("short", 1.0));
    let text = serde_json::to_string(&round).expect("serialize");
    let again: SearchHit = serde_json::from_str(&text).expect("re-decode");
    assert_eq!(again.signal, round.signal);
    assert_eq!(again.contributors, round.contributors);
    assert_eq!(again.margin, round.margin);
    assert_eq!(again.excerpt, round.excerpt);

    // Facet 3: finish gates — limits, determinism, count-only, dedup, filters, bytes.
    fn scored(file: &str, score: f64, excerpt: &str) -> SearchHit {
        let mut hit = mk_hit(HitKind::Asgrep, file, 1, score);
        hit.excerpt = excerpt.to_string();
        hit
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let parsed = ParsedQuery::parse("defs:needle");
    let hits = || {
        vec![
            scored("s1.rs", 5.0, "e1"),
            scored("s2.rs", 4.0, "e2"),
            scored("s3.rs", 3.0, "e3"),
            scored("s4.rs", 2.0, "e4"),
            scored("s5.rs", 1.0, "e5"),
        ]
    };

    // Result count is exactly min(limit, available); echo of query and limit.
    for limit in 0..=7usize {
        let response = finish_response(&parsed, &finish_options(dir.path(), limit), hits(), true);
        assert_eq!(response.hits.len(), limit.min(5), "limit={limit}");
        assert_eq!(response.query, "defs:needle");
        assert_eq!(response.limit, limit);
    }
    // Score order decides rank; excerpt bytes are exactly the survivors' sum.
    let two = finish_response(&parsed, &finish_options(dir.path(), 2), hits(), true);
    assert_eq!(
        two.hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>(),
        vec!["s1.rs", "s2.rs"]
    );
    assert_eq!(two.returned_excerpt_bytes, 4);
    // Absent files contribute zero estimated read bytes; prevented saturates.
    assert_eq!(two.read_bytes_estimate, 0);
    assert_eq!(two.prevented_read_bytes, 0);
    // Determinism under repetition: identical keys in identical order.
    let again = finish_response(&parsed, &finish_options(dir.path(), 2), hits(), true);
    assert_eq!(
        two.hits.iter().map(hit_key_bits).collect::<Vec<_>>(),
        again.hits.iter().map(hit_key_bits).collect::<Vec<_>>()
    );

    // Count-only: no hits, per-file counts sorted by file, summing to the input.
    let mut counted = finish_options(dir.path(), 10);
    counted.count_only = true;
    let counts = finish_response(&parsed, &counted, hits(), true);
    assert!(counts.hits.is_empty());
    assert_eq!(
        counts.counts,
        vec![
            ("s1.rs".to_string(), 1),
            ("s2.rs".to_string(), 1),
            ("s3.rs".to_string(), 1),
            ("s4.rs".to_string(), 1),
            ("s5.rs".to_string(), 1),
        ]
    );
    assert_eq!(counts.counts.iter().map(|(_, n)| n).sum::<u32>(), 5);

    // The dedup flag collapses same-location rows; false preserves them.
    let dupes = vec![
        scored("d.rs", 5.0, "x"),
        scored("d.rs", 4.0, "x"),
        scored("e.rs", 1.0, "y"),
    ];
    let with_dedup = finish_response(
        &parsed,
        &finish_options(dir.path(), 10),
        dupes.clone(),
        true,
    );
    let without_dedup = finish_response(&parsed, &finish_options(dir.path(), 10), dupes, false);
    assert_eq!(with_dedup.hits.len(), 2);
    assert_eq!(without_dedup.hits.len(), 3);

    // file_filter keeps matches even when the top score is filtered out.
    let mixed = vec![
        scored("src/a.rs", 3.0, "x"),
        scored("src/b.rs", 2.0, "x"),
        scored("other/c.rs", 5.0, "x"),
    ];
    let mut filtered = finish_options(dir.path(), 10);
    filtered.file_filter = Some("src/**".to_string());
    let response = finish_response(&parsed, &filtered, mixed.clone(), true);
    assert_eq!(
        response
            .hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>(),
        vec!["src/a.rs", "src/b.rs"]
    );
    // Legacy compat: an invalid (empty / control-char) filter is ignored, never fatal.
    for bad in ["", "a\nb"] {
        let mut options = finish_options(dir.path(), 10);
        options.file_filter = Some(bad.to_string());
        let response = finish_response(&parsed, &options, mixed.clone(), true);
        assert_eq!(response.hits.len(), 3, "filter {bad:?} must be ignored");
    }
    // Differential: an `in:` scope applies the same glob as an explicit filter.
    let scoped = ParsedQuery::parse("needle in:src");
    let via_scope = finish_response(&scoped, &finish_options(dir.path(), 10), mixed, true);
    assert_eq!(via_scope.query, "needle");
    assert_eq!(
        via_scope
            .hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>(),
        vec!["src/a.rs", "src/b.rs"]
    );

    // Hybrid def promotion: the definition is moved into the head at head-1.
    let hybrid = ParsedQuery::parse("needle");
    let hybrid_hits = || {
        let mut def = scored("d.rs", 1.0, "e");
        def.kind = HitKind::Def;
        def.symbol = Some("needle".to_string());
        def.signal = HitKind::Def.signal();
        def.contributors = vec![HitKind::Def];
        vec![
            scored("t1.rs", 5.0, "e1"),
            scored("t2.rs", 4.0, "e2"),
            scored("t3.rs", 3.0, "e3"),
            scored("t4.rs", 2.0, "e4"),
            def,
        ]
    };
    let promoted = finish_response(&hybrid, &finish_options(dir.path(), 2), hybrid_hits(), true);
    assert_eq!(
        promoted
            .hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>(),
        vec!["t1.rs", "d.rs"]
    );
    let promoted = finish_response(&hybrid, &finish_options(dir.path(), 3), hybrid_hits(), true);
    assert_eq!(
        promoted
            .hits
            .iter()
            .map(|hit| hit.file.as_str())
            .collect::<Vec<_>>(),
        vec!["t1.rs", "t2.rs", "d.rs"]
    );
}

/// INTENT: planning and resolution knowledge — causal planner (decisive 10%
/// boundary, follow-up drill-downs, suggestion chain, lexical pool floor),
/// resolution tiers (rank ladder, precision, upgrade algebra, candidates,
/// describe/qualified), and file filters (skip tables, case-insensitive
/// extensions, ignore+negation).
/// KILLS: ratio/order/quote/floor, rank-swap/precision/upgrade/cap,
/// skip-set/extension-case/negation mutants.
/// ABSORBS: planner_decisive_followups_and_pool_floor,
/// resolution_tiers_upgrade_and_candidates,
/// file_filters_skip_hidden_foreign_and_negate.
#[test]
fn planner_resolution_and_file_filters() {
    // Facet 1: causal planner — decisive boundary, follow-ups, suggestions, pool floor.
    fn hit_with(
        symbol: Option<&str>,
        contributors: Vec<HitKind>,
        score: f64,
        margin: f64,
    ) -> SearchHit {
        let mut hit = mk_hit(HitKind::Asgrep, "a.rs", 1, score);
        hit.symbol = symbol.map(str::to_string);
        hit.contributors = contributors;
        hit.margin = margin;
        hit
    }
    // Decisive boundary is 10% of score: at-ratio true, below false, zero never.
    assert!(margin_is_decisive(&hit_with(
        None,
        vec![HitKind::Asgrep],
        10.0,
        1.0
    )));
    assert!(margin_is_decisive(&hit_with(
        None,
        vec![HitKind::Asgrep],
        10.0,
        2.0
    )));
    assert!(!margin_is_decisive(&hit_with(
        None,
        vec![HitKind::Asgrep],
        10.0,
        0.9
    )));
    assert!(!margin_is_decisive(&hit_with(
        None,
        vec![HitKind::Asgrep],
        10.0,
        0.0
    )));
    assert!(!margin_is_decisive(&hit_with(
        None,
        vec![HitKind::Asgrep],
        0.0,
        0.0
    )));
    assert!(!margin_is_decisive(&hit_with(
        None,
        vec![HitKind::Asgrep],
        -5.0,
        1.0
    )));

    // No symbol anywhere: no drill-down exists.
    assert!(follow_ups_for_hit("foo", &hit_with(None, vec![HitKind::Asgrep], 5.0, 0.0)).is_empty());
    // Settled evidence (def + usage + decisive): the plan is "done".
    let settled = hit_with(Some("foo"), vec![HitKind::Def, HitKind::Caller], 10.0, 1.0);
    assert!(follow_ups_for_hit("foo", &settled).is_empty());
    // Missing definition and usage: both drill-downs, in defs/callers order.
    let bare = hit_with(Some("foo"), vec![HitKind::Asgrep], 5.0, 0.0);
    assert_eq!(
        follow_ups_for_hit("foo", &bare),
        vec!["defs:foo", "callers:foo"]
    );
    // Definition present but usage missing: only the usage drill-down.
    let def_only = hit_with(Some("foo"), vec![HitKind::Def], 10.0, 1.0);
    assert_eq!(follow_ups_for_hit("foo", &def_only), vec!["callers:foo"]);
    // Complete but indecisive: confirm with exact text, not a re-run.
    let tied = hit_with(Some("foo"), vec![HitKind::Def, HitKind::Caller], 5.0, 0.0);
    assert_eq!(follow_ups_for_hit("foo", &tied), vec!["literal:foo"]);
    // Identifier collision drills into the query's compound identifier, not the fragment.
    let mut collision = hit_with(Some("refresh"), vec![HitKind::Def], 5.0, 0.0);
    collision.critic = vec![CriticNote::IdentifierCollision];
    assert_eq!(
        follow_ups_for_hit("auth_refresh", &collision),
        vec!["defs:auth_refresh", "callers:auth_refresh"]
    );
    // Determinism under repetition.
    assert_eq!(
        follow_ups_for_hit("foo", &bare),
        follow_ups_for_hit("foo", &bare)
    );

    fn response(query: &str, hits: Vec<SearchHit>) -> SearchResponse {
        SearchResponse {
            query: query.to_string(),
            limit: 10,
            hits,
            counts: vec![],
            read_bytes_estimate: 0,
            returned_excerpt_bytes: 0,
            prevented_read_bytes: 0,
            snapshot: SnapshotStamp::default(),
            query_expansions: vec![],
        }
    }
    // Empty shortlist: semantic probe first, agent-format command last.
    assert_eq!(
        plan_suggested_next(&response("foo bar", vec![])),
        vec![
            "asgrep semantic 'foo bar'",
            "asgrep --json --format agent 'foo bar'",
        ]
    );
    // Top hit with gaps and no semantic evidence anywhere: full causal chain.
    assert_eq!(
        plan_suggested_next(&response("q", vec![bare])),
        vec![
            "asgrep 'defs:foo'",
            "asgrep 'callers:foo'",
            "asgrep semantic 'q'",
            "asgrep --json --format agent 'q'",
        ]
    );
    // Settled top hit plus semantic evidence: only the agent command remains.
    let settled_response = response(
        "q",
        vec![
            hit_with(Some("foo"), vec![HitKind::Def, HitKind::Caller], 10.0, 1.0),
            mk_hit(HitKind::Embed, "b.rs", 1, 0.5),
        ],
    );
    assert_eq!(
        plan_suggested_next(&settled_response),
        vec!["asgrep --json --format agent 'q'"]
    );
    // Shell quoting escapes a single quote without interpolation.
    assert_eq!(
        plan_suggested_next(&response("it's", vec![]))[0],
        "asgrep semantic 'it'\\''s'"
    );

    // Lexical pool: floor 100, identity above, monotone non-decreasing.
    assert_eq!(LEXICAL_POOL_FLOOR, 100);
    for (limit, expected) in [(0, 100), (5, 100), (99, 100), (100, 100), (500, 500)] {
        let options = SearchOptions {
            limit,
            ..SearchOptions::default()
        };
        assert_eq!(lexical_pool_limit(&options), expected, "limit={limit}");
    }
    let mut previous = 0;
    for limit in [0, 1, 50, 99, 100, 101, 1000] {
        let options = SearchOptions {
            limit,
            ..SearchOptions::default()
        };
        let pool = lexical_pool_limit(&options);
        assert!(pool >= previous, "monotone at {limit}");
        previous = pool;
    }

    // Facet 2: resolution tiers — order, precision, upgrade algebra, candidates.
    // Strength rank is a strict hand-pinned ladder, strongest first.
    let ladder = [
        (Resolution::CompilerExact, 0u8),
        (Resolution::ImportResolved, 1),
        (Resolution::FileLocalUnique, 2),
        (Resolution::ScipOccurrence, 3),
        (Resolution::RepositoryUnique, 4),
        (Resolution::NameOnly, 5),
        (Resolution::Ambiguous { candidates: vec![] }, 6),
    ];
    for (tier, rank) in &ladder {
        assert_eq!(tier.rank(), *rank, "{tier:?}");
    }
    for pair in ladder.windows(2) {
        assert!(pair[0].1 < pair[1].1);
    }
    // Precision is exactly the top three tiers; occurrence evidence is not precise.
    for (tier, _) in &ladder {
        let precise = matches!(
            tier,
            Resolution::CompilerExact | Resolution::ImportResolved | Resolution::FileLocalUnique
        );
        assert_eq!(tier.is_precise(), precise, "{tier:?}");
    }
    // Wire names are stable.
    assert_eq!(Resolution::CompilerExact.as_str(), "compiler_exact");
    assert_eq!(Resolution::ScipOccurrence.as_str(), "scip_occurrence");
    assert_eq!(Resolution::ImportResolved.as_str(), "import_resolved");
    assert_eq!(Resolution::FileLocalUnique.as_str(), "file_local_unique");
    assert_eq!(Resolution::RepositoryUnique.as_str(), "repository_unique");
    assert_eq!(Resolution::NameOnly.as_str(), "name_only");
    assert_eq!(
        Resolution::Ambiguous { candidates: vec![] }.as_str(),
        "ambiguous"
    );
    // Upgrade algebra: idempotent, commutative, keeps the stronger tier.
    let tiers: Vec<Resolution> = ladder.into_iter().map(|(tier, _)| tier).collect();
    for left in &tiers {
        assert_eq!(left.clone().upgrade(left.clone()), *left);
        for right in &tiers {
            let forward = left.clone().upgrade(right.clone());
            let backward = right.clone().upgrade(left.clone());
            assert_eq!(forward, backward, "{left:?} vs {right:?}");
            assert_eq!(forward.rank(), left.rank().min(right.rank()));
        }
    }
    // Candidate table: file-unique beats repo-unique beats name-only;
    // genuine ambiguity needs at least two collected candidates, capped at four.
    let id = |module: &str, name: &str| SymbolId::new(module, name);
    assert_eq!(
        Resolution::from_candidates(1, 9, Vec::<SymbolId>::new()),
        Resolution::FileLocalUnique
    );
    assert_eq!(
        Resolution::from_candidates(2, 1, Vec::<SymbolId>::new()),
        Resolution::RepositoryUnique
    );
    assert_eq!(
        Resolution::from_candidates(0, 0, Vec::<SymbolId>::new()),
        Resolution::NameOnly
    );
    assert_eq!(
        Resolution::from_candidates(5, 9, vec![id("m", "a")]),
        Resolution::NameOnly
    );
    assert_eq!(
        Resolution::from_candidates(5, 9, Vec::<SymbolId>::new()),
        Resolution::NameOnly
    );
    let three: Vec<SymbolId> = ["a", "b", "c"].iter().map(|n| id("m", n)).collect();
    assert!(matches!(
        Resolution::from_candidates(5, 9, three),
        Resolution::Ambiguous { candidates } if candidates.len() == 3
    ));
    let six: Vec<SymbolId> = ["a", "b", "c", "d", "e", "f"]
        .iter()
        .map(|n| id("m", n))
        .collect();
    assert!(matches!(
        Resolution::from_candidates(2, 9, six),
        Resolution::Ambiguous { candidates } if candidates.len() == 4
    ));
    // Describe: precise edges state a call; guesses say "may call" with the tier.
    let precise = ast_sgrep_core::resolution::ResolvedEdge {
        caller: id("m", "a"),
        callee: id("m", "b"),
        resolution: Resolution::CompilerExact,
    };
    assert_eq!(precise.describe(), ("m::a calls m::b".to_string(), true));
    let guess = ast_sgrep_core::resolution::ResolvedEdge {
        caller: id("m", "a"),
        callee: id("m", "b"),
        resolution: Resolution::NameOnly,
    };
    let (label, is_precise) = guess.describe();
    assert!(!is_precise);
    assert!(label.contains("may call"), "{label}");
    assert!(label.contains("name_only"), "{label}");
    // Qualified identity joins module, owners outermost-first, and name.
    assert_eq!(id("m", "a").qualified(), "m::a");
    assert_eq!(id("m", "a").with_owner("O").qualified(), "m::O::a");
    assert_eq!(
        id("m", "a").with_owner("A").with_owner("B").qualified(),
        "m::A::B::a"
    );

    // Facet 3: file filters — skip tables, indexable extensions, ignore + negation.
    use std::path::Path;
    // Directory skip set is exactly the VCS + tool pair.
    assert_eq!(DEFAULT_SKIP_DIR_NAMES.len(), 2);
    assert!(DEFAULT_SKIP_DIR_NAMES.contains(&".git"));
    assert!(DEFAULT_SKIP_DIR_NAMES.contains(&".asgrep"));
    assert!(should_skip_dir(Path::new(".git")));
    assert!(should_skip_dir(Path::new("a/.git")));
    assert!(should_skip_dir(Path::new(".asgrep")));
    assert!(!should_skip_dir(Path::new("src")));
    assert!(!should_skip_dir(Path::new(".")));

    // File skip: hidden files and foreign/missing extensions go; source+docs stay.
    assert!(should_skip_file(Path::new(".hidden")));
    assert!(should_skip_file(Path::new("x.xyz")));
    assert!(should_skip_file(Path::new("Makefile")));
    assert!(should_skip_file(Path::new("a.")));
    assert!(!should_skip_file(Path::new("a.rs")));
    assert!(!should_skip_file(Path::new("README.md")));
    // Extension matching is case-insensitive on both source and document sets.
    assert!(!should_skip_file(Path::new("a.RS")));
    assert!(!should_skip_file(Path::new("data.JSON")));
    assert!(is_indexable_extension("rs"));
    assert!(is_indexable_extension("RS"));
    assert!(is_indexable_extension("py"));
    assert!(is_indexable_extension("md"));
    assert!(is_indexable_extension("MD"));
    assert!(!is_indexable_extension("xyz"));
    assert!(!is_indexable_extension(""));
    assert_eq!(DOCUMENT_EXTENSIONS.len(), 6);
    for ext in ["toml", "md", "txt", "json", "yaml", "yml"] {
        assert!(DOCUMENT_EXTENSIONS.contains(&ext), "{ext}");
    }

    // Default rules ignore VCS/tool directory CONTENTS; a same-named FILE is kept.
    let dir = tempfile::tempdir().expect("tempdir");
    let matcher = IgnoreMatcher::new(dir.path());
    assert!(matcher.is_ignored(Path::new(".git/config")));
    assert!(matcher.is_ignored(Path::new(".asgrep/x")));
    assert!(!matcher.is_ignored(Path::new(".git")));
    assert!(!matcher.is_ignored(Path::new("src/main.rs")));
    // Determinism under repetition (rule chains are cached per prefix).
    assert_eq!(
        matcher.is_ignored(Path::new(".git/config")),
        matcher.is_ignored(Path::new(".git/config"))
    );

    // .gitignore: later negation re-includes; comments and blanks are inert.
    std::fs::write(
        dir.path().join(".gitignore"),
        "*.log\n!important.log\n# comment\n\n",
    )
    .expect("gitignore");
    let matcher = IgnoreMatcher::new(dir.path());
    assert!(matcher.is_ignored(Path::new("a.log")));
    assert!(matcher.is_ignored(Path::new("sub/a.log")));
    assert!(!matcher.is_ignored(Path::new("important.log")));
    assert!(!matcher.is_ignored(Path::new("src/main.rs")));
    // Differential: the free function agrees with the matcher on every probe.
    for probe in [
        "a.log",
        "sub/a.log",
        "important.log",
        "src/main.rs",
        ".git/config",
    ] {
        assert_eq!(
            is_ignored(dir.path(), Path::new(probe)),
            matcher.is_ignored(Path::new(probe)),
            "{probe}"
        );
    }
}

/// INTENT: index-side learned/external knowledge — lexicon (subtoken/prose
/// splitting, MIN_SUPPORT gate, PPMI=ln2, order, expand stability, template
/// gate, per-term cap) and SCIP (hostile-input degrade-never-fail,
/// path/ident/role/line tables).
/// KILLS: support-gate, PPMI, order, expand, truncation, fail-open, path,
/// ident, role, and line mutants.
/// ABSORBS: lexicon_support_gate_expand_stable,
/// scip_degrades_never_fails_and_ident_table.
#[test]
fn lexicon_and_scip_knowledge() {
    // Facet 1: lexicon — support gate, PPMI values, expansion stability.
    assert_eq!(MIN_SUPPORT, 3);
    assert_eq!(MAX_PER_TERM, 8);
    // Subtoken splitter: camel + snake agree, stops and short tokens drop.
    assert_eq!(subtokens("refreshToken"), vec!["refresh", "token"]);
    assert_eq!(subtokens("refresh_token"), vec!["refresh", "token"]);
    assert_eq!(subtokens("HTTPRequest"), vec!["httprequest"]);
    assert!(subtokens("the").is_empty());
    assert!(subtokens("ab").is_empty());
    assert!(subtokens("").is_empty());
    assert!(subtokens("a::b").is_empty());
    assert_eq!(subtokens("foo foo"), vec!["foo"]);
    // Prose terms lowercase, split on punctuation, and drop stops.
    assert_eq!(prose_terms("Refresh the token!"), vec!["refresh", "token"]);
    assert!(prose_terms("").is_empty());
    assert!(prose_terms("   ").is_empty());

    fn observation(terms: &[&str]) -> Observation {
        Observation {
            identifier_terms: terms.iter().map(|term| term.to_string()).collect(),
            prose_terms: vec![],
        }
    }
    // Below MIN_SUPPORT observations, finish emits nothing (fail-closed).
    let mut thin = LexiconBuilder::new();
    thin.observe(&observation(&["alpha", "beta"]));
    thin.observe(&observation(&["alpha", "beta"]));
    assert!(thin.finish().is_empty());
    // A lone term is not evidence of any pairing.
    let mut lone = LexiconBuilder::new();
    lone.observe(&observation(&["solo"]));
    assert!(lone.finish().is_empty());

    // Balanced corpus: joint 1/2 against expected 1/4, so PPMI is ln 2.
    let mut builder = LexiconBuilder::new();
    for _ in 0..3 {
        builder.observe(&observation(&["alpha", "beta"]));
        builder.observe(&observation(&["gamma", "delta"]));
    }
    let associations = builder.finish();
    assert_eq!(associations.len(), 4);
    for association in &associations {
        assert_eq!(association.support, 3);
        assert!(
            (association.ppmi - 2.0f64.ln()).abs() < 1e-12,
            "{association:?}"
        );
    }
    // Emission order is deterministic: sorted by (term, related).
    let pairs: Vec<(&str, &str)> = associations
        .iter()
        .map(|a| (a.term.as_str(), a.related.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("alpha", "beta"),
            ("beta", "alpha"),
            ("delta", "gamma"),
            ("gamma", "delta"),
        ]
    );
    assert_eq!(builder.finish(), associations);

    // Reverse lookup is symmetric: both directions resolve with equal weight.
    let lexicon = Lexicon::from_associations(associations);
    assert_eq!(lexicon.related("alpha").len(), 1);
    assert_eq!(lexicon.related("alpha")[0].related, "beta");
    assert_eq!(lexicon.related("beta")[0].related, "alpha");
    assert_eq!(
        lexicon.related("alpha")[0].ppmi,
        lexicon.related("beta")[0].ppmi
    );

    // Expansion is prefix-stable under max_added growth (sorted, then truncated).
    let weighted = Lexicon::from_associations(vec![
        Association {
            term: "a".into(),
            related: "b".into(),
            ppmi: 3.0,
            support: 5,
        },
        Association {
            term: "a".into(),
            related: "c".into(),
            ppmi: 2.0,
            support: 5,
        },
        Association {
            term: "a".into(),
            related: "d".into(),
            ppmi: 1.0,
            support: 5,
        },
    ]);
    let related: Vec<&str> = weighted
        .related("a")
        .iter()
        .map(|a| a.related.as_str())
        .collect();
    assert_eq!(related, vec!["b", "c", "d"]);
    let two: Vec<String> = weighted
        .expand(&["a".to_string()], 2)
        .iter()
        .map(|a| a.related.clone())
        .collect();
    let three: Vec<String> = weighted
        .expand(&["a".to_string()], 3)
        .iter()
        .map(|a| a.related.clone())
        .collect();
    assert_eq!(two, vec!["b", "c"]);
    assert_eq!(three, vec!["b", "c", "d"]);
    assert_eq!(&three[..two.len()], two.as_slice());
    assert!(weighted.expand(&["a".to_string()], 0).is_empty());

    // Template words never widen discovery, even when the lexicon knows them.
    let templated = Lexicon::from_associations(vec![Association {
        term: "how".into(),
        related: "zzz".into(),
        ppmi: 9.0,
        support: 9,
    }]);
    assert!(templated.expand(&["how".to_string()], 5).is_empty());
    // The gate is one-sided: other terms may still expand TO a template word.
    assert_eq!(templated.expand(&["zzz".to_string()], 5).len(), 1);

    // Per-term storage truncates to the strongest MAX_PER_TERM associations.
    let many: Vec<Association> = (0..10)
        .map(|i| Association {
            term: "hub".into(),
            related: format!("leaf{i}"),
            ppmi: i as f64,
            support: 4,
        })
        .collect();
    let capped = Lexicon::from_associations(many);
    assert_eq!(capped.related("hub").len(), MAX_PER_TERM);
    assert!(capped.related("hub").iter().all(|a| a.ppmi >= 2.0));

    // Facet 2: SCIP — degrades never fail; path/ident/occurrence tables.
    assert_eq!(SCIP_CHANNEL, "scip");
    assert_eq!(SCIP_ROLE_DEFINITION, 1);

    // Every hostile input degrades with a reason; none of them errors.
    let missing = load_scip_index(std::path::Path::new(
        "/nonexistent-dir-7f3a/index.scip.json",
    ));
    assert!(!missing.is_loaded());
    assert!(missing.degraded_reason().is_some());
    for bytes in [
        b"".as_slice(),
        b"  \n\t ",
        b"not json at all",
        b"{oops",
        b"\xff\xfe{binary}",
    ] {
        let file = write_temp(bytes);
        let loaded = load_scip_index(file.path());
        assert!(!loaded.is_loaded(), "must degrade for {bytes:?}");
        assert!(loaded.degraded_reason().is_some());
    }
    // Minimal valid JSON loads with zero documents.
    let minimal = write_temp(b"{}");
    assert!(
        matches!(load_scip_index(minimal.path()), ScipLoad::Loaded(index) if index.documents.is_empty())
    );
    // A document round-trips its relative path.
    let doc = write_temp(br#"{"documents":[{"relativePath":"a.rs","occurrences":[]}]}"#);
    match load_scip_index(doc.path()) {
        ScipLoad::Loaded(index) => {
            assert_eq!(index.documents.len(), 1);
            assert_eq!(index.documents[0].relative_path, "a.rs");
        }
        ScipLoad::Degraded { .. } => panic!("valid SCIP JSON must load"),
    }

    // Path normalization: backslashes to slashes, leading ./ stripped.
    assert_eq!(normalize_scip_path("a\\b\\c"), "a/b/c");
    assert_eq!(normalize_scip_path("./a"), "a");
    assert_eq!(normalize_scip_path("a/b"), "a/b");
    // Symbol ident: last identifier, call/parens/qualifiers stripped.
    assert_eq!(
        scip_symbol_ident("rust+crate+auth+refresh()."),
        Some("refresh".to_string())
    );
    assert_eq!(scip_symbol_ident("a::b"), Some("b".to_string()));
    assert_eq!(scip_symbol_ident("foo"), Some("foo".to_string()));
    assert_eq!(scip_symbol_ident(""), None);
    assert_eq!(scip_symbol_ident("   "), None);
    assert_eq!(scip_symbol_ident("..."), None);
    // Definition bit: only bit 0 marks a definition.
    for (roles, expected) in [(0u32, false), (1, true), (2, false), (3, true)] {
        let occurrence = ScipOccurrence {
            symbol: "s".into(),
            symbol_roles: roles,
            range: vec![0],
        };
        assert_eq!(occurrence.is_definition(), expected, "roles={roles}");
    }
    // Ranges are 0-based; indexed lines are 1-based with saturating add.
    let at_zero = ScipOccurrence {
        symbol: "s".into(),
        symbol_roles: 0,
        range: vec![0],
    };
    assert_eq!(at_zero.start_line_1based(), Some(1));
    let at_41 = ScipOccurrence {
        symbol: "s".into(),
        symbol_roles: 0,
        range: vec![41, 2, 41, 9],
    };
    assert_eq!(at_41.start_line_1based(), Some(42));
    let unranged = ScipOccurrence {
        symbol: "s".into(),
        symbol_roles: 0,
        range: vec![],
    };
    assert_eq!(unranged.start_line_1based(), None);
    let saturated = ScipOccurrence {
        symbol: "s".into(),
        symbol_roles: 0,
        range: vec![u32::MAX],
    };
    assert_eq!(saturated.start_line_1based(), Some(u32::MAX));
}

/// INTENT: store boundary contracts — schema-mismatch message round-trip with
/// (disk, supported) field order, garbage→None, and mmap read-only exact bytes.
/// KILLS: format/parse, tuple-swap, magnitude-order, truncation, and encoding mutants.
/// ABSORBS: schema_mismatch_roundtrips_and_rejects_garbage,
/// schema_mismatch_field_order_kills_swap, mmap_readonly_returns_exact_bytes.
#[test]
fn store_boundaries_schema_and_mmap() {
    // Facet 1: schema-mismatch message round-trips; garbage yields None.
    let err = StoreError::schema_newer_than_binary(7, 5);
    let message = match &err {
        StoreError::Other(m) => m.clone(),
        other => panic!("expected Other, got {other:?}"),
    };
    assert_eq!(StoreError::parse_schema_mismatch(&message), Some((7, 5)));
    // Absence, not failure-shaped success: unparseable input yields None.
    assert_eq!(StoreError::parse_schema_mismatch("garbage"), None);
    assert_eq!(StoreError::parse_schema_mismatch(""), None);
    assert_eq!(
        StoreError::parse_schema_mismatch(
            "index schema version x is newer than supported version y"
        ),
        None
    );
    // Facet 2: (disk, supported) field order, incl inverted magnitude (5, 7).
    let err = StoreError::schema_newer_than_binary(12, 3);
    let message = match &err {
        StoreError::Other(m) => m.clone(),
        other => panic!("expected Other, got {other:?}"),
    };
    assert_eq!(StoreError::parse_schema_mismatch(&message), Some((12, 3)));
    assert_eq!(
        StoreError::parse_schema_mismatch(
            "index schema version 5 is newer than supported version 7"
        ),
        Some((5, 7))
    );
    // Facet 3: mmap returns exact bytes incl NUL/multibyte, stable on remap.
    let bytes: &[u8] = b"hello\x00mmap-\xc3\xa9\n";
    let file = write_temp(bytes);
    let mapped = map_readonly(file.as_file()).expect("map");
    assert_eq!(mapped.len(), bytes.len());
    assert_eq!(&mapped[..], bytes);
    // A second mapping of the same inode agrees (stable read-only view).
    let remapped = map_readonly(file.as_file()).expect("remap");
    assert_eq!(&remapped[..], &mapped[..]);
}
