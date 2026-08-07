#![no_main]

//! ANN cluster index body fuzzer (length/magic OOB class).
//!
//! - Crash oracle on `read_clusters_bounded` with capped k/dim/chunk_count.
//! - Strength ≥3: build tiny index via `write_to` and re-read (round-trip).

use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use libfuzzer_sys::fuzz_target;

const MAX_PAYLOAD: usize = 16 * 1024;
const MAX_K: usize = 8;
const MAX_DIM: usize = 32;
const MAX_N: usize = 64;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_PAYLOAD {
        return;
    }

    // --- Path A: arbitrary bytes with params from prefix ---
    if data.len() >= 4 {
        let k = (data[0] as usize % MAX_K).max(1);
        let dim = (data[1] as usize % MAX_DIM).max(1);
        let chunk_count = (data[2] as usize % MAX_N).max(1);
        let body = &data[3..];
        let _ = SemanticAnnIndex::read_clusters_bounded(body, k, dim, chunk_count);
    }

    // --- Path B: round-trip oracle on a tiny built index ---
    // Use a few bytes to build 1..=4 vectors of dim 2..=8.
    let n = (data.first().copied().unwrap_or(1) as usize % 4).max(1);
    let dim = (data.get(1).copied().unwrap_or(2) as usize % 8).max(2);
    let mut flat = vec![0.0f32; n * dim];
    for (i, slot) in flat.iter_mut().enumerate() {
        let b = data.get(2 + (i % data.len().max(1))).copied().unwrap_or(0);
        *slot = (b as f32) / 255.0;
    }

    let index = SemanticAnnIndex::build_from_flat(&flat, dim);
    let mut buf = Vec::new();
    if index.write_to(&mut buf, dim).is_err() {
        return;
    }
    if buf.len() < 4 {
        return;
    }
    let k = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if k == 0 || k > MAX_K * 4 {
        // Empty index path is ok.
        return;
    }
    let rt = SemanticAnnIndex::read_clusters_bounded(&buf, k, dim, n);
    assert!(
        rt.is_ok(),
        "write_to → read_clusters_bounded round-trip failed for n={n} dim={dim} k={k}"
    );
});
