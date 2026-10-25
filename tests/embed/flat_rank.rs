//! Kernel contract for `top_k_flat_similarity`: the flat top-k ranker must
//! equal a per-chunk `cosine_similarity` reference — same scores, same order
//! (score desc, then index asc), on both sides of the serial/parallel size
//! gate, including zero-norm chunks, duplicate vectors, and limit truncation.
//!
//! This is the invariant that lets execution-shape changes (hoisted query
//! norm, serial fallback, cached chunk norms) land without a quality gate:
//! any deviation is a semantics change, not an optimization.

use ast_sgrep_embed::{cosine_similarity, top_k_flat_similarity};

/// Deterministic xorshift64* — no external RNG dependency.
struct Rng(u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32 * 2.0 - 1.0
    }
}

fn reference(q: &[f32], flat: &[f32], dim: usize, limit: usize) -> Vec<(usize, f32)> {
    let n = flat.len() / dim;
    let mut all: Vec<(usize, f32)> = (0..n)
        .map(|i| (i, cosine_similarity(q, &flat[i * dim..(i + 1) * dim])))
        .filter(|(_, s)| s.is_finite())
        .collect();
    all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then_with(|| a.0.cmp(&b.0)));
    all.truncate(limit);
    all
}

#[test]
fn flat_rank_matches_per_chunk_cosine_reference() {
    // (n, dim, limit) spans both sides of the serial/parallel gate and
    // exercises SIMD (>=64) and scalar (<64) dot paths.
    for (n, dim, limit) in [(5usize, 8usize, 3usize), (300, 256, 50), (1500, 64, 7)] {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (n as u64) ^ (dim as u64) << 32);
        let q: Vec<f32> = (0..dim).map(|_| rng.next_f32()).collect();
        let mut flat: Vec<f32> = Vec::with_capacity(n * dim);
        for i in 0..n {
            match i % 10 {
                // A zero-norm chunk (cosine -> 0.0) and a duplicate of chunk
                // 0 (tie must break by index) keep the contract honest.
                3 => flat.extend(std::iter::repeat(0.0).take(dim)),
                7 if i > 7 => {
                    let head = flat[0..dim].to_vec();
                    flat.extend_from_slice(&head);
                }
                _ => flat.extend((0..dim).map(|_| rng.next_f32())),
            }
        }
        let got = top_k_flat_similarity(&q, &flat, dim, limit, None);
        let want = reference(&q, &flat, dim, limit);
        assert_eq!(got.len(), want.len(), "n={n} dim={dim}");
        for (g, w) in got.iter().zip(want.iter()) {
            assert_eq!(g.0, w.0, "idx n={n} dim={dim}");
            assert_eq!(g.1.to_bits(), w.1.to_bits(), "score bits n={n} dim={dim}");
        }
    }
}

#[test]
fn flat_rank_min_similarity_boundary_is_strictly_above() {
    let dim = 256usize;
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    // Positive-component vectors keep the cosine positive so the ULP
    // boundary arithmetic below moves in one direction.
    let q: Vec<f32> = (0..dim).map(|_| rng.next_f32() * 0.5 + 0.5).collect();
    let chunk: Vec<f32> = (0..dim).map(|_| rng.next_f32() * 0.5 + 0.5).collect();
    let flat = chunk.clone();
    let top = cosine_similarity(&q, &chunk);
    assert!(top > 0.0, "fixture must be positive-correlated, got {top}");
    // Actual exceeds_threshold contract: a score is admitted only when
    // sim > next_up(min). So min = top-2ulp admits top; min = top-1ulp,
    // min = top, and min = top+1ulp all exclude it.
    let two_below = f32::from_bits(top.to_bits() - 2);
    let one_below = f32::from_bits(top.to_bits() - 1);
    let one_above = f32::from_bits(top.to_bits() + 1);
    assert_eq!(
        top_k_flat_similarity(&q, &flat, dim, 5, Some(two_below)),
        vec![(0usize, top)]
    );
    assert!(top_k_flat_similarity(&q, &flat, dim, 5, Some(one_below)).is_empty());
    assert!(top_k_flat_similarity(&q, &flat, dim, 5, Some(top)).is_empty());
    assert!(top_k_flat_similarity(&q, &flat, dim, 5, Some(one_above)).is_empty());
}

#[test]
fn flat_rank_degenerate_inputs_return_empty() {
    let q = vec![0.1_f32; 64];
    let flat = vec![0.1_f32; 64];
    assert!(top_k_flat_similarity(&q, &flat, 64, 0, None).is_empty());
    assert!(top_k_flat_similarity(&q, &flat, 0, 5, None).is_empty());
    assert!(top_k_flat_similarity(&q, &[], 64, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[], &flat, 64, 5, None).is_empty());
}
