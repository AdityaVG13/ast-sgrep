use ast_sgrep_core::semantic_ann::SemanticAnnIndex;
use ast_sgrep_core::semantic_ivf::{
    compute_ann_fingerprint, load_semantic_ivf, save_semantic_ivf_with_publication,
};
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

const COUNT: usize = 10_000;
const DIM: usize = 8;

fn fingerprint() -> [u8; 32] {
    compute_ann_fingerprint(COUNT, COUNT as i64, DIM, Some("open-probe"), 0)
}

fn vectors() -> Vec<f32> {
    (0..COUNT * DIM)
        .map(|index| ((index.wrapping_mul(2_654_435_761) % 10_007) as f32 / 5_003.5) - 1.0)
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().ok_or("expected prepare or open")?;
    let path = arguments.next().ok_or("expected sidecar path")?;
    let path = Path::new(&path);
    match command.as_str() {
        "prepare" => {
            let vectors = vectors();
            let index = SemanticAnnIndex::build_from_flat(&vectors, DIM);
            if !save_semantic_ivf_with_publication(path, fingerprint(), DIM, &vectors, &index)? {
                return Err("sidecar publication was deferred".into());
            }
            println!(
                "prepared={} bytes={}",
                path.display(),
                path.metadata()?.len()
            );
        }
        "open" => {
            // An explicit count measures repeated opens in one process, not
            // fresh-process opens of an already page-cached file.
            let samples = arguments
                .next()
                .map(|value| value.parse::<usize>())
                .transpose()?
                .unwrap_or(1);
            let mut nanos = Vec::with_capacity(samples);
            for _ in 0..samples {
                let started = Instant::now();
                let sidecar = load_semantic_ivf(path, fingerprint())?.ok_or("sidecar rejected")?;
                if !sidecar.is_mapped() {
                    return Err("sidecar vectors are not mapped".into());
                }
                black_box(sidecar.chunk_count());
                nanos.push(started.elapsed().as_nanos());
            }
            if samples == 1 {
                println!("{}", nanos[0]);
            } else {
                println!("{}", serde_json::to_string(&nanos)?);
            }
        }
        _ => return Err("expected prepare or open".into()),
    }
    Ok(())
}
