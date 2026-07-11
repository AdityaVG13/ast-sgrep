#![no_main]

use ast_sgrep_core::query::ParsedQuery;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let _ = ParsedQuery::parse(input);
});
