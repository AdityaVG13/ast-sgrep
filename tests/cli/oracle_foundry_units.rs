//! Consolidated CLI oracle units: independent L1 hand oracles (pass1) folded
//! together with their L2 mutation-discriminating rows (pass2), grouped by
//! function under test. All 7 pass2 MERGE verdicts fold here with zero loss
//! (literally duplicated rows merged once, noted per test); the 3 pass2 KEEP
//! text-edit tests join the text-edit contracts test as new facets.
//!
//! Expectations are hand-computed. Failures assert discriminants (`is_err` /
//! `None`), never message text.

use ast_sgrep_cli::supervisor::{
    duty_cycle_ms, parse_cpu_limit, DEFAULT_CPU_LIMIT, MAX_CPU_LIMIT, MIN_CPU_LIMIT,
};
use ast_sgrep_lsp::support::{document_symbol_kind, try_apply_text_edit};
use ast_sgrep_lsp::symbols::line_at_index;
use ast_sgrep_lsp::text_edit::{apply_text_edit, extract_identifier_at, utf16_char_to_byte};
use ast_sgrep_lsp::uri::{file_uri_to_path, path_to_file_uri, uri_to_rel_path};
use ast_sgrep_testkit::{edit_full_replace, edit_ranged, edit_ranged_len};

/// INTENT: cpu-limit bounds contract — (min,max,default) are (1,80,80),
/// in-range parses, out-of-range/unparsable/edge spellings fall back to 80.
/// KILLS: clamp-removal, fallback-default, trim-removal, underscore/hex/
/// fullwidth leniency mutants.
/// ABSORBS: cpu_limit_bounds_match_contract + parse_cpu_limit_edge_spellings
/// (6 rows folded in; `"1"` duplicated across passes, pinned once).
#[test]
fn cpu_limit_bounds_match_contract() {
    assert_eq!(
        (MIN_CPU_LIMIT, MAX_CPU_LIMIT, DEFAULT_CPU_LIMIT),
        (1, 80, 80)
    );
    let cases: &[(&str, u8)] = &[
        ("50", 50),
        ("1", 1),
        ("80", 80),
        ("007", 7),
        (" 50 ", 50),
        // Out-of-range or unparsable input falls back to the default bound.
        ("0", 80),
        ("81", 80),
        ("256", 80),
        ("abc", 80),
        ("", 80),
        // Edge spellings: surrounding whitespace trims, exotic spellings fall back.
        ("50\n", 50),
        ("\t80\t", 80),
        ("5_0", 80),
        ("0x10", 80),
        ("５０", 80),
    ];
    for (raw, expected) in cases {
        assert_eq!(parse_cpu_limit(raw), *expected, "raw={raw:?}");
    }
}

/// INTENT: duty-cycle windows — hand 10ms windows, 0 stays (0,10), 1..=9
/// clamp work up to 1, work monotone over 0..=100, work+sleep==10 always.
/// KILLS: formula/rounding, `.max(1)`-arm confusion (zero arm and nonzero
/// arm), monotonicity mutants.
/// ABSORBS: duty_cycle_windows_match_hand_windows +
/// duty_cycle_zero_stays_zero_nonzero_clamped_up (hand rows + full-range
/// loop folded in; the full 0..=100 loop supersedes the 6-point sum loop).
#[test]
fn duty_cycle_windows_match_hand_windows() {
    let cases: &[(u8, (u64, u64))] = &[
        (50, (5, 5)),
        (0, (0, 10)),
        (100, (10, 0)),
        (1, (1, 9)),
        (2, (1, 9)),
        (10, (1, 9)),
        (20, (2, 8)),
    ];
    for (pct, expected) in cases {
        assert_eq!(duty_cycle_ms(*pct), *expected, "pct={pct}");
    }
    // Work + sleep always fill the 10ms cycle; work never decreases.
    let mut prev = 0u64;
    for pct in 0..=100u8 {
        let (work, sleep) = duty_cycle_ms(pct);
        assert_eq!(work + sleep, 10, "pct={pct}");
        assert!(work >= prev, "pct={pct}");
        prev = work;
    }
}

/// INTENT: UTF-16→byte mapping incl BMP é and surrogate-pair 𝄞 midpoints;
/// OOB and empty-line offsets clamp to len.
/// KILLS: surrogate-midpoint `<`→`<=` flip, end-clamp removal,
/// empty-line mutants.
/// ABSORBS: utf16_offsets_map_to_hand_bytes +
/// utf16_mid_surrogate_and_end_clamps (3 asserts literally duplicated
/// across passes, pinned once; end/empty clamp rows folded in).
#[test]
fn utf16_offsets_map_to_hand_bytes() {
    let cases: &[(&str, u32, usize)] = &[
        ("abc", 0, 0),
        ("abc", 3, 3),
        ("abc", 99, 3),
        // 'é' is one UTF-16 unit over two bytes.
        ("aé", 1, 1),
        ("aé", 2, 3),
        // '𝄞' is a surrogate pair: two units over four bytes; a midpoint
        // offset maps to the pair start, not past it.
        ("a𝄞b", 0, 0),
        ("a𝄞b", 1, 1),
        ("a𝄞b", 2, 1),
        ("a𝄞b", 3, 5),
        ("a𝄞b", 4, 6),
        // End and empty clamps.
        ("a𝄞b", 99, 6),
        ("", 0, 0),
        ("", 5, 0),
    ];
    for (line, offset, expected) in cases {
        assert_eq!(
            utf16_char_to_byte(line, *offset),
            *expected,
            "line={line:?} offset={offset}"
        );
    }
}

/// INTENT: identifier at cursor — space/punct snaps left, past-end snaps to
/// the last ident, empty/blank/leading-punct is None, mid-multibyte safe.
/// KILLS: snap-direction swap, snap-right, OOB-None, mid-multibyte panic mutants.
/// ABSORBS: identifier_extraction_matches_hand_spans +
/// extract_identifier_snaps_and_rejects (disjoint rows, one table).
#[test]
fn identifier_extraction_matches_hand_spans() {
    let cases: &[(&str, usize, Option<&str>)] = &[
        ("foo bar", 0, Some("foo")),
        ("foo bar", 4, Some("bar")),
        // On the space between words the cursor snaps left.
        ("foo bar", 3, Some("foo")),
        ("_x1", 2, Some("_x1")),
        ("", 0, None),
        ("   ", 1, None),
        // Past-end snaps to the last ident; punctuation snaps left.
        ("foo", 99, Some("foo")),
        ("foo;", 3, Some("foo")),
        ("a+b", 1, Some("a")),
        ("+b", 0, None),
        ("+b", 1, Some("b")),
        // Mid-multibyte cursor stays safe; OOB on empty is None.
        ("café", 4, Some("café")),
        ("", 5, None),
    ];
    for (line, col, expected) in cases {
        assert_eq!(
            extract_identifier_at(line, *col).as_deref(),
            *expected,
            "line={line:?} col={col}"
        );
    }
}

/// INTENT: line lookup distinguishes empty from absent — trailing newline
/// yields Some(""), past-end is None, empty doc has exactly one empty line.
/// KILLS: trailing-newline miscount, empty-content-None mutants.
/// ABSORBS: line_lookup_distinguishes_empty_from_absent +
/// line_lookup_empty_and_trailing_edges (`("",1)` duplicated, pinned once).
#[test]
fn line_lookup_distinguishes_empty_from_absent() {
    let cases: &[(&str, usize, Option<&str>)] = &[
        ("a\nb\n", 0, Some("a")),
        ("a\nb\n", 1, Some("b")),
        ("a\nb\n", 2, Some("")),
        ("a\nb\n", 3, None),
        ("", 1, None),
        // Empty content is one empty line; trailing-newline edges.
        ("", 0, Some("")),
        ("a\n", 0, Some("a")),
        ("a\n", 1, Some("")),
        ("a\n", 2, None),
        ("a", 1, None),
    ];
    for (content, idx, expected) in cases {
        assert_eq!(
            line_at_index(content, *idx).as_deref(),
            *expected,
            "content={content:?} idx={idx}"
        );
    }
}

/// INTENT: symbol kinds match LSP spec ints; matching is case-sensitive and
/// unknown/empty/case-variants default to Function (12).
/// KILLS: kind-table/alias, case-folding mutants (`Method`/`struct` must
/// NOT map; only `type` maps to 23).
/// ABSORBS: symbol_kinds_match_lsp_spec_numbers +
/// symbol_kind_is_case_sensitive_with_function_default (`method`/`type`
/// duplicated, pinned once; 4 case rows folded in).
#[test]
fn symbol_kinds_match_lsp_spec_numbers() {
    // Independent oracle: LSP SymbolKind values (Method=6, Class=5,
    // Interface=11, Enum=10, Struct=23, Function=12).
    let cases: &[(&str, u32)] = &[
        ("method", 6),
        ("class", 5),
        ("interface", 11),
        ("enum", 10),
        ("type", 23),
        ("fnord", 12),
        ("", 12),
        // Case variants and near-aliases fall back to the default.
        ("Method", 12),
        ("METHOD", 12),
        ("Class", 12),
        ("struct", 12),
    ];
    for (kind, expected) in cases {
        assert_eq!(document_symbol_kind(kind), *expected, "kind={kind:?}");
    }
}

/// INTENT: file-URI decode contract (scheme-gated, %-decoding) plus
/// path→URI→path round-trip through a canonicalized tempdir file.
/// KILLS: scheme-check/pct-decode removal, scheme case-folding,
/// BEHAVIOR-ONLY canonical-roundtrip drift (only roundtrip pin).
/// ABSORBS: file_uri_decode_matches_hand_paths +
/// file_uri_scheme_is_case_sensitive (folded into the decode table) +
/// file_uri_roundtrips_through_canonical_path (kept as its own facet).
#[test]
fn file_uri_decode_matches_hand_paths() {
    // Decode table: Ok rows pin exact paths, Err rows pin the discriminant.
    assert_eq!(
        file_uri_to_path("file:///tmp/x.rs").expect("uri"),
        std::path::PathBuf::from("/tmp/x.rs")
    );
    assert_eq!(
        file_uri_to_path("file:///tmp/a%20b.rs").expect("pct"),
        std::path::PathBuf::from("/tmp/a b.rs")
    );
    assert_eq!(
        file_uri_to_path("file:///a%2Fb.rs").expect("pct"),
        std::path::PathBuf::from("/a/b.rs")
    );
    assert!(file_uri_to_path("file://").is_ok());
    for bad in [
        "http://example.com/x",
        "",
        "/tmp/x.rs",
        "FILE:///tmp/x.rs",
        "ftp://x.rs",
    ] {
        assert!(file_uri_to_path(bad).is_err(), "uri={bad:?}");
    }

    // Round-trip facet: path→URI→path through a canonicalized tempdir file.
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("roundtrip.rs");
    std::fs::write(&file, b"fn main() {}\n").expect("write");
    let uri = path_to_file_uri(&file);
    assert!(uri.starts_with("file://"), "uri={uri}");
    let back = file_uri_to_path(&uri).expect("decode");
    assert_eq!(back, file.canonicalize().expect("canonical"));
}

#[cfg(unix)]
#[test]
fn path_identity_survives_lsp_and_ignore_boundaries() {
    use ast_sgrep_core::gitignore::IgnoreMatcher;
    use std::path::Path;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".gitignore"), "literal/\n").unwrap();
    let file = dir.path().join(r"literal\name.rs");
    std::fs::write(&file, "fn intended() {}\n").unwrap();
    let uri = path_to_file_uri(&file);
    assert_eq!(
        file_uri_to_path(&uri).unwrap(),
        file.canonicalize().unwrap()
    );
    assert_eq!(
        uri_to_rel_path(&uri, dir.path()).unwrap(),
        r"literal\name.rs"
    );
    let ignores = IgnoreMatcher::new(dir.path());
    assert!(!ignores.is_ignored(Path::new(r"literal\name.rs")));
    assert!(!ignores.is_dir_ignored(Path::new(r"literal\sub")));
    assert!(ignores.is_ignored(Path::new("literal/name.rs")));
}

#[test]
fn file_uris_reject_invalid_utf8_instead_of_aliasing_replacement_characters() {
    assert!(file_uri_to_path("file:///tmp/%FF.rs").is_err());
    assert!(file_uri_to_path("file:///tmp/%ED%A0%80.rs").is_err());
}

#[cfg(unix)]
#[test]
fn scip_overlay_keeps_distinct_indexed_paths_distinct() {
    let session = ast_sgrep_testkit::isolated_index_session();
    session.write(r"literal\name.rs", "fn backslash_symbol() {}\n");
    session.write("literal/name.rs", "fn slash_symbol() {}\n");
    session.index_all(ast_sgrep_core::IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let index = serde_json::from_value(serde_json::json!({"documents": [
        {"relative_path": "literal\\name.rs", "occurrences": [{"symbol": "backslash_symbol", "symbol_roles": 1, "range": [0, 0, 5]}]},
        {"relative_path": "literal/name.rs", "occurrences": [{"symbol": "slash_symbol", "symbol_roles": 1, "range": [0, 0, 5]}]}
    ]})).unwrap();
    let stats = session.open_store().apply_scip(&index).unwrap();
    assert_eq!(
        stats.defs_upgraded, 2,
        "a normalized file map must not merge different DB keys"
    );
    assert_eq!(stats.skipped, 0);
}

#[test]
fn indexed_lines_preserve_final_carriage_returns() {
    assert_eq!(
        ast_sgrep_core::index::split_content_lines("first\r\nlast\r").lines,
        vec![(1, "first".into()), (2, "last\r".into())]
    );
}

#[cfg(unix)]
#[test]
fn native_patterns_preserve_backslash_paths_with_and_without_index_candidates() {
    use ast_sgrep_core::{IndexOptions, SearchOptions};
    let session = ast_sgrep_testkit::isolated_index_session();
    session.write(r"literal\name.rs", "fn run() { needle(1); }\n");
    session.write("literal/name.rs", "fn wrong_file() {}\n");
    let store = session.open_store();
    let hits = ast_sgrep_core::pattern::search_pattern(
        "needle($$$ARGS)",
        &store,
        &session.corpus_root,
        Some("rust"),
        20,
    )
    .unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter().all(|hit| hit.file == r"literal\name.rs"),
        "{hits:?}"
    );
    session.index_all(IndexOptions {
        embed_semantic: false,
        ..session.index_options()
    });
    let searcher = session.searcher(SearchOptions {
        use_embed: false,
        ..session.search_options()
    });
    let response = searcher.search("pattern:needle($$$ARGS)").unwrap();
    assert!(!response.hits.is_empty());
    assert!(
        response
            .hits
            .iter()
            .all(|hit| hit.file == r"literal\name.rs"),
        "{:?}",
        response.hits
    );
}

/// INTENT: text-edit contracts — full/ranged replaces apply, rangeLength
/// overrides end, reversed/OOB/mid-surrogate ranges are Err (never silent
/// clamp or panic), and the best-effort `apply_` falls back to the content.
/// KILLS: OOB-clamp, rangeLength-ignored, order-check-removal/slice-panic,
/// and `unwrap_or_else`→`unwrap` mutants.
/// ABSORBS: text_edit_applies_replace_and_rejects_bad_ranges +
/// text_edit_range_length_overrides_end +
/// text_edit_reversed_and_oob_rejected +
/// apply_text_edit_never_panics_falls_back (4 facets, one contract).
#[test]
fn text_edit_contracts_apply_and_reject() {
    // Facet 1: full and ranged replaces apply; OOB line range is Err.
    assert_eq!(
        try_apply_text_edit("hello", &edit_full_replace("bye")).expect("full"),
        "bye"
    );
    assert_eq!(
        try_apply_text_edit("hello", &edit_ranged((0, 0), (0, 5), "bye")).expect("ranged"),
        "bye"
    );
    assert_eq!(
        try_apply_text_edit("hello", &edit_ranged((0, 1), (0, 4), "i")).expect("inner"),
        "hio"
    );
    assert!(try_apply_text_edit("a\nb", &edit_ranged((5, 0), (5, 1), "x")).is_err());

    // Facet 2: rangeLength overrides end (insert vs span); mid-surrogate
    // truncation and overrun are Err.
    assert_eq!(
        try_apply_text_edit("hello", &edit_ranged_len((0, 1), (0, 4), Some(0), "X")).expect("ins"),
        "hXello"
    );
    assert_eq!(
        try_apply_text_edit("hello", &edit_ranged_len((0, 0), (0, 0), Some(5), "X")).expect("span"),
        "X"
    );
    assert!(try_apply_text_edit("a𝄞", &edit_ranged_len((0, 1), (0, 3), Some(1), "x")).is_err());
    assert!(try_apply_text_edit("hi", &edit_ranged_len((0, 0), (0, 2), Some(5), "x")).is_err());

    // Facet 3: reversed and OOB ranges are Err; empty-doc (0,0) insert ok.
    assert!(try_apply_text_edit("hello", &edit_ranged_len((0, 3), (0, 1), None, "x")).is_err());
    assert!(try_apply_text_edit("hello", &edit_ranged_len((0, 0), (0, 9), None, "x")).is_err());
    assert!(try_apply_text_edit("a", &edit_ranged_len((2, 0), (2, 0), None, "x")).is_err());
    assert_eq!(
        try_apply_text_edit("", &edit_ranged_len((0, 0), (0, 0), None, "x")).expect("empty ins"),
        "x"
    );

    // Facet 4: best-effort `apply_` returns content on bad ranges, edited
    // text on good ones — never panics.
    let bad = edit_ranged_len((0, 3), (0, 1), None, "x");
    assert_eq!(apply_text_edit("ab", &bad), "ab");
    let good = edit_ranged_len((0, 0), (0, 2), None, "xy");
    assert_eq!(apply_text_edit("ab", &good), "xy");
}
