//! Throwaway diagnostic: compare per-item vs batched fastembed throughput,
//! and CoreML vs CPU-only execution providers. Not part of the shipped
//! crate surface -- used to decide the batching strategy for index-time
//! embedding. Run with:
//!   cargo run -p ast-sgrep-embed --features neural-embed --example bench_neural --release

use std::time::Instant;

use fastembed::{EmbeddingModel, ExecutionProviderDispatch, InitOptions, TextEmbedding};

fn make_texts(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("fn handle_request_{i}(req: Request) -> Response {{ auth_refresh(req.token); process(req) }}")) .collect()
}

fn bench(label: &str, eps: Vec<ExecutionProviderDispatch>, n: usize, batch_size: Option<usize>) {
    bench_with_threads(label, eps, n, batch_size, None);
}

fn bench_with_threads(
    label: &str, eps: Vec<ExecutionProviderDispatch>, n: usize, batch_size: Option<usize>, intra_threads: Option<usize>,
) {
    let cache_dir = ast_sgrep_embed::neural_default_cache_dir(); let mut options = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        .with_cache_dir(cache_dir) .with_execution_providers(eps) .with_show_download_progress(false);
    if let Some(t) = intra_threads { options = options.with_intra_threads(t); } let t0 = Instant::now(); let mut model = TextEmbedding::try_new(options).expect("model loads"); let load_time = t0.elapsed();

    let texts = make_texts(n); let refs: Vec<&str> = texts.iter().map(String::as_str).collect();

    let t1 = Instant::now(); let _ = model.embed(refs, batch_size).expect("embed succeeds"); let embed_time = t1.elapsed();

    println!(
        "{label}: load={:?} embed({n} items, batch={:?})={:?} ({:.2}ms/item)", load_time, batch_size, embed_time, embed_time.as_secs_f64() * 1000.0 / n as f64
    );
}

/// Simulates the real indexing pattern: many small per-file calls (avg
/// ~6.5 chunks/file over ~166 files for the "self" corpus) instead of one
/// big call, to see whether per-call thread-pool sync overhead dominates.
fn bench_many_small_calls(
    label: &str, intra_threads: Option<usize>, files: usize, per_file: usize,
) {
    let cache_dir = ast_sgrep_embed::neural_default_cache_dir(); let mut options = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        .with_cache_dir(cache_dir) .with_execution_providers(vec![]) .with_show_download_progress(false);
    if let Some(t) = intra_threads { options = options.with_intra_threads(t); } let mut model = TextEmbedding::try_new(options).expect("model loads"); let texts = make_texts(files * per_file);

    let t0 = Instant::now(); for chunk in texts.chunks(per_file) { let _ = model.embed(chunk.to_vec(), None).expect("embed succeeds"); } let elapsed = t0.elapsed(); let n = files * per_file; println!(
        "{label}: {files} calls x {per_file} items = {elapsed:?} ({:.2}ms/item, {:.2}ms/call)", elapsed.as_secs_f64() * 1000.0 / n as f64, elapsed.as_secs_f64() * 1000.0 / files as f64
    );
}

fn main() {
    let n = 200;

    #[cfg(target_os = "macos")] let coreml = vec![ort::ep::CoreML::default().build()]; #[cfg(not(target_os = "macos"))] let coreml: Vec<ExecutionProviderDispatch> = vec![];

    let _ = coreml; bench("cpu-only single-batch(1)", vec![], n, Some(1)); bench("cpu-only batched(4)", vec![], n, Some(4)); bench("cpu-only batched(8)", vec![], n, Some(8));
    bench("cpu-only batched(16)", vec![], n, Some(16)); bench("cpu-only batched(32)", vec![], n, Some(32)); bench("cpu-only batched(64)", vec![], n, Some(64)); bench("cpu-only batched(256)", vec![], n, Some(256));

    println!("--- many-small-calls (per-file pattern, 166 files x 6.5 chunks/file) ---"); bench_many_small_calls("intra_threads=None (default)", None, 166, 7); bench_many_small_calls("intra_threads=1", Some(1), 166, 7);
    bench_many_small_calls("intra_threads=2", Some(2), 166, 7); bench_many_small_calls("intra_threads=4", Some(4), 166, 7);
}
