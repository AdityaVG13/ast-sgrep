//! Pass 2 (oracle-foundry, Mission 3): L2 mutation-discriminating oracles for
//! CLI supervisor arithmetic and LSP text/URI/position helpers.
//!
//! Each test names the mutant class it kills: zero-vs-nonzero duty-cycle
//! confusion, trim removal, mid-surrogate comparison flips, snap-direction
//! swaps, case-folding, rangeLength-ignored edits, and order-check removal.
//! MCP protocol-boundary mutants live in tests/mcp/oracle_foundry_pass2.rs.

use ast_sgrep_cli::supervisor::{duty_cycle_ms, parse_cpu_limit};
use ast_sgrep_lsp::support::{document_symbol_kind, try_apply_text_edit};
use ast_sgrep_lsp::symbols::line_at_index;
use ast_sgrep_lsp::text_edit::{apply_text_edit, extract_identifier_at, utf16_char_to_byte};
use ast_sgrep_lsp::types::{Position, Range, TextDocumentContentChangeEvent};
use ast_sgrep_lsp::uri::file_uri_to_path;

#[test]
fn duty_cycle_zero_stays_zero_nonzero_clamped_up() {
    // Kills: `.max(1)` applied to the zero arm (would be (1,9)), `.max(1)`
    // removed (pct 1..9 would be (0,10)), and formula mutants (monotonicity).
    assert_eq!(duty_cycle_ms(0), (0, 10));
    assert_eq!(duty_cycle_ms(1), (1, 9));
    assert_eq!(duty_cycle_ms(2), (1, 9));
    assert_eq!(duty_cycle_ms(10), (1, 9));
    assert_eq!(duty_cycle_ms(20), (2, 8));
    let mut prev = 0u64;
    for pct in 0..=100u8 {
        let (work, sleep) = duty_cycle_ms(pct);
        assert_eq!(work + sleep, 10, "pct={pct}");
        assert!(work >= prev, "pct={pct}");
        prev = work;
    }
}

#[test]
fn parse_cpu_limit_edge_spellings() {
    // Kills: trim removal (newline/tab-padded values would fall to default),
    // underscore/hex leniency, and fullwidth-digit acceptance.
    assert_eq!(parse_cpu_limit("50\n"), 50);
    assert_eq!(parse_cpu_limit("\t80\t"), 80);
    assert_eq!(parse_cpu_limit("5_0"), 80);
    assert_eq!(parse_cpu_limit("0x10"), 80);
    assert_eq!(parse_cpu_limit("５０"), 80);
    assert_eq!(parse_cpu_limit("1"), 1);
}

#[test]
fn line_lookup_empty_and_trailing_edges() {
    // Kills: empty-content None (split yields one empty line), and
    // trailing-newline miscounts.
    assert_eq!(line_at_index("", 0).as_deref(), Some(""));
    assert_eq!(line_at_index("", 1), None);
    assert_eq!(line_at_index("a\n", 0).as_deref(), Some("a"));
    assert_eq!(line_at_index("a\n", 1).as_deref(), Some(""));
    assert_eq!(line_at_index("a\n", 2), None);
    assert_eq!(line_at_index("a", 1), None);
}

#[test]
fn utf16_mid_surrogate_and_end_clamps() {
    // Kills: `<` flipped to `<=` at surrogate midpoints (offset 2 would map
    // to byte 5 instead of 1), end-clamp removal, and empty-line mutants.
    assert_eq!(utf16_char_to_byte("a𝄞b", 2), 1);
    assert_eq!(utf16_char_to_byte("a𝄞b", 3), 5);
    assert_eq!(utf16_char_to_byte("a𝄞b", 4), 6);
    assert_eq!(utf16_char_to_byte("a𝄞b", 99), 6);
    assert_eq!(utf16_char_to_byte("", 0), 0);
    assert_eq!(utf16_char_to_byte("", 5), 0);
}

#[test]
fn extract_identifier_snaps_and_rejects() {
    // Kills: OOB-None (past-end must snap to the last ident), snap-right
    // (leading punctuation at 0 must be None, not the next ident),
    // punctuation snap-left removal, and mid-multibyte panics.
    assert_eq!(extract_identifier_at("foo", 99).as_deref(), Some("foo"));
    assert_eq!(extract_identifier_at("foo;", 3).as_deref(), Some("foo"));
    assert_eq!(extract_identifier_at("a+b", 1).as_deref(), Some("a"));
    assert_eq!(extract_identifier_at("+b", 0), None);
    assert_eq!(extract_identifier_at("+b", 1).as_deref(), Some("b"));
    assert_eq!(extract_identifier_at("café", 4).as_deref(), Some("café"));
    assert_eq!(extract_identifier_at("", 5), None);
}

#[test]
fn symbol_kind_is_case_sensitive_with_function_default() {
    // Kills: case-folding ("Method" would become 6) and alias mutants
    // ("struct" is not in the table; only "type" maps to 23).
    assert_eq!(document_symbol_kind("Method"), 12);
    assert_eq!(document_symbol_kind("METHOD"), 12);
    assert_eq!(document_symbol_kind("Class"), 12);
    assert_eq!(document_symbol_kind("struct"), 12);
    assert_eq!(document_symbol_kind("method"), 6);
    assert_eq!(document_symbol_kind("type"), 23);
}

#[test]
fn file_uri_scheme_is_case_sensitive() {
    // Kills: scheme case-folding and pct-decode removal.
    assert!(file_uri_to_path("FILE:///tmp/x.rs").is_err());
    assert!(file_uri_to_path("ftp://x.rs").is_err());
    assert_eq!(
        file_uri_to_path("file:///a%2Fb.rs").expect("pct"),
        std::path::PathBuf::from("/a/b.rs")
    );
    assert!(file_uri_to_path("file://").is_ok());
}

fn ranged_len(
    start: (u32, u32),
    end: (u32, u32),
    range_length: Option<u32>,
    text: &str,
) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: Some(Range {
            start: Position {
                line: start.0,
                character: start.1,
            },
            end: Position {
                line: end.0,
                character: end.1,
            },
        }),
        range_length,
        text: text.to_string(),
    }
}

#[test]
fn text_edit_range_length_overrides_end() {
    // Kills: rangeLength ignored (end position used instead), zero-length
    // insertion mishandled, mid-surrogate truncation, and overrun silence.
    assert_eq!(
        try_apply_text_edit("hello", &ranged_len((0, 1), (0, 4), Some(0), "X")).expect("ins"),
        "hXello"
    );
    assert_eq!(
        try_apply_text_edit("hello", &ranged_len((0, 0), (0, 0), Some(5), "X")).expect("span"),
        "X"
    );
    assert!(try_apply_text_edit("a𝄞", &ranged_len((0, 1), (0, 3), Some(1), "x")).is_err());
    assert!(try_apply_text_edit("hi", &ranged_len((0, 0), (0, 2), Some(5), "x")).is_err());
}

#[test]
fn text_edit_reversed_and_oob_rejected() {
    // Kills: start > end check removed (would panic on [3..1] slicing
    // instead of Err), end-OOB clamping, and empty-content insertion refusal.
    assert!(try_apply_text_edit("hello", &ranged_len((0, 3), (0, 1), None, "x")).is_err());
    assert!(try_apply_text_edit("hello", &ranged_len((0, 0), (0, 9), None, "x")).is_err());
    assert!(try_apply_text_edit("a", &ranged_len((2, 0), (2, 0), None, "x")).is_err());
    assert_eq!(
        try_apply_text_edit("", &ranged_len((0, 0), (0, 0), None, "x")).expect("empty ins"),
        "x"
    );
}

#[test]
fn apply_text_edit_never_panics_falls_back() {
    // Kills: best-effort `unwrap_or_else` replaced by unwrap (panic on bad
    // ranges) and fallback returning the edit instead of the content.
    let bad = ranged_len((0, 3), (0, 1), None, "x");
    assert_eq!(apply_text_edit("ab", &bad), "ab");
    let good = ranged_len((0, 0), (0, 2), None, "xy");
    assert_eq!(apply_text_edit("ab", &good), "xy");
}
