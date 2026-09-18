//! Pass 1 (oracle-foundry, Mission 3): independent L1 oracles for CLI
//! supervisor arithmetic and LSP text/URI/position helpers.
//!
//! Expectations are hand-computed (CPU windows, UTF-16 mappings, LSP
//! SymbolKind numbers from the protocol spec). Failures assert
//! discriminants (`is_err` / `None`), never message text.
//!
//! MCP JSON-RPC boundary oracles already exist in tests/mcp/protocol.rs
//! (`unknown_method_is_json_rpc_method_not_found`,
//! `unparsable_stdio_is_ignored`, `tool_roots_are_sandboxed_under_configured_workspace`);
//! this file does not duplicate them.

use ast_sgrep_cli::supervisor::{
    duty_cycle_ms, parse_cpu_limit, DEFAULT_CPU_LIMIT, MAX_CPU_LIMIT, MIN_CPU_LIMIT,
};
use ast_sgrep_lsp::support::{document_symbol_kind, try_apply_text_edit};
use ast_sgrep_lsp::symbols::line_at_index;
use ast_sgrep_lsp::text_edit::{extract_identifier_at, utf16_char_to_byte};
use ast_sgrep_lsp::types::{Position, Range, TextDocumentContentChangeEvent};
use ast_sgrep_lsp::uri::{file_uri_to_path, path_to_file_uri};

#[test]
fn cpu_limit_bounds_match_contract() {
    assert_eq!((MIN_CPU_LIMIT, MAX_CPU_LIMIT, DEFAULT_CPU_LIMIT), (1, 80, 80));
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
    ];
    for (raw, expected) in cases {
        assert_eq!(parse_cpu_limit(raw), *expected, "raw={raw:?}");
    }
}

#[test]
fn duty_cycle_windows_match_hand_windows() {
    assert_eq!(duty_cycle_ms(50), (5, 5));
    assert_eq!(duty_cycle_ms(0), (0, 10));
    assert_eq!(duty_cycle_ms(100), (10, 0));
    assert_eq!(duty_cycle_ms(1), (1, 9));
    // Work + sleep always fill the 10ms cycle.
    for pct in [0u8, 1, 25, 50, 80, 100] {
        let (work, sleep) = duty_cycle_ms(pct);
        assert_eq!(work + sleep, 10, "pct={pct}");
    }
}

#[test]
fn utf16_offsets_map_to_hand_bytes() {
    assert_eq!(utf16_char_to_byte("abc", 0), 0);
    assert_eq!(utf16_char_to_byte("abc", 3), 3);
    assert_eq!(utf16_char_to_byte("abc", 99), 3);
    // 'é' is one UTF-16 unit over two bytes.
    assert_eq!(utf16_char_to_byte("aé", 1), 1);
    assert_eq!(utf16_char_to_byte("aé", 2), 3);
    // '𝄞' is a surrogate pair: two units over four bytes.
    assert_eq!(utf16_char_to_byte("a𝄞b", 0), 0);
    assert_eq!(utf16_char_to_byte("a𝄞b", 1), 1);
    assert_eq!(utf16_char_to_byte("a𝄞b", 2), 1);
    assert_eq!(utf16_char_to_byte("a𝄞b", 3), 5);
    assert_eq!(utf16_char_to_byte("a𝄞b", 4), 6);
}

#[test]
fn identifier_extraction_matches_hand_spans() {
    assert_eq!(extract_identifier_at("foo bar", 0).as_deref(), Some("foo"));
    assert_eq!(extract_identifier_at("foo bar", 4).as_deref(), Some("bar"));
    // On the space between words the cursor snaps left.
    assert_eq!(extract_identifier_at("foo bar", 3).as_deref(), Some("foo"));
    assert_eq!(extract_identifier_at("_x1", 2).as_deref(), Some("_x1"));
    assert_eq!(extract_identifier_at("", 0), None);
    assert_eq!(extract_identifier_at("   ", 1), None);
}

#[test]
fn line_lookup_distinguishes_empty_from_absent() {
    assert_eq!(line_at_index("a\nb\n", 0).as_deref(), Some("a"));
    assert_eq!(line_at_index("a\nb\n", 1).as_deref(), Some("b"));
    assert_eq!(line_at_index("a\nb\n", 2).as_deref(), Some(""));
    assert_eq!(line_at_index("a\nb\n", 3), None);
    assert_eq!(line_at_index("", 1), None);
}

#[test]
fn symbol_kinds_match_lsp_spec_numbers() {
    // Independent oracle: LSP SymbolKind values (Method=6, Class=5,
    // Interface=11, Enum=10, Struct=23, Function=12).
    assert_eq!(document_symbol_kind("method"), 6);
    assert_eq!(document_symbol_kind("class"), 5);
    assert_eq!(document_symbol_kind("interface"), 11);
    assert_eq!(document_symbol_kind("enum"), 10);
    assert_eq!(document_symbol_kind("type"), 23);
    assert_eq!(document_symbol_kind("fnord"), 12);
    assert_eq!(document_symbol_kind(""), 12);
}

#[test]
fn file_uri_decode_matches_hand_paths() {
    assert_eq!(
        file_uri_to_path("file:///tmp/x.rs").expect("uri"),
        std::path::PathBuf::from("/tmp/x.rs")
    );
    assert_eq!(
        file_uri_to_path("file:///tmp/a%20b.rs").expect("pct"),
        std::path::PathBuf::from("/tmp/a b.rs")
    );
    assert!(file_uri_to_path("http://example.com/x").is_err());
    assert!(file_uri_to_path("").is_err());
    assert!(file_uri_to_path("/tmp/x.rs").is_err());
}

#[test]
fn file_uri_roundtrips_through_canonical_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("roundtrip.rs");
    std::fs::write(&file, b"fn main() {}\n").expect("write");
    let uri = path_to_file_uri(&file);
    assert!(uri.starts_with("file://"), "uri={uri}");
    let back = file_uri_to_path(&uri).expect("decode");
    assert_eq!(back, file.canonicalize().expect("canonical"));
}

fn full_replace(text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: text.to_string(),
    }
}

fn ranged(
    start: (u32, u32),
    end: (u32, u32),
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
        range_length: None,
        text: text.to_string(),
    }
}

#[test]
fn text_edit_applies_replace_and_rejects_bad_ranges() {
    assert_eq!(
        try_apply_text_edit("hello", &full_replace("bye")).expect("full"),
        "bye"
    );
    assert_eq!(
        try_apply_text_edit("hello", &ranged((0, 0), (0, 5), "bye")).expect("ranged"),
        "bye"
    );
    assert_eq!(
        try_apply_text_edit("hello", &ranged((0, 1), (0, 4), "i")).expect("inner"),
        "hio"
    );
    // Out-of-bounds ranges fail loudly instead of clamping silently.
    assert!(try_apply_text_edit("a\nb", &ranged((5, 0), (5, 1), "x")).is_err());
}
