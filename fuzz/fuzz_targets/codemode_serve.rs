#![no_main]

//! CodeMode NDJSON / batch request serde fuzzer (wire parse boundary only).
//!
//! Does not open Searcher or execute tools — pure JSON parse oracles.

use ast_sgrep_codemode::{BatchRequest, ServeRequest, MAX_BATCH_CALLS};
use libfuzzer_sys::fuzz_target;

const MAX_LINE: usize = 8 * 1024;

fuzz_target!(|input: &str| {
    if input.len() > MAX_LINE {
        return;
    }

    // ServeRequest (sticky worker lines).
    if let Ok(req) = serde_json::from_str::<ServeRequest>(input) {
        match req {
            ServeRequest::Batch { ref calls, .. } => {
                // Soft invariant: oversized batches are the executor's problem,
                // but parsing must not panic. Document MAX for harness awareness.
                let _ = calls.len() > MAX_BATCH_CALLS;
            }
            ServeRequest::Call { .. } | ServeRequest::End => {}
        }
    }

    // BatchRequest (one-shot batch envelope).
    if let Ok(batch) = serde_json::from_str::<BatchRequest>(input) {
        let _ = batch.calls.len();
    }
});
