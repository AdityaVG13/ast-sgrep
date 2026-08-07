#![no_main]

//! LE f32 embedding codec fuzzer with round-trip oracle (strength 4).
//!
//! Binary OOB/length class: odd lengths must reject without panic.

use ast_sgrep_embed::{embed_from_bytes, embed_to_bytes};
use libfuzzer_sys::fuzz_target;

/// Cap embedding payload (e.g. 256 dims × 4 bytes).
const MAX_BYTES: usize = 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_BYTES {
        return;
    }

    match embed_from_bytes(data) {
        Ok(vec) => {
            // Round-trip: encode → decode must reproduce the floats.
            let encoded = embed_to_bytes(&vec);
            let again = embed_from_bytes(&encoded).expect("round-trip decode");
            assert_eq!(again.len(), vec.len());
            for (a, b) in again.iter().zip(vec.iter()) {
                // Bit-identical for finite values; NaN bits may compare unequal via ==
                // so compare raw bits.
                assert_eq!(a.to_bits(), b.to_bits());
            }
            assert_eq!(encoded, data);
        }
        Err(_) => {
            // Odd length (or future validation) must not panic — Err is success.
            assert!(
                !data.len().is_multiple_of(4),
                "valid length should not error"
            );
        }
    }
});
