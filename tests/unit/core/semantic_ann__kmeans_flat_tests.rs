use super::SemanticAnnIndex;

fn synthetic_flat(n: usize, dim: usize) -> Vec<f32> {
    let mut flat = Vec::with_capacity(n * dim);
    for i in 0..n {
        for d in 0..dim {
            flat.push(((i * 17 + d * 3) % 97) as f32 * 0.01 + 0.001);
        }
    }
    flat
}

#[test]
fn build_from_flat_is_deterministic_bit_identical_sidecar() {
    let dim = 8usize;
    let n = 64usize;
    let flat = synthetic_flat(n, dim);
    let a = SemanticAnnIndex::build_from_flat(&flat, dim);
    let b = SemanticAnnIndex::build_from_flat(&flat, dim);
    assert!(a.validate_partition(n));
    assert!(b.validate_partition(n));
    let mut wa = Vec::new();
    let mut wb = Vec::new();
    a.write_to(&mut wa, dim).expect("serialize a");
    b.write_to(&mut wb, dim).expect("serialize b");
    assert_eq!(
        wa, wb,
        "two builds on same input must produce bit-identical IVF payload"
    );
    let q = &flat[..dim];
    assert_eq!(
        a.search_flat(&flat, dim, q, 10),
        b.search_flat(&flat, dim, q, 10)
    );
}

#[test]
fn build_from_flat_empty_and_zero_dim() {
    let empty = SemanticAnnIndex::build_from_flat(&[], 8);
    assert!(empty.candidate_indices(&[1.0; 8], Some(1)).is_empty());
    let zero_dim = SemanticAnnIndex::build_from_flat(&[1.0, 2.0], 0);
    assert!(zero_dim.candidate_indices(&[1.0], Some(1)).is_empty());
}

#[test]
fn search_flat_edge_paths_empty_zero_dim_limit() {
    let dim = 4usize;
    let flat = synthetic_flat(8, dim);
    let empty_idx = SemanticAnnIndex::build_from_flat(&[], dim);
    let q = &flat[..dim];
    // empty corpus (n=0) → no hits
    assert!(empty_idx.search_flat(&[], dim, q, 5).is_empty());
    // zero dim → checked_div path, no panic
    let built = SemanticAnnIndex::build_from_flat(&flat, dim);
    assert!(built.search_flat(&flat, 0, q, 5).is_empty());
    // limit 0 → empty
    assert!(built.search_flat(&flat, dim, q, 0).is_empty());
    // max limit caps to corpus size via top-k
    let hits = built.search_flat(&flat, dim, q, usize::MAX);
    assert!(!hits.is_empty());
    assert!(hits.len() <= 8);
}

#[test]
fn ann_result_is_sufficient_edges() {
    use super::ann_result_is_sufficient;
    // empty / under-filled must not short-circuit flat
    assert!(!ann_result_is_sufficient(0, 100, 50));
    assert!(!ann_result_is_sufficient(10, 100, 50));
    assert!(ann_result_is_sufficient(50, 100, 50));
    // total smaller than limit
    assert!(ann_result_is_sufficient(10, 10, 50));
    // limit 0: vacuously sufficient (product clamps limit ≥ 1)
    assert!(ann_result_is_sufficient(0, 0, 0));
    assert!(ann_result_is_sufficient(0, 5, 0));
}

#[test]
fn kmeans_flat_matches_row_layout_reference() {
    // Reference: same algorithm as pre-T1 `&[Vec<f32>]` k-means, for a small
    // fixed matrix. Asserts flat-slice kmeans produces identical centroids.
    let dim = 4usize;
    let n = 12usize;
    let flat = synthetic_flat(n, dim);
    // Normalize like build_from_flat.
    let mut norm = flat.clone();
    for i in 0..n {
        ast_sgrep_embed::normalize_vec_in_place(&mut norm[i * dim..(i + 1) * dim]);
    }
    let rows: Vec<Vec<f32>> = (0..n)
        .map(|i| norm[i * dim..(i + 1) * dim].to_vec())
        .collect();
    let k = ((n as f64).sqrt() as usize).clamp(16, 256).min(n).max(1);
    let (c_flat, a_flat) = super::kmeans(&norm, dim, k, 12);
    let (c_rows, a_rows) = kmeans_row_reference(&rows, k, 12);
    assert_eq!(a_flat, a_rows);
    assert_eq!(c_flat.len(), c_rows.len());
    for (a, b) in c_flat.iter().zip(c_rows.iter()) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                x.to_bits(),
                y.to_bits(),
                "centroid float bits must match row-layout reference"
            );
        }
    }
}

/// Serial row-layout k-means reference for isomorphism (same metric as
fn kmeans_row_reference(
    vectors: &[Vec<f32>],
    k: usize,
    max_iters: usize,
) -> (Vec<Vec<f32>>, Vec<usize>) {
    use ast_sgrep_embed::{dot_similarity, normalize_vec};
    let k = k.min(vectors.len()).max(1);
    let dim = vectors[0].len();
    let mut centroids = {
        let mut c = vec![vectors[0].clone()];
        while c.len() < k {
            let best = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let nearest_sim = c
                        .iter()
                        .map(|cent| dot_similarity(v, cent))
                        .fold(f32::NEG_INFINITY, f32::max);
                    (i, 1.0 - nearest_sim)
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            c.push(vectors[best].clone());
        }
        c
    };
    let mut assignments = vec![0usize; vectors.len()];
    for _ in 0..max_iters {
        let mut changed = false;
        for (i, v) in vectors.iter().enumerate() {
            let best = centroids
                .iter()
                .enumerate()
                .map(|(ci, c)| (ci, dot_similarity(v, c)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(ci, _)| ci)
                .unwrap_or(0);
            changed |= assignments[i] != best;
            assignments[i] = best;
        }
        if !changed {
            break;
        }
        let mut sums = vec![vec![0.0f32; dim]; k];
        let mut counts = vec![0usize; k];
        for (i, v) in vectors.iter().enumerate() {
            let c = assignments[i];
            counts[c] += 1;
            for (j, val) in v.iter().enumerate() {
                sums[c][j] += val;
            }
        }
        centroids = sums
            .iter()
            .zip(counts.iter())
            .zip(centroids.iter())
            .map(|((sum, &count), prev)| {
                if count == 0 {
                    prev.clone()
                } else {
                    normalize_vec(&sum.iter().map(|v| v / count as f32).collect::<Vec<_>>())
                }
            })
            .collect();
    }
    (centroids, assignments)
}

fn assert_kmeans_matches_serial_ref(flat: &[f32], dim: usize, max_iters: usize) {
    let n = flat.len() / dim;
    let mut norm = flat.to_vec();
    for i in 0..n {
        ast_sgrep_embed::normalize_vec_in_place(&mut norm[i * dim..(i + 1) * dim]);
    }
    let rows: Vec<Vec<f32>> = (0..n)
        .map(|i| norm[i * dim..(i + 1) * dim].to_vec())
        .collect();
    let k = ((n as f64).sqrt() as usize).clamp(16, 256).min(n).max(1);
    let (c_ref, a_ref) = kmeans_row_reference(&rows, k, max_iters);
    let (c_par, a_par) = super::kmeans(&norm, dim, k, max_iters);
    assert_eq!(
        a_par, a_ref,
        "assignments must match serial row-layout reference (n={n} dim={dim} k={k})"
    );
    assert_eq!(c_par.len(), c_ref.len());
    for (ci, (a, b)) in c_par.iter().zip(c_ref.iter()).enumerate() {
        assert_eq!(a.len(), b.len());
        for (j, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(
                x.to_bits(),
                y.to_bits(),
                "centroid[{ci}][{j}] bits must match serial ref (n={n} dim={dim})"
            );
        }
    }
}

#[test]
fn kmeans_parallel_matches_serial_on_synthetics() {
    // Deterministic seeds via synthetic_flat formula; vary n/dim to cover
    // k-clamp paths (k=min(n, clamp(sqrt(n),16,256))).
    for &(n, dim) in &[(12, 4), (32, 8), (64, 16), (100, 8), (256, 4)] {
        let flat = synthetic_flat(n, dim);
        assert_kmeans_matches_serial_ref(&flat, dim, 12);
    }
    // Fixed alternate pattern (still deterministic).
    let dim = 6usize;
    let n = 48usize;
    let mut flat = Vec::with_capacity(n * dim);
    for i in 0..n {
        for d in 0..dim {
            flat.push(((i * 31 + d * 7) % 53) as f32 * 0.02 - 0.1);
        }
    }
    assert_kmeans_matches_serial_ref(&flat, dim, 12);
}

#[test]
fn kmeans_bit_identical_under_1_and_4_rayon_threads() {
    // Local pools via install so thread count is controlled even if the
    // global Rayon pool was already initialized by other tests.
    let dim = 8usize;
    let n = 128usize;
    let flat = synthetic_flat(n, dim);
    let mut norm = flat.clone();
    for i in 0..n {
        ast_sgrep_embed::normalize_vec_in_place(&mut norm[i * dim..(i + 1) * dim]);
    }
    let rows: Vec<Vec<f32>> = (0..n)
        .map(|i| norm[i * dim..(i + 1) * dim].to_vec())
        .collect();
    let k = ((n as f64).sqrt() as usize).clamp(16, 256).min(n).max(1);
    let (c_ref, a_ref) = kmeans_row_reference(&rows, k, 12);

    for threads in [1usize, 4usize] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("build rayon pool");
        let (c_par, a_par) = pool.install(|| super::kmeans(&norm, dim, k, 12));
        assert_eq!(
            a_par, a_ref,
            "assignments must match serial ref at RAYON threads={threads}"
        );
        for (a, b) in c_par.iter().zip(c_ref.iter()) {
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(
                    x.to_bits(),
                    y.to_bits(),
                    "centroid bits must match at threads={threads}"
                );
            }
        }
    }
}
