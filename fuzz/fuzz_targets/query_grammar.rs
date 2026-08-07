#![no_main]

//! Structural query grammar fuzzer.
//!
//! Oracle (strength ≥3): parse never panics; mode/target/raw invariants hold
//! for every input; re-parse of `raw` is stable on mode + target shape.

use ast_sgrep_core::{ParsedQuery, QueryMode};
use libfuzzer_sys::fuzz_target;

/// PASS5 budget: 8 KiB query strings.
const MAX_QUERY_BYTES: usize = 8 * 1024;

fuzz_target!(|input: &str| {
    // Size guard (also pass -max_len via libFuzzer when desired).
    if input.len() > MAX_QUERY_BYTES {
        return;
    }

    let parsed = ParsedQuery::parse(input);

    // raw is always the trimmed input (including mode prefix when present).
    assert_eq!(parsed.raw, input.trim());

    // Prefixed modes always set target (possibly empty string).
    match parsed.mode {
        QueryMode::Callers
        | QueryMode::Defs
        | QueryMode::Imports
        | QueryMode::Pattern
        | QueryMode::Literal
        | QueryMode::Regex
        | QueryMode::Word => {
            assert!(
                parsed.target.is_some(),
                "prefixed mode {:?} must set target",
                parsed.mode
            );
        }
        QueryMode::Hybrid => {
            // Hybrid is unprefixed: target stays None.
            assert!(parsed.target.is_none());
        }
    }

    // Re-parse of stored raw is stable on mode and target.
    let again = ParsedQuery::parse(&parsed.raw);
    assert_eq!(again.mode, parsed.mode);
    assert_eq!(again.target, parsed.target);
    assert_eq!(again.raw, parsed.raw);
});
