//! Lang parser-cache contract suite: per-key parser reuse is unobservable.
//!
//! Contract under test: the thread-local parser maps in `extract`/`templates`
//! are append-only with NO invalidation API — every cache is a pure function
//! of its key, pinned through the public API only. Discriminants are symbol
//! names and line sets, never message text.

use ast_sgrep_lang::{match_pattern, Language, ParserRegistry};
use std::collections::HashSet;

// WHY area-local: symbol-name projection is the discriminant for extraction
// equality; only this suite projects extraction symbols — single-suite helper.
fn symbol_names(result: &ast_sgrep_lang::ExtractionResult) -> Vec<String> {
    result
        .symbols
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>()
}

/// INTENT: sequential, interleaved, grown, and rescheduled parses return
/// identical per-key extractions with exact symbols.
/// KILLS: stale-parser-reuse / parse-nondeterminism / cross-source-symbol-leak
/// / cross-language-parser-poisoning / per-language-key-collapse /
/// growth-eviction / parser-population-order-dependence.
/// ABSORBS: interleaved_sources_do_not_leak_symbols,
/// interleaved_languages_do_not_pollute_parsers,
/// parser_keys_are_per_language_under_growth, parser_round_robin_matches_batched.
#[test]
fn parser_reuse_returns_identical_extraction() {
    // Leg 1 (anchor): sequential same-source parses are identical.
    let registry = ParserRegistry::new();
    let source = "fn alpha() {}\nfn beta() {}\n";
    let first = registry.parse(Language::Rust, source).unwrap();
    let second = registry.parse(Language::Rust, source).unwrap();
    assert_eq!(first, second);
    assert_eq!(symbol_names(&first), vec!["alpha", "beta"]);

    // Leg 2 (interleaved sources): A-B-A keeps per-source symbols isolated.
    let src_a = "fn alpha() {}\n";
    let src_b = "fn beta() {}\n";
    let a1 = registry.parse(Language::Rust, src_a).unwrap();
    let b = registry.parse(Language::Rust, src_b).unwrap();
    let a2 = registry.parse(Language::Rust, src_a).unwrap();
    assert_eq!(a1, a2);
    assert_eq!(symbol_names(&a1), vec!["alpha"]);
    assert_eq!(symbol_names(&b), vec!["beta"]);
    assert_ne!(a1, b);

    // Leg 3 (interleaved languages): Rust-Python-Rust keeps parsers isolated.
    let rust_src = "fn alpha() {}\n";
    let py_src = "def beta():\n    pass\n";
    let r1 = registry.parse(Language::Rust, rust_src).unwrap();
    let p = registry.parse(Language::Python, py_src).unwrap();
    let r2 = registry.parse(Language::Rust, rust_src).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(symbol_names(&r1), vec!["alpha"]);
    assert_eq!(symbol_names(&p), vec!["beta"]);

    // Leg 4 (growth): per-language keys stable after growth to further keys.
    let js_src = "function gamma() {}\n";
    let rust_before = registry.parse(Language::Rust, rust_src).unwrap();
    let py_before = registry.parse(Language::Python, py_src).unwrap();
    let js_before = registry.parse(Language::JavaScript, js_src).unwrap();
    assert_eq!(symbol_names(&rust_before), vec!["alpha"]);
    assert_eq!(symbol_names(&py_before), vec!["beta"]);
    assert_eq!(symbol_names(&js_before), vec!["gamma"]);
    let go_src = "package main\n\nfunc delta() {}\n";
    let ts_src = "function epsilon() {}\n";
    let go_first = registry.parse(Language::Go, go_src).unwrap();
    let ts_first = registry.parse(Language::TypeScript, ts_src).unwrap();
    assert_eq!(go_first, registry.parse(Language::Go, go_src).unwrap());
    assert_eq!(
        ts_first,
        registry.parse(Language::TypeScript, ts_src).unwrap()
    );
    assert_eq!(
        rust_before,
        registry.parse(Language::Rust, rust_src).unwrap()
    );
    assert_eq!(py_before, registry.parse(Language::Python, py_src).unwrap());
    assert_eq!(
        js_before,
        registry.parse(Language::JavaScript, js_src).unwrap()
    );
    assert_ne!(symbol_names(&rust_before), symbol_names(&py_before));
    assert_ne!(symbol_names(&rust_before), symbol_names(&js_before));
    assert_ne!(symbol_names(&py_before), symbol_names(&js_before));

    // Leg 5 (round-robin): batched vs interleaved schedules agree per cell.
    let cells = [
        (Language::Rust, "fn alpha() {}\n"),
        (Language::Python, "def beta():\n    pass\n"),
        (Language::JavaScript, "function gamma() {}\n"),
        (Language::Go, "package main\n\nfunc delta() {}\n"),
    ];
    let registry_a = ParserRegistry::new();
    let batched: Vec<Vec<String>> = cells
        .iter()
        .map(|(l, src)| symbol_names(&registry_a.parse(*l, src).unwrap()))
        .collect();
    let registry_b = ParserRegistry::new();
    let order = [2usize, 0, 3, 1];
    let mut interleaved = vec![Vec::new(); cells.len()];
    for cell in order {
        let (l, src) = cells[cell];
        interleaved[cell] = symbol_names(&registry_b.parse(l, src).unwrap());
    }
    assert_eq!(batched, interleaved);
    assert_eq!(batched[0], vec!["alpha"]);
    assert_eq!(batched[1], vec!["beta"]);
    assert_eq!(batched[2], vec!["gamma"]);
    assert_eq!(batched[3], vec!["delta"]);
}

/// INTENT: same pattern follows the current source across v1→v2→v1 with exact
/// line sets.
/// KILLS: stale-parse-reuse-on-source-mutation.
/// ABSORBS: (standalone — no merges).
#[test]
fn match_pattern_tracks_source_mutations() {
    let v1 = "fn foo() {}\n";
    let v2 = "fn foo() {}\nfn foo() {}\n";
    let hits_v1 = match_pattern(Language::Rust, v1, "foo").unwrap();
    let hits_v2 = match_pattern(Language::Rust, v2, "foo").unwrap();
    let hits_v1_again = match_pattern(Language::Rust, v1, "foo").unwrap();
    assert!(!hits_v1.is_empty());
    assert!(hits_v1.iter().all(|h| h.line_start == 1));
    let lines_v2: HashSet<u32> = hits_v2.iter().map(|h| h.line_start).collect();
    assert_eq!(lines_v2, HashSet::from([1, 2]));
    assert!(hits_v2.len() > hits_v1.len());
    assert_eq!(hits_v1, hits_v1_again);
}
