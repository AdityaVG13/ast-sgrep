#![no_main]

use ast_sgrep_core::ParsedQuery;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let _ = ParsedQuery::parse(input);
});
