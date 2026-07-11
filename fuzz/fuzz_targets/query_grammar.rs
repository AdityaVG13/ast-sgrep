#![no_main]

use ast_sgrep_core::QueryPlan;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let _ = QueryPlan::parse(input);
});
