#![no_main]

//! Native pattern classifier fuzzer + fallback consistency oracle.
//!
//! `classify_native` is pure Rust (no tree-sitter). Consistency: when
//! classification succeeds, the pattern should not require external
//! fallback for the same structural class (and vice-versa for empty).

use ast_sgrep_lang::{classify_native, needs_ast_grep_fallback};
use libfuzzer_sys::fuzz_target;

const MAX_PATTERN_BYTES: usize = 256;

fuzz_target!(|input: &str| {
    if input.len() > MAX_PATTERN_BYTES {
        return;
    }

    let kind = classify_native(input);
    let needs_fallback = needs_ast_grep_fallback(input);

    // Consistency: native-classifiable patterns must not demand external fallback
    // (needs_ast_grep_fallback is defined as structure+$ with classify_native None).
    if kind.is_some() {
        assert!(
            !needs_fallback,
            "classify_native succeeded but needs_ast_grep_fallback is true for {input:?}"
        );
    }
    // Patterns without `$` never need external fallback.
    if !input.contains('$') {
        assert!(!needs_fallback);
    }
});
