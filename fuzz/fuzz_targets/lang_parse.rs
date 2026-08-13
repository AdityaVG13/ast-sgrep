#![no_main]

//! Polyglot tree-sitter parse fuzzer (CVE class: grammar C parsers).
//!
//! Init `ParserRegistry` once per process via `OnceLock` — never reconstruct
//! per input (exec/s floor).

use ast_sgrep_lang::{Language, ParserRegistry};
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

/// PASS5 default CI budget; hard guard below.
const MAX_SOURCE_BYTES: usize = 4 * 1024;

fn registry() -> &'static ParserRegistry {
    static REG: OnceLock<ParserRegistry> = OnceLock::new();
    REG.get_or_init(ParserRegistry::new)
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > MAX_SOURCE_BYTES + 1 {
        return;
    }
    // First byte selects language; remainder is source.
    let langs = Language::all();
    let lang = langs[data[0] as usize % langs.len()];
    let Ok(source) = std::str::from_utf8(&data[1..]) else {
        return;
    };

    // Crash oracle: no panic/abort. Err from tree-sitter is fine.
    let _ = registry().parse(lang, source);
});
