#![no_main]

//! LSP `Content-Length` framing fuzzer (framing DoS / UTF-8 body class).
//!
//! Harness size budget ≪ product `MAX_MESSAGE_BYTES` (8 MiB): cap input at 64 KiB.

use ast_sgrep_lsp::transport::read_message;
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

/// PASS5 harness budget — never feed product 8 MiB into the fuzzer.
const MAX_INPUT: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }

    let mut cursor = Cursor::new(data);
    match read_message(&mut cursor) {
        Ok(Some(body)) => {
            // Valid framed message must be UTF-8 (read_message returns String).
            assert!(std::str::from_utf8(body.as_bytes()).is_ok());
        }
        Ok(None) => {
            // EOF / incomplete — fine.
        }
        Err(_) => {
            // Malformed framing / oversize Content-Length — fine, no panic.
        }
    }
});
