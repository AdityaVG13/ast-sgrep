//! Phase-level profile of the static embedder hot path (ledger lever E1).
//! Run: `cargo run --profile release-perf -p ast-sgrep-embed --example embed_phase_profile`
//! Prints per-text us for tokenization (normalize+wordpiece) vs pooling
//! (f16 decode+accumulate) over the S2 tripwire sample shape, plus a
//! repeated-word corpus that mimics real source batches.

use ast_sgrep_embed::StaticEmbedder;
use std::time::Instant;

fn bench(label: &str, texts: &[String]) {
    let e = StaticEmbedder::new();
    let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let _ = e.embed_batch(&refs); // warm

    let n = 6;
    let t0 = Instant::now();
    for _ in 0..n {
        for t in &refs {
            std::hint::black_box(e.token_ids(t));
        }
    }
    let tok_us = t0.elapsed().as_micros() as f64 / (n as f64 * texts.len() as f64);

    let t1 = Instant::now();
    for _ in 0..n {
        for t in &refs {
            std::hint::black_box(e.embed(t));
        }
    }
    let full_us = t1.elapsed().as_micros() as f64 / (n as f64 * texts.len() as f64);

    println!(
        "{label:<22} token_ids {tok_us:8.1} us/text | embed {full_us:8.1} us/text | pooling {:+8.1}",
        full_us - tok_us
    );
}

fn main() {
    let sample = "symbol: embed_parallel kind: fn scope: store excerpt: pub(super) fn \
embed_parallel(conn: &Connection, chunks: &[SemanticChunkInput], source_path: &str) \
-> Result<Vec<EmbeddedChunk>> { let mid = active_model_id(); let mut jobs = Vec::new(); }";

    let varied: Vec<String> = (0..64)
        .map(|i| format!("{sample} variant {i} tail tokens to vary the hash input"))
        .collect();
    bench("tripwire-sample", &varied);

    // Realistic index batch: heavy word repetition across chunks.
    let repeated: Vec<String> = (0..256)
        .map(|i| {
            format!(
                "pub fn handler_{i}(conn: &Connection, path: &str) -> Result<Vec<Hit>> {{ \
                 let store = open(path)?; store.query(conn) }}"
            )
        })
        .collect();
    bench("source-like-batch", &repeated);

    // One long identifier-heavy line: worst case for O(L^2) wordpiece.
    let worst = vec!["fn compress_index_store_migration_writer_generation(&mut self, conn: &Connection) -> Result<()>".to_string(); 64];
    bench("long-identifiers", &worst);
}
